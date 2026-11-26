//! A tiny `{variable}` template engine for checkout lines and reputation posts.
//! `{{` and `}}` are literal braces. Unknown variables are errors so typos surface in the editor.

use serde::{Deserialize, Serialize};

/// Variables a checkout line can use (per key), plus every trade variable.
pub const KEY_VARS: &[&str] = &["name", "key", "bundle", "platform", "steam_url", "notes"];
/// Variables describing the trade.
pub const TRADE_VARS: &[&str] = &[
    "partner",
    "partner_url",
    "trade_url",
    "site",
    "date",
    "count",
    "games",
    "received",
];

/// Variables for the "have list": game details only, never the key.
pub const HAVE_VARS: &[&str] = &[
    "name",
    "count",
    "steam_url",
    "bundle",
    "platform",
    "rating",
    "price",
];

pub const DEFAULT_CHECKOUT_LINE: &str = "{name}: {key}";
pub const DEFAULT_HAVE_LINE: &str = "{name}{count}";
pub const DEFAULT_REP: &str = "+Rep: An amazing & quick trader, recommended! We successfully completed this trade: {trade_url}\nThanks a lot for the trade!";

/// The user's templates, stored in the vault.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct Templates {
    /// One line per key at checkout, e.g. `{name}: {key}`.
    pub checkout_line: String,
    /// The steamtrades reputation post.
    pub rep: String,
    /// One line per game in the "have list" copied for trading sites (no keys).
    pub have_line: String,
}

impl Default for Templates {
    fn default() -> Self {
        Templates {
            checkout_line: DEFAULT_CHECKOUT_LINE.into(),
            rep: DEFAULT_REP.into(),
            have_line: DEFAULT_HAVE_LINE.into(),
        }
    }
}

/// A "have list" for trading sites: one line per game among `keys` that are still tradable
/// (Available / Unknown), copies of a game merged (`{count}` is " x2" for two copies, empty
/// for one). Keys never appear: the template only gets [`HAVE_VARS`]. Lines come in the order
/// the games first appear in `keys`.
pub fn have_list(
    repo: &crate::repo::Repo,
    keys: &[uuid::Uuid],
    line: &str,
) -> Result<(String, usize), TemplateError> {
    let mut games: Vec<(String, Vec<&crate::model::GameKey>)> = Vec::new();
    for k in keys.iter().filter_map(|id| repo.key(*id)) {
        if !k.status.can_trade() {
            continue;
        }
        let ident = crate::query::game_identity(k);
        match games.iter_mut().find(|(g, _)| *g == ident) {
            Some((_, v)) => v.push(k),
            None => games.push((ident, vec![k])),
        }
    }
    let mut out = Vec::with_capacity(games.len());
    for (_, copies) in &games {
        let k = copies[0];
        let n = copies.len();
        let rendered = render(line, |v| {
            Some(match v {
                "name" => k.game_name.clone(),
                "count" if n > 1 => format!(" x{n}"),
                "count" => String::new(),
                "steam_url" => k.steam_url().unwrap_or_default(),
                "bundle" => k.bundle.clone().unwrap_or_default(),
                "platform" => k.platform.label().to_string(),
                "rating" => k
                    .steam
                    .as_ref()
                    .and_then(|m| m.rating.as_ref())
                    .map(|r| format!("{}%", r.percent))
                    .unwrap_or_default(),
                "price" => repo
                    .key_prices(k)
                    .and_then(|p| p.best().map(|(m, _)| m.format()))
                    .unwrap_or_default(),
                _ => return None,
            })
        })?;
        out.push(rendered);
    }
    Ok((out.join("\n"), games.len()))
}

#[derive(Clone, Debug, PartialEq, Eq)]
enum Piece<'a> {
    Text(&'a str),
    Var(&'a str),
}

#[derive(Clone, Debug, PartialEq, Eq, thiserror::Error)]
pub enum TemplateError {
    #[error("unknown variable {{{0}}}")]
    Unknown(String),
    #[error("'{{' without a matching '}}' (write {{{{ for a literal brace)")]
    Unclosed,
    #[error("'}}' without a matching '{{' (write }}}} for a literal brace)")]
    StrayClose,
}

fn parse(t: &str) -> Result<Vec<Piece<'_>>, TemplateError> {
    let mut out = Vec::new();
    let mut rest = t;
    while !rest.is_empty() {
        let next = rest.find(['{', '}']);
        let Some(i) = next else {
            out.push(Piece::Text(rest));
            break;
        };
        if i > 0 {
            out.push(Piece::Text(&rest[..i]));
        }
        let tail = &rest[i..];
        if let Some(r) = tail.strip_prefix("{{") {
            out.push(Piece::Text("{"));
            rest = r;
        } else if let Some(r) = tail.strip_prefix("}}") {
            out.push(Piece::Text("}"));
            rest = r;
        } else if tail.starts_with('}') {
            return Err(TemplateError::StrayClose);
        } else {
            let end = tail.find('}').ok_or(TemplateError::Unclosed)?;
            let name = tail[1..end].trim();
            if name.is_empty() || name.contains('{') {
                return Err(TemplateError::Unclosed);
            }
            out.push(Piece::Var(name));
            rest = &tail[end + 1..];
        }
    }
    Ok(out)
}

