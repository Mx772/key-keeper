//! The library as a grid of cards with each game's artwork.

use keykeeper_core::GameKey;
use rune::{El, Tx, col, row, text};
use rune_kit::Kit;

use super::card::{HEADER_RATIO, art, relative_days};
use super::{spacer, status_pill};
use crate::app::{AppState, Session, today};
use crate::layout::DisplayRow;

pub const CARD_W: f32 = 228.0;

fn rating_color(k: &Kit, p: u8) -> rune::Color {
    match p {
        80.. => k.c.success.icon,
        60..80 => k.c.text_base,
        40..60 => k.c.warning.icon,
        _ => k.c.danger.icon,
    }
}

fn game_card(s: &AppState, sess: &Session, key: &GameKey, k: &Kit) -> El {
    let id = key.id;
    let selected = sess.table.is_selected(&id.to_string());
    let art_h = (CARD_W * HEADER_RATIO).round();
    let r = k.radius.lg;

    let mut meta = row()
        .w_full()
        .gap(6.0)
        .center_y()
        .child(status_pill(k, key.status));
    meta = meta.child(spacer());
    if let Some(rt) = key.steam.as_ref().and_then(|m| m.rating.as_ref()) {
        meta = meta.child(
            text(format!("{}%", rt.percent))
                .font(k.ty.mono)
                .color(rating_color(k, rt.percent)),
        );
    }

    let mut foot = row().w_full().gap(6.0).center_y();
    if let Some(p) = sess.repo.key_prices(key)
        && let Some((best, _)) = p.best()
    {
        let low = p
            .history_low
            .as_ref()
            .is_some_and(|l| best.cents <= l.cents);
        foot = foot.child(text(best.format()).font(k.ty.mono).color(if low {
            k.c.success.icon
        } else {
            k.c.text_muted
        }));
    }
    foot = foot.child(spacer());
    if let Some(d) = key.redemption_deadline
        && !key.status.is_gone()
    {
        let t = today();
        let days = (d - t).get_days();
        if days <= 60 {
            foot = foot.child(
                text(relative_days(d, t))
                    .font(k.ty.text_xsmall_medium)
                    .color(if days < 0 {
                        k.c.danger.icon
                    } else if days <= 30 {
                        k.c.warning.icon
                    } else {
                        k.c.text_muted
                    }),
            );
        }
    }
    if let Some(m) = key.steam.as_ref() {
        match m.ownership {
            keykeeper_core::Ownership::Owned => {
                foot = foot.child(
                    text("Owned")
                        .font(k.ty.text_xsmall_medium)
                        .color(k.c.success.icon),
                )
            }
            keykeeper_core::Ownership::Wishlisted => {
                foot = foot.child(
                    text("Wishlist")
                        .font(k.ty.text_xsmall_medium)
                        .color(k.c.text_accent),
                )
            }
            _ => {}
        }
    }

    let body = col()
        .w_full()
        .px(12.0)
        .pt(10.0)
        .pb(12.0)
        .gap(8.0)
        .child(
            text(key.game_name.clone())
                .font(k.ty.ui_medium)
                .color(k.c.text_strong)
                .truncate()
                .w_full(),
        )
        .child(meta)
        .child(foot);

    col()
        .key(format!("name-{id}"))
        .role("button")
        .label(format!("{}, {}", key.game_name, key.status.label()))
        .checked(selected)
        .focusable(true)
        .w(CARD_W)
        .radius(r)
        .fill(k.c.bg_base)
        .border(
            if selected { 2.0 } else { 1.0 },
            if selected {
                k.c.bg_accent
            } else {
                k.c.border_weak
            },
        )
        .clip_rounded(r)
        .hover(k.c.surface_hover)
        .cursor(rune::Cursor::Pointer)
        .on_press(move |s: &mut AppState| s.select_card(id))
        .on_secondary_press(move |s: &mut AppState, p, _| s.open_row_menu(id, Some(p)))
        .on_hover_change(move |s: &mut AppState, inside, tx: &mut Tx| {
            s.hover_key(id, inside, tx.clock());
        })
        .child(art(s, key, CARD_W, art_h, [r, r, 0.0, 0.0]))
        .child(body)
}

fn group_header(k: &Kit, name: &str, count: usize, collapsed: bool) -> El {
    let toggle = name.to_string();
    row()
        .key(format!("grid-group-{name}"))
        .role("button")
        .label(format!("{name}, {count} keys"))
        .w_full()
        .pt(6.0)
        .gap(8.0)
        .center_y()
        .cursor(rune::Cursor::Pointer)
        .on_press(move |s: &mut AppState| s.toggle_group(&toggle))
        .child(
            rune::icon(
                if collapsed {
                    k.glyph.caret_right
                } else {
                    k.glyph.chevron_down
                },
                14.0,
            )
            .color(k.c.text_muted),
        )
        .child(
            text(name.to_string())
                .font(super::heading_style())
                .color(k.c.text_strong),
        )
        .child(
            text(count.to_string())
                .font(k.ty.text_small_medium)
                .color(k.c.text_faint),
        )
}

pub fn view(s: &AppState, sess: &Session, k: &Kit) -> El {
    let mut sections = col().key("library-grid").w_full().gap(14.0);
    let mut cards = row().w_full().gap(14.0).flex_wrap();
    let mut any = false;
    for r in &sess.display {
        match r {
            DisplayRow::Group {
                name,
                count,
                collapsed,
            } => {
                if any {
                    sections = sections.child(cards);
                    cards = row().w_full().gap(14.0).flex_wrap();
                    any = false;
                }
                sections = sections.child(group_header(k, name, *count, *collapsed));
            }
            DisplayRow::Key(id) => {
                if let Some(key) = sess.repo.key(*id) {
                    cards = cards.child(game_card(s, sess, key, k));
                    any = true;
                }
            }
        }
    }
    if any {
        sections = sections.child(cards);
    }
    col()
        .key("library-grid-scroll")
        .w_full()
        .grow(1.0)
        .min_h(0.0)
        .min_w(0.0)
        .scroll(rune::ScrollSpec::default())
        .child(sections.pb(24.0))
}
