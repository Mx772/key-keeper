//! Turning raw input (CSV/TSV file, clipboard paste, free text) into a [`Table`] with a
//! suggested column → [`Field`] mapping.

use std::sync::LazyLock;

use regex::Regex;

use super::Field;
use super::values::{clean, is_url};
use crate::links;

/// Parsed input, before field mapping is applied.
#[derive(Clone, Debug, PartialEq)]
pub struct Table {
    /// Column titles: the header row, or synthetic `Column N` / `Game Name`, `Key` names.
    pub headers: Vec<String>,
    /// Whether `headers` came from the input (false for synthetic headers).
    pub has_header: bool,
    pub rows: Vec<Vec<String>>,
    /// 1-based source line for each row, for error messages.
    pub lines: Vec<usize>,
    pub kind: TableKind,
    /// Suggested mapping, one entry per column. `None` = ignore the column.
    pub mapping: Vec<Option<Field>>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum TableKind {
    Delimited(u8),
    /// One key per line, the game name around it.
    Text,
}

const DELIMITERS: [u8; 4] = *b",\t;|";

/// Parse anything: a CSV/TSV/semicolon/pipe table if one is detected, otherwise lines of text.
pub fn parse(input: &str) -> Table {
    let input = input.strip_prefix('\u{feff}').unwrap_or(input);
    match detect_delimiter(input) {
        Some(d) => parse_delimited(input, d),
        None => parse_text(input),
    }
}

fn records(input: &str, delimiter: u8) -> Vec<(usize, Vec<String>)> {
    let mut rdr = csv::ReaderBuilder::new()
        .delimiter(delimiter)
        .has_headers(false)
        .flexible(true)
        .from_reader(input.as_bytes());
    rdr.records()
        .filter_map(Result::ok)
        .map(|r| {
            let line = r.position().map_or(0, |p| p.line() as usize);
            (line, r.iter().map(str::to_string).collect())
        })
        .collect()
}

/// A delimiter is plausible when ≥ 80% of the first 50 non-blank records have the same field
/// count, and that count is ≥ 2. Among plausible ones, the most consistent then widest wins.
pub fn detect_delimiter(input: &str) -> Option<u8> {
    let sample: String = input
        .lines()
        .filter(|l| !l.trim().is_empty())
        .take(50)
        .collect::<Vec<_>>()
        .join("\n");
    let mut best: Option<(u8, f32, usize)> = None;
    for d in DELIMITERS {
        let counts: Vec<usize> = records(&sample, d).iter().map(|(_, r)| r.len()).collect();
        if counts.is_empty() {
            continue;
        }
        let mut freq = std::collections::HashMap::new();
        for c in &counts {
            *freq.entry(*c).or_insert(0usize) += 1;
        }
        let Some((&mode, &n)) = freq.iter().max_by_key(|(c, n)| (**n, **c)) else {
            continue;
        };
        let share = n as f32 / counts.len() as f32;
        if mode >= 2 && share >= 0.8 && best.is_none_or(|(_, s, m)| (share, mode) > (s, m)) {
            best = Some((d, share, mode));
        }
    }
    // A comma inside free text ("Haiku, the Robot - KEY") must not make it a 2-column CSV:
    // if every line has exactly one key-looking token and fields don't separate name from key,
    // text mode is better. We check: does some column hold mostly keys?
    let (d, _, _) = best?;
    let rows = records(&sample, d);
    let width = rows.iter().map(|(_, r)| r.len()).max().unwrap_or(0);
    let body = &rows[usize::from(rows.len() > 1)..];
    let key_col = (0..width).any(|c| share_of(body, c, is_key) >= 0.5);
    let header_hit = rows
        .first()
        .is_some_and(|(_, r)| r.iter().any(|h| Field::from_header(h).is_some()));
    (key_col || header_hit).then_some(d)
}

fn share_of(rows: &[(usize, Vec<String>)], col: usize, pred: impl Fn(&str) -> bool) -> f32 {
    let cells: Vec<&str> = rows
        .iter()
        .filter_map(|(_, r)| r.get(col))
        .map(|s| clean(s))
        .filter(|s| !s.is_empty())
        .collect();
    if cells.is_empty() {
        return 0.0;
    }
    cells.iter().filter(|s| pred(s)).count() as f32 / cells.len() as f32
}

pub fn parse_delimited(input: &str, delimiter: u8) -> Table {
    let mut recs = records(input.strip_prefix('\u{feff}').unwrap_or(input), delimiter);
    recs.retain(|(_, r)| r.iter().any(|c| !c.trim().is_empty()));
    let width = recs.iter().map(|(_, r)| r.len()).max().unwrap_or(0);
    let has_header = recs.first().is_some_and(|(_, r)| looks_like_header(r));
    let (headers, body) = if has_header {
        let (_, h) = recs.remove(0);
        let mut h: Vec<String> = h.into_iter().map(|s| s.trim().to_string()).collect();
        h.resize(width, String::new());
        (h, recs)
    } else {
        ((1..=width).map(|i| format!("Column {i}")).collect(), recs)
    };
    let mut rows: Vec<Vec<String>> = Vec::with_capacity(body.len());
    let mut lines = Vec::with_capacity(body.len());
    for (line, mut r) in body {
        r.resize(width, String::new());
        rows.push(r);
        lines.push(line);
    }
    let mapping = if has_header {
        map_headers(&headers)
    } else {
        map_by_content(&rows, width)
    };
    Table {
        headers,
        has_header,
        rows,
        lines,
        kind: TableKind::Delimited(delimiter),
        mapping,
    }
}

fn looks_like_header(row: &[String]) -> bool {
    row.iter().any(|c| Field::from_header(c).is_some()) && !row.iter().any(|c| is_key(c))
}

/// Map header titles through the alias table; a field is only used once (first column wins).
pub fn map_headers(headers: &[String]) -> Vec<Option<Field>> {
    let mut used = Vec::new();
    headers
        .iter()
        .map(|h| {
            let f = Field::from_header(h).filter(|f| !used.contains(f))?;
            used.push(f);
            Some(f)
        })
        .collect()
}

/// No header: find the key column, trade-link and store-link columns, and take the first
/// remaining text column as the game name.
fn map_by_content(rows: &[Vec<String>], width: usize) -> Vec<Option<Field>> {
    let recs: Vec<(usize, Vec<String>)> = rows.iter().cloned().map(|r| (0, r)).collect();
    let mut mapping = vec![None; width];
    let key_col = (0..width)
        .map(|c| (c, share_of(&recs, c, is_key)))
        .filter(|(_, s)| *s >= 0.6)
        .max_by(|a, b| a.1.total_cmp(&b.1))
        .map(|(c, _)| c);
    if let Some(c) = key_col {
        mapping[c] = Some(Field::Key);
    }
    for c in 0..width {
        if mapping[c].is_some() {
            continue;
        }
        if share_of(&recs, c, is_url) >= 0.6 {
            let trade = share_of(&recs, c, |s| {
                links::host(s).is_some_and(|h| h.contains("lestrades") || h.contains("steamtrades"))
            }) >= 0.5;
            let field = if trade {
                Field::TradeUrl
            } else {
                Field::KeyUrl
            };
            if !mapping.contains(&Some(field)) {
                mapping[c] = Some(field);
            }
        }
    }
    if let Some(c) = (0..width).find(|&c| {
        mapping[c].is_none() && share_of(&recs, c, |s| s.chars().any(char::is_alphabetic)) >= 0.6
    }) {
        mapping[c] = Some(Field::GameName);
    }
    mapping
}

// ── Free text ─────────────────────────────────────────────────────────────────────────────────

/// Dash-grouped keys: `XXXXX-XXXXX-XXXXX`, `XXXX-XXXX-XXXX-XXXX`, 5×5 Steam, etc. (uppercase).
static DASHED: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"\b(?:[A-Z0-9]{4,6}-){2,4}[A-Z0-9]{4,6}\b").unwrap_or_else(|_| unreachable!())
});
/// Undashed keys (e.g. GOG): 15–25 uppercase alphanumerics with both letters and digits.
static PLAIN: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"\b[A-Z0-9]{15,25}\b").unwrap_or_else(|_| unreachable!()));

