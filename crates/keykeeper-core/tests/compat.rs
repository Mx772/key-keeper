//! Vault compatibility: every vault in `tests/fixtures/vaults/` was written by a released
//! KeyKeeper version (`v<version>.kkvault`), and this build must still open all of them.
//!
//! Before each release, generate the fixture for the new version and commit it:
//!
//! ```sh
//! cargo test -p keykeeper-core --test compat -- --ignored generate_fixture
//! ```
//!
//! The fixtures hold fake keys only. Password: [`FIXTURE_PASSWORD`]; the KDF parameters are the
//! insecure test ones, stored in each file's header, so opening them is fast.
//!
//! [`fixture_data`] must keep producing the same keys and trade, because [`check`] holds every
//! fixture, old and new, to them. New fields can be added to the data (and checked for the
//! versions that have them).

#![allow(clippy::unwrap_used, clippy::expect_used)] // test code

use std::path::{Path, PathBuf};

use keykeeper_core::{
    DateFormat, GameKey, KdfParams, KeyStatus, Secret, Trade, TradeStatus, Vault, VaultData,
};

/// The password of every fixture vault. Not a secret.
const FIXTURE_PASSWORD: &str = "keykeeper-fixture";

fn fixtures_dir() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/vaults")
}

/// (name, key, status)
const KEYS: [(&str, &str, KeyStatus); 4] = [
    ("Celeste", "FAKE1-AAAAA-00001", KeyStatus::Available),
    ("Hades", "FAKE2-BBBBB-00002", KeyStatus::Reserved),
    ("Undertale", "FAKE3-CCCCC-00003", KeyStatus::Traded),
    ("Outer Wilds", "FAKE4-DDDDD-00004", KeyStatus::Redeemed),
];

fn fixture_data(app_version: &str) -> VaultData {
    let mut d = VaultData::default();
    for (name, key, status) in KEYS {
        let mut k = GameKey::new(name, key);
        k.status = status;
        k.bundle = Some("Fixture Bundle".into());
        k.tags = vec!["fixture".into()];
        d.keys.push(k);
    }
    d.keys[0].steam_app_id = Some(504230);
    let mut t = Trade::new();
    t.status = TradeStatus::Completed;
    t.partner_name = "fixture-partner".into();
    t.key_ids = vec![d.keys[2].id];
    d.keys[2].trade_id = Some(t.id);
    d.trades.push(t);
    d.settings.date_format = DateFormat::DayMonthYear;
    d.settings.auto_lock_minutes = 15;
    d.settings.steam_id = Some("76561190000000000".into());
    d.api_keys.steam = Some(Secret::new("00000000000000000000000000000000"));
    d.last_app_version = Some(app_version.to_string());
    d
}

/// What every fixture must contain, whichever version wrote it.
fn check(file: &Path, d: &VaultData) {
    let name = file.display();
    assert_eq!(d.keys.len(), KEYS.len(), "{name}: key count");
    for (k, (n, key, status)) in d.keys.iter().zip(KEYS) {
        assert_eq!(k.game_name, n, "{name}");
        assert_eq!(k.key.expose(), key, "{name}: key of {n}");
        assert_eq!(k.status, status, "{name}: status of {n}");
        assert_eq!(k.bundle.as_deref(), Some("Fixture Bundle"), "{name}");
        assert_eq!(k.tags, ["fixture"], "{name}");
    }
    assert_eq!(d.keys[0].steam_app_id, Some(504230), "{name}");
    assert_eq!(d.trades.len(), 1, "{name}: trades");
    let t = &d.trades[0];
    assert_eq!(t.status, TradeStatus::Completed, "{name}");
    assert_eq!(t.partner_name, "fixture-partner", "{name}");
    assert_eq!(t.key_ids, [d.keys[2].id], "{name}");
    assert_eq!(d.keys[2].trade_id, Some(t.id), "{name}");
    assert_eq!(d.settings.date_format, DateFormat::DayMonthYear, "{name}");
    assert_eq!(d.settings.auto_lock_minutes, 15, "{name}");
    assert_eq!(
        d.settings.steam_id.as_deref(),
        Some("76561190000000000"),
        "{name}"
    );
    assert_eq!(
        d.api_keys.steam.as_ref().map(|s| s.expose().to_string()),
        Some("00000000000000000000000000000000".into()),
        "{name}"
    );
}

fn fixtures() -> Vec<PathBuf> {
    let mut v: Vec<PathBuf> = std::fs::read_dir(fixtures_dir())
        .expect("tests/fixtures/vaults exists")
        .filter_map(|e| e.ok().map(|e| e.path()))
        .filter(|p| p.extension().is_some_and(|x| x == "kkvault"))
        .collect();
    v.sort();
    v
}

#[test]
fn every_released_vault_still_opens() {
    let files = fixtures();
    assert!(!files.is_empty(), "no fixtures in {:?}", fixtures_dir());
    for file in &files {
        let (vault, data) = Vault::open(file, FIXTURE_PASSWORD)
            .unwrap_or_else(|e| panic!("{} no longer opens: {e}", file.display()));
        assert_eq!(
            vault.params(),
            KdfParams::INSECURE_FAST_FOR_TESTS,
            "{}: fixtures use the fast test KDF",
            file.display()
        );
        check(file, &data);
    }
}

#[test]
fn fixture_names_are_versions() {
    for file in fixtures() {
        let stem = file.file_stem().unwrap().to_string_lossy().to_string();
        let v = stem
            .strip_prefix('v')
            .unwrap_or_else(|| panic!("{stem}: name fixtures v<version>.kkvault"));
        semver_like(v).unwrap_or_else(|| panic!("{stem}: not a version"));
    }
}

fn semver_like(v: &str) -> Option<()> {
    let parts: Vec<&str> = v.split('.').collect();
    (parts.len() == 3 && parts.iter().all(|p| p.parse::<u64>().is_ok())).then_some(())
}

#[test]
fn generated_data_passes_the_check() {
    check(Path::new("fixture_data"), &fixture_data("0.0.0"));
}

/// Writes `tests/fixtures/vaults/v<this version>.kkvault`. Run before tagging a release (see the
/// module docs); refuses to overwrite an existing fixture.
#[test]
#[ignore = "generator: run by hand before a release"]
fn generate_fixture() {
    let version = env!("CARGO_PKG_VERSION");
    let path = fixtures_dir().join(format!("v{version}.kkvault"));
    std::fs::create_dir_all(fixtures_dir()).unwrap();
    Vault::create(
        &path,
        FIXTURE_PASSWORD,
        KdfParams::INSECURE_FAST_FOR_TESTS,
        &fixture_data(version),
    )
    .unwrap_or_else(|e| panic!("{}: {e}", path.display()));
    println!("wrote {}", path.display());
}
