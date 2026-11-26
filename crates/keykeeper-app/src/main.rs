//! KeyKeeper: an encrypted, spreadsheet-style manager for game keys and trades.
//!
//! `cargo run -p keykeeper-app` opens the app. Screenshots of each screen with demo data:
//! `cargo run -p keykeeper-app -- --snap target/library.png --scene library --size 1280x800`
//! (scenes: see `SCENES`).
//!
//! `keykeeper --self-test` is internal: a headless smoke test for release builds (`selftest.rs`).

// A GUI app on Windows: no console window opens alongside it. When started from a terminal,
// `attach_console` reconnects stdout/stderr so CLI flags and logs still print there.
#![windows_subsystem = "windows"]

mod app;
mod config;
mod demo;
mod form;
mod info;
mod library_actions;
mod price_actions;
mod selftest;
mod steam_actions;
mod theme;
mod trade_actions;
mod ui;
mod update;
mod update_actions;

#[cfg(test)]
mod tests;

use rune::{App, AppSpec, KeyEvent, Tx, Waker};

use app::{AppState, LockMode, Page};
use theme::ThemeId;

fn setup(s: &mut AppState, waker: Waker) {
    s.waker = Some(waker);
}

fn poll(s: &mut AppState, now: f64, tx: &mut Tx) -> bool {
    s.poll(now, tx)
}

fn on_key(s: &mut AppState, e: &KeyEvent, tx: &mut Tx) -> bool {
    s.on_key(e, tx)
}

fn on_close(s: &mut AppState, tx: &mut Tx) {
    s.on_close(tx);
}

