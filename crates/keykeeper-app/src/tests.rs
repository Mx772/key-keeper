//! Headless end-to-end tests: the real view and state, driven through RUNE's `Runtime` with
//! fast (insecure) KDF parameters and vaults in temp dirs.

use std::path::Path;
use std::time::{Duration, Instant};

use keykeeper_core::{KdfParams, KeyStatus, Vault, VaultData};
use rune::{Env, Key, KeyEvent, Mods, Runtime, TextEdit};

use crate::app::{AppState, Confirm, LockMode, Page, Screen};
use crate::config::Config;
use crate::{demo, ui};

const SIZE: (f32, f32) = (1280.0, 800.0);

fn state_for(vault: &Path) -> AppState {
    let config = Config {
        vault_path: Some(vault.to_path_buf()),
        ..Config::default()
    };
    let mut s = AppState::new(config, None, KdfParams::INSECURE_FAST_FOR_TESTS);
    // Tests never talk to GitHub.
    s.updates.source = std::sync::Arc::new(crate::update::NoUpdates);
    s
}

fn runtime(s: AppState) -> Runtime<AppState> {
    runtime_sized(s, SIZE)
}

/// Settings is a long scrolling page; tall tests keep every control on screen.
const TALL: (f32, f32) = (1280.0, 3000.0);

fn runtime_sized(s: AppState, size: (f32, f32)) -> Runtime<AppState> {
    let mut rt = Runtime::new(s, ui::view, size, Env::new(true, 1.0));
    // What AppSpec wires in the real app.
    rt.on_key = Some(crate::on_key);
    rt.poll = Some(crate::poll);
    rt.frame(0.0);
    rt
}

fn demo_rt() -> Runtime<AppState> {
    let dir = std::env::temp_dir();
    let mut s = state_for(&dir.join("unused.kkvault"));
    demo::unlock(&mut s, demo::sample_data());
    runtime(s)
}

fn click(rt: &mut Runtime<AppState>, key: &str) {
    let (x, y) = rt
        .find(key)
        .unwrap_or_else(|| panic!("no element '{key}' on screen"));
    rt.pointer_down(x, y);
    rt.pointer_up(x, y);
    rt.frame(1.0 / 60.0);
}

fn poll(rt: &mut Runtime<AppState>, now: f64) {
    rt.update(|s, tx| {
        s.poll(now, tx);
    });
    rt.frame(1.0 / 60.0);
}

/// Unlocking runs on a thread; poll until it lands.
fn wait_for_unlock_result(rt: &mut Runtime<AppState>) {
    let start = Instant::now();
    while start.elapsed() < Duration::from_secs(10) {
        poll(rt, 0.0);
        let busy = matches!(&rt.state.screen, Screen::Locked(l) if l.busy);
        if !busy {
            return;
        }
        std::thread::sleep(Duration::from_millis(5));
    }
    panic!("unlock never finished");
}

fn ctrl(c: char) -> KeyEvent {
    KeyEvent {
        key: Key::Command(c),
        mods: Mods {
            cmd: true,
            ctrl: true,
            ..Mods::default()
        },
    }
}

fn shown(rt: &Runtime<AppState>) -> Vec<String> {
    let s = rt.state.session().expect("unlocked");
    s.rows
        .iter()
        .filter_map(|id| s.repo.key(*id))
        .map(|k| k.game_name.clone())
        .collect()
}

#[cfg(not(target_os = "macos"))]
#[test]
fn window_icon_is_a_valid_image() {
    rune::set_app_icon(crate::ICON_PNG).expect("icon-256.png decodes");
}

// ── lock screen ──────────────────────────────────────────────────────────────────────────────

#[test]
fn create_vault_then_lock_and_unlock_again() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("sub").join("keys.kkvault");
    let mut rt = runtime(state_for(&path));
    assert!(matches!(&rt.state.screen, Screen::Locked(l) if l.mode == LockMode::Create));

    rt.update(|s, _| {
        let l = s.lock_ui_mut().unwrap();
        l.password = TextEdit::new("correct horse");
        l.confirm = TextEdit::new("correct horse");
    });
    click(&mut rt, "lock-submit");
    wait_for_unlock_result(&mut rt);
    assert!(rt.state.is_unlocked(), "vault created and opened");
    assert!(path.exists());
    assert!(rt.find("library").is_some());
    assert!(
        rt.find("library-empty").is_some(),
        "new vault shows the empty state"
    );

    rt.key(ctrl('l'));
    rt.frame(1.0 / 60.0);
    assert!(matches!(&rt.state.screen, Screen::Locked(l) if l.mode == LockMode::Unlock));

    rt.update(|s, _| s.lock_ui_mut().unwrap().password = TextEdit::new("wrong password"));
    click(&mut rt, "lock-submit");
    wait_for_unlock_result(&mut rt);
    assert!(!rt.state.is_unlocked());
    assert!(rt.find("lock-error").is_some());
    assert_eq!(rt.state.session().map(|_| ()), None);

    rt.update(|s, _| s.lock_ui_mut().unwrap().password = TextEdit::new("correct horse"));
    click(&mut rt, "lock-submit");
    wait_for_unlock_result(&mut rt);
    assert!(rt.state.is_unlocked());
}

#[test]
fn create_requires_matching_passwords_of_eight_chars() {
    let dir = tempfile::tempdir().unwrap();
    let mut rt = runtime(state_for(&dir.path().join("v.kkvault")));
    rt.update(|s, _| {
        let l = s.lock_ui_mut().unwrap();
        l.password = TextEdit::new("short");
        l.confirm = TextEdit::new("short");
    });
    click(&mut rt, "lock-submit");
    assert!(
        matches!(&rt.state.screen, Screen::Locked(l) if l.error.as_deref() == Some("Use at least 8 characters") && !l.busy)
    );
    rt.update(|s, _| {
        let l = s.lock_ui_mut().unwrap();
        l.password = TextEdit::new("long enough");
        l.confirm = TextEdit::new("long enouhg");
    });
    click(&mut rt, "lock-submit");
    assert!(
        matches!(&rt.state.screen, Screen::Locked(l) if l.error.as_deref() == Some("Passwords don't match"))
    );
}

// ── library ──────────────────────────────────────────────────────────────────────────────────

#[test]
fn status_chips_and_search_filter_rows() {
    let mut rt = demo_rt();
    assert_eq!(shown(&rt).len(), 18);

    click(&mut rt, "filter-traded");
    poll(&mut rt, 0.0);
    let mut traded = shown(&rt);
    traded.sort();
    assert_eq!(traded, ["Hollow Knight", "Undertale"]);

    click(&mut rt, "filter-all");
    rt.update(|s, _| {
        let x = s.session_mut().unwrap();
        x.search = TextEdit::new("humble choice");
        x.filters_changed();
    });
    poll(&mut rt, 0.0);
    assert!(shown(&rt).len() >= 5);
    assert!(shown(&rt).iter().all(|n| n != "Celeste"));

    click(&mut rt, "filter-hide-gone");
    poll(&mut rt, 0.0);
    assert!(
        shown(&rt)
            .iter()
            .all(|n| n != "Hollow Knight" && n != "Outer Wilds")
    );
}

#[test]
fn add_edit_delete_and_undo_through_the_ui() {
    let mut rt = demo_rt();
    rt.key(ctrl('n'));
    rt.frame(1.0 / 60.0);
    assert!(rt.find("detail").is_some(), "Ctrl+N opens the editor");

    rt.update(|s, _| {
        let f = s.session_mut().unwrap().form.as_mut().unwrap();
        f.name = TextEdit::new("Animal Well");
        f.key = TextEdit::new("ANIM4-LWELL-12345");
        f.deadline = TextEdit::new("2/1/2027");
    });
    rt.frame(1.0 / 60.0);
    click(&mut rt, "form-save");
    poll(&mut rt, 0.0);
    let id = {
        let s = rt.state.session().unwrap();
        let k = s.repo.find_by_key("ANIM4-LWELL-12345").expect("added");
        assert_eq!(k.redemption_deadline, Some(jiff::civil::date(2027, 2, 1)));
        k.id
    };
    assert_eq!(shown(&rt).len(), 19);

    // Duplicate keys are refused.
    rt.update(|s, _| {
        s.new_key();
        let f = s.session_mut().unwrap().form.as_mut().unwrap();
        f.name = TextEdit::new("Again");
        f.key = TextEdit::new("anim4 lwell 12345");
    });
    rt.frame(1.0 / 60.0);
    click(&mut rt, "form-save");
    assert_eq!(rt.state.session().unwrap().repo.keys().len(), 19);
    assert!(rt.state.toasts.iter().any(|t| t.title == "Duplicate key"));

    // Edit status via the form.
    rt.update(|s, _| {
        s.open_key(id);
        s.session_mut().unwrap().form.as_mut().unwrap().status = KeyStatus::Reserved;
    });
    rt.frame(1.0 / 60.0);
    click(&mut rt, "form-save");
    assert_eq!(
        rt.state.session().unwrap().repo.key(id).unwrap().status,
        KeyStatus::Reserved
    );

    // Delete with confirmation, then undo.
    click(&mut rt, "form-delete");
    assert_eq!(
        rt.state.session().unwrap().confirm,
        Some(Confirm::DeleteKeys(vec![id]))
    );
    click(&mut rt, "confirm-ok");
    poll(&mut rt, 0.0);
    assert!(rt.state.session().unwrap().repo.key(id).is_none());
    assert!(rt.find("detail").is_none(), "editor closes with its key");
    rt.key(ctrl('z'));
    poll(&mut rt, 0.0);
    assert_eq!(
        rt.state.session().unwrap().repo.key(id).unwrap().status,
        KeyStatus::Reserved
    );
}

#[test]
fn bulk_status_change_is_one_undo_step() {
    let mut rt = demo_rt();
    let ids: Vec<_> = rt
        .state
        .session()
        .unwrap()
        .rows
        .iter()
        .take(3)
        .copied()
        .collect();
    rt.update(|s, _| s.set_status(&ids, KeyStatus::Gifted));
    let s = rt.state.session().unwrap();
    assert!(
        ids.iter()
            .all(|id| s.repo.key(*id).unwrap().status == KeyStatus::Gifted)
    );
    assert!(
        ids.iter()
            .all(|id| s.repo.key(*id).unwrap().status_date.is_some()),
        "gone keys get today's date"
    );
    rt.update(|s, _| s.undo());
    let s = rt.state.session().unwrap();
    assert!(
        ids.iter()
            .all(|id| s.repo.key(*id).unwrap().status != KeyStatus::Gifted)
    );
}

#[test]
fn escape_closes_editor_and_delete_key_asks_first() {
    let mut rt = demo_rt();
    let id = rt.state.session().unwrap().rows[0];
    rt.update(|s, _| {
        s.open_key(id);
        s.session_mut().unwrap().table.select(id.to_string());
    });
    rt.frame(1.0 / 60.0);
    rt.key(KeyEvent {
        key: Key::Escape,
        mods: Mods::default(),
    });
    rt.frame(1.0 / 60.0);
    assert!(rt.state.session().unwrap().form.is_none());
    rt.key(KeyEvent {
        key: Key::Delete,
        mods: Mods::default(),
    });
    rt.frame(1.0 / 60.0);
    assert!(rt.find("confirm-actions").is_some());
    assert_eq!(
        rt.state.session().unwrap().repo.keys().len(),
        18,
        "nothing deleted until confirmed"
    );
}