fn plausible(s: &str) -> bool {
    let digits = s.chars().filter(char::is_ascii_digit).count();
    let letters = s.chars().filter(char::is_ascii_alphabetic).count();
    // Steam's 5-5-5 shape is accepted as is; anything else needs a digit (so "HALF-LIFE-WORLD" isn't a key).
    let steam = s.len() == 17 && s.split('-').all(|p| p.len() == 5);
    (steam || digits > 0) && (s.contains('-') || (letters > 0 && digits > 0))
}

/// Finds the first key-looking token in `s` and returns its byte range.
pub fn find_key(s: &str) -> Option<(usize, usize)> {
    DASHED
        .find_iter(s)
        .chain(PLAIN.find_iter(s))
        .filter(|m| plausible(m.as_str()))
        .map(|m| (m.start(), m.end()))
        .min_by_key(|r| r.0)
}

/// Whether the whole cell is a key (ignoring case and surrounding spaces; inner spaces allowed).
pub fn is_key(s: &str) -> bool {
    let t = clean(s).to_ascii_uppercase();
    let t = t.split_whitespace().collect::<Vec<_>>().join("-");
    find_key(&t).is_some_and(|(a, b)| a == 0 && b == t.len())
}

const SEPARATORS: &[char] = &[':', '-', '–', '—', '|', '\t', ',', '=', ' ', ';'];