/// A named starting state for `--scene` screenshots and the render test.
type Scene = (&'static str, fn(&mut AppState));

const SCENES: &[Scene] = &[
    ("lock", |s| s.set_lock_mode(LockMode::Unlock)),
    ("create", |s| s.set_lock_mode(LockMode::Create)),
    ("library", |s| demo::unlock(s, demo::sample_data())),
    ("detail", |s| {
        demo::unlock(s, demo::sample_data());
        let id = s.session().and_then(|x| {
            x.rows.iter().copied().find(|id| {
                x.repo.key(*id).is_some_and(|k| {
                    k.status == keykeeper_core::KeyStatus::Available && k.steam_app_id.is_some()
                })
            })
        });
        if let Some(id) = id {
            s.open_key(id);
        }
    }),
    ("hover-card", |s| {
        demo::unlock(s, demo::sample_data());
        let id = s.session().and_then(|x| {
            x.rows.iter().copied().find(|id| {
                x.repo
                    .key(*id)
                    .is_some_and(|k| k.steam_app_id == Some(1231880))
            })
        });
        if let Some(id) = id {
            s.hover = Some(app::Hover {
                key: id,
                since: 0.0,
                open: true,
            });
        }
    }),
    ("row-menu", |s| {
        demo::unlock(s, demo::sample_data());
        let id = s.session().and_then(|x| x.rows.get(4).copied());
        if let Some(id) = id {
            s.open_row_menu(id, Some(rune::PointerPress { x: 520.0, y: 380.0 }));
            s.set_row_menu_status_open(true);
        }
    }),
    ("cart-button", |s| {
        demo::unlock(s, demo::sample_data());
        let ids: Vec<_> = s
            .session()
            .map(|x| x.rows.iter().take(3).copied().collect())
            .unwrap_or_default();
        s.add_to_cart(&ids);
        s.toasts.clear();
    }),
    ("new-key", |s| {
        demo::unlock(s, demo::sample_data());
        s.new_key();
    }),
    ("import", |s| {
        demo::unlock(s, keykeeper_core::VaultData::default());
        s.go(Page::Import);
        s.import_text("example_import.csv", demo::SAMPLE_CSV);
    }),
    ("import-start", |s| {
        demo::unlock(s, demo::sample_data());
        s.go(Page::Import);
    }),
    ("settings", |s| {
        demo::unlock(s, demo::sample_data());
        s.go(Page::Settings);
    }),
    ("trades", |s| {
        demo::unlock(s, demo::sample_data());
        if let Some(id) = s
            .session()
            .and_then(|x| x.repo.trades().first().map(|t| t.id))
        {
            s.select_trade(id);
        }
    }),
    ("cart", |s| {
        demo::unlock(s, demo::sample_data());
        let ids: Vec<_> = s
            .session()
            .map(|x| x.rows.iter().take(4).copied().collect())
            .unwrap_or_default();
        s.add_to_cart(&ids);
        s.toasts.clear();
        s.go(Page::Cart);
        if let Some(f) = s.session_mut().and_then(|x| x.trade_form.as_mut()) {
            f.partner = rune::TextEdit::new("lumen");
            f.trade_url = rune::TextEdit::new("https://lestrades.com/offer/100001/");
            f.received = rune::TextEdit::new("Tunic\nA Short Hike: SH0RT-H1KE2-BBBB2");
        }
    }),
    ("templates", |s| {
        demo::unlock(s, demo::sample_data());
        s.go(Page::Settings);
    }),
    ("shortcuts", |s| {
        demo::unlock(s, demo::sample_data());
        s.show_shortcuts(true);
    }),
    ("empty", |s| {
        demo::unlock(s, keykeeper_core::VaultData::default())
    }),
    ("update", |s| {
        demo::unlock(s, demo::sample_data());
        demo::offer_update(s);
    }),
    ("update-notes", |s| {
        demo::unlock(s, demo::sample_data());
        demo::offer_update(s);
        s.show_update_notes(true);
    }),
    ("update-lock", |s| {
        s.set_lock_mode(LockMode::Unlock);
        demo::offer_update(s);
    }),
    ("light", |s| {
        s.theme = ThemeId::Light;
        demo::unlock(s, demo::sample_data());
    }),
];

/// The window and taskbar icon (Windows, Linux). On macOS the bundle's `.icns` is used instead,
/// drawn on Apple's icon grid.
#[cfg(not(target_os = "macos"))]
const ICON_PNG: &[u8] = include_bytes!("../assets/icon-256.png");

#[cfg(not(target_os = "macos"))]
fn set_icon() {
    if let Err(e) = rune::set_app_icon(ICON_PNG) {
        eprintln!("keykeeper: app icon: {e}");
    }
}

#[cfg(target_os = "macos")]
fn set_icon() {}

/// Windows: print to the terminal that launched us, if any. Started from Explorer or the Start
/// menu there is no parent console and this does nothing.
#[cfg(windows)]
#[allow(unsafe_code)]
fn attach_console() {
    use windows_sys::Win32::System::Console::{ATTACH_PARENT_PROCESS, AttachConsole};
    // SAFETY: plain Win32 call with a constant argument; failure only means "no parent console".
    unsafe {
        AttachConsole(ATTACH_PARENT_PROCESS);
    }
}

#[cfg(not(windows))]
fn attach_console() {}

fn main() {
    attach_console();
    // Internal: the release workflow's smoke test (see `selftest.rs`). Not a user feature.
    if std::env::args().skip(1).any(|a| a == selftest::FLAG) {
        std::process::exit(selftest::run());
    }
    set_icon();
    let theme = config::Config::default_path()
        .map(|p| config::Config::load(&p))
        .unwrap_or_default()
        .theme();
    let spec = AppSpec::<AppState> {
        min_size: Some((960.0, 620.0)),
        setup: Some(setup),
        poll: Some(poll),
        on_key: Some(on_key),
        scenes: SCENES,
        ..AppSpec::new("KeyKeeper", (1280.0, 800.0), AppState::load, ui::view)
    };
    App::new(spec)
        .color_scheme(theme.color_scheme())
        .on_close(on_close)
        .run();
}
