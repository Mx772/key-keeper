//! Search, filter and sort over the key library.

use std::cmp::Ordering;

use jiff::Span;
use jiff::civil::Date;
use uuid::Uuid;

use crate::model::{GameKey, KeyStatus, Platform};
use crate::repo::Repo;
use crate::secret::normalize_key;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub enum SortField {
    #[default]
    Added,
    Name,
    Bundle,
    Source,
    Status,
    Platform,
    Updated,
    StatusDate,
    Deadline,
    Rating,
    /// Cheapest current store price.
    Price,
    /// Steam's own current price.
    SteamPrice,
    /// Cheapest grey-market keyshop price (gg.deals).
    KeyshopPrice,
    /// Steam ownership: in library, then wishlisted, then not owned; unchecked last.
    Ownership,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub enum SortDir {
    #[default]
    Asc,
    Desc,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Sort {
    pub field: SortField,
    pub dir: SortDir,
}

impl Default for Sort {
    /// Newest first.
    fn default() -> Self {
        Sort {
            field: SortField::Added,
            dir: SortDir::Desc,
        }
    }
}

#[derive(Clone, Debug, Default, PartialEq)]
pub struct Query {
    /// Whitespace-separated terms; every term must match. Matches name, bundle, source, notes,
    /// tags, and (for terms of 5+ key characters) the key itself.
    pub text: String,
    /// Empty means all statuses.
    pub statuses: Vec<KeyStatus>,
    /// Empty means all platforms.
    pub platforms: Vec<Platform>,
    /// Only keys still held whose redemption deadline is within this many days (or already past).
    pub deadline_within_days: Option<i64>,
    /// Only keys without a Steam app id (to batch-match).
    pub missing_app_id: bool,
    /// Only keys still in hand for games already owned on Steam: spares to trade away.
    pub trade_fodder: bool,
    /// Only games with more than one key (any status).
    pub duplicates: bool,
    pub sort: Sort,
}

/// The grouping form of a free-text label (bundle, source): lowercase, spaces collapsed, so
/// "Humble", "humble" and " Humble  " group together.
pub fn label_key(s: &str) -> String {
    s.split_whitespace()
        .collect::<Vec<_>>()
        .join(" ")
        .to_lowercase()
}

/// Which free-text label of a key: its bundle or its source.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum LabelField {
    Bundle,
    Source,
}

impl LabelField {
    pub fn of(self, k: &GameKey) -> Option<&str> {
        match self {
            LabelField::Bundle => k.bundle.as_deref(),
            LabelField::Source => k.source.as_deref(),
        }
        .filter(|s| !s.trim().is_empty())
    }
}

/// Every distinct value of `field` in `keys` (ignoring case and spacing), each spelled the way
/// most keys spell it, with how many keys use it. Most used first.
pub fn label_values(keys: &[GameKey], field: LabelField) -> Vec<(String, usize)> {
    let mut by_key: std::collections::HashMap<String, std::collections::HashMap<&str, usize>> =
        Default::default();
    for k in keys {
        if let Some(v) = field.of(k) {
            *by_key
                .entry(label_key(v))
                .or_default()
                .entry(v.trim())
                .or_default() += 1;
        }
    }
    let mut out: Vec<(String, usize)> = by_key
        .into_values()
        .map(|spellings| {
            let total = spellings.values().sum();
            let best = spellings
                .into_iter()
                .max_by(|a, b| a.1.cmp(&b.1).then_with(|| b.0.cmp(a.0)))
                .map(|(s, _)| s.to_string())
                .unwrap_or_default();
            (best, total)
        })
        .collect();
    out.sort_by(|a, b| b.1.cmp(&a.1).then_with(|| natural_cmp(&a.0, &b.0)));
    out
}

/// Up to `limit` existing values of `field` for what's typed: values starting with it first,
/// then ones containing it; nothing once it already names a value exactly.
pub fn label_suggestions(
    keys: &[GameKey],
    field: LabelField,
    typed: &str,
    limit: usize,
) -> Vec<String> {
    let t = label_key(typed);
    if t.is_empty() {
        return Vec::new();
    }
    let values = label_values(keys, field);
    if values.iter().any(|(v, _)| v.trim() == typed.trim()) {
        return Vec::new();
    }
    let mut starts = Vec::new();
    let mut contains = Vec::new();
    for (v, _) in values {
        let k = label_key(&v);
        if k.starts_with(&t) {
            starts.push(v);
        } else if k.contains(&t) {
            contains.push(v);
        }
    }
    starts.extend(contains);
    starts.truncate(limit);
    starts
}

/// `value` spelled the way the library already spells it, when it differs only in case or
/// spacing ("humble" becomes "Humble" when that's what the other keys say).
pub fn canonical_label(keys: &[GameKey], field: LabelField, value: &str) -> String {
    let k = label_key(value);
    label_values(keys, field)
        .into_iter()
        .find(|(v, _)| label_key(v) == k)
        .map_or_else(|| value.trim().to_string(), |(v, _)| v)
}

/// What makes two keys "the same game": the Steam app id, else the name in lowercase with only
/// letters and digits ("Hollow Knight" and "hollow-knight" match).
pub fn game_identity(k: &GameKey) -> String {
    match k.steam_app_id {
        Some(a) => format!("app:{a}"),
        None => format!(
            "name:{}",
            k.game_name
                .chars()
                .filter(|c| c.is_alphanumeric())
                .flat_map(char::to_lowercase)
                .collect::<String>()
        ),
    }
}

impl Query {
    pub fn with_text(text: impl Into<String>) -> Query {
        Query {
            text: text.into(),
            ..Query::default()
        }
    }
}

impl Repo {
    /// Ids of matching keys in display order.
    pub fn query(&self, q: &Query, today: Date) -> Vec<Uuid> {
        let terms: Vec<(String, String)> = q
            .text
            .split_whitespace()
            .map(|t| (t.to_lowercase(), normalize_key(t)))
            .collect();
        let limit = q
            .deadline_within_days
            .and_then(|d| today.checked_add(Span::new().days(d)).ok());
        let mut copies: std::collections::HashMap<String, usize> = std::collections::HashMap::new();
        if q.duplicates {
            for k in self.keys() {
                *copies.entry(game_identity(k)).or_default() += 1;
            }
        }
        let mut hits: Vec<&GameKey> = self
            .keys()
            .iter()
            .filter(|k| q.statuses.is_empty() || q.statuses.contains(&k.status))
            .filter(|k| q.platforms.is_empty() || q.platforms.contains(&k.platform))
            .filter(|k| !q.missing_app_id || k.steam_app_id.is_none())
            .filter(|k| {
                !q.trade_fodder
                    || (k.status.can_trade()
                        && k.steam
                            .as_ref()
                            .is_some_and(|m| m.ownership == crate::model::Ownership::Owned))
            })
            .filter(|k| !q.duplicates || copies.get(&game_identity(k)).copied().unwrap_or(0) > 1)
            .filter(|k| match (q.deadline_within_days, limit) {
                (None, _) => true,
                (Some(_), Some(limit)) => {
                    !k.status.is_gone() && k.redemption_deadline.is_some_and(|d| d <= limit)
                }
                (Some(_), None) => false,
            })
            .filter(|k| {
                terms
                    .iter()
                    .all(|(lower, norm)| matches_term(k, lower, norm))
            })
            .collect();
        hits.sort_by(|a, b| {
            compare(self, a, b, q.sort)
                .then_with(|| a.created_at.cmp(&b.created_at))
                .then_with(|| a.id.cmp(&b.id))
        });
        hits.into_iter().map(|k| k.id).collect()
    }
}

fn matches_term(k: &GameKey, lower: &str, norm: &str) -> bool {
    let has = |s: &str| s.to_lowercase().contains(lower);
    has(&k.game_name)
        || k.bundle.as_deref().is_some_and(has)
        || k.source.as_deref().is_some_and(has)
        || has(&k.notes)
        || k.tags.iter().any(|t| has(t))
        || (norm.len() >= 5 && k.key.normalized().contains(norm))
}

fn compare(repo: &Repo, a: &GameKey, b: &GameKey, sort: Sort) -> Ordering {
    let dir = |o: Ordering| {
        if sort.dir == SortDir::Desc {
            o.reverse()
        } else {
            o
        }
    };
    match sort.field {
        SortField::Added => dir(a.created_at.cmp(&b.created_at)),
        SortField::Updated => dir(a.updated_at.cmp(&b.updated_at)),
        SortField::Name => dir(natural_cmp(&a.game_name, &b.game_name)),
        SortField::Status => dir(a.status.cmp(&b.status)),
        SortField::Platform => dir(a
            .platform
            .label()
            .to_lowercase()
            .cmp(&b.platform.label().to_lowercase())),
        SortField::Bundle => opt(
            a.bundle.as_deref(),
            b.bundle.as_deref(),
            sort.dir,
            natural_cmp,
        ),
        SortField::Source => opt(
            a.source.as_deref(),
            b.source.as_deref(),
            sort.dir,
            natural_cmp,
        ),
        SortField::StatusDate => opt(a.status_date, b.status_date, sort.dir, |x, y| x.cmp(&y)),
        SortField::Deadline => opt(
            a.redemption_deadline,
            b.redemption_deadline,
            sort.dir,
            |x, y| x.cmp(&y),
        ),
        SortField::Rating => {
            let r = |k: &GameKey| {
                k.steam
                    .as_ref()
                    .and_then(|m| m.rating.as_ref())
                    .map(|r| r.percent)
            };
            opt(r(a), r(b), sort.dir, |x, y| x.cmp(&y))
        }
        SortField::Price => {
            let p = |k: &GameKey| repo.key_prices(k).and_then(|p| p.best_cents());
            opt(p(a), p(b), sort.dir, |x, y| x.cmp(&y))
        }
        SortField::SteamPrice => {
            let p = |k: &GameKey| {
                repo.key_prices(k)
                    .and_then(|p| p.steam())
                    .map(|s| s.price.cents)
            };
            opt(p(a), p(b), sort.dir, |x, y| x.cmp(&y))
        }
        SortField::KeyshopPrice => {
            let p = |k: &GameKey| {
                repo.key_prices(k)
                    .and_then(|p| p.keyshop_low.as_ref())
                    .map(|m| m.cents)
            };
            opt(p(a), p(b), sort.dir, |x, y| x.cmp(&y))
        }
        SortField::Ownership => {
            use crate::model::Ownership as O;
            let r = |k: &GameKey| match k.steam.as_ref().map(|m| m.ownership) {
                Some(O::Owned) => Some(0u8),
                Some(O::Wishlisted) => Some(1),
                Some(O::NotOwned) => Some(2),
                _ => None,
            };
            opt(r(a), r(b), sort.dir, |x, y| x.cmp(&y))
        }
    }
}

/// Missing values always sort last, whichever the direction.
fn opt<T>(a: Option<T>, b: Option<T>, dir: SortDir, cmp: impl Fn(T, T) -> Ordering) -> Ordering {
    match (a, b) {
        (Some(x), Some(y)) => {
            let o = cmp(x, y);
            if dir == SortDir::Desc { o.reverse() } else { o }
        }
        (Some(_), None) => Ordering::Less,
        (None, Some(_)) => Ordering::Greater,
        (None, None) => Ordering::Equal,
    }
}

/// Case-insensitive comparison where digit runs compare as numbers: "Game 2" < "Game 10".
pub fn natural_cmp(a: &str, b: &str) -> Ordering {
    let (mut a, mut b) = (a.chars().peekable(), b.chars().peekable());
    loop {
        match (a.peek().copied(), b.peek().copied()) {
            (None, None) => return Ordering::Equal,
            (None, Some(_)) => return Ordering::Less,
            (Some(_), None) => return Ordering::Greater,
            (Some(x), Some(y)) if x.is_ascii_digit() && y.is_ascii_digit() => {
                let take = |it: &mut std::iter::Peekable<std::str::Chars<'_>>| {
                    let mut s = String::new();
                    while let Some(c) = it.peek().copied().filter(char::is_ascii_digit) {
                        s.push(c);
                        it.next();
                    }
                    s
                };
                let (na, nb) = (take(&mut a), take(&mut b));
                let (ta, tb) = (na.trim_start_matches('0'), nb.trim_start_matches('0'));
                let o = ta
                    .len()
                    .cmp(&tb.len())
                    .then_with(|| ta.cmp(tb))
                    .then_with(|| na.len().cmp(&nb.len()));
                if o != Ordering::Equal {
                    return o;
                }
            }
            (Some(x), Some(y)) => {
                let o = x.to_lowercase().cmp(y.to_lowercase());
                if o != Ordering::Equal {
                    return o;
                }
                a.next();
                b.next();
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn labels_group_ignoring_case_and_spacing() {
        let mk = |src: &str| {
            let mut k = GameKey::new("G", "K");
            k.source = Some(src.into());
            k
        };
        let keys = vec![
            mk("Humble"),
            mk("humble"),
            mk(" Humble "),
            mk("Fanatical"),
            mk("  "),
        ];
        assert_eq!(
            label_values(&keys, LabelField::Source),
            vec![("Humble".to_string(), 3), ("Fanatical".to_string(), 1)]
        );
        assert_eq!(
            canonical_label(&keys, LabelField::Source, "HUMBLE "),
            "Humble"
        );
        assert_eq!(canonical_label(&keys, LabelField::Source, "GOG"), "GOG");
        assert_eq!(
            label_suggestions(&keys, LabelField::Source, "hum", 5),
            vec!["Humble"]
        );
        assert_eq!(
            label_suggestions(&keys, LabelField::Source, "a", 5),
            vec!["Fanatical"]
        );
        assert!(label_suggestions(&keys, LabelField::Source, "Humble", 5).is_empty());
        assert!(label_suggestions(&keys, LabelField::Bundle, "x", 5).is_empty());
    }
    use crate::data::VaultData;
    use crate::model::{SteamMeta, SteamRating};
    use jiff::civil::date;

    fn today() -> Date {
        date(2026, 1, 7)
    }

    fn repo() -> Repo {
        let mut r = Repo::new(VaultData::default());
        let mk = |name: &str, key: &str, bundle: Option<&str>, status: KeyStatus| {
            let mut k = GameKey::new(name, key);
            k.bundle = bundle.map(str::to_string);
            k.status = status;
            k
        };
        let mut a = mk(
            "Undertale",
            "UNDR7-TALE0-00001",
            Some("Humble Conquer COVID-19 Bundle"),
            KeyStatus::Traded,
        );
        a.redemption_deadline = Some(date(2026, 1, 10));
        let mut b = mk(
            "Haiku, the Robot",
            "HA1KU-R0B0T-00002",
            Some("In Case You Missed It: Gems of 2022"),
            KeyStatus::Available,
        );
        b.redemption_deadline = Some(date(2026, 1, 20));
        b.steam_app_id = Some(1);
        b.steam = Some(SteamMeta {
            rating: Some(SteamRating {
                desc: "Very Positive".into(),
                percent: 94,
                total: 100,
            }),
            ownership: Default::default(),
            fetched_at: jiff::Timestamp::now(),
        });
        let mut c = mk("Game 10", "AAAAA-11111-BBBBB", None, KeyStatus::Available);
        c.redemption_deadline = Some(date(2026, 1, 1)); // already expired
        c.tags = vec!["rare".into()];
        let d = mk("Game 2", "CCCCC-22222-DDDDD", None, KeyStatus::Unknown);
        r.add_keys(vec![a, b, c, d], "seed");
        r
    }

    fn names(r: &Repo, q: &Query) -> Vec<String> {
        r.query(q, today())
            .into_iter()
            .map(|id| r.key(id).unwrap().game_name.clone())
            .collect()
    }

    #[test]
    fn default_is_newest_first() {
        let mut r = Repo::new(VaultData::default());
        let keys = ["old", "mid", "new"].iter().enumerate().map(|(i, n)| {
            let mut k = GameKey::new(*n, format!("K{i}"));
            k.created_at = jiff::Timestamp::from_second(1_700_000_000 + i as i64).unwrap();
            k
        });
        r.add_keys(keys.collect(), "seed");
        assert_eq!(names(&r, &Query::default()), ["new", "mid", "old"]);
    }

    #[test]
    fn text_search_is_case_insensitive_and_anded() {
        let r = repo();
        assert_eq!(names(&r, &Query::with_text("UNDER")), ["Undertale"]);
        assert_eq!(names(&r, &Query::with_text("gems")), ["Haiku, the Robot"]);
        assert_eq!(names(&r, &Query::with_text("humble covid")), ["Undertale"]);
        assert!(names(&r, &Query::with_text("humble gems")).is_empty());
        assert_eq!(
            names(&r, &Query::with_text("rare")),
            ["Game 10"],
            "tags are searched"
        );
    }

    #[test]
    fn key_search_needs_five_characters() {
        let r = repo();
        assert_eq!(
            names(&r, &Query::with_text("ha1ku-r0")),
            ["Haiku, the Robot"]
        );
        assert!(names(&r, &Query::with_text("ha1")).is_empty());
    }

    #[test]
    fn status_and_app_id_filters() {
        let r = repo();
        let q = Query {
            statuses: vec![KeyStatus::Available, KeyStatus::Unknown],
            sort: Sort {
                field: SortField::Name,
                dir: SortDir::Asc,
            },
            ..Query::default()
        };
        assert_eq!(names(&r, &q), ["Game 2", "Game 10", "Haiku, the Robot"]);
        let q = Query {
            missing_app_id: true,
            ..q
        };
        assert_eq!(names(&r, &q), ["Game 2", "Game 10"]);
    }

    #[test]
    fn deadline_filter_skips_gone_keys_and_includes_expired() {
        let r = repo();
        let q = Query {
            deadline_within_days: Some(30),
            sort: Sort {
                field: SortField::Deadline,
                dir: SortDir::Asc,
            },
            ..Query::default()
        };
        // Undertale is traded (gone), so it's excluded even though its deadline is near.
        assert_eq!(names(&r, &q), ["Game 10", "Haiku, the Robot"]);
        let q = Query {
            deadline_within_days: Some(5),
            ..q
        };
        assert_eq!(names(&r, &q), ["Game 10"]);
    }

    #[test]
    fn sorting_puts_missing_values_last_both_ways() {
        let r = repo();
        let asc = Query {
            sort: Sort {
                field: SortField::Bundle,
                dir: SortDir::Asc,
            },
            ..Query::default()
        };
        let got = names(&r, &asc);
        assert_eq!(&got[..2], ["Undertale", "Haiku, the Robot"]);
        let desc = Query {
            sort: Sort {
                field: SortField::Bundle,
                dir: SortDir::Desc,
            },
            ..Query::default()
        };
        let got = names(&r, &desc);
        assert_eq!(&got[..2], ["Haiku, the Robot", "Undertale"]);
        let rating = Query {
            sort: Sort {
                field: SortField::Rating,
                dir: SortDir::Desc,
            },
            ..Query::default()
        };
        assert_eq!(names(&r, &rating)[0], "Haiku, the Robot");
    }

    #[test]
    fn ownership_sorts_library_then_wishlist_then_unchecked() {
        let mut r = repo();
        let with = |name: &str, key: &str, o: crate::model::Ownership| {
            let mut k = GameKey::new(name, key);
            k.steam = Some(SteamMeta {
                rating: None,
                ownership: o,
                fetched_at: jiff::Timestamp::now(),
            });
            k
        };
        r.add_keys(
            vec![
                with(
                    "Not owned",
                    "NOTOW-NED00-00001",
                    crate::model::Ownership::NotOwned,
                ),
                with(
                    "Wished",
                    "WISHE-D0000-00001",
                    crate::model::Ownership::Wishlisted,
                ),
                with("Owned", "OWNED-00000-00001", crate::model::Ownership::Owned),
            ],
            "more",
        );
        let q = |dir| Query {
            sort: Sort {
                field: SortField::Ownership,
                dir,
            },
            ..Query::default()
        };
        assert_eq!(
            &names(&r, &q(SortDir::Asc))[..3],
            ["Owned", "Wished", "Not owned"]
        );
        assert_eq!(
            &names(&r, &q(SortDir::Desc))[..3],
            ["Not owned", "Wished", "Owned"]
        );
    }

    #[test]
    fn natural_ordering() {
        let mut v = vec!["Game 10", "game 2", "Game 1", "Game 02", "Alpha", "game"];
        v.sort_by(|a, b| natural_cmp(a, b));
        assert_eq!(
            v,
            ["Alpha", "game", "Game 1", "game 2", "Game 02", "Game 10"]
        );
        assert_eq!(natural_cmp("Hades II", "hades ii"), Ordering::Equal);
    }

    #[test]
    fn trade_fodder_and_duplicates() {
        let mut r = repo();
        let owned = |k: &mut GameKey| {
            k.steam = Some(SteamMeta {
                rating: None,
                ownership: crate::model::Ownership::Owned,
                fetched_at: jiff::Timestamp::now(),
            })
        };
        let mut spare = GameKey::new("Haiku, the Robot", "HA1KU-R0B0T-99999");
        spare.steam_app_id = Some(1);
        owned(&mut spare);
        let mut gone = GameKey::new("Owned but traded", "OWNED-TRADE-00001");
        gone.status = KeyStatus::Traded;
        owned(&mut gone);
        let same_name = GameKey::new("game-2", "GAME2-AGAIN-00002");
        r.add_keys(vec![spare, gone, same_name], "more");

        let q = Query {
            trade_fodder: true,
            ..Query::default()
        };
        assert_eq!(
            names(&r, &q),
            ["Haiku, the Robot"],
            "only spares still in hand"
        );

        let q = Query {
            duplicates: true,
            sort: Sort {
                field: SortField::Name,
                dir: SortDir::Asc,
            },
            ..Query::default()
        };
        assert_eq!(
            names(&r, &q),
            ["Game 2", "game-2", "Haiku, the Robot", "Haiku, the Robot"],
            "same app id, or the same name ignoring case and punctuation"
        );
    }

    #[test]
    fn identity() {
        let mut a = GameKey::new("Hollow Knight", "X");
        assert_eq!(game_identity(&a), "name:hollowknight");
        a.steam_app_id = Some(367520);
        assert_eq!(game_identity(&a), "app:367520");
    }
}