/// Split one line into `(game name, key)`. No key found → `(line, "")`.
pub fn split_line(line: &str) -> (String, String) {
    let Some((a, b)) = find_key(line) else {
        return (line.trim().to_string(), String::new());
    };
    let before = line[..a].trim_matches(SEPARATORS);
    let after = line[b..].trim_matches(SEPARATORS);
    let name = match (before.is_empty(), after.is_empty()) {
        (false, true) => before.to_string(),
        (true, false) => after.to_string(),
        (false, false) => format!("{before} {after}"),
        (true, true) => String::new(),
    };
    (name.trim().to_string(), line[a..b].to_string())
}

pub fn parse_text(input: &str) -> Table {
    let mut rows = Vec::new();
    let mut lines = Vec::new();
    for (i, line) in input.lines().enumerate() {
        if line.trim().is_empty() {
            continue;
        }
        let (name, key) = split_line(line);
        rows.push(vec![name, key]);
        lines.push(i + 1);
    }
    Table {
        headers: vec!["Game Name".into(), "Key".into()],
        has_header: false,
        rows,
        lines,
        kind: TableKind::Text,
        mapping: vec![Some(Field::GameName), Some(Field::Key)],
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn key_shapes() {
        for k in [
            "UNDR7-TALE0-00001",
            "AAAAA-BBBBB-CCCCC",
            "HL2SR-C3000-ABCDE-FGH12-IJK34",
            "ABCD-1234-EFGH-5678",
            "AB12CD34EF56GH78IJ",
        ] {
            assert!(is_key(k), "{k}");
        }
        assert!(is_key(" ab12c de34f gh56i "), "lowercase with spaces");
        for k in [
            "Undertale",
            "HALF-LIFE-WORLD",
            "12345",
            "https://lestrades.com/offer/1/",
            "Haiku, the Robot",
        ] {
            assert!(!is_key(k), "{k}");
        }
    }

    #[test]
    fn split_lines() {
        let cases = [
            (
                "Undertale: UNDR7-TALE0-00001",
                "Undertale",
                "UNDR7-TALE0-00001",
            ),
            (
                "Haiku, the Robot - HA1KU-R0B0T-00002",
                "Haiku, the Robot",
                "HA1KU-R0B0T-00002",
            ),
            ("QQ7RT-PP2MN-ZZ9XY Celeste", "Celeste", "QQ7RT-PP2MN-ZZ9XY"),
            ("Hades\tHD5S2-77KLM-NB3VC", "Hades", "HD5S2-77KLM-NB3VC"),
            (
                "Half-Life: Source - HL2SR-C3000-ABCDE-FGH12-IJK34",
                "Half-Life: Source",
                "HL2SR-C3000-ABCDE-FGH12-IJK34",
            ),
            ("no key here", "no key here", ""),
        ];
        for (line, name, key) in cases {
            assert_eq!(
                split_line(line),
                (name.to_string(), key.to_string()),
                "{line}"
            );
        }
    }

    #[test]
    fn free_text_is_not_mistaken_for_csv() {
        let t =
            parse("Haiku, the Robot - HA1KU-R0B0T-00002\nCeleste, Deluxe - QQ7RT-PP2MN-ZZ9XY\n");
        assert_eq!(t.kind, TableKind::Text);
        assert_eq!(t.rows[0], ["Haiku, the Robot", "HA1KU-R0B0T-00002"]);
    }

    #[test]
    fn headerless_csv_is_mapped_by_content() {
        let t = parse(
            "Undertale,UNDR7-TALE0-00001,https://lestrades.com/offer/1/\nCeleste,QQ7RT-PP2MN-ZZ9XY,https://lestrades.com/offer/2/\n",
        );
        assert_eq!(t.kind, TableKind::Delimited(b','));
        assert!(!t.has_header);
        assert_eq!(
            t.mapping,
            [
                Some(Field::GameName),
                Some(Field::Key),
                Some(Field::TradeUrl)
            ]
        );
        assert_eq!(t.rows.len(), 2);
    }

    #[test]
    fn bom_and_blank_rows() {
        let t = parse("\u{feff}Name,Key\n\n,\nCeleste,QQ7RT-PP2MN-ZZ9XY\n");
        assert!(t.has_header);
        assert_eq!(t.headers, ["Name", "Key"]);
        assert_eq!(t.rows.len(), 1);
        assert_eq!(t.lines, [4]);
    }

    #[test]
    fn duplicate_header_fields_map_once() {
        let m = map_headers(&["Name".into(), "Title".into(), "Key".into()]);
        assert_eq!(m, [Some(Field::GameName), None, Some(Field::Key)]);
    }
}