#[test]
fn shortcuts_overlay_opens_from_keys_and_sidebar_and_esc_closes_it() {
    let mut rt = demo_rt();
    assert!(rt.find("shortcut-list").is_none());
    let open = |rt: &Runtime<AppState>| rt.state.session().unwrap().shortcuts_open;

    rt.key(KeyEvent {
        key: Key::Function(1),
        mods: Mods::default(),
    });
    rt.frame(1.0 / 60.0);
    assert!(open(&rt), "F1 opens the overlay");
    assert!(rt.find("shortcut-list").is_some());
    assert!(
        rt.find(&format!("shortcut-{}", ui::SHORTCUT_COUNT - 1))
            .is_some()
    );

    rt.key(KeyEvent {
        key: Key::Escape,
        mods: Mods::default(),
    });
    rt.frame(1.0 / 60.0);
    assert!(!open(&rt), "Esc closes it");
    assert!(rt.find("shortcut-list").is_none());

    rt.key(ctrl('/'));
    rt.frame(1.0 / 60.0);
    assert!(open(&rt), "Ctrl+/ opens it");
    rt.key(ctrl('/'));
    rt.frame(1.0 / 60.0);
    assert!(!open(&rt), "and toggles it shut");

    click(&mut rt, "nav-shortcuts");
    assert!(open(&rt), "the sidebar entry opens it");
    assert!(rt.find("shortcut-list").is_some());

    // Esc with the overlay over an open editor closes only the overlay.
    rt.update(|s, _| s.new_key());
    rt.frame(1.0 / 60.0);
    rt.key(KeyEvent {
        key: Key::Escape,
        mods: Mods::default(),
    });
    rt.frame(1.0 / 60.0);
    assert!(!open(&rt));
    assert!(rt.state.session().unwrap().form.is_some());
}

// ── persistence ──────────────────────────────────────────────────────────────────────────────

#[test]
fn autosave_after_delay_and_auto_lock_after_idle() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("v.kkvault");
    let mut s = state_for(&path);
    let v = Vault::create(
        &path,
        "pw",
        KdfParams::INSECURE_FAST_FOR_TESTS,
        &VaultData::default(),
    )
    .unwrap();
    s.open_session(v, VaultData::default());
    let mut rt = runtime(s);

    rt.update(|s, _| {
        s.update_settings(|x| x.auto_lock_minutes = 1);
        s.new_key();
        let f = s.session_mut().unwrap().form.as_mut().unwrap();
        f.name = TextEdit::new("Tunic");
        f.key = TextEdit::new("TUN1C-F0X99-AAAA1");
        s.save_form();
    });
    poll(&mut rt, 10.0);
    assert!(
        rt.state.session().unwrap().is_dirty(),
        "not saved before the delay"
    );
    poll(&mut rt, 10.0 + crate::app::AUTOSAVE_DELAY + 0.1);
    assert!(!rt.state.session().unwrap().is_dirty());
    let (_, on_disk) = Vault::open(&path, "pw").unwrap();
    assert_eq!(on_disk.keys.len(), 1);
    assert_eq!(on_disk.settings.auto_lock_minutes, 1);

    rt.update(|s, _| s.last_activity = 20.0);
    poll(&mut rt, 20.0 + 59.0);
    assert!(rt.state.is_unlocked());
    poll(&mut rt, 20.0 + 61.0);
    assert!(!rt.state.is_unlocked(), "locked after a minute idle");
    assert!(rt.find("lock").is_some());
}

#[test]
fn locking_saves_pending_changes() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("v.kkvault");
    let mut s = state_for(&path);
    let v = Vault::create(
        &path,
        "pw",
        KdfParams::INSECURE_FAST_FOR_TESTS,
        &VaultData::default(),
    )
    .unwrap();
    s.open_session(v, VaultData::default());
    let mut rt = runtime(s);
    rt.update(|s, _| s.import_text("t", "Hades: HD5S2-77KLM-NB3VC\n"));
    rt.update(|s, _| s.commit_import());
    click(&mut rt, "nav-lock");
    assert!(!rt.state.is_unlocked());
    let (_, data) = Vault::open(&path, "pw").unwrap();
    assert_eq!(data.keys.len(), 1);
}

#[test]
fn change_password() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("v.kkvault");
    let mut s = state_for(&path);
    let v = Vault::create(
        &path,
        "old password",
        KdfParams::INSECURE_FAST_FOR_TESTS,
        &VaultData::default(),
    )
    .unwrap();
    s.open_session(v, VaultData::default());
    s.go(Page::Settings);
    let mut rt = runtime_sized(s, TALL);
    let set = |rt: &mut Runtime<AppState>, old: &str, new: &str, confirm: &str| {
        let (old, new, confirm) = (old.to_string(), new.to_string(), confirm.to_string());
        rt.update(move |s, _| {
            let ui = &mut s.session_mut().unwrap().settings_ui;
            ui.old_pw = TextEdit::new(old);
            ui.new_pw = TextEdit::new(new);
            ui.confirm_pw = TextEdit::new(confirm);
        });
        click(rt, "pw-change");
        rt.state
            .session()
            .unwrap()
            .settings_ui
            .pw_message
            .clone()
            .unwrap()
    };
    assert_eq!(
        set(&mut rt, "nope", "new password", "new password"),
        (false, "Current password is wrong".into())
    );
    assert_eq!(
        set(&mut rt, "old password", "new password", "different"),
        (false, "New passwords don't match".into())
    );
    assert!(set(&mut rt, "old password", "new password", "new password").0);
    assert!(Vault::open(&path, "new password").is_ok());
    assert!(Vault::open(&path, "old password").is_err());
}

// ── import ───────────────────────────────────────────────────────────────────────────────────

#[test]
fn import_wizard_previews_remaps_and_commits() {
    let mut rt = demo_rt();
    click(&mut rt, "nav-import");
    assert!(rt.find("import-paste-text").is_some());
    rt.update(|s, _| {
        s.session_mut().unwrap().import.paste = TextEdit::new(
            "Animal Well: ANIM4-LWELL-12345\nUndertale: UNDR7-TALE0-00001\nno key here\n",
        );
    });
    click(&mut rt, "import-preview");
    let p = rt.state.session().unwrap().import.preview.clone().unwrap();
    assert_eq!(
        (p.new_count(), p.duplicate_count(), p.error_count()),
        (1, 1, 1)
    );
    assert!(rt.find("import-commit").is_some());

    click(&mut rt, "import-commit");
    poll(&mut rt, 0.0);
    let s = rt.state.session().unwrap();
    assert_eq!(s.page, Page::Library);
    assert_eq!(s.repo.keys().len(), 19);
    assert!(
        rt.state
            .toasts
            .iter()
            .any(|t| t.title.starts_with("Imported 1 key"))
    );
}

#[test]
fn import_date_format_and_mapping_changes_rebuild_preview() {
    let mut rt = demo_rt();
    rt.update(|s, _| {
        s.go(Page::Import);
        s.import_text("x.csv", "Game,Key,Date\nA,AAAAA-11111-AAAAA,1/7/2026\n");
    });
    let date = |rt: &Runtime<AppState>| {
        rt.state
            .session()
            .unwrap()
            .import
            .preview
            .as_ref()
            .unwrap()
            .rows[0]
            .key
            .as_ref()
            .unwrap()
            .status_date
    };
    assert_eq!(date(&rt), Some(jiff::civil::date(2026, 1, 7)));
    rt.update(|s, _| s.set_import_date_format(keykeeper_core::DateFormat::DayMonthYear));
    assert_eq!(date(&rt), Some(jiff::civil::date(2026, 7, 1)));
    rt.update(|s, _| s.set_column_mapping(2, None));
    assert_eq!(date(&rt), None);
}

// ── everything renders ───────────────────────────────────────────────────────────────────────

#[test]
fn every_scene_and_theme_renders() {
    for (name, scene) in crate::SCENES {
        for theme in crate::theme::ThemeId::ALL {
            let dir = tempfile::tempdir().unwrap();
            let mut s = state_for(&dir.path().join("v.kkvault"));
            scene(&mut s);
            s.theme = theme;
            let mut rt = Runtime::new(s, ui::view, SIZE, Env::new(true, 1.0));
            rt.frame(0.0);
            assert!(rt.find("app").is_some(), "scene {name} with {theme:?}");
        }
    }
}

// ── trade flow ───────────────────────────────────────────────────────────────────────────────

fn available_ids(rt: &Runtime<AppState>, n: usize) -> Vec<uuid::Uuid> {
    let s = rt.state.session().unwrap();
    s.rows
        .iter()
        .copied()
        .filter(|id| s.repo.key(*id).unwrap().status == KeyStatus::Available)
        .take(n)
        .collect()
}

#[test]
fn cart_checkout_send_complete_and_rep() {
    let mut rt = demo_rt();
    let ids = available_ids(&rt, 2);
    rt.update(|s, _| {
        let x = s.session_mut().unwrap();
        for id in &ids {
            x.table.select(id.to_string());
        }
    });
    // Select both (select() replaces, so add the first back via the API used by the bulk bar).
    rt.update(|s, _| s.add_to_cart(&ids));
    poll(&mut rt, 0.0);
    assert_eq!(rt.state.session().unwrap().repo.cart_len(), 2);
    assert!(ids.iter().all(
        |id| rt.state.session().unwrap().repo.key(*id).unwrap().status == KeyStatus::Reserved
    ));

    click(&mut rt, "nav-cart");
    poll(&mut rt, 0.0);
    assert!(rt.find("cart-items").is_some());
    assert!(rt.find("copy-checkout").is_some());
    rt.update(|s, _| {
        let f = s
            .session_mut()
            .unwrap()
            .trade_form
            .as_mut()
            .expect("cart form");
        f.partner = TextEdit::new("lumen");
        f.trade_url = TextEdit::new("https://lestrades.com/offer/400001/");
        f.received = TextEdit::new("Animal Well: ANIM4-LWELL-12345\nBalatro");
    });
    click(&mut rt, "cart-send");
    poll(&mut rt, 0.0);
    let trade_id = {
        let s = rt.state.session().unwrap();
        assert_eq!(s.page, Page::Trades);
        assert!(s.repo.cart().is_none());
        let t = s.repo.trade(s.selected_trade.unwrap()).unwrap();
        assert_eq!(t.status, keykeeper_core::TradeStatus::Pending);
        assert_eq!(t.partner_name, "lumen", "details were saved on send");
        assert_eq!(t.received.len(), 2);
        t.id
    };
    assert!(rt.find("trade-complete").is_some());

    click(&mut rt, "trade-complete");
    poll(&mut rt, 0.0);
    let s = rt.state.session().unwrap();
    let t = s.repo.trade(trade_id).unwrap();
    assert_eq!(t.status, keykeeper_core::TradeStatus::Completed);
    assert!(
        ids.iter()
            .all(|id| s.repo.key(*id).unwrap().status == KeyStatus::Traded)
    );
    assert!(
        s.repo.find_by_key("ANIM4-LWELL-12345").is_some(),
        "received games added by default"
    );
    assert!(s.repo.keys().iter().any(|k| k.game_name == "Balatro"));
    assert!(rt.find("rep-text").is_some());
    assert!(
        s.repo
            .rep_text(t)
            .unwrap()
            .contains("https://lestrades.com/offer/400001/")
    );

    click(&mut rt, "copy-rep");
    assert!(
        rt.state
            .toasts
            .iter()
            .any(|t| t.title.starts_with("Reputation text copied"))
    );
    click(&mut rt, "rep-posted");
    assert!(
        rt.state
            .session()
            .unwrap()
            .repo
            .trade(trade_id)
            .unwrap()
            .rep_posted
    );
}

