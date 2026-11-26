//! Importing keys from CSV/TSV files, spreadsheet pastes and plain-text lists.
//!
//! 1. [`parse`] raw input into a [`Table`] with a suggested column mapping.
//! 2. The user may change `table.mapping` and the [`ImportOptions`].
//! 3. [`preview`] validates every row against the vault: new / duplicate / error, plus warnings.
//! 4. [`commit`] adds the accepted rows (and any trades their links imply) as one undo step.

mod table;
pub mod values;

use std::collections::HashMap;

use jiff::Timestamp;
use jiff::tz::TimeZone;
use uuid::Uuid;

pub use table::{
    Table, TableKind, detect_delimiter, find_key, is_key, map_headers, parse, parse_delimited,
    parse_text, split_line,
};
pub use values::StatusRules;

use crate::model::{DateFormat, GameKey, Platform, Trade, TradeSite, TradeStatus};
use crate::repo::Repo;
use crate::secret::{Secret, normalize_key};

/// A destination for one input column.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Field {
    GameName,
    Key,
    Platform,
    Status,
    Bundle,
    Source,
    KeyUrl,
    SteamAppId,
    StatusDate,
    TradeUrl,
    TradePartner,
    RedemptionDeadline,
    Tags,
    Notes,
}

impl Field {
    pub const ALL: [Field; 14] = [
        Field::GameName,
        Field::Key,
        Field::Platform,
        Field::Status,
        Field::Bundle,
        Field::Source,
        Field::KeyUrl,
        Field::SteamAppId,
        Field::StatusDate,
        Field::TradeUrl,
        Field::TradePartner,
        Field::RedemptionDeadline,
        Field::Tags,
        Field::Notes,
    ];

    pub fn label(self) -> &'static str {
        match self {
            Field::GameName => "Game name",
            Field::Key => "Key",
            Field::Platform => "Platform",
            Field::Status => "Status",
            Field::Bundle => "Bundle",
            Field::Source => "Source / website",
            Field::KeyUrl => "Key link",
            Field::SteamAppId => "Steam app id / link",
            Field::StatusDate => "Redeem / trade date",
            Field::TradeUrl => "Trade link",
            Field::TradePartner => "Trade partner",
            Field::RedemptionDeadline => "Redemption deadline",
            Field::Tags => "Tags",
            Field::Notes => "Notes",
        }
    }

    /// Header aliases, compared after lowercasing and dropping non-alphanumerics
    /// (`"Redeem/Trade Date"` → `"redeemtradedate"`).
    fn aliases(self) -> &'static [&'static str] {
        match self {
            Field::GameName => &["gamename", "name", "title", "game", "product", "gametitle"],
            Field::Key => &[
                "key",
                "code",
                "serial",
                "cdkey",
                "gamekey",
                "steamkey",
                "activationkey",
                "productkey",
            ],
            Field::Platform => &["platform", "drm", "launcher"],
            Field::Status => &[
                "status",
                "redeemed",
                "traded",
                "used",
                "state",
                "redeemedtraded",
                "tradestatus",
            ],
            Field::Bundle => &["bundle", "bundlename", "collection"],
            Field::Source => &[
                "website", "source", "site", "store", "shop", "from", "seller",
            ],
            Field::KeyUrl => &[
                "keylink",
                "keyurl",
                "link",
                "url",
                "revealurl",
                "redeemlink",
                "keypage",
            ],
            Field::SteamAppId => &[
                "steamappid",
                "appid",
                "steamlink",
                "steamurl",
                "storelink",
                "steampage",
            ],
            Field::StatusDate => &[
                "redeemtradedate",
                "tradedate",
                "redeemdate",
                "redeemeddate",
                "date",
                "dateredeemed",
                "datetraded",
                "statusdate",
                "redeemedtradedate",
            ],
            Field::TradeUrl => &[
                "tradelink",
                "tradeurl",
                "offer",
                "offerlink",
                "tradeoffer",
                "trade",
            ],
            Field::TradePartner => &[
                "tradepartner",
                "partner",
                "tradedto",
                "tradeuser",
                "tradedwith",
                "user",
            ],
            Field::RedemptionDeadline => &[
                "redemptiondeadline",
                "deadline",
                "expires",
                "expiry",
                "expiration",
                "redeemby",
                "expirydate",
                "expirationdate",
            ],
            Field::Tags => &["tags", "tag", "labels"],
            Field::Notes => &["notes", "note", "comment", "comments", "description"],
        }
    }

    pub fn from_header(header: &str) -> Option<Field> {
        let h: String = header
            .chars()
            .filter(|c| c.is_alphanumeric())
            .flat_map(char::to_lowercase)
            .collect();
        if h.is_empty() {
            return None;
        }
        Field::ALL
            .into_iter()
            .find(|f| f.aliases().contains(&h.as_str()))
    }
}

