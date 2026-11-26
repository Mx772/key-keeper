//! Import tests against the fixtures in `tests/fixtures/`.

use jiff::civil::date;
use proptest::prelude::*;

use super::*;
use crate::data::VaultData;
use crate::model::{KeyStatus, TradeSite, TradeStatus};

const EXAMPLE: &str = include_str!("../../tests/fixtures/example_import.csv");
const EXCEL: &str = include_str!("../../tests/fixtures/excel_sheet.csv");
const SEMICOLON: &str = include_str!("../../tests/fixtures/semicolon_dmy.csv");
const TSV: &str = include_str!("../../tests/fixtures/excel_paste.tsv");
const TEXT: &str = include_str!("../../tests/fixtures/key_list.txt");

fn us() -> ImportOptions {
    ImportOptions::new(DateFormat::MonthDayYear)
}

fn key<'a>(p: &'a ImportPreview, name: &str) -> &'a GameKey {
    p.rows
        .iter()
        .filter_map(|r| r.key.as_ref())
        .find(|k| k.game_name == name)
        .unwrap_or_else(|| panic!("no row for {name}"))
}

#[test]
fn user_example_csv() {
    let t = parse(EXAMPLE);
    assert_eq!(t.kind, TableKind::Delimited(b','));
    assert!(t.has_header);
    assert_eq!(
        t.mapping,
        [
            Some(Field::Source),
            Some(Field::Bundle),
            Some(Field::GameName),
            Some(Field::Key),
            Some(Field::Status),
            Some(Field::StatusDate),
            Some(Field::TradeUrl),
            Some(Field::RedemptionDeadline),
        ]
    );
    let mut repo = Repo::new(VaultData::default());
    let p = preview(&t, &us(), &repo);
    assert_eq!(
        (p.new_count(), p.duplicate_count(), p.error_count()),
        (1, 0, 0)
    );
    let k = key(&p, "Undertale");
    assert_eq!(k.key.expose(), "UNDR7-TALE0-00001");
    assert_eq!(
        k.status,
        KeyStatus::Traded,
        "'Redeemed' + a trade link = traded"
    );
    assert_eq!(k.status_date, Some(date(2025, 8, 20)));
    assert_eq!(k.redemption_deadline, None, "'None' is null");
    assert_eq!(k.source.as_deref(), Some("Humble"));
    assert_eq!(k.bundle.as_deref(), Some("Humble Conquer COVID-19 Bundle"));

    let s = commit(&mut repo, &p, false);
    assert_eq!(
        s,
        ImportSummary {
            added: 1,
            skipped_duplicates: 0,
            errors: 0,
            trades_created: 1,
            trades_linked: 1
        }
    );
    let trade = &repo.trades()[0];
    assert_eq!(trade.status, TradeStatus::Completed);
    assert_eq!(
        trade.trade_url.as_deref(),
        Some("https://lestrad..."),
        "the example's truncated link is kept verbatim"
    );
    assert_eq!(repo.keys()[0].trade_id, Some(trade.id));
}

#[test]
fn excel_sheet_statuses_dates_errors_and_duplicates() {
    let t = parse(EXCEL);
    let repo = Repo::new(VaultData::default());
    let p = preview(&t, &us(), &repo);
    assert_eq!(
        (p.new_count(), p.duplicate_count(), p.error_count()),
        (4, 1, 2),
        "{p:#?}"
    );

    let haiku = key(&p, "Haiku, the Robot");
    assert_eq!(haiku.status, KeyStatus::Traded);
    assert_eq!(
        haiku.status_date,
        Some(date(2026, 1, 7)),
        "1/7/2026 is 7 January"
    );
    assert_eq!(key(&p, "Dredge").status, KeyStatus::Available);
    assert_eq!(
        key(&p, "Dredge").redemption_deadline,
        Some(date(2026, 12, 31))
    );
    assert_eq!(
        key(&p, "Celeste").status,
        KeyStatus::Redeemed,
        "X without a link = redeemed by me"
    );

    let hades = p
        .rows
        .iter()
        .find(|r| r.key.as_ref().is_some_and(|k| k.game_name == "Hades"))
        .unwrap();
    assert_eq!(hades.key.as_ref().unwrap().status, KeyStatus::Unknown);
    assert_eq!(hades.warnings.len(), 1);

    let errors: Vec<(usize, &RowOutcome)> = p
        .rows
        .iter()
        .filter(|r| matches!(r.outcome, RowOutcome::Error(_)))
        .map(|r| (r.line, &r.outcome))
        .collect();
    assert_eq!(
        errors,
        [
            (6, &RowOutcome::Error("no game name and no key".into())),
            (7, &RowOutcome::Error("no key".into()))
        ]
    );

    let dup = p
        .rows
        .iter()
        .find(|r| matches!(r.outcome, RowOutcome::Duplicate { .. }))
        .unwrap();
    assert_eq!(
        dup.outcome,
        RowOutcome::Duplicate {
            existing: None,
            earlier_row: Some(1)
        },
        "matches Dredge despite case/spaces"
    );
}