#[test]
fn cancel_pending_trade_restores_keys() {
    let mut rt = demo_rt();
    let ids = available_ids(&rt, 1);
    rt.update(|s, _| {
        s.add_to_cart(&ids);
        s.go(Page::Cart);
    });
    poll(&mut rt, 0.0);
    click(&mut rt, "cart-send");
    poll(&mut rt, 0.0);
    click(&mut rt, "trade-cancel");
    assert!(rt.find("confirm-actions").is_some());
    click(&mut rt, "confirm-ok");
    poll(&mut rt, 0.0);
    let s = rt.state.session().unwrap();
    assert_eq!(s.repo.key(ids[0]).unwrap().status, KeyStatus::Available);
    assert!(
        s.repo
            .trades()
            .iter()
            .any(|t| t.status == keykeeper_core::TradeStatus::Cancelled)
    );
}

#[test]
fn cart_remove_clear_and_unavailable_keys() {
    let mut rt = demo_rt();
    let ids = available_ids(&rt, 3);
    let traded = rt
        .state
        .session()
        .unwrap()
        .repo
        .keys()
        .iter()
        .find(|k| k.status == KeyStatus::Traded)
        .unwrap()
        .id;
    let mut all = ids.clone();
    all.push(traded);
    rt.update(|s, _| s.add_to_cart(&all));
    assert!(
        rt.state
            .toasts
            .iter()
            .any(|t| t.title == "Added 3 to cart, skipped 1 unavailable")
    );
    rt.update(|s, _| s.go(Page::Cart));
    poll(&mut rt, 0.0);
    click(&mut rt, &format!("cart-remove-{}", ids[0]));
    assert_eq!(rt.state.session().unwrap().repo.cart_len(), 2);
    click(&mut rt, "cart-clear");
    poll(&mut rt, 0.0);
    assert_eq!(rt.state.session().unwrap().repo.cart_len(), 0);
    assert!(rt.find("cart-empty").is_some());
}

#[test]
fn templates_validate_save_and_drive_checkout() {
    let mut s = state_for(&std::env::temp_dir().join("unused.kkvault"));
    demo::unlock(&mut s, demo::sample_data());
    let mut rt = runtime_sized(s, TALL);
    rt.update(|s, _| s.go(Page::Settings));
    rt.frame(1.0 / 60.0);
    rt.update(|s, _| s.session_mut().unwrap().templates_ui.rep = TextEdit::new("{key} {oops"));
    rt.frame(1.0 / 60.0);
    click(&mut rt, "tpl-save");
    assert_eq!(
        rt.state.session().unwrap().repo.data().templates,
        keykeeper_core::Templates::default(),
        "invalid templates aren't saved"
    );

    rt.update(|s, _| {
        let ui = &mut s.session_mut().unwrap().templates_ui;
        ui.checkout = TextEdit::new("- {name} | {key}");
        ui.rep = TextEdit::new("+rep {partner} — {games}");
    });
    rt.frame(1.0 / 60.0);
    click(&mut rt, "tpl-save");
    let s = rt.state.session().unwrap();
    assert_eq!(s.repo.data().templates.checkout_line, "- {name} | {key}");

    let ids = available_ids(&rt, 1);
    rt.update(|s, _| s.add_to_cart(&ids));
    let s = rt.state.session().unwrap();
    let text = s.repo.checkout_text(s.repo.cart().unwrap(), false).unwrap();
    assert!(text.starts_with("- ") && text.contains(" | "), "{text}");

    rt.update(|s, _| s.reset_templates());
    assert_eq!(
        rt.state.session().unwrap().templates_ui.to_templates(),
        keykeeper_core::Templates::default()
    );
}

#[test]
fn trade_details_survive_navigation_and_lock() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("v.kkvault");
    let mut s = state_for(&path);
    let data = demo::sample_data();
    let v = Vault::create(&path, "pw", KdfParams::INSECURE_FAST_FOR_TESTS, &data).unwrap();
    s.open_session(v, data);
    let mut rt = runtime(s);
    let ids = available_ids(&rt, 1);
    rt.update(|s, _| {
        s.add_to_cart(&ids);
        s.go(Page::Cart);
    });
    poll(&mut rt, 0.0);
    rt.update(|s, _| {
        s.session_mut()
            .unwrap()
            .trade_form
            .as_mut()
            .unwrap()
            .partner = TextEdit::new("nomad")
    });
    rt.key(ctrl('l'));
    rt.frame(1.0 / 60.0);
    let (_, data) = Vault::open(&path, "pw").unwrap();
    let cart = data
        .trades
        .iter()
        .find(|t| t.status == keykeeper_core::TradeStatus::Draft)
        .unwrap();
    assert_eq!(
        cart.partner_name, "nomad",
        "unsaved form text is flushed on lock"
    );
}

// ── Steam ────────────────────────────────────────────────────────────────────────────────────

use keykeeper_steam::{AppMatch, Review, SteamApi, SteamError};

/// Canned Steam responses.
struct FakeSteam;

impl SteamApi for FakeSteam {
    fn search(&self, term: &str) -> Result<Vec<AppMatch>, SteamError> {
        Ok(match term {
            "Animal Well" => vec![
                AppMatch {
                    app_id: 813230,
                    name: "ANIMAL WELL".into(),
                },
                AppMatch {
                    app_id: 9,
                    name: "Animal Well OST".into(),
                },
            ],
            "Doom" => vec![
                AppMatch {
                    app_id: 379720,
                    name: "DOOM".into(),
                },
                AppMatch {
                    app_id: 2280,
                    name: "Doom".into(),
                },
            ],
            "Offline" => return Err(SteamError::Network("unreachable".into())),
            _ => Vec::new(),
        })
    }
    fn reviews(&self, app_ids: &[u32]) -> Result<Vec<Review>, SteamError> {
        Ok(app_ids
            .iter()
            .map(|&app_id| Review {
                app_id,
                desc: "Very Positive".into(),
                percent: 88,
                total: 1234,
            })
            .collect())
    }
    fn resolve_profile(&self, input: &str) -> Result<u64, SteamError> {
        if input.contains("nobody") {
            Err(SteamError::ProfileNotFound(input.into()))
        } else {
            Ok(76561197960287930)
        }
    }
    fn wishlist(&self, _: u64) -> Result<Vec<u32>, SteamError> {
        Ok(vec![813230])
    }
    fn owned_games(&self, key: &str, _: u64) -> Result<Vec<u32>, SteamError> {
        if key == "good-key" {
            Ok(vec![379720])
        } else {
            Err(SteamError::BadApiKey)
        }
    }
}

fn steam_rt(names: &[&str]) -> Runtime<AppState> {
    let mut s = state_for(&std::env::temp_dir().join("unused.kkvault"));
    s.steam = std::sync::Arc::new(FakeSteam);
    let mut data = VaultData::default();
    for (i, n) in names.iter().enumerate() {
        data.keys.push(keykeeper_core::GameKey::new(
            *n,
            format!("AAAAA-BBBBB-{i:05}"),
        ));
    }
    demo::unlock(&mut s, data);
    runtime_sized(s, TALL)
}

/// Poll until background Steam work (including chained jobs) is done.
fn wait_for_steam(rt: &mut Runtime<AppState>) {
    let start = Instant::now();
    loop {
        poll(rt, 0.0);
        if rt.state.job.is_none() {
            return;
        }
        assert!(
            start.elapsed() < Duration::from_secs(10),
            "steam job never finished"
        );
        std::thread::sleep(Duration::from_millis(5));
    }
}

fn key_named<'a>(rt: &'a Runtime<AppState>, name: &str) -> &'a keykeeper_core::GameKey {
    rt.state
        .session()
        .unwrap()
        .repo
        .keys()
        .iter()
        .find(|k| k.game_name == name)
        .unwrap()
}

#[test]
fn find_on_steam_exact_match_applies_id_and_rating() {
    let mut rt = steam_rt(&["Animal Well"]);
    let id = key_named(&rt, "Animal Well").id;
    rt.update(|s, _| s.open_key(id));
    rt.frame(1.0 / 60.0);
    click(&mut rt, "form-find-steam");
    wait_for_steam(&mut rt);
    let k = key_named(&rt, "Animal Well");
    assert_eq!(
        k.steam_app_id,
        Some(813230),
        "ANIMAL WELL matches case-insensitively"
    );
    assert_eq!(
        k.steam
            .as_ref()
            .and_then(|m| m.rating.as_ref())
            .map(|r| r.percent),
        Some(88),
        "rating fetched right after"
    );
    assert_eq!(
        rt.state
            .session()
            .unwrap()
            .form
            .as_ref()
            .unwrap()
            .app_id
            .text,
        "813230"
    );
    assert!(rt.find("form-steam-meta").is_some());
}

#[test]
fn ambiguous_search_offers_candidates() {
    let mut rt = steam_rt(&["Doom"]);
    let id = key_named(&rt, "Doom").id;
    rt.update(|s, _| s.open_key(id));
    rt.frame(1.0 / 60.0);
    click(&mut rt, "form-find-steam");
    wait_for_steam(&mut rt);
    assert!(rt.find("steam-candidates").is_some());
    assert_eq!(key_named(&rt, "Doom").steam_app_id, None);
    click(&mut rt, "candidate-2280");
    wait_for_steam(&mut rt);
    assert_eq!(key_named(&rt, "Doom").steam_app_id, Some(2280));
    assert!(rt.find("steam-candidates").is_none());
}

#[test]
fn bulk_match_then_filter_unmatched() {
    let mut rt = steam_rt(&["Animal Well", "Doom", "Unknown Indie"]);
    click(&mut rt, "steam-match-all");
    wait_for_steam(&mut rt);
    assert_eq!(key_named(&rt, "Animal Well").steam_app_id, Some(813230));
    assert_eq!(
        key_named(&rt, "Doom").steam_app_id,
        None,
        "ambiguous stays for a manual pick"
    );
    assert!(
        rt.state
            .toasts
            .iter()
            .any(|t| t.title.starts_with("Matched 1 key, 2 need a manual pick")),
        "{:?}",
        rt.state.toasts
    );
    click(&mut rt, "filter-no-steam");
    poll(&mut rt, 0.0);
    let mut names = shown(&rt);
    names.sort();
    assert_eq!(names, ["Doom", "Unknown Indie"]);
    // Ratings were fetched for the matched key.
    assert!(
        key_named(&rt, "Animal Well")
            .steam
            .as_ref()
            .is_some_and(|m| m.rating.is_some())
    );
    // One undo step per batch (ratings, then the match).
    rt.update(|s, _| {
        s.undo();
        s.undo();
    });
    assert_eq!(key_named(&rt, "Animal Well").steam_app_id, None);
}

