//! Exporting keys: CSV (round-trips through import), JSON, and encrypted vault copies.
//! CSV and JSON contain plaintext keys; the UI must warn before writing them.

use std::path::Path;

use uuid::Uuid;

use crate::error::VaultError;
use crate::model::GameKey;
use crate::repo::Repo;
use crate::vault::Vault;

/// Column order of [`to_csv`]. Every title is an import alias, so exports re-import cleanly.
pub const CSV_HEADERS: [&str; 14] = [
    "Website",
    "Bundle",
    "Game Name",
    "Key",
    "Status",
    "Status Date",
    "Trade Link",
    "Trade Partner",
    "Redemption Deadline",
    "Platform",
    "Steam App ID",
    "Key Link",
    "Tags",
    "Notes",
];

/// Spreadsheet apps execute cells starting with `= + - @` (and tab/CR) as formulas. Prefix those
/// with `'` (OWASP CSV-injection guidance); import strips it again.
pub fn sanitize_cell(s: &str) -> String {
    if s.starts_with(['=', '+', '-', '@', '\t', '\r']) {
        format!("'{s}")
    } else {
        s.to_string()
    }
}

/// CSV of the given keys in the given order (pass `repo.query(..)` output to export a view).
/// Dates are ISO `YYYY-MM-DD` so they never depend on the date-format setting.
pub fn to_csv(repo: &Repo, ids: &[Uuid]) -> String {
    let mut w = csv::WriterBuilder::new().from_writer(Vec::new());
    // Writing to a Vec cannot fail; errors are ignored rather than unwrapped.
    let _ = w.write_record(CSV_HEADERS);
    for k in ids.iter().filter_map(|id| repo.key(*id)) {
        let trade = k.trade_id.and_then(|t| repo.trade(t));
        let date = |d: Option<jiff::civil::Date>| d.map(|d| d.to_string()).unwrap_or_default();
        let row = [
            k.source.clone().unwrap_or_default(),
            k.bundle.clone().unwrap_or_default(),
            k.game_name.clone(),
            k.key.expose().to_string(),
            k.status.label().to_string(),
            date(k.status_date),
            trade.and_then(|t| t.trade_url.clone()).unwrap_or_default(),
            trade.map(|t| t.partner_name.clone()).unwrap_or_default(),
            date(k.redemption_deadline),
            k.platform.label().to_string(),
            k.steam_app_id.map(|i| i.to_string()).unwrap_or_default(),
            k.key_url.clone().unwrap_or_default(),
            k.tags.join("; "),
            k.notes.clone(),
        ];
        let _ = w.write_record(row.iter().map(|c| sanitize_cell(c)));
    }
    let bytes = w.into_inner().unwrap_or_default();
    String::from_utf8(bytes).unwrap_or_default()
}

/// Pretty JSON array of the given keys (full model, including metadata).
pub fn to_json(repo: &Repo, ids: &[Uuid]) -> String {
    let keys: Vec<&GameKey> = ids.iter().filter_map(|id| repo.key(*id)).collect();
    serde_json::to_string_pretty(&keys).unwrap_or_else(|_| "[]".into())
}