#[test]
fn reimport_is_all_duplicates_and_skipped() {
    let t = parse(EXCEL);
    let mut repo = Repo::new(VaultData::default());
    let first = {
        let p = preview(&t, &us(), &repo);
        commit(&mut repo, &p, false)
    };
    assert_eq!(first.added, 4);
    assert_eq!(first.skipped_duplicates, 1);

    let again = preview(&t, &us(), &repo);
    assert_eq!(again.new_count(), 0);
    assert!(
        again
            .rows
            .iter()
            .filter(|r| r.key.is_some())
            .all(|r| matches!(
                r.outcome,
                RowOutcome::Duplicate {
                    existing: Some(_),
                    ..
                }
            ))
    );
    let s = commit(&mut repo, &again, false);
    assert_eq!(s.added, 0);
    assert_eq!(repo.keys().len(), 4);
}

#[test]
fn import_is_one_undo_step() {
    let mut repo = Repo::new(VaultData::default());
    {
        let p = preview(&parse(EXCEL), &us(), &repo);
        commit(&mut repo, &p, false)
    };
    assert_eq!(repo.keys().len(), 4);
    assert_eq!(repo.trades().len(), 1);
    assert_eq!(repo.undo_label(), Some("Import 4 keys"));
    repo.undo();
    assert!(repo.keys().is_empty());
    assert!(repo.trades().is_empty());
}

#[test]
fn semicolon_day_first_file() {
    let t = parse(SEMICOLON);
    assert_eq!(t.kind, TableKind::Delimited(b';'));
    let p = preview(
        &t,
        &ImportOptions::new(DateFormat::DayMonthYear),
        &Repo::new(VaultData::default()),
    );
    assert_eq!(p.new_count(), 2);
    let s = key(&p, "Spiritfarer");
    assert_eq!(s.status, KeyStatus::Traded);
    assert_eq!(s.status_date, Some(date(2026, 1, 7)));
    assert_eq!(
        key(&p, "Inscryption").redemption_deadline,
        Some(date(2026, 12, 31))
    );

    // Same file read month-first: 07/01 becomes 1 July, and 31.12 is invalid (warning, not error).
    let p = preview(&t, &us(), &Repo::new(VaultData::default()));
    assert_eq!(key(&p, "Spiritfarer").status_date, Some(date(2026, 7, 1)));
    let ins = p
        .rows
        .iter()
        .find(|r| r.key.as_ref().is_some_and(|k| k.game_name == "Inscryption"))
        .unwrap();
    assert_eq!(ins.outcome, RowOutcome::New);
    assert_eq!(ins.key.as_ref().unwrap().redemption_deadline, None);
    assert_eq!(ins.warnings.len(), 1);
}

#[test]
fn excel_clipboard_paste() {
    let t = parse(TSV);
    assert_eq!(t.kind, TableKind::Delimited(b'\t'));
    let p = preview(&t, &us(), &Repo::new(VaultData::default()));
    assert_eq!(p.new_count(), 2);
    assert_eq!(key(&p, "Undertale").notes, "from a friend");
    assert_eq!(key(&p, "=HYPERLINK(\"x\")").notes, "-careful");
}

