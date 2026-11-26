//! Views. Each page is a function `(&AppState, &Kit, &mut overlays) -> El`; all behaviour goes
//! through `AppState` methods.

mod cart;
mod detail;
mod import;
mod library;
mod lock;
mod settings;
mod shell;
mod shortcuts;
mod trades;

pub use shortcuts::shortcut;
#[cfg(test)]
pub const SHORTCUT_COUNT: usize = shortcuts::SHORTCUTS.len();

use rune::{Color, Cx, El, TextEdit, TextStyle, col, el, row, text};
use rune_kit::{Kit, TableState};

use crate::app::{AppState, LockUi, Screen, Session};
use crate::form::KeyForm;
use crate::trade_actions::TradeForm;
use keykeeper_core::KeyStatus;

pub fn view(s: &AppState, cx: &mut Cx<AppState>) -> El {
    let k = s.theme.kit(cx.system_dark());
    let mut overlays = Vec::new();
    let base = match &s.screen {
        Screen::Locked(l) => lock::view(l, k),
        Screen::Unlocked(sess) => shell::view(s, sess, k, &mut overlays),
    };
    overlays.extend(shell::toasts(s, k));
    let root = col()
        .key("app")
        .w_full()
        .h_full()
        .fill(k.c.page)
        .on_pointer_move_local(|s: &mut AppState, _, tx| {
            s.touch(tx);
            tx.live_only();
        })
        .child(base);
    rune_kit::root(root, overlays)
}

// ── text-field accessors ──────────────────────────────────────────────────────────────────────
// `on_edit` needs `&mut TextEdit` from `&mut AppState` even when that screen isn't showing, so
// these fall back to a scratch buffer. Destructuring keeps the two borrows disjoint.

pub fn sess_edit<'a>(
    s: &'a mut AppState,
    f: impl FnOnce(&'a mut Session) -> &'a mut TextEdit,
) -> &'a mut TextEdit {
    let AppState {
        screen, scratch, ..
    } = s;
    match screen {
        Screen::Unlocked(x) => f(x),
        Screen::Locked(_) => scratch,
    }
}

pub fn lock_edit<'a>(
    s: &'a mut AppState,
    f: impl FnOnce(&'a mut LockUi) -> &'a mut TextEdit,
) -> &'a mut TextEdit {
    let AppState {
        screen, scratch, ..
    } = s;
    match screen {
        Screen::Locked(l) => f(l),
        Screen::Unlocked(_) => scratch,
    }
}

pub fn trade_edit<'a>(
    s: &'a mut AppState,
    f: impl FnOnce(&'a mut TradeForm) -> &'a mut TextEdit,
) -> &'a mut TextEdit {
    let AppState {
        screen, scratch, ..
    } = s;
    match screen {
        Screen::Unlocked(x) => match x.trade_form.as_mut() {
            Some(t) => f(t),
            None => scratch,
        },
        Screen::Locked(_) => scratch,
    }
}

pub fn form_edit<'a>(
    s: &'a mut AppState,
    f: impl FnOnce(&'a mut KeyForm) -> &'a mut TextEdit,
) -> &'a mut TextEdit {
    let AppState {
        screen, scratch, ..
    } = s;
    match screen {
        Screen::Unlocked(x) => match x.form.as_mut() {
            Some(form) => f(form),
            None => scratch,
        },
        Screen::Locked(_) => scratch,
    }
}

pub fn table_state(s: &AppState) -> &TableState {
    match &s.screen {
        Screen::Unlocked(x) => &x.table,
        Screen::Locked(_) => &s.scratch_table,
    }
}

pub fn table_state_mut(s: &mut AppState) -> &mut TableState {
    let AppState {
        screen,
        scratch_table,
        ..
    } = s;
    match screen {
        Screen::Unlocked(x) => &mut x.table,
        Screen::Locked(_) => scratch_table,
    }
}

// ── shared pieces ─────────────────────────────────────────────────────────────────────────────

pub fn title_style() -> TextStyle {
    TextStyle::new(28.0, 700.0).lh(34.0).tracking(-0.4)
}

pub fn heading_style() -> TextStyle {
    TextStyle::new(15.0, 600.0).lh(20.0)
}