#[test]
fn network_errors_are_reported() {
    let mut rt = steam_rt(&["Offline"]);
    rt.update(|s, _| s.match_missing(&[]));
    wait_for_steam(&mut rt);
    assert!(rt.state.toasts.iter().any(|t| {
        t.error
            && t.detail
                .as_deref()
                .is_some_and(|d| d.contains("unreachable"))
    }));
}

#[test]
fn wishlist_and_library_check() {
    let mut rt = steam_rt(&["Animal Well", "Doom", "Celeste"]);
    rt.update(|s, _| {
        let x = s.session_mut().unwrap();
        for (name, app) in [
            ("Animal Well", 813230),
            ("Doom", 379720),
            ("Celeste", 504230),
        ] {
            let id = x
                .repo
                .keys()
                .iter()
                .find(|k| k.game_name == name)
                .unwrap()
                .id;
            x.repo
                .update_key(id, |k| k.steam_app_id = Some(app))
                .unwrap();
        }
        s.go(Page::Settings);
    });
    rt.frame(1.0 / 60.0);

    // Without an API key: wishlist only, everything else stays Unknown.
    rt.update(|s, _| {
        s.session_mut().unwrap().steam_ui.profile =
            TextEdit::new("https://steamcommunity.com/id/someone")
    });
    click(&mut rt, "steam-check");
    wait_for_steam(&mut rt);
    let own =
        |rt: &Runtime<AppState>, n: &str| key_named(rt, n).steam.as_ref().map(|m| m.ownership);
    assert_eq!(
        own(&rt, "Animal Well"),
        Some(keykeeper_core::Ownership::Wishlisted)
    );
    assert_eq!(own(&rt, "Doom"), Some(keykeeper_core::Ownership::Unknown));
    let status = rt.state.session().unwrap().steam_ui.status.clone().unwrap();
    assert!(
        status.0 && status.1.contains("Add a Web API key"),
        "{status:?}"
    );
    assert_eq!(
        rt.state.session().unwrap().settings().steam_id.as_deref(),
        Some("https://steamcommunity.com/id/someone"),
        "profile saved to the vault"
    );

    // With a key: owned and not-owned are known too.
    rt.update(|s, _| s.session_mut().unwrap().steam_ui.api_key = TextEdit::new("good-key"));
    click(&mut rt, "steam-check");
    wait_for_steam(&mut rt);
    assert_eq!(own(&rt, "Doom"), Some(keykeeper_core::Ownership::Owned));
    assert_eq!(
        own(&rt, "Celeste"),
        Some(keykeeper_core::Ownership::NotOwned)
    );
    assert!(
        rt.state.session().unwrap().repo.api_keys().steam.is_some(),
        "API key stored in the vault"
    );

    // A bad key is reported; the wishlist is still applied.
    rt.update(|s, _| s.session_mut().unwrap().steam_ui.api_key = TextEdit::new("bad"));
    click(&mut rt, "steam-check");
    wait_for_steam(&mut rt);
    let status = rt.state.session().unwrap().steam_ui.status.clone().unwrap();
    assert!(status.1.contains("API key was rejected"), "{status:?}");

    // Unknown profile.
    rt.update(|s, _| s.session_mut().unwrap().steam_ui.profile = TextEdit::new("nobody"));
    click(&mut rt, "steam-check");
    wait_for_steam(&mut rt);
    assert!(
        !rt.state
            .session()
            .unwrap()
            .steam_ui
            .status
            .clone()
            .unwrap()
            .0
    );
}

#[test]
fn ratings_respect_the_cache_unless_forced() {
    let mut rt = steam_rt(&["Animal Well"]);
    let id = key_named(&rt, "Animal Well").id;
    rt.update(|s, _| {
        s.session_mut()
            .unwrap()
            .repo
            .update_key(id, |k| k.steam_app_id = Some(813230))
            .unwrap();
        s.refresh_ratings(false);
    });
    wait_for_steam(&mut rt);
    let first = key_named(&rt, "Animal Well").steam.clone().unwrap();
    rt.update(|s, _| s.refresh_ratings(false));
    assert!(rt.state.job.is_none(), "fresh ratings aren't refetched");
    rt.update(|s, _| s.refresh_ratings(true));
    assert!(rt.state.job.is_some());
    wait_for_steam(&mut rt);
    assert!(
        key_named(&rt, "Animal Well")
            .steam
            .as_ref()
            .unwrap()
            .fetched_at
            >= first.fetched_at
    );
}

// ── updates ──────────────────────────────────────────────────────────────────────────────────

use crate::update::{Release, STARTUP_DELAY_SECS, UpdateError, UpdateSource};
use crate::update_actions::{CheckStatus, Phase};
use std::sync::{Arc, Mutex};

/// Scripted update source; records installs instead of running anything.
struct FakeUpdates {
    latest: Mutex<Result<Release, UpdateError>>,
    download: Result<Vec<u8>, UpdateError>,
    installed: Mutex<Vec<(String, Vec<u8>)>>,
    checks: Mutex<u32>,
}

impl FakeUpdates {
    fn offering(version: &str) -> Arc<FakeUpdates> {
        Arc::new(FakeUpdates {
            latest: Mutex::new(Ok(release(version))),
            download: Ok(b"installer bytes".to_vec()),
            installed: Mutex::new(Vec::new()),
            checks: Mutex::new(0),
        })
    }
}

fn release(version: &str) -> Release {
    Release::new(
        semver::Version::parse(version).unwrap(),
        "## Notes\n- Something new",
    )
}

impl UpdateSource for FakeUpdates {
    fn latest(&self) -> Result<Release, UpdateError> {
        *self.checks.lock().unwrap() += 1;
        self.latest.lock().unwrap().clone()
    }
    fn download(
        &self,
        _: &Release,
        progress: &(dyn Fn(u64, Option<u64>) + Sync),
    ) -> Result<Vec<u8>, UpdateError> {
        progress(15, Some(15));
        self.download.clone()
    }
    fn install(&self, r: &Release, bytes: Vec<u8>) -> Result<(), UpdateError> {
        self.installed
            .lock()
            .unwrap()
            .push((r.version.to_string(), bytes));
        Ok(())
    }
}

/// A demo session (unlocked) with `src` as the update source and a real config file.
fn update_rt(src: Arc<FakeUpdates>, dir: &Path) -> Runtime<AppState> {
    let mut s = state_for(&dir.join("unused.kkvault"));
    s.config_path = Some(dir.join("config.toml"));
    demo::unlock(&mut s, demo::sample_data());
    s.updates.source = src;
    s.updates.current = semver::Version::new(0, 1, 0);
    runtime(s)
}

/// Poll until background update work is done.
fn settle_updates(rt: &mut Runtime<AppState>, now: f64) {
    let start = Instant::now();
    loop {
        poll(rt, now);
        let u = &rt.state.updates;
        if !u.checking && !matches!(u.phase, Phase::Downloading { .. }) {
            poll(rt, now);
            return;
        }
        assert!(
            start.elapsed() < Duration::from_secs(10),
            "update work hung"
        );
        std::thread::sleep(Duration::from_millis(5));
    }
}

#[test]
fn automatic_check_shows_banner_then_later_and_skip() {
    let dir = tempfile::tempdir().unwrap();
    let src = FakeUpdates::offering("0.2.0");
    let mut rt = update_rt(src.clone(), dir.path());

    settle_updates(&mut rt, 1.0);
    assert_eq!(*src.checks.lock().unwrap(), 0, "no check right at startup");
    assert!(rt.find("update-banner").is_none());

    settle_updates(&mut rt, STARTUP_DELAY_SECS + 0.1);
    assert_eq!(*src.checks.lock().unwrap(), 1);
    assert!(rt.find("update-banner").is_some(), "banner after the check");
    assert!(rt.state.is_unlocked(), "non-modal: nothing else changed");

    // What's new opens the notes; Close shuts them.
    click(&mut rt, "update-notes");
    assert!(rt.find("update-notes-text").is_some());
    click(&mut rt, "update-notes-close");
    assert!(rt.find("update-notes-text").is_none());

    // Later hides it, also across later automatic checks.
    click(&mut rt, "update-later");
    assert!(rt.find("update-banner").is_none());
    settle_updates(
        &mut rt,
        STARTUP_DELAY_SECS + crate::update::CHECK_INTERVAL_SECS + 1.0,
    );
    assert_eq!(*src.checks.lock().unwrap(), 2, "checks again a day later");
    assert!(
        rt.find("update-banner").is_none(),
        "Later lasts until relaunch"
    );

    // "Check now" brings it back.
    rt.update(|s, _| s.go(Page::Settings));
    rt.update(|s, _| s.check_for_updates(true));
    settle_updates(&mut rt, 1.0e6);
    assert!(rt.find("update-banner").is_some());

    // Skip persists in config.toml and survives automatic checks.
    click(&mut rt, "update-skip");
    assert!(rt.find("update-banner").is_none());
    assert_eq!(rt.state.config.skipped_version.as_deref(), Some("0.2.0"));
    let saved = Config::load(&dir.path().join("config.toml"));
    assert_eq!(saved.skipped_version.as_deref(), Some("0.2.0"));
    rt.update(|s, _| s.check_for_updates(false));
    settle_updates(&mut rt, 1.0e6);
    assert!(rt.find("update-banner").is_none());
    assert!(matches!(rt.state.updates.status, CheckStatus::Skipped(_)));

    // A newer version than the skipped one is offered again.
    *src.latest.lock().unwrap() = Ok(release("0.3.0"));
    rt.update(|s, _| s.check_for_updates(false));
    settle_updates(&mut rt, 1.0e6);
    assert!(rt.find("update-banner").is_some());
}

#[test]
fn same_or_older_versions_are_not_offered() {
    let dir = tempfile::tempdir().unwrap();
    for v in ["0.1.0", "0.0.9", "0.2.0-beta.1"] {
        let mut rt = update_rt(FakeUpdates::offering(v), dir.path());
        rt.update(|s, _| s.check_for_updates(true));
        settle_updates(&mut rt, 0.0);
        assert!(rt.find("update-banner").is_none(), "{v}");
        assert_eq!(rt.state.updates.status, CheckStatus::UpToDate, "{v}");
    }
}