#[derive(Clone, Debug, PartialEq)]
pub struct ImportOptions {
    pub date_format: DateFormat,
    pub status: StatusRules,
    /// Used when there is no platform column (or the cell is empty).
    pub default_platform: Platform,
}

impl ImportOptions {
    pub fn new(date_format: DateFormat) -> ImportOptions {
        ImportOptions {
            date_format,
            status: StatusRules::default(),
            default_platform: Platform::Steam,
        }
    }
}

#[derive(Clone, Debug, PartialEq)]
pub enum RowOutcome {
    New,
    /// The key is already in the vault (`existing`) or earlier in this file (`earlier_row`).
    Duplicate {
        existing: Option<Uuid>,
        earlier_row: Option<usize>,
    },
    Error(String),
}

#[derive(Clone, Debug, PartialEq)]
pub struct RowResult {
    /// Index into `table.rows`.
    pub row: usize,
    /// 1-based source line.
    pub line: usize,
    pub outcome: RowOutcome,
    /// The key this row would create (absent for errors).
    pub key: Option<GameKey>,
    pub trade_url: Option<String>,
    pub trade_partner: Option<String>,
    pub warnings: Vec<String>,
}

#[derive(Clone, Debug, Default, PartialEq)]
pub struct ImportPreview {
    pub rows: Vec<RowResult>,
}

impl ImportPreview {
    pub fn count(&self, pred: impl Fn(&RowOutcome) -> bool) -> usize {
        self.rows.iter().filter(|r| pred(&r.outcome)).count()
    }
    pub fn new_count(&self) -> usize {
        self.count(|o| *o == RowOutcome::New)
    }
    pub fn duplicate_count(&self) -> usize {
        self.count(|o| matches!(o, RowOutcome::Duplicate { .. }))
    }
    pub fn error_count(&self) -> usize {
        self.count(|o| matches!(o, RowOutcome::Error(_)))
    }
}