#[test]
fn plain_text_key_list() {
    let t = parse(TEXT);
    assert_eq!(t.kind, TableKind::Text);
    let p = preview(&t, &us(), &Repo::new(VaultData::default()));
    assert_eq!((p.new_count(), p.error_count()), (5, 1), "{p:#?}");
    assert_eq!(key(&p, "Celeste").key.expose(), "QQ7RT-PP2MN-ZZ9XY");
    assert_eq!(
        key(&p, "Half-Life: Source").key.expose(),
        "HL2SR-C3000-ABCDE-FGH12-IJK34"
    );
    let err = p
        .rows
        .iter()
        .find(|r| matches!(r.outcome, RowOutcome::Error(_)))
        .unwrap();
    assert_eq!(err.line, 6);
    assert!(
        p.rows
            .iter()
            .filter_map(|r| r.key.as_ref())
            .all(|k| k.status == KeyStatus::Available)
    );
}

#[test]
fn rows_sharing_a_link_share_a_trade_and_existing_trades_are_reused() {
    let csv = "Game,Key,Trade Link,Partner\n\
               A,AAAAA-11111-AAAAA,https://lestrades.com/offer/9/,bob\n\
               B,BBBBB-22222-BBBBB,https://lestrades.com/offer/9/,\n\
               C,CCCCC-33333-CCCCC,https://www.steamtrades.com/trade/x/,carol\n";
    let mut repo = Repo::new(VaultData::default());
    let s = {
        let p = preview(&parse(csv), &us(), &repo);
        commit(&mut repo, &p, false)
    };
    assert_eq!((s.trades_created, s.trades_linked), (2, 3));
    let t9 = repo
        .trades()
        .iter()
        .find(|t| t.site == TradeSite::Lestrades)
        .unwrap();
    assert_eq!(t9.key_ids.len(), 2);
    assert_eq!(t9.partner_name, "bob");
    assert_eq!(
        t9.status,
        TradeStatus::Completed,
        "no status column + link = traded"
    );

    let more = "Game,Key,Trade Link\nD,DDDDD-44444-DDDDD,https://lestrades.com/offer/9/\n";
    let s = {
        let p = preview(&parse(more), &us(), &repo);
        commit(&mut repo, &p, false)
    };
    assert_eq!(s.trades_created, 0);
    let t9 = repo
        .trades()
        .iter()
        .find(|t| t.site == TradeSite::Lestrades)
        .unwrap();
    assert_eq!(t9.key_ids.len(), 3);
    assert_eq!(repo.trades().len(), 2);
}

#[test]
fn user_can_remap_columns() {
    let mut t = parse(
        "Title,Code,Comment\nCeleste,QQ7RT-PP2MN-ZZ9XY,https://store.steampowered.com/app/504230/\n",
    );
    assert_eq!(t.mapping[2], Some(Field::Notes));
    t.mapping[2] = Some(Field::SteamAppId);
    let p = preview(&t, &us(), &Repo::new(VaultData::default()));
    assert_eq!(key(&p, "Celeste").steam_app_id, Some(504230));
    assert_eq!(key(&p, "Celeste").notes, "");
}

#[test]
fn include_duplicates_when_asked() {
    let mut repo = Repo::new(VaultData::default());
    let s = {
        let p = preview(&parse(EXCEL), &us(), &repo);
        commit(&mut repo, &p, true)
    };
    assert_eq!((s.added, s.skipped_duplicates), (5, 0));
}

proptest! {
    #![proptest_config(ProptestConfig::with_cases(300))]

    #[test]
    fn arbitrary_input_never_panics(input in "\\PC{0,400}") {
        let repo = Repo::new(VaultData::default());
        let t = parse(&input);
        let p = preview(&t, &us(), &repo);
        prop_assert!(p.rows.len() <= t.rows.len());
    }

    #[test]
    fn arbitrary_csv_like_input_never_panics(lines in prop::collection::vec("[A-Za-z0-9 ,;\t\"/:.-]{0,60}", 0..12)) {
        let repo = Repo::new(VaultData::default());
        let input = lines.join("\n");
        let mut repo2 = Repo::new(VaultData::default());
        let p = preview(&parse(&input), &us(), &repo);
        let _ = commit(&mut repo2, &p, true);
    }
}