#[test]
fn check_errors_are_silent_unless_manual() {
    let dir = tempfile::tempdir().unwrap();
    let src = FakeUpdates::offering("0.2.0");
    *src.latest.lock().unwrap() = Err(UpdateError::Network("offline".into()));
    let mut rt = update_rt(src.clone(), dir.path());
    rt.update(|s, _| {
        s.go(Page::Settings);
        s.toasts.clear();
    });

    settle_updates(&mut rt, STARTUP_DELAY_SECS + 1.0);
    assert_eq!(*src.checks.lock().unwrap(), 1);
    assert!(rt.state.toasts.is_empty(), "automatic failure: no toast");
    assert_eq!(rt.state.updates.status, CheckStatus::NotChecked);
    assert!(rt.find("update-banner").is_none());

    // "Check now" in Settings (tall window: the card is at the bottom).
    let mut rt = {
        let mut s = state_for(&dir.path().join("unused.kkvault"));
        demo::unlock(&mut s, demo::sample_data());
        s.updates.source = src;
        s.go(Page::Settings);
        runtime_sized(s, TALL)
    };
    click(&mut rt, "update-check");
    settle_updates(&mut rt, 0.0);
    assert!(
        matches!(&rt.state.updates.status, CheckStatus::Failed(m) if m.contains("offline")),
        "{:?}",
        rt.state.updates.status
    );
    assert!(rt.find("update-status").is_some());
}

#[test]
fn turning_automatic_checks_off_stops_them() {
    let dir = tempfile::tempdir().unwrap();
    let src = FakeUpdates::offering("0.2.0");
    let mut s = state_for(&dir.path().join("unused.kkvault"));
    s.config_path = Some(dir.path().join("config.toml"));
    demo::unlock(&mut s, demo::sample_data());
    s.updates.source = src.clone();
    s.go(Page::Settings);
    let mut rt = runtime_sized(s, TALL);
    click(&mut rt, "update-auto");
    assert!(!rt.state.config.check_updates());
    assert_eq!(
        Config::load(&dir.path().join("config.toml")).check_updates,
        Some(false)
    );
    settle_updates(&mut rt, 1.0e6);
    assert_eq!(*src.checks.lock().unwrap(), 0);
}

#[test]
fn update_saves_the_vault_downloads_installs_and_quits() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("v.kkvault");
    let mut s = state_for(&path);
    let v = Vault::create(
        &path,
        "pw",
        KdfParams::INSECURE_FAST_FOR_TESTS,
        &VaultData::default(),
    )
    .unwrap();
    s.open_session(v, VaultData::default());
    let src = FakeUpdates::offering("0.2.0");
    s.updates.source = src.clone();
    s.updates.current = semver::Version::new(0, 1, 0);
    let mut rt = runtime(s);
    rt.update(|s, _| s.check_for_updates(true));
    settle_updates(&mut rt, 0.0);

    // An unsaved change, well before autosave would write it.
    rt.update(|s, _| s.import_text("t", "Hades: HD5S2-77KLM-NB3VC\n"));
    rt.update(|s, _| s.commit_import());
    assert!(rt.state.session().unwrap().is_dirty());

    click(&mut rt, "update-install");
    assert!(
        !rt.state.session().unwrap().is_dirty(),
        "saved before downloading"
    );
    let (_, on_disk) = Vault::open(&path, "pw").unwrap();
    assert_eq!(on_disk.keys.len(), 1);

    settle_updates(&mut rt, 0.0);
    let installed = src.installed.lock().unwrap().clone();
    assert_eq!(
        installed,
        [("0.2.0".to_string(), b"installer bytes".to_vec())]
    );
    assert!(matches!(rt.state.updates.phase, Phase::Installing));
    assert!(
        rt.shell_request.is_some(),
        "the app quits for the installer"
    );
}

#[test]
fn bad_signature_is_refused_and_explained() {
    let dir = tempfile::tempdir().unwrap();
    let src = Arc::new(FakeUpdates {
        latest: Mutex::new(Ok(release("0.2.0"))),
        download: Err(UpdateError::BadSignature("signature mismatch".into())),
        installed: Mutex::new(Vec::new()),
        checks: Mutex::new(0),
    });
    let mut rt = update_rt(src.clone(), dir.path());
    rt.update(|s, _| s.check_for_updates(true));
    settle_updates(&mut rt, 0.0);
    click(&mut rt, "update-install");
    settle_updates(&mut rt, 0.0);
    assert!(src.installed.lock().unwrap().is_empty(), "never installed");
    assert!(rt.shell_request.is_none(), "still running");
    assert!(
        matches!(&rt.state.updates.phase, Phase::Failed(m) if m.contains("not installed")),
        "{:?}",
        rt.state.updates.phase
    );
    assert!(rt.find("update-retry").is_some());
    click(&mut rt, "update-dismiss");
    assert!(matches!(rt.state.updates.phase, Phase::Idle));
}

#[test]
fn banner_shows_on_the_lock_screen() {
    let dir = tempfile::tempdir().unwrap();
    let mut s = state_for(&dir.path().join("v.kkvault"));
    s.updates.source = FakeUpdates::offering("9.0.0");
    let mut rt = runtime(s);
    rt.update(|s, _| s.check_for_updates(true));
    settle_updates(&mut rt, 0.0);
    assert!(rt.find("lock-submit").is_some());
    assert!(rt.find("update-banner").is_some());
}

#[test]
fn unlocking_with_a_newer_app_writes_the_pre_upgrade_backup() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("v.kkvault");
    let data = VaultData {
        last_app_version: Some("0.0.1".into()),
        ..VaultData::default()
    };
    Vault::create(
        &path,
        "correct horse",
        KdfParams::INSECURE_FAST_FOR_TESTS,
        &data,
    )
    .unwrap();
    let original = std::fs::read(&path).unwrap();
    let mut rt = runtime(state_for(&path));
    rt.update(|s, _| s.lock_ui_mut().unwrap().password = TextEdit::new("correct horse"));
    click(&mut rt, "lock-submit");
    wait_for_unlock_result(&mut rt);
    assert!(rt.state.is_unlocked());
    let backup = keykeeper_core::upgrade::pre_upgrade_path(&path, env!("CARGO_PKG_VERSION"));
    assert_eq!(std::fs::read(&backup).unwrap(), original);
    assert!(
        rt.state
            .toasts
            .iter()
            .any(|t| t.title.contains("before this update"))
    );
    let (_, on_disk) = Vault::open(&path, "correct horse").unwrap();
    assert_eq!(
        on_disk.last_app_version.as_deref(),
        Some(env!("CARGO_PKG_VERSION"))
    );
}

#[test]
fn huge_paste_keeps_preview_reachable() {
    let mut rt = demo_rt();
    click(&mut rt, "nav-import");
    let mut text = String::from(
        "Name	Key	Bundle
",
    );
    for i in 0..3000 {
        text.push_str(&format!(
            "Game {i}	AAAAA-BBBBB-{i:05}	Some Bundle
"
        ));
    }
    rt.update(|s, _| s.session_mut().unwrap().import.paste = TextEdit::new(&text));
    rt.frame(1.0);
    // The paste box keeps a fixed height and scrolls inside, so Preview stays on screen.
    let (_, y) = rt.find("import-preview").expect("Preview button on screen");
    assert!(y < SIZE.1, "Preview pushed off screen (y = {y})");
    click(&mut rt, "import-preview");
    let p = rt.state.session().unwrap().import.preview.clone().unwrap();
    assert_eq!(p.new_count(), 3000);
    assert!(rt.find("import-commit").is_some());
}

#[test]
fn steam_api_key_hint_links_to_the_key_page() {
    let dir = std::env::temp_dir();
    let mut s = state_for(&dir.join("unused.kkvault"));
    demo::unlock(&mut s, demo::sample_data());
    let mut rt = runtime_sized(s, TALL);
    click(&mut rt, "nav-settings");
    rt.take_url_requests();
    click(&mut rt, "https://steamcommunity.com/dev/apikey");
    assert_eq!(
        rt.take_url_requests(),
        ["https://steamcommunity.com/dev/apikey"]
    );
}

// ── self-redeem, cart button, links and prices ──────────────────────────────────────────────

fn available_steam_ids(rt: &Runtime<AppState>, n: usize) -> Vec<uuid::Uuid> {
    let s = rt.state.session().unwrap();
    s.rows
        .iter()
        .copied()
        .filter(|id| {
            let k = s.repo.key(*id).unwrap();
            k.status == KeyStatus::Available && k.platform == keykeeper_core::Platform::Steam
        })
        .take(n)
        .collect()
}

#[test]
fn non_steam_keys_are_marked_redeemed_without_opening_steam() {
    let mut rt = demo_rt();
    let id = {
        let s = rt.state.session().unwrap();
        s.repo
            .keys()
            .iter()
            .find(|k| k.status.can_trade() && k.platform != keykeeper_core::Platform::Steam)
            .expect("demo data has a non-Steam key")
            .id
    };
    rt.update(|s, _| s.open_key(id));
    rt.frame(1.0 / 60.0);
    rt.take_url_requests();
    click(&mut rt, "form-redeem");
    assert!(rt.take_url_requests().is_empty());
    let k = rt.state.session().unwrap().repo.key(id).unwrap();
    assert_eq!(k.status, KeyStatus::Redeemed);
}

#[test]
fn redeem_on_steam_opens_activation_and_dates_the_key() {
    let mut rt = demo_rt();
    let id = available_steam_ids(&rt, 1)[0];
    rt.update(|s, _| s.open_key(id));
    rt.frame(1.0 / 60.0);
    rt.take_url_requests();
    click(&mut rt, "form-redeem");
    rt.frame(1.0 / 60.0);
    let code = rt
        .state
        .session()
        .unwrap()
        .repo
        .key(id)
        .unwrap()
        .key
        .expose()
        .to_string();
    assert_eq!(
        rt.take_url_requests(),
        [format!(
            "https://store.steampowered.com/account/registerkey?key={code}"
        )]
    );
    let k = rt.state.session().unwrap().repo.key(id).unwrap();
    assert_eq!(k.status, KeyStatus::Redeemed);
    assert_eq!(k.status_date, Some(crate::app::today()));
    assert!(
        rt.find("form-redeem").is_none(),
        "a redeemed key can't be redeemed again"
    );
}

#[test]
fn cart_bauble_appears_springs_and_goes_to_the_cart() {
    let mut rt = demo_rt();
    let ids = available_ids(&rt, 1);
    rt.update(|s, _| s.add_to_cart(&ids));
    poll(&mut rt, 2.0);
    assert!(rt.find("cart-bauble").is_some());
    assert!(rt.state.cart_bumping());
    poll(&mut rt, 2.5);
    assert!(!rt.state.cart_bumping(), "the bump settles");
    click(&mut rt, "cart-bauble");
    assert_eq!(rt.state.session().unwrap().page, Page::Cart);
    assert!(
        rt.find("cart-bauble").is_none(),
        "hidden on the Cart page itself"
    );
}

