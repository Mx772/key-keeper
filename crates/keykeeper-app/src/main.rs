//! KeyKeeper: an encrypted, spreadsheet-style manager for game keys and trades.
//!
//! `cargo run -p keykeeper-app` opens the app. Screenshots of each screen with demo data:
//! `cargo run -p keykeeper-app -- --snap target/library.png --scene library --size 1280x800`
//! (scenes: lock, create, library, detail, new-key, import, settings, trades, empty, light).

mod app;
mod config;
mod demo;
mod form;
mod theme;
mod ui;

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
        if let Some(id) = s.session().and_then(|x| x.rows.get(1).copied()) {
            s.open_key(id);
        }
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
        s.go(Page::Trades);
    }),
    ("empty", |s| {
        demo::unlock(s, keykeeper_core::VaultData::default())
    }),
    ("light", |s| {
        s.theme = ThemeId::Light;
        demo::unlock(s, demo::sample_data());
    }),
];

fn main() {
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
