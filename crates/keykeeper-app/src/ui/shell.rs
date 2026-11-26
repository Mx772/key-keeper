//! The unlocked window: sidebar, page, detail pane, dialogs and toasts.

use rune::{El, Tx, col, el, row, text};
use rune_kit::{Button, ButtonVariant, Dialog, Kit, Toast};

use super::{cart, detail, import, library, settings, shortcut, shortcuts, trades};
use crate::app::{AppState, Confirm, ExportKind, Page, Session};

pub fn view(s: &AppState, sess: &Session, k: &Kit, overlays: &mut Vec<El>) -> El {
    let page = match sess.page {
        Page::Library => library::view(s, sess, k, overlays),
        Page::Cart => cart::view(sess, k, overlays),
        Page::Trades => trades::view(sess, k, overlays),
        Page::Import => import::view(s, sess, k, overlays),
        Page::Settings => settings::view(s, sess, k, overlays),
    };
    let mut body = row()
        .key("body")
        .w_full()
        .h_full()
        .child(sidebar(s, sess, k))
        .child(col().key("page").grow(1.0).min_w(0.0).h_full().child(page));
    let detail_open = sess.page == Page::Library && (sess.form.is_some() || sess.bulk.is_some());
    if sess.page == Page::Library
        && let Some(form) = &sess.form
    {
        body = body.child(detail::view(s, sess, form, s.steam_busy(), k, overlays));
    } else if sess.page == Page::Library
        && let Some(b) = &sess.bulk
    {
        body = body.child(super::bulk::view(sess, b, k, overlays));
    }
    overlays.extend(cart_bauble(s, sess, detail_open, k));
    confirm_dialog(sess, k, overlays);
    super::match_review::dialog(s, sess, k, overlays);
    shortcuts::dialog(sess, k, overlays);
    body
}

fn sidebar(s: &AppState, sess: &Session, k: &Kit) -> El {
    let count = sess.repo.keys().len();
    let nav = |page: Page| {
        let active = sess.page == page;
        let badge = match page {
            Page::Library => Some(count.to_string()),
            Page::Cart => Some(sess.repo.cart_len().to_string()).filter(|n| n != "0"),
            Page::Trades => Some(
                sess.repo
                    .trades()
                    .iter()
                    .filter(|t| t.status == keykeeper_core::TradeStatus::Pending)
                    .count()
                    .to_string(),
            )
            .filter(|n| n != "0"),
            _ => None,
        };
        let mut r = row()
            .key(format!("nav-{}", page.slug()))
            .role("tab")
            .label(page.label())
            .checked(active)
            .focusable(true)
            .w_full()
            .h(40.0)
            .px(14.0)
            .gap(10.0)
            .center_y()
            .radius(10.0)
            .fill(if active {
                k.c.bg_accent
            } else {
                k.c.bg_base.alpha(0.0)
            })
            .hover(if active {
                k.c.bg_accent
            } else {
                k.c.surface_hover
            })
            .on_press(move |s: &mut AppState| s.go(page))
            .child(
                rune::icon(
                    match page {
                        Page::Library => &super::NAV_LIBRARY,
                        Page::Cart => &super::CART,
                        Page::Trades => &super::NAV_TRADES,
                        Page::Import => &super::NAV_IMPORT,
                        Page::Settings => &super::NAV_SETTINGS,
                    },
                    17.0,
                )
                .color(if active {
                    k.c.text_contrast
                } else {
                    k.c.text_muted
                })
                .a11y_hidden(),
            )
            .child(
                text(page.label())
                    .font(k.ty.ui_medium)
                    .color(if active {
                        k.c.text_contrast
                    } else {
                        k.c.text_base
                    })
                    .grow(1.0)
                    .a11y_hidden(),
            );
        if page == Page::Library {
            let soon = sess.stats(crate::app::today()).expiring;
            if soon > 0 {
                r = r.child(
                    el().key("nav-expiring")
                        .role("button")
                        .label(format!("{soon} keys with a deadline soon"))
                        .h(20.0)
                        .min_w(20.0)
                        .px(7.0)
                        .radius(10.0)
                        .center()
                        .fill(if active {
                            k.c.text_contrast.alpha(0.22)
                        } else {
                            k.c.warning.icon.alpha(0.18)
                        })
                        .cursor(rune::Cursor::Pointer)
                        .help("Redemption deadlines within 30 days or past. Click to show them.")
                        .on_press(|s: &mut AppState| s.show_expiring())
                        .child(
                            text(format!("{soon} soon"))
                                .font(k.ty.text_xsmall_medium)
                                .color(if active {
                                    k.c.text_contrast
                                } else {
                                    k.c.warning.icon
                                })
                                .a11y_hidden(),
                        ),
                );
            }
        }
        if let Some(b) = badge {
            r = r.child(
                text(b)
                    .font(k.ty.text_xsmall_medium)
                    .color(if active {
                        k.c.text_contrast
                    } else {
                        k.c.text_faint
                    })
                    .a11y_hidden(),
            );
        }
        r
    };
    let mut rows = col().key("nav").w_full().gap(6.0);
    for p in Page::ALL {
        rows = rows.child(nav(p));
    }
    let footer = col()
        .w_full()
        .gap(10.0)
        .child(el().h(1.0).w_full().fill(k.c.border_weak))
        .child(
            footer_item(k, "nav-shortcuts", "Keyboard shortcuts", "F1".into())
                .on_press(|s: &mut AppState| s.show_shortcuts(true)),
        )
        .child(
            footer_item(k, "nav-lock", "Lock", shortcut("Ctrl+L"))
                .on_press_tx(|s: &mut AppState, tx: &mut Tx| s.lock(tx)),
        )
        .child(
            row()
                .w_full()
                .center_y()
                .child(
                    text(
                        s.job
                            .as_ref()
                            .map_or_else(|| save_status(sess), |j| j.progress()),
                    )
                    .key("save-status")
                    .font(k.ty.text_xsmall)
                    .color(k.c.text_faint)
                    .grow(1.0),
                )
                .child(
                    text(concat!("v", env!("CARGO_PKG_VERSION")))
                        .font(k.ty.text_xsmall)
                        .color(k.c.text_faint),
                ),
        );
    col()
        .key("sidebar")
        .w(232.0)
        .h_full()
        .padding(18.0)
        .gap(26.0)
        .fill(k.c.bg_base)
        .border(1.0, k.c.border_weak)
        .child(
            row()
                .gap(9.0)
                .center_y()
                .pl(6.0)
                .child(
                    el().size(30.0, 30.0)
                        .radius(9.0)
                        .center()
                        .fill(k.c.bg_accent)
                        .shadow(2.0, 10.0, k.c.bg_accent.alpha(0.35))
                        .child(
                            rune::icon(&super::KEY, 18.0)
                                .color(k.c.text_contrast)
                                .a11y_hidden(),
                        ),
                )
                .child(
                    text("KeyKeeper")
                        .font(rune::TextStyle::new(21.0, 750.0).lh(28.0).tracking(-0.3))
                        .color(k.c.text_strong),
                ),
        )
        .child(rows.grow(1.0))
        .child(footer)
}