#[test]
fn self_checkout_redeems_the_whole_cart() {
    let mut rt = demo_rt();
    let ids = available_ids(&rt, 2);
    rt.update(|s, _| s.add_to_cart(&ids));
    click(&mut rt, "nav-cart");
    poll(&mut rt, 0.0);
    let cart: Vec<uuid::Uuid> = rt
        .state
        .session()
        .unwrap()
        .repo
        .cart()
        .unwrap()
        .key_ids
        .clone();
    click(&mut rt, "cart-self-checkout");
    assert_eq!(
        rt.state.session().unwrap().confirm,
        Some(Confirm::RedeemCart)
    );
    click(&mut rt, "confirm-ok");
    poll(&mut rt, 0.0);
    let s = rt.state.session().unwrap();
    assert!(s.repo.cart().is_none());
    for id in cart {
        let k = s.repo.key(id).unwrap();
        assert_eq!(
            (k.status, k.status_date),
            (KeyStatus::Redeemed, Some(crate::app::today()))
        );
    }
    assert!(rt.find("cart-empty").is_some());
}

#[test]
fn cart_row_redeem_takes_one_key_out() {
    let mut rt = demo_rt();
    let ids = available_steam_ids(&rt, 2);
    rt.update(|s, _| s.add_to_cart(&ids));
    click(&mut rt, "nav-cart");
    poll(&mut rt, 0.0);
    let before = rt.state.session().unwrap().repo.cart_len();
    rt.take_url_requests();
    click(&mut rt, &format!("cart-redeem-{}", ids[0]));
    assert_eq!(
        rt.take_url_requests().len(),
        1,
        "Steam activation page opened"
    );
    let s = rt.state.session().unwrap();
    assert_eq!(s.repo.cart_len(), before - 1);
    assert_eq!(s.repo.key(ids[0]).unwrap().status, KeyStatus::Redeemed);
}

#[test]
fn game_names_with_a_steam_id_link_to_the_store() {
    let mut rt = demo_rt();
    let key = {
        let s = rt.state.session().unwrap();
        s.rows
            .iter()
            .filter_map(|id| s.repo.key(*id))
            .find(|k| k.steam_app_id.is_some())
            .unwrap()
            .clone()
    };
    rt.take_url_requests();
    click(&mut rt, &format!("name-link-{}", key.id));
    assert_eq!(rt.take_url_requests(), [key.steam_url().unwrap()]);
}

#[test]
fn detail_links_are_pills() {
    let dir = std::env::temp_dir();
    let mut s = state_for(&dir.join("unused.kkvault"));
    demo::unlock(&mut s, demo::sample_data());
    let mut rt = runtime_sized(s, TALL);
    let id = {
        let s = rt.state.session().unwrap();
        s.repo
            .keys()
            .iter()
            .find(|k| k.steam_app_id.is_some())
            .unwrap()
            .id
    };
    rt.update(|s, _| s.open_key(id));
    rt.frame(1.0 / 60.0);
    rt.take_url_requests();
    click(&mut rt, "link-gg-deals");
    let url = rt
        .state
        .session()
        .unwrap()
        .repo
        .key(id)
        .unwrap()
        .gg_deals_url()
        .unwrap();
    assert_eq!(rt.take_url_requests(), [url]);
    assert!(rt.find("link-steam").is_some() && rt.find("link-itad").is_some());
}

/// FakeSteam plus canned prices.
struct PricedSteam;

impl SteamApi for PricedSteam {
    fn search(&self, t: &str) -> Result<Vec<AppMatch>, SteamError> {
        FakeSteam.search(t)
    }
    fn reviews(&self, a: &[u32]) -> Result<Vec<Review>, SteamError> {
        FakeSteam.reviews(a)
    }
    fn resolve_profile(&self, i: &str) -> Result<u64, SteamError> {
        FakeSteam.resolve_profile(i)
    }
    fn wishlist(&self, id: u64) -> Result<Vec<u32>, SteamError> {
        FakeSteam.wishlist(id)
    }
    fn owned_games(&self, k: &str, id: u64) -> Result<Vec<u32>, SteamError> {
        FakeSteam.owned_games(k, id)
    }
    fn itad_prices(
        &self,
        key: &str,
        country: &str,
        apps: &[u32],
    ) -> Result<Vec<keykeeper_steam::ItadPrices>, SteamError> {
        assert_eq!((key, country), ("itad-key", "GB"));
        let p = |c: u64| keykeeper_steam::Price {
            cents: c,
            currency: "GBP".into(),
        };
        Ok(apps
            .iter()
            .map(|&app_id| keykeeper_steam::ItadPrices {
                app_id,
                deals: vec![
                    keykeeper_steam::ShopDeal {
                        shop: "Steam".into(),
                        price: p(1999),
                        regular: Some(p(1999)),
                        cut: 0,
                        url: Some("https://itad.example/steam".into()),
                    },
                    keykeeper_steam::ShopDeal {
                        shop: "Fanatical".into(),
                        price: p(899),
                        regular: Some(p(1999)),
                        cut: 55,
                        url: Some("https://itad.example/fanatical".into()),
                    },
                ],
                history_low: Some(p(499)),
            })
            .collect())
    }
    fn gg_deals_prices(
        &self,
        _: &str,
        region: &str,
        _: &[u32],
    ) -> Result<keykeeper_steam::GgBatch, SteamError> {
        assert_eq!(region, "gb");
        Err(SteamError::BadServiceKey("gg.deals"))
    }
}

#[test]
fn prices_fetch_fill_the_deals_column_and_editor() {
    let dir = std::env::temp_dir();
    let mut s = state_for(&dir.join("unused.kkvault"));
    s.steam = std::sync::Arc::new(PricedSteam);
    demo::unlock(&mut s, demo::sample_data());
    let mut rt = runtime(s);

    // No keys yet: explain where to add one.
    rt.update(|s, _| s.refresh_prices(true));
    assert!(rt.state.job.is_none());
    assert!(matches!(
        &rt.state.session().unwrap().steam_ui.prices_status,
        Some((false, m)) if m.contains("Settings")
    ));

    rt.update(|s, _| {
        let ui = &mut s.session_mut().unwrap().steam_ui;
        ui.country = TextEdit::new("gb");
        ui.itad_key = TextEdit::new("itad-key");
        ui.gg_key = TextEdit::new("bad-gg");
    });
    click(&mut rt, "prices-refresh");
    wait_for_steam(&mut rt);
    let s = rt.state.session().unwrap();
    assert_eq!(s.settings().price_country, "GB");
    assert_eq!(
        s.repo.api_keys().itad.as_ref().unwrap().expose(),
        "itad-key"
    );
    let key = s
        .rows
        .iter()
        .filter_map(|id| s.repo.key(*id))
        .find(|k| k.steam_app_id.is_some())
        .unwrap()
        .clone();
    let p = s.repo.key_prices(&key).unwrap();
    assert_eq!(p.best().unwrap().1, Some("Fanatical"), "cheapest first");
    assert_eq!(p.best_cents(), Some(899));
    assert!(
        matches!(
            &s.steam_ui.prices_status,
            Some((false, m)) if m.contains("gg.deals") && m.contains("rejected")
        ),
        "one failed service is reported, the other still applies"
    );

    rt.frame(1.0 / 60.0);
    rt.take_url_requests();
    click(&mut rt, &format!("deal-{}", key.id));
    assert_eq!(rt.take_url_requests(), ["https://itad.example/fanatical"]);

    rt.update(|s, _| s.open_key(key.id));
    rt.frame(1.0 / 60.0);
    assert!(rt.find("form-prices").is_some());

    // Sorting by price puts priced games first.
    rt.update(|s, _| s.sort_by("deals", rune_kit::SortDir::Ascending));
    poll(&mut rt, 0.0);
    let s = rt.state.session().unwrap();
    let first = s.repo.key(s.rows[0]).unwrap();
    assert!(s.repo.key_prices(first).is_some());
}

// ── game details: hover card, store details, artwork cache ───────────────────────────────────

/// FakeSteam plus store details and one downloadable image.
struct InfoSteam {
    downloads: Arc<Mutex<Vec<String>>>,
}

impl SteamApi for InfoSteam {
    fn search(&self, t: &str) -> Result<Vec<AppMatch>, SteamError> {
        FakeSteam.search(t)
    }
    fn reviews(&self, a: &[u32]) -> Result<Vec<Review>, SteamError> {
        FakeSteam.reviews(a)
    }
    fn resolve_profile(&self, i: &str) -> Result<u64, SteamError> {
        FakeSteam.resolve_profile(i)
    }
    fn wishlist(&self, id: u64) -> Result<Vec<u32>, SteamError> {
        FakeSteam.wishlist(id)
    }
    fn owned_games(&self, k: &str, id: u64) -> Result<Vec<u32>, SteamError> {
        FakeSteam.owned_games(k, id)
    }
    fn store_details(
        &self,
        app_ids: &[u32],
    ) -> Result<Vec<(keykeeper_steam::StoreDetails, Vec<String>)>, SteamError> {
        Ok(app_ids
            .iter()
            .map(|&app_id| {
                (
                    keykeeper_steam::StoreDetails {
                        app_id,
                        name: format!("Store name {app_id}"),
                        description: "A test game.".into(),
                        developers: vec!["Test Dev".into()],
                        tag_ids: vec![1],
                        released: Some(1_651_154_399),
                        deck_category: 3,
                        header_url: Some(format!(
                            "https://shared.akamai.steamstatic.com/{app_id}/header.jpg"
                        )),
                        capsule_url: None,
                    },
                    vec!["Metroidvania".to_string()],
                )
            })
            .collect())
    }
    fn download_image(&self, url: &str) -> Result<Vec<u8>, SteamError> {
        self.downloads.lock().unwrap().push(url.to_string());
        Ok(include_bytes!("../assets/icon-32.png").to_vec())
    }
}

fn info_rt(downloads: Arc<Mutex<Vec<String>>>) -> Runtime<AppState> {
    let mut s = state_for(&std::env::temp_dir().join("unused.kkvault"));
    let mut data = demo::sample_data();
    data.app_info.clear();
    demo::unlock(&mut s, data);
    s.offline = false;
    s.steam = Arc::new(InfoSteam { downloads });
    runtime(s)
}

/// Poll until `done` holds (background threads).
fn poll_until(rt: &mut Runtime<AppState>, now: f64, done: impl Fn(&AppState) -> bool) {
    let start = Instant::now();
    while start.elapsed() < Duration::from_secs(10) {
        poll(rt, now);
        if done(&rt.state) {
            return;
        }
        std::thread::sleep(Duration::from_millis(5));
    }
    panic!("background work never finished");
}