/// Validate every row of `table` (with its current `mapping`) against `repo`.
pub fn preview(table: &Table, opts: &ImportOptions, repo: &Repo) -> ImportPreview {
    let col = |f: Field| table.mapping.iter().position(|m| *m == Some(f));
    let cols: HashMap<Field, usize> = Field::ALL
        .into_iter()
        .filter_map(|f| col(f).map(|c| (f, c)))
        .collect();
    let mut seen: HashMap<String, usize> = HashMap::new();
    let mut out = Vec::with_capacity(table.rows.len());

    for (i, row) in table.rows.iter().enumerate() {
        let get = |f: Field| {
            cols.get(&f)
                .and_then(|&c| row.get(c))
                .map(|s| values::clean(s))
                .unwrap_or("")
        };
        if Field::ALL.iter().all(|&f| get(f).is_empty()) {
            continue; // nothing mapped on this row
        }
        let line = table.lines.get(i).copied().unwrap_or(i + 1);
        let mut warnings = Vec::new();
        let name = get(Field::GameName);
        let code = get(Field::Key);
        let error = match (name.is_empty(), code.is_empty()) {
            (true, true) => Some("no game name and no key".to_string()),
            (_, true) => Some("no key".to_string()),
            (true, _) => Some("no game name".to_string()),
            _ => None,
        };
        if let Some(e) = error {
            out.push(RowResult {
                row: i,
                line,
                outcome: RowOutcome::Error(e),
                key: None,
                trade_url: None,
                trade_partner: None,
                warnings,
            });
            continue;
        }

        let mut k = GameKey::new(name, Secret::new(code));
        let trade_url = Some(get(Field::TradeUrl))
            .filter(|s| !s.is_empty())
            .map(str::to_string);
        if let Some(u) = &trade_url
            && !values::is_url(u)
        {
            warnings.push(format!("trade link '{u}' doesn't look like a URL"));
        }
        let (status, warn) = if cols.contains_key(&Field::Status) {
            opts.status.map(get(Field::Status), trade_url.is_some())
        } else if trade_url.is_some() {
            (opts.status.done_with_link, None)
        } else {
            (opts.status.not_done, None)
        };
        k.status = status;
        warnings.extend(warn);
        k.platform = match get(Field::Platform) {
            "" => opts.default_platform.clone(),
            p => Platform::parse(p),
        };
        let text = |f: Field| Some(get(f)).filter(|s| !s.is_empty()).map(str::to_string);
        k.bundle = text(Field::Bundle);
        k.source = text(Field::Source);
        k.notes = get(Field::Notes).to_string();
        k.tags = get(Field::Tags)
            .split([';', ','])
            .map(str::trim)
            .filter(|t| !t.is_empty())
            .map(str::to_string)
            .collect();
        if let Some(u) = text(Field::KeyUrl) {
            if values::is_url(&u) {
                k.key_url = Some(u);
            } else {
                warnings.push(format!("key link '{u}' ignored: not a URL"));
            }
        }
        if let Some(s) = text(Field::SteamAppId) {
            match values::parse_app_id(&s) {
                Some(id) => k.steam_app_id = Some(id),
                None => warnings.push(format!("'{s}' is not a Steam app id or store link")),
            }
        }
        for (field, slot) in [
            (Field::StatusDate, &mut k.status_date),
            (Field::RedemptionDeadline, &mut k.redemption_deadline),
        ] {
            match values::parse_date(get(field), opts.date_format) {
                Ok(d) => *slot = d,
                Err(e) => warnings.push(format!("{}: {e}", field.label())),
            }
        }

        let norm = normalize_key(code);
        let outcome = match (
            repo.find_by_key(code).map(|e| e.id),
            seen.get(&norm).copied(),
        ) {
            (None, None) => RowOutcome::New,
            (existing, earlier_row) => RowOutcome::Duplicate {
                existing,
                earlier_row,
            },
        };
        seen.entry(norm).or_insert(i);
        out.push(RowResult {
            row: i,
            line,
            outcome,
            key: Some(k),
            trade_url,
            trade_partner: text(Field::TradePartner),
            warnings,
        });
    }
    ImportPreview { rows: out }
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct ImportSummary {
    pub added: usize,
    pub skipped_duplicates: usize,
    pub errors: usize,
    pub trades_created: usize,
    pub trades_linked: usize,
}

/// Add the previewed keys to `repo` as one undo step. Rows sharing a trade link share one
/// [`Trade`]; a link already used by a trade in the vault attaches to that trade.
pub fn commit(repo: &mut Repo, preview: &ImportPreview, include_duplicates: bool) -> ImportSummary {
    let mut summary = ImportSummary {
        errors: preview.error_count(),
        ..ImportSummary::default()
    };
    let mut by_url: HashMap<String, Trade> = repo
        .trades()
        .iter()
        .filter_map(|t| t.trade_url.clone().map(|u| (u, t.clone())))
        .collect();
    let mut touched: Vec<String> = Vec::new();
    let mut keys = Vec::new();

    for r in &preview.rows {
        let take = match r.outcome {
            RowOutcome::New => true,
            RowOutcome::Duplicate { .. } => include_duplicates,
            RowOutcome::Error(_) => false,
        };
        let Some(mut k) = r.key.clone().filter(|_| take) else {
            if matches!(r.outcome, RowOutcome::Duplicate { .. }) {
                summary.skipped_duplicates += 1;
            }
            continue;
        };
        if let Some(url) = &r.trade_url {
            let trade = by_url.entry(url.clone()).or_insert_with(|| {
                summary.trades_created += 1;
                let mut t = Trade::new();
                t.site =
                    TradeSite::from_url(url).unwrap_or_else(|| TradeSite::Other(String::new()));
                t.trade_url = Some(url.clone());
                t.status = if k.status.is_gone() {
                    TradeStatus::Completed
                } else {
                    TradeStatus::Pending
                };
                t.completed_at = (t.status == TradeStatus::Completed)
                    .then(|| k.status_date.and_then(midnight_utc))
                    .flatten();
                t.created_at = t.completed_at.unwrap_or(t.created_at);
                t
            });
            if trade.partner_name.is_empty()
                && let Some(p) = &r.trade_partner
            {
                trade.partner_name = p.clone();
            }
            trade.key_ids.push(k.id);
            k.trade_id = Some(trade.id);
            if !touched.contains(url) {
                touched.push(url.clone());
            }
            summary.trades_linked += 1;
        }
        keys.push(k);
    }
    summary.added = keys.len();
    let trades = touched
        .into_iter()
        .filter_map(|u| by_url.remove(&u))
        .collect();
    repo.import(keys, trades, &format!("Import {} keys", summary.added));
    summary
}

fn midnight_utc(d: jiff::civil::Date) -> Option<Timestamp> {
    d.to_zoned_datetime(TimeZone::UTC)
        .ok()
        .map(|z| z.timestamp())
}

#[cfg(test)]
mod tests;