/// A floating cart button in the corner of the page while the cart has keys (except on the Cart
/// page itself). It springs up briefly when keys are added.
fn cart_bauble(s: &AppState, sess: &Session, detail_open: bool, k: &Kit) -> Option<El> {
    let n = sess.repo.cart_len();
    if n == 0 || sess.page == Page::Cart {
        return None;
    }
    let right = 28.0 + if detail_open { 400.0 } else { 0.0 };
    let badge = el()
        .pin(None, Some(-4.0), Some(-4.0), None)
        .h(22.0)
        .min_w(22.0)
        .px(6.0)
        .radius(11.0)
        .center()
        .fill(k.c.danger.icon)
        .border(2.0, k.c.page)
        .child(
            text(n.to_string())
                .font(k.ty.text_xsmall_medium)
                .color(k.c.white)
                .a11y_hidden(),
        );
    Some(
        el().key("cart-bauble")
            .role("button")
            .label(format!("Cart, {n} key{}", if n == 1 { "" } else { "s" }))
            .focusable(true)
            .pin(None, None, Some(right), Some(28.0))
            .size(56.0, 56.0)
            .radius(28.0)
            .center()
            .fill(k.c.bg_accent)
            .hover(k.c.bg_accent.mix(k.c.white, 0.12))
            .press(k.c.bg_accent.mix(k.c.black, 0.15))
            .shadow(4.0, 18.0, k.c.black.alpha(0.35))
            .scale(if s.cart_bumping() { 1.18 } else { 1.0 })
            .cursor(rune::Cursor::Pointer)
            .help("Go to cart")
            .on_press(|s: &mut AppState| s.go(Page::Cart))
            .child(
                rune::icon(&super::CART, 26.0)
                    .color(k.c.text_contrast)
                    .a11y_hidden(),
            )
            .child(badge),
    )
}