/// Write an encrypted copy of the whole vault (same password) to `path`.
pub fn encrypted_backup(vault: &Vault, repo: &Repo, path: &Path) -> Result<(), VaultError> {
    vault.save_copy(path, repo.data())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::data::VaultData;
    use crate::import::{self, ImportOptions};
    use crate::model::{DateFormat, KeyStatus, Platform, Trade};
    use crate::query::Query;
    use jiff::civil::date;

    fn sample_repo() -> Repo {
        let mut r = Repo::new(VaultData::default());
        let mut t = Trade::new();
        t.trade_url = Some("https://lestrades.com/offer/100001/".into());
        t.partner_name = "alice".into();
        let mut a = GameKey::new("Haiku, the Robot", "HA1KU-R0B0T-00002");
        a.status = KeyStatus::Traded;
        a.status_date = Some(date(2026, 1, 7));
        a.source = Some("Humble".into());
        a.bundle = Some("In Case You Missed It: Gems of 2022".into());
        a.trade_id = Some(t.id);
        t.key_ids.push(a.id);
        let mut b = GameKey::new("=cmd|' /C calc'!A0", "AB12C-DE34F-GH56I");
        b.platform = Platform::Gog;
        b.redemption_deadline = Some(date(2026, 12, 31));
        b.steam_app_id = Some(972660);
        b.tags = vec!["rare".into(), "dlc".into()];
        b.notes = "-careful, \"quoted\"\nmultiline".into();
        b.key_url = Some("https://www.humblebundle.com/downloads?key=x".into());
        r.import(vec![a, b], vec![t], "seed");
        r
    }

    fn all(r: &Repo) -> Vec<Uuid> {
        r.query(
            &Query {
                sort: crate::query::Sort {
                    field: crate::query::SortField::Name,
                    dir: crate::query::SortDir::Asc,
                },
                ..Query::default()
            },
            date(2026, 1, 1),
        )
    }

    #[test]
    fn formula_cells_are_neutralised() {
        let r = sample_repo();
        let csv = to_csv(&r, &all(&r));
        assert!(csv.contains("'=cmd"), "{csv}");
        assert!(csv.contains("'-careful"));
        assert!(!csv.contains(",=cmd"));
    }

    #[test]
    fn csv_round_trips_through_import() {
        let src = sample_repo();
        let csv = to_csv(&src, &all(&src));
        let table = import::parse(&csv);
        assert!(table.has_header);
        assert!(
            table.mapping.iter().all(Option::is_some),
            "every exported column maps back: {:?}",
            table.mapping
        );

        // Day-first setting must not matter: dates are ISO.
        let mut dst = Repo::new(VaultData::default());
        let p = import::preview(&table, &ImportOptions::new(DateFormat::DayMonthYear), &dst);
        assert_eq!(p.new_count(), 2, "{p:#?}");
        assert!(p.rows.iter().all(|r| r.warnings.is_empty()), "{p:#?}");
        let s = import::commit(&mut dst, &p, false);
        assert_eq!((s.added, s.trades_created), (2, 1));

        for k in src.keys() {
            let back = dst.find_by_key(k.key.expose()).expect("re-imported");
            assert_eq!(back.game_name, k.game_name);
            assert_eq!(back.status, k.status);
            assert_eq!(back.platform, k.platform);
            assert_eq!(back.bundle, k.bundle);
            assert_eq!(back.source, k.source);
            assert_eq!(back.status_date, k.status_date);
            assert_eq!(back.redemption_deadline, k.redemption_deadline);
            assert_eq!(back.steam_app_id, k.steam_app_id);
            assert_eq!(back.key_url, k.key_url);
            assert_eq!(back.tags, k.tags);
            assert_eq!(back.notes, k.notes);
            assert_eq!(back.trade_id.is_some(), k.trade_id.is_some());
        }
        let t = &dst.trades()[0];
        assert_eq!(t.partner_name, "alice");
        assert_eq!(
            t.trade_url.as_deref(),
            Some("https://lestrades.com/offer/100001/")
        );
    }

    #[test]
    fn json_contains_requested_keys_only() {
        let r = sample_repo();
        let ids = all(&r);
        let json = to_json(&r, &ids[..1]);
        let back: Vec<GameKey> = serde_json::from_str(&json).unwrap();
        assert_eq!(back.len(), 1);
        assert_eq!(&back[0], r.key(ids[0]).unwrap());
    }

    #[test]
    fn encrypted_backup_opens_with_same_password() {
        let dir = tempfile::tempdir().unwrap();
        let r = sample_repo();
        let vault = Vault::create(
            dir.path().join("v.kkvault"),
            "pw",
            crate::vault::KdfParams::INSECURE_FAST_FOR_TESTS,
            r.data(),
        )
        .unwrap();
        let copy = dir.path().join("backup.kkvault");
        encrypted_backup(&vault, &r, &copy).unwrap();
        let (_, data) = Vault::open(&copy, "pw").unwrap();
        assert_eq!(&data, r.data());
    }
}
