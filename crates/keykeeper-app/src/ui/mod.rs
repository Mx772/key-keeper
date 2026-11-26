//! Views. Each page is a function `(&AppState, &Kit, &mut overlays) -> El`; all behaviour goes
//! through `AppState` methods.

mod card;
mod cart;
mod detail;
mod import;
mod library;
mod lock;
mod row_menu;
mod settings;
mod shell;
mod shortcuts;
mod trades;
mod update;

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
    update::notes_dialog(&s.updates, k, &mut overlays);
    overlays.extend(shell::toasts(s, k));
    let base = match update::banner(&s.updates, k) {
        Some(banner) => col()
            .w_full()
            .h_full()
            .child(banner)
            .child(el().w_full().grow(1.0).min_h(0.0).child(base)),
        None => base,
    };
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

// ── icons and link pills ──────────────────────────────────────────────────────────────────────

pub static EXTERNAL: rune::Icon = rune::Icon {
    name: "kk-external",
    view: 16.0,
    body: r#"<path d="M9.5 3H13V6.5" stroke="currentColor" stroke-width="1.4" fill="none" stroke-linecap="round" stroke-linejoin="round"/><path d="M13 3L7.5 8.5" stroke="currentColor" stroke-width="1.4" stroke-linecap="round"/><path d="M11 9.5V12.5C11 12.78 10.78 13 10.5 13H3.5C3.22 13 3 12.78 3 12.5V5.5C3 5.22 3.22 5 3.5 5H6.5" stroke="currentColor" stroke-width="1.4" fill="none" stroke-linecap="round"/>"#,
};

pub static CART: rune::Icon = rune::Icon {
    name: "kk-cart",
    view: 24.0,
    body: r#"<path d="M3 4H5.2L7.3 14.2C7.4 14.7 7.8 15 8.3 15H17.2C17.7 15 18.1 14.7 18.2 14.2L19.5 8H6.3" stroke="currentColor" stroke-width="1.8" fill="none" stroke-linecap="round" stroke-linejoin="round"/><circle cx="9.5" cy="19" r="1.5" fill="currentColor"/><circle cx="16.5" cy="19" r="1.5" fill="currentColor"/>"#,
};

pub static KEY: rune::Icon = rune::Icon {
    name: "kk-key",
    view: 16.0,
    body: r#"<circle cx="5.5" cy="10.5" r="2.75" stroke="currentColor" stroke-width="1.4" fill="none"/><path d="M7.5 8.5L13 3M11 5L12.5 6.5M9.5 6.5L10.75 7.75" stroke="currentColor" stroke-width="1.4" stroke-linecap="round"/>"#,
};

/// A rounded, clearly clickable pill that opens `url` in the browser.
pub fn link_pill(k: &Kit, key: impl Into<String>, label: &str, url: String) -> El {
    let help = url.clone();
    row()
        .key(key)
        .role("link")
        .label(format!("Open {label}"))
        .focusable(true)
        .h(34.0)
        .px(14.0)
        .gap(7.0)
        .radius(17.0)
        .center_y()
        .fill(k.c.bg_accent.alpha(0.12))
        .border(1.0, k.c.bg_accent.alpha(0.45))
        .hover(k.c.bg_accent.alpha(0.24))
        .press(k.c.bg_accent.alpha(0.32))
        .cursor(rune::Cursor::Pointer)
        .help(help)
        .on_press_tx(move |_: &mut AppState, tx: &mut rune::Tx| {
            let _ = tx.open_url(&url);
        })
        .child(
            text(label.to_string())
                .font(k.ty.ui_medium)
                .color(k.c.text_accent)
                .a11y_hidden(),
        )
        .child(
            rune::icon(&EXTERNAL, 14.0)
                .color(k.c.text_accent)
                .a11y_hidden(),
        )
}

/// Hover text for a game's prices: the best few stores, the all-time low and keyshops.
pub fn price_help(p: &keykeeper_core::GamePrices) -> Vec<String> {
    let mut rows: Vec<String> = p
        .shops
        .iter()
        .take(10)
        .map(|s| {
            let cut = if s.cut > 0 {
                format!("  (−{}%)", s.cut)
            } else {
                String::new()
            };
            format!("{}  {}{cut}", s.shop, s.price.format())
        })
        .collect();
    if p.shops.len() > 10 {
        rows.push(format!("…and {} more stores", p.shops.len() - 10));
    }
    if p.shops.is_empty()
        && let Some(r) = &p.retail_low
    {
        rows.push(format!("Lowest store price  {}", r.format()));
    }
    if let Some(l) = &p.history_low {
        rows.push(format!("All-time low  {}", l.format()));
    }
    if let Some(ks) = &p.keyshop_low {
        rows.push(format!("Keyshops (gg.deals)  {}", ks.format()));
    }
    if rows.is_empty() {
        rows.push("No current prices found".into());
    }
    rows
}

/// Where a click on a game's price goes: the best deal's link, else its gg.deals page.
pub fn price_url(p: &keykeeper_core::GamePrices) -> Option<String> {
    p.shops
        .first()
        .and_then(|s| s.url.clone())
        .or_else(|| p.gg_deals_url.clone())
}