/// A quiet sidebar-footer button with its shortcut on the right.
fn footer_item(k: &Kit, key: &str, label: &str, hint: String) -> El {
    row()
        .key(key.to_string())
        .role("button")
        .label(format!("{label} ({hint})"))
        .focusable(true)
        .w_full()
        .h(32.0)
        .px(14.0)
        .center_y()
        .radius(10.0)
        .hover(k.c.surface_hover)
        .child(
            text(label.to_string())
                .font(k.ty.ui_medium)
                .color(k.c.text_muted)
                .grow(1.0)
                .a11y_hidden(),
        )
        .child(
            text(hint)
                .font(k.ty.text_xsmall)
                .color(k.c.text_faint)
                .a11y_hidden(),
        )
}

fn save_status(s: &Session) -> String {
    match (&s.save_error, s.is_dirty()) {
        (Some(_), _) => "Save failed".into(),
        (None, true) => "Saving…".into(),
        (None, false) => "All changes saved".into(),
    }
}

fn confirm_dialog(sess: &Session, k: &Kit, overlays: &mut Vec<El>) {
    let Some(c) = &sess.confirm else { return };
    let (title, desc, action, danger) = match c {
        Confirm::DeleteKeys(ids) => (
            if ids.len() == 1 { "Delete this key?".to_string() } else { format!("Delete {} keys?", ids.len()) },
            format!("They're removed from the vault. You can undo with {} until you lock.", shortcut("Ctrl+Z")),
            "Delete",
            true,
        ),
        Confirm::ExportPlain(kind) => (
            format!("Export {} without encryption?", if *kind == ExportKind::Csv { "CSV" } else { "JSON" }),
            "The file will contain every key in the current view in plain text. Anyone who can read the file can redeem them.".to_string(),
            "Export",
            false,
        ),
        Confirm::RedeemCart => (
            format!("Redeem all {} cart keys yourself?", sess.repo.cart_len()),
            format!("They're marked Redeemed with today's date and leave the cart. Activate each one on Steam with its Redeem button or by pasting it. You can undo with {}.", shortcut("Ctrl+Z")),
            "Mark all redeemed",
            false,
        ),
        Confirm::BulkEdit => {
            let n = sess.bulk.as_ref().map_or(0, |b| b.ids.len());
            let status = sess.bulk.as_ref().and_then(|b| b.status);
            (
                format!("Change {n} keys?"),
                match status {
                    Some(st) if st.is_gone() => format!(
                        "They'll all be marked {}. You can undo with {}.",
                        st.label(),
                        shortcut("Ctrl+Z")
                    ),
                    _ => format!("You can undo with {}.", shortcut("Ctrl+Z")),
                },
                "Apply",
                false,
            )
        }
        Confirm::CancelTrade(_) => (
            "Cancel this trade?".to_string(),
            format!("Its keys go back to the status they had before you added them to the cart. You can undo with {}.", shortcut("Ctrl+Z")),
            "Cancel trade",
            true,
        ),
    };
    let buttons = row()
        .key("confirm-actions")
        .w_full()
        .gap(8.0)
        .justify_end()
        .child(
            Button::new("Cancel")
                .variant(ButtonVariant::Neutral)
                .on_press(|s: &mut AppState| s.cancel_confirm())
                .build(k),
        )
        .child(
            el().key("confirm-ok").child(
                Button::new(action)
                    .variant(if danger {
                        ButtonVariant::Danger
                    } else {
                        ButtonVariant::Submit
                    })
                    .on_press_tx(|s: &mut AppState, tx: &mut Tx| s.confirm(tx))
                    .build(k),
            ),
        );
    Dialog::new("confirm", title)
        .open(true) // rune-kit dialogs render nothing unless opened
        .description(desc)
        .width(420.0)
        .child(buttons)
        .on_close(|s: &mut AppState| s.cancel_confirm())
        .build(k, overlays);
}

pub fn toasts(s: &AppState, k: &Kit) -> Option<El> {
    if s.toasts.is_empty() {
        return None;
    }
    // Bottom-left of the page area, so toasts never cover the editor's Save / Delete footer
    // (rune-kit's ToastRegion pins bottom-right).
    let mut region = col()
        .key("toast-region")
        .role("region")
        .label("Notifications")
        .pin(Some(256.0), None, None, Some(24.0))
        .w(380.0)
        .gap(10.0);
    for t in &s.toasts {
        let id = t.id;
        let mut toast = Toast::new(format!("toast-{id}"), t.title.clone())
            .closable()
            .on_close(move |s: &mut AppState| s.dismiss_toast(id));
        if let Some(d) = &t.detail {
            toast = toast.description(d.clone());
        }
        if t.error {
            toast = toast.icon(k, k.glyph.circle_x);
        }
        if t.undo {
            toast = toast.action(k, "Undo", true, move |s: &mut AppState, _: &mut Tx| {
                s.undo_from_toast(id)
            });
        }
        region = region.child(toast.build(k));
    }
    Some(region)
}