/// Every problem in `template` given the allowed variable names.
pub fn check(template: &str, allowed: &[&[&str]]) -> Vec<TemplateError> {
    match parse(template) {
        Err(e) => vec![e],
        Ok(pieces) => {
            let mut errs: Vec<TemplateError> = Vec::new();
            for p in pieces {
                if let Piece::Var(v) = p
                    && !allowed.iter().any(|set| set.contains(&v))
                {
                    let e = TemplateError::Unknown(v.to_string());
                    if !errs.contains(&e) {
                        errs.push(e);
                    }
                }
            }
            errs
        }
    }
}

/// Render with `lookup` providing values (`None` = unknown variable).
pub fn render(
    template: &str,
    lookup: impl Fn(&str) -> Option<String>,
) -> Result<String, TemplateError> {
    let mut out = String::with_capacity(template.len() + 32);
    for p in parse(template)? {
        match p {
            Piece::Text(t) => out.push_str(t),
            Piece::Var(v) => {
                out.push_str(&lookup(v).ok_or_else(|| TemplateError::Unknown(v.to_string()))?)
            }
        }
    }
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn vars(name: &str) -> Option<String> {
        match name {
            "name" => Some("Undertale".into()),
            "key" => Some("UNDR7-TALE0-00001".into()),
            "trade_url" => Some("https://lestrades.com/offer/100002/".into()),
            "empty" => Some(String::new()),
            _ => None,
        }
    }

    #[test]
    fn renders_defaults() {
        assert_eq!(
            render(DEFAULT_CHECKOUT_LINE, vars).unwrap(),
            "Undertale: UNDR7-TALE0-00001"
        );
        assert_eq!(
            render(DEFAULT_REP, vars).unwrap(),
            "+Rep: An amazing & quick trader, recommended! We successfully completed this trade: https://lestrades.com/offer/100002/\nThanks a lot for the trade!"
        );
    }

    #[test]
    fn escapes_and_spacing() {
        assert_eq!(
            render("{{literal}} {name} { key }", vars).unwrap(),
            "{literal} Undertale UNDR7-TALE0-00001"
        );
        assert_eq!(render("[{empty}]", vars).unwrap(), "[]");
        assert_eq!(render("", vars).unwrap(), "");
        assert_eq!(render("no vars", vars).unwrap(), "no vars");
    }

    #[test]
    fn errors() {
        assert_eq!(
            render("{nope}", vars),
            Err(TemplateError::Unknown("nope".into()))
        );
        assert_eq!(render("{name", vars), Err(TemplateError::Unclosed));
        assert_eq!(render("{}", vars), Err(TemplateError::Unclosed));
        assert_eq!(render("a } b", vars), Err(TemplateError::StrayClose));
        assert_eq!(render("{a{b}", vars), Err(TemplateError::Unclosed));
    }

    #[test]
    fn check_lists_each_unknown_once() {
        assert!(check(DEFAULT_CHECKOUT_LINE, &[KEY_VARS, TRADE_VARS]).is_empty());
        assert!(check(DEFAULT_REP, &[TRADE_VARS]).is_empty());
        assert_eq!(
            check("{name} {foo} {foo} {bar}", &[KEY_VARS]),
            vec![
                TemplateError::Unknown("foo".into()),
                TemplateError::Unknown("bar".into())
            ]
        );
        assert_eq!(
            check("{key}", &[TRADE_VARS]),
            vec![TemplateError::Unknown("key".into())],
            "rep posts can't leak keys"
        );
    }

    #[test]
    fn unicode_text_is_preserved() {
        assert_eq!(
            render("✓ {name} — merci", vars).unwrap(),
            "✓ Undertale — merci"
        );
    }

    #[test]
    fn have_list_merges_copies_and_never_shows_keys() {
        use crate::model::{GameKey, KeyStatus};
        let mut r = crate::repo::Repo::new(crate::data::VaultData::default());
        let mut a = GameKey::new("Tunic", "TUN1C-F0X99-AAAA1");
        a.steam_app_id = Some(553420);
        let mut b = a.clone();
        b.id = uuid::Uuid::now_v7();
        b.key = crate::secret::Secret::new("TUN1C-F0X99-AAAA2");
        let c = GameKey::new("Celeste", "QQ7RT-PP2MN-ZZ9XY");
        let mut d = GameKey::new("Undertale", "UNDR7-TALE0-00001");
        d.status = KeyStatus::Traded;
        let ids = r.add_keys(vec![a, b, c, d], "seed");
        let (text, n) = have_list(&r, &ids, DEFAULT_HAVE_LINE).unwrap();
        assert_eq!(
            text,
            "Tunic x2
Celeste"
        );
        assert_eq!(n, 2);
        let (text, _) = have_list(&r, &ids, "{name} {steam_url} [{platform}]").unwrap();
        assert!(text.starts_with("Tunic https://store.steampowered.com/app/553420"));
        assert!(text.contains("Celeste  [Steam]"));
        assert!(!text.contains("TUN1C"));
        assert!(
            check("{key}", &[HAVE_VARS]).len() == 1,
            "keys are not a have-list variable"
        );
        assert!(check(DEFAULT_HAVE_LINE, &[HAVE_VARS]).is_empty());
        assert!(have_list(&r, &ids, "{nope}").is_err());
    }
}