#[test]
fn resting_on_a_name_opens_the_details_card_and_fetches_details_once() {
    let downloads = Arc::new(Mutex::new(Vec::new()));
    let mut rt = info_rt(downloads.clone());
    let (id, app) = {
        let s = rt.state.session().unwrap();
        let k = s
            .rows
            .iter()
            .filter_map(|id| s.repo.key(*id))
            .find(|k| k.steam_app_id.is_some())
            .unwrap();
        (k.id, k.steam_app_id.unwrap())
    };
    let (x, y) = rt.find(&format!("name-{id}")).expect("name cell");
    rt.pointer_move(x, y);
    rt.frame(1.0 / 60.0);
    assert!(rt.state.hover.is_some_and(|h| h.key == id && !h.open));
    assert!(rt.find("game-card").is_none(), "not before the delay");

    poll_until(&mut rt, 30.0, |s| {
        s.session().unwrap().repo.app_info(app).is_some()
            && s.info.art(app).is_some_and(|i| i.is_ready())
    });
    assert!(rt.find("game-card").is_some(), "open after the delay");
    let s = rt.state.session().unwrap();
    let info = s.repo.app_info(app).unwrap();
    assert_eq!(info.tags, vec!["Metroidvania".to_string()]);
    assert_eq!(info.deck, keykeeper_core::DeckCompat::Verified);
    assert!(
        s.repo
            .keys()
            .iter()
            .filter_map(|k| k.steam_app_id)
            .all(|a| s.repo.app_info(a).is_some()),
        "the whole library's details came in one batch"
    );
    assert_eq!(
        downloads.lock().unwrap().as_slice(),
        [format!(
            "https://shared.akamai.steamstatic.com/{app}/header.jpg"
        )]
    );

    // The artwork is cached encrypted next to the vault: a fresh session reads it from disk.
    let cache = rt.state.info.cache().unwrap().clone();
    let cached = cache.get(&format!("art-{app}")).expect("cached");
    assert_eq!(cached, include_bytes!("../assets/icon-32.png"));
    let names: Vec<String> = std::fs::read_dir(cache.dir())
        .unwrap()
        .map(|e| e.unwrap().file_name().to_string_lossy().into_owned())
        .collect();
    assert!(
        names.iter().all(|n| !n.contains(&app.to_string())),
        "{names:?}"
    );
    let raw = std::fs::read(cache.dir().join(&names[0])).unwrap();
    assert!(!raw.windows(4).any(|w| w == b"\x89PNG"), "stored encrypted");

    // Leaving the name closes the card.
    rt.pointer_move(2.0, 2.0);
    rt.frame(1.0 / 60.0);
    assert!(rt.state.hover.is_none());
    rt.frame(1.0 / 60.0);
    assert!(rt.find("game-card").is_none());

    // Clearing the cache from Settings deletes it.
    rt.update(|s, _| s.clear_art_cache());
    assert_eq!(cache.size(), 0);
}

#[test]
fn hover_cards_can_be_turned_off_and_fetching_can_be_refused() {
    let downloads = Arc::new(Mutex::new(Vec::new()));
    let mut rt = info_rt(downloads.clone());
    rt.update(|s, _| {
        s.config.hover_cards = Some(false);
        s.config.fetch_store_info = Some(false);
    });
    let id = rt.state.session().unwrap().rows[0];
    let (x, y) = rt.find(&format!("name-{id}")).unwrap();
    rt.pointer_move(x, y);
    poll(&mut rt, 30.0);
    assert!(rt.state.hover.is_none());
    assert!(rt.find("game-card").is_none());

    rt.update(|s, _| s.config.hover_cards = Some(true));
    rt.pointer_move(2.0, 2.0);
    rt.frame(1.0 / 60.0);
    rt.pointer_move(x, y);
    rt.frame(1.0 / 60.0);
    poll(&mut rt, 40.0);
    std::thread::sleep(Duration::from_millis(50));
    poll(&mut rt, 41.0);
    assert!(rt.find("game-card").is_some(), "the card shows local data");
    assert!(downloads.lock().unwrap().is_empty(), "nothing fetched");
    assert!(rt.state.session().unwrap().repo.data().app_info.is_empty());
}

// ── row menu and keyboard ─────────────────────────────────────────────────────────────────────

fn right_click(rt: &mut Runtime<AppState>, key: &str) {
    let (x, y) = rt
        .find(key)
        .unwrap_or_else(|| panic!("no element '{key}' on screen"));
    rt.secondary_down(x, y);
    rt.secondary_up(x, y);
    rt.frame(1.0 / 60.0);
    for _ in 0..30 {
        rt.frame(1.0 / 60.0);
    }
}

#[test]
fn right_click_menu_acts_on_the_row_or_the_selection() {
    let mut rt = demo_rt();
    let (a, b) = {
        let s = rt.state.session().unwrap();
        let avail: Vec<_> = s
            .rows
            .iter()
            .copied()
            .filter(|id| {
                let k = s.repo.key(*id).unwrap();
                k.status == KeyStatus::Available && k.steam_app_id.is_some()
            })
            .collect();
        (avail[0], avail[1])
    };
    right_click(&mut rt, &format!("name-{a}"));
    assert!(rt.find("menu-edit").is_some(), "single-key menu");
    assert!(rt.find("menu-link-steam").is_some());
    assert!(
        rt.state
            .session()
            .unwrap()
            .table
            .is_selected(&a.to_string())
    );
    click(&mut rt, "menu-cart");
    rt.frame(1.0 / 60.0);
    let s = rt.state.session().unwrap();
    assert!(s.row_menu.is_none(), "choosing closes the menu");
    assert!(s.in_cart(a));

    // With two rows selected, the menu acts on both.
    rt.update(|s, _| {
        let x = s.session_mut().unwrap();
        x.table.select(b.to_string());
    });
    rt.update(|s, _| s.set_status(&[b], KeyStatus::Available));
    rt.update(|s, _| {
        let x = s.session_mut().unwrap();
        x.table.clear_selection();
    });
    let c = rt
        .state
        .session()
        .unwrap()
        .rows
        .iter()
        .copied()
        .find(|id| *id != a && *id != b)
        .unwrap();
    rt.update(|s, _| {
        let x = s.session_mut().unwrap();
        x.table.select(b.to_string());
    });
    rt.frame(1.0 / 60.0);
    // Shift-extend the selection in state, as a shift-click would.
    rt.update(|s, _| {
        let x = s.session_mut().unwrap();
        let ids = vec![b, c];
        x.row_menu = None;
        x.table.clear_selection();
        for id in &ids {
            x.table.select(id.to_string());
        }
    });
    rt.update(|s, _| s.open_row_menu(b, None));
    rt.frame(1.0 / 60.0);
    let menu_ids = rt
        .state
        .session()
        .unwrap()
        .row_menu
        .as_ref()
        .unwrap()
        .ids
        .clone();
    assert!(menu_ids.contains(&b));
    rt.update(|s, _| s.close_row_menu());

    right_click(&mut rt, &format!("name-{b}"));
    click(&mut rt, "menu-delete");
    assert!(matches!(
        rt.state.session().unwrap().confirm,
        Some(Confirm::DeleteKeys(_))
    ));
    rt.key(KeyEvent::from(Key::Escape));
    rt.frame(1.0 / 60.0);
    assert!(rt.state.session().unwrap().confirm.is_none());
}

#[test]
fn status_submenu_and_keyboard_menu() {
    let mut rt = demo_rt();
    let id = rt.state.session().unwrap().rows[0];
    rt.update(|s, _| {
        s.session_mut().unwrap().table.select(id.to_string());
    });
    rt.key(KeyEvent::from(Key::ContextMenu));
    for _ in 0..30 {
        rt.frame(1.0 / 60.0);
    }
    assert!(
        rt.find("menu-status").is_some(),
        "Menu key opens the row menu"
    );
    rt.update(|s, _| s.set_row_menu_status_open(true));
    for _ in 0..30 {
        rt.frame(1.0 / 60.0);
    }
    click(&mut rt, "menu-status-gifted");
    let s = rt.state.session().unwrap();
    assert_eq!(s.repo.key(id).unwrap().status, KeyStatus::Gifted);
    assert!(s.row_menu.is_none());
}

fn key(k: Key) -> KeyEvent {
    KeyEvent {
        key: k,
        mods: Mods::default(),
    }
}

#[test]
fn arrows_ctrl_b_and_ctrl_c_drive_the_library() {
    let mut rt = demo_rt();
    let rows = rt.state.session().unwrap().rows.clone();
    rt.key(key(Key::Down));
    rt.frame(1.0 / 60.0);
    assert_eq!(rt.state.session().unwrap().cursor_id(), Some(rows[0]));
    rt.key(key(Key::Down));
    rt.frame(1.0 / 60.0);
    rt.key(key(Key::Down));
    rt.frame(1.0 / 60.0);
    rt.key(key(Key::Up));
    rt.frame(1.0 / 60.0);
    let s = rt.state.session().unwrap();
    assert_eq!(s.cursor_id(), Some(rows[1]));
    assert_eq!(
        s.form.as_ref().and_then(|f| f.target),
        Some(rows[1]),
        "opens like a click"
    );

    // An available key: Ctrl+Enter puts it in the cart, and again takes it out.
    let id = {
        let s = rt.state.session().unwrap();
        *rows
            .iter()
            .find(|id| s.repo.key(**id).unwrap().status == KeyStatus::Available)
            .unwrap()
    };
    rt.update(|s, _| s.session_mut().unwrap().table.select(id.to_string()));
    rt.key(ctrl('b'));
    rt.frame(1.0 / 60.0);
    assert!(rt.state.session().unwrap().in_cart(id));
    rt.update(|s, _| s.session_mut().unwrap().table.select(id.to_string()));
    rt.key(ctrl('b'));
    rt.frame(1.0 / 60.0);
    assert!(!rt.state.session().unwrap().in_cart(id));

    rt.update(|s, _| s.session_mut().unwrap().table.select(id.to_string()));
    rt.key(ctrl('c'));
    rt.frame(1.0 / 60.0);
    assert!(
        rt.state
            .toasts
            .iter()
            .any(|t| t.title.starts_with("Copied key"))
    );

    rt.key(key(Key::Function(2)));
    rt.frame(1.0 / 60.0);
    assert_eq!(
        rt.state
            .session()
            .unwrap()
            .form
            .as_ref()
            .and_then(|f| f.target),
        Some(id)
    );
}

// ── bulk edit and undo toasts ─────────────────────────────────────────────────────────────────

fn select_rows(rt: &mut Runtime<AppState>, n: usize) -> Vec<uuid::Uuid> {
    let ids: Vec<_> = rt.state.session().unwrap().rows[..n].to_vec();
    rt.update(|s, _| {
        let x = s.session_mut().unwrap();
        for id in &ids {
            x.table.select(id.to_string());
        }
    });
    rt.frame(1.0 / 60.0);
    ids
}