/// Background / foreground for a status pill.
pub fn status_colors(k: &Kit, s: KeyStatus) -> (Color, Color) {
    match s {
        KeyStatus::Available => (k.c.success.bg, k.c.success.fg),
        KeyStatus::Reserved => (k.c.info.bg, k.c.info.fg),
        KeyStatus::Unknown => (k.c.warning.bg, k.c.warning.fg),
        KeyStatus::Gifted => (k.c.surface_raised, k.c.text_accent),
        KeyStatus::Traded | KeyStatus::Redeemed => (k.c.surface_raised, k.c.text_muted),
    }
}

pub fn status_pill(k: &Kit, s: KeyStatus) -> El {
    let (bg, fg) = status_colors(k, s);
    el().h(20.0)
        .px(8.0)
        .radius(10.0)
        .center()
        .fill(bg)
        .child(text(s.label()).font(k.ty.text_xsmall_medium).color(fg))
}

/// A rounded toggle chip used for filters and option rows.
pub fn chip(
    k: &Kit,
    key: impl Into<String>,
    label: impl Into<String>,
    on: bool,
    press: impl Fn(&mut AppState) + 'static,
) -> El {
    let label = label.into();
    el().key(key)
        .role("checkbox")
        .label(label.clone())
        .checked(on)
        .focusable(true)
        .h(28.0)
        .px(12.0)
        .radius(14.0)
        .center()
        .fill(if on {
            k.c.bg_accent
        } else {
            k.c.surface_raised
        })
        .border(1.0, if on { k.c.bg_accent } else { k.c.border_weak })
        .hover(if on { k.c.bg_accent } else { k.c.surface_hover })
        .on_press(press)
        .child(
            text(label)
                .font(k.ty.text_small_medium)
                .color(if on { k.c.text_contrast } else { k.c.text_base })
                .a11y_hidden(),
        )
}

/// A titled card section, like the settings groups.
pub fn card(k: &Kit, key: &str, title: &str, body: El) -> El {
    col()
        .key(key.to_string())
        .w_full()
        .gap(10.0)
        .child(
            text(title.to_string())
                .font(heading_style())
                .color(k.c.text_strong),
        )
        .child(
            col()
                .w_full()
                .padding(18.0)
                .gap(14.0)
                .radius(k.radius.lg)
                .fill(k.c.bg_base)
                .border(1.0, k.c.border_weak)
                .child(body),
        )
}

/// Label + control + optional error, stacked.
pub fn field(k: &Kit, label: &str, control: El, error: Option<String>) -> El {
    let mut c = col()
        .w_full()
        .gap(6.0)
        .child(
            text(label.to_string())
                .font(k.ty.text_small_medium)
                .color(k.c.text_muted),
        )
        .child(control);
    if let Some(e) = error {
        c = c.child(text(e).font(k.ty.text_small).color(k.c.danger.icon).wrap());
    }
    c
}

/// The accent-filled call to action (rune-kit's `Submit` reads as white on these palettes).
pub fn primary(
    k: &Kit,
    key: &str,
    label: impl Into<String>,
    enabled: bool,
    press: impl Fn(&mut AppState, &mut rune::Tx) + 'static,
) -> El {
    let label = label.into();
    let mut b = el()
        .key(key.to_string())
        .role("button")
        .label(label.clone())
        .h(34.0)
        .px(16.0)
        .radius(k.radius.md)
        .center()
        .fill(if enabled {
            k.c.bg_accent
        } else {
            k.c.surface_raised
        })
        .child(
            text(label)
                .font(k.ty.ui_medium)
                .color(if enabled {
                    k.c.text_contrast
                } else {
                    k.c.text_faint
                })
                .a11y_hidden(),
        );
    if enabled {
        b = b
            .focusable(true)
            .hover(k.c.bg_accent.mix(k.c.white, 0.12))
            .press(k.c.bg_accent.mix(k.c.black, 0.15))
            .shadow(3.0, 12.0, k.c.bg_accent.alpha(0.30))
            .on_press_tx(press);
    }
    b
}

pub fn muted(k: &Kit, s: impl Into<String>) -> El {
    text(s.into()).font(k.ty.text_small).color(k.c.text_muted)
}

pub fn spacer() -> El {
    el().grow(1.0)
}

pub fn hstack() -> El {
    row().gap(8.0).center_y()
}
