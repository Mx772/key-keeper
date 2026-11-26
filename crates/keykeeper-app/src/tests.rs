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
    let mut rt = Runtime::new(s, ui::view, SIZE, Env::new(true, 1.0));
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
    let mut rt = runtime(s);
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