#[test]
fn bulk_edit_changes_only_what_was_filled_in_as_one_undo_step() {
    let mut rt = demo_rt();
    let ids = select_rows(&mut rt, 1);
    // `select` replaces the selection; build a multi-selection by hand.
    let ids: Vec<_> = {
        let rows = rt.state.session().unwrap().rows.clone();
        let mut v = ids;
        v.extend(rows[1..3].iter().copied());
        v
    };
    rt.update(|s, _| {
        let x = s.session_mut().unwrap();
        x.bulk = Some(crate::bulk::BulkForm::new(ids.clone()));
    });
    rt.frame(1.0 / 60.0);
    assert!(rt.find("bulk-edit").is_some());
    let before: Vec<_> = ids
        .iter()
        .map(|id| rt.state.session().unwrap().repo.key(*id).unwrap().clone())
        .collect();
    rt.update(|s, _| {
        s.with_bulk(|b| {
            b.bundle = TextEdit::new("Bulk Bundle");
            b.add_tags = TextEdit::new("bulk");
        })
    });
    rt.frame(1.0 / 60.0);
    click(&mut rt, "bulk-edit-apply");
    let s = rt.state.session().unwrap();
    assert!(s.bulk.is_none(), "applying closes the pane");
    for (id, old) in ids.iter().zip(&before) {
        let k = s.repo.key(*id).unwrap();
        assert_eq!(k.bundle.as_deref(), Some("Bulk Bundle"));
        assert!(k.tags.contains(&"bulk".to_string()));
        assert_eq!(k.status, old.status, "status untouched");
        assert_eq!(k.redemption_deadline, old.redemption_deadline);
    }
    let toast = rt.state.toasts.last().unwrap();
    assert!(toast.undo && toast.title == "Updated 3 keys");
    let tid = toast.id;
    rt.update(|s, _| s.undo_from_toast(tid));
    let s = rt.state.session().unwrap();
    for (id, old) in ids.iter().zip(&before) {
        assert_eq!(
            s.repo.key(*id).unwrap().bundle,
            old.bundle,
            "one undo reverts all"
        );
    }

    // Marking keys redeemed in bulk asks first.
    rt.update(|s, _| {
        let x = s.session_mut().unwrap();
        x.bulk = Some(crate::bulk::BulkForm::new(ids.clone()));
    });
    rt.update(|s, _| s.with_bulk(|b| b.status = Some(KeyStatus::Redeemed)));
    rt.frame(1.0 / 60.0);
    click(&mut rt, "bulk-edit-apply");
    assert_eq!(rt.state.session().unwrap().confirm, Some(Confirm::BulkEdit));
    click(&mut rt, "confirm-ok");
    let s = rt.state.session().unwrap();
    assert!(
        ids.iter()
            .all(|id| s.repo.key(*id).unwrap().status == KeyStatus::Redeemed)
    );
}

#[test]
fn bulk_bar_and_row_menu_open_the_bulk_editor() {
    let mut rt = demo_rt();
    let rows = rt.state.session().unwrap().rows.clone();
    rt.update(|s, _| {
        let x = s.session_mut().unwrap();
        x.table.select(rows[0].to_string());
    });
    rt.frame(1.0 / 60.0);
    // Ctrl+A selects every row once the table has focus.
    rt.key(KeyEvent::from(Key::Down));
    rt.frame(1.0 / 60.0);
    rt.key(ctrl('a'));
    rt.frame(1.0 / 60.0);
    assert!(rt.state.session().unwrap().selected_ids().len() > 2);
    let n = rt.state.session().unwrap().selected_ids().len();
    assert!(n > 2);
    for _ in 0..10 {
        rt.frame(1.0 / 60.0);
    }
    click(&mut rt, "bulk-edit-open");
    assert!(rt.state.session().unwrap().bulk.is_some());
    assert!(rt.find("bulk-edit").is_some());
    rt.key(KeyEvent::from(Key::Escape));
    rt.frame(1.0 / 60.0);
    assert!(rt.state.session().unwrap().bulk.is_none());
}

// ── at a glance ───────────────────────────────────────────────────────────────────────────────

#[test]
fn stats_count_tradable_keys_value_and_deadlines() {
    let mut rt = demo_rt();
    let today = jiff::civil::date(2026, 11, 26);
    let st = rt.state.session().unwrap().stats(today);
    assert_eq!(st.total, 18);
    // Demo deadlines: Dredge 11/20 (past), Celeste 12/2 (reserved, still held), Haiku 12/15,
    // Tunic 2027-03-01 (far).
    assert_eq!(st.expired, 1);
    assert_eq!(st.expiring, 3);
    let s = rt.state.session().unwrap();
    let tradable = s
        .repo
        .keys()
        .iter()
        .filter(|k| k.status.can_trade())
        .count();
    assert_eq!(st.tradable, tradable);
    assert!(st.priced > 0 && st.priced <= tradable);
    assert_eq!(st.value.as_ref().unwrap().currency, "USD");

    click(&mut rt, "stat-expiring");
    poll(&mut rt, 0.0);
    let s = rt.state.session().unwrap();
    assert!(s.deadline_soon);
    assert_eq!(s.rows.len(), s.stats(crate::app::today()).expiring);
    click(&mut rt, "stat-tradable");
    poll(&mut rt, 0.0);
    assert_eq!(
        rt.state.session().unwrap().statuses,
        vec![KeyStatus::Available]
    );
    assert!(rt.find("nav-expiring").is_some(), "sidebar badge");
}

// ── trading helpers ───────────────────────────────────────────────────────────────────────────

#[test]
fn spares_and_duplicates_filters_and_have_list() {
    let mut rt = demo_rt();
    let board = rune::clipboard::memory();
    rt.use_clipboard(&board);

    click(&mut rt, "filter-fodder");
    poll(&mut rt, 0.0);
    // Demo: Undertale and Hollow Knight are owned, but both keys are already traded.
    assert!(shown(&rt).is_empty());
    rt.update(|s, _| {
        let x = s.session_mut().unwrap();
        let id = x
            .repo
            .keys()
            .iter()
            .find(|k| k.game_name == "Hollow Knight")
            .unwrap()
            .id;
        let _ = x.repo.update_key(id, |k| k.status = KeyStatus::Available);
    });
    poll(&mut rt, 0.0);
    assert_eq!(shown(&rt), ["Hollow Knight"]);
    click(&mut rt, "filter-fodder");

    rt.update(|s, _| {
        let x = s.session_mut().unwrap();
        let mut k = keykeeper_core::GameKey::new("Tunic", "TUN1C-SECND-COPY2");
        k.steam_app_id = Some(553420);
        x.repo.add_key(k);
    });
    click(&mut rt, "filter-dupes");
    poll(&mut rt, 0.0);
    assert_eq!(shown(&rt), ["Tunic", "Tunic"]);

    click(&mut rt, "copy-have-list");
    assert_eq!(
        board.text().as_deref().map(String::as_str),
        Some("Tunic x2")
    );
    click(&mut rt, "filter-dupes");
    poll(&mut rt, 0.0);
    click(&mut rt, "copy-have-list");
    let text = board.text().unwrap().to_string();
    assert!(text.contains("Hollow Knight") && text.contains("Tunic x2"));
    assert!(
        !text.contains("Undertale"),
        "traded keys aren't on the list"
    );
    assert!(!text.contains('-'), "no keys: {text}");
}

// ── table layout ──────────────────────────────────────────────────────────────────────────────

#[test]
fn column_menu_hides_columns_and_the_layout_survives_a_lock() {
    let dir = tempfile::tempdir().unwrap();
    let vault = dir.path().join("v.kkvault");
    let cfg = dir.path().join("config.toml");
    Vault::create(
        &vault,
        "pw-12345678",
        KdfParams::INSECURE_FAST_FOR_TESTS,
        &demo::sample_data(),
    )
    .unwrap();
    let mut s = state_for(&vault);
    s.config_path = Some(cfg.clone());
    let mut rt = runtime(s);
    rt.update(|s, _| s.lock_ui_mut().unwrap().password = TextEdit::new("pw-12345678"));
    click(&mut rt, "lock-submit");
    wait_for_unlock_result(&mut rt);
    poll(&mut rt, 0.0);

    right_click(&mut rt, "library-table");
    // The click landed on a row (no header there): open the header menu by hand instead.
    rt.update(|s, _| {
        s.close_row_menu();
        s.session_mut().unwrap().header_menu = Some(rune::PointerPress { x: 600.0, y: 300.0 });
    });
    for _ in 0..20 {
        rt.frame(1.0 / 60.0);
    }
    click(&mut rt, "col-platform");
    assert!(
        rt.state
            .session()
            .unwrap()
            .table
            .is_column_hidden("platform")
    );
    rt.update(|s, _| {
        s.sort_by("name", rune_kit::SortDir::Ascending);
        s.toggle_group_by_bundle();
        s.set_density(crate::config::Density::Compact);
    });
    poll(&mut rt, 1.0);
    poll(&mut rt, 3.0);
    let saved = Config::load(&cfg);
    assert_eq!(
        saved.library.hidden_columns,
        vec!["platform".to_string(), "added".to_string()]
    );
    assert_eq!(saved.library.sort.as_deref(), Some("name:asc"));
    assert!(saved.library.group_by_bundle);
    assert_eq!(saved.library.density, crate::config::Density::Compact);

    // Lock and unlock: the table comes back the same.
    rt.key(ctrl('l'));
    rt.frame(1.0 / 60.0);
    rt.update(|s, _| s.lock_ui_mut().unwrap().password = TextEdit::new("pw-12345678"));
    click(&mut rt, "lock-submit");
    wait_for_unlock_result(&mut rt);
    poll(&mut rt, 4.0);
    let s = rt.state.session().unwrap();
    assert!(s.table.is_column_hidden("platform"));
    assert!(s.group_by_bundle);
    assert_eq!(s.sort.field, keykeeper_core::SortField::Name);
}

#[test]
fn grouping_by_bundle_folds_and_unfolds() {
    let mut rt = demo_rt();
    rt.update(|s, _| s.toggle_group_by_bundle());
    poll(&mut rt, 0.0);
    let s = rt.state.session().unwrap();
    let groups: Vec<String> = s
        .display
        .iter()
        .filter_map(|r| match r {
            crate::layout::DisplayRow::Group { name, .. } => Some(name.clone()),
            _ => None,
        })
        .collect();
    assert_eq!(
        groups.last().map(String::as_str),
        Some(crate::layout::NO_BUNDLE)
    );
    assert!(groups.contains(&"Humble Choice".to_string()));
    let n = s.display.len();
    assert_eq!(n, s.rows.len() + groups.len());

    let first = groups[0].clone();
    // Enter on a bundle header (keyboard cursor on it) folds it, through the table's handler.
    rt.update(|s, tx| {
        let x = s.session_mut().unwrap();
        x.table.select(crate::layout::group_row_id(&first));
        tx.focus("library-table", false);
    });
    rt.frame(1.0 / 60.0);
    rt.key(KeyEvent::from(Key::Enter));
    rt.frame(1.0 / 60.0);
    poll(&mut rt, 0.0);
    let s = rt.state.session().unwrap();
    assert!(s.collapsed.contains(&first), "{first} folds");
    let humble = s
        .rows
        .iter()
        .filter(|id| s.repo.key(**id).unwrap().bundle.as_deref() == Some(first.as_str()))
        .count();
    assert_eq!(s.display.len(), n - humble, "its keys are hidden");
    assert_eq!(
        s.selected_ids(),
        Vec::<uuid::Uuid>::new(),
        "a header isn't a key"
    );

    rt.update(|s, _| s.collapse_all_groups(true));
    poll(&mut rt, 0.0);
    assert_eq!(rt.state.session().unwrap().display.len(), groups.len());
    rt.update(|s, _| s.collapse_all_groups(false));
    poll(&mut rt, 0.0);
    assert_eq!(rt.state.session().unwrap().display.len(), n);
}
