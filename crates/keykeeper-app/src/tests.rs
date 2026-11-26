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
        theme: None,
    };
    AppState::new(config, None, KdfParams::INSECURE_FAST_FOR_TESTS)
}

fn runtime(s: AppState) -> Runtime<AppState> {
    runtime_sized(s, SIZE)
}

/// Settings is a long scrolling page; tall tests keep every control on screen.
const TALL: (f32, f32) = (1280.0, 2400.0);

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
