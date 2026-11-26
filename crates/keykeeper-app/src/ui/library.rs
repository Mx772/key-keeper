//! "My Library": search, filters, bulk actions and the key table.

use std::borrow::Cow;

use keykeeper_core::{GameKey, KeyStatus};
use rune::{Align, El, Tx, col, el, row, text};
use rune_kit::{Button, ButtonVariant, Cell, Kit, Select, Table, TableColumn, TextInput};
use uuid::Uuid;

use super::{
    chip, hstack, muted, primary, sess_edit, spacer, status_colors, table_state, table_state_mut,
    title_style,
};
use crate::app::{AppState, Page, Session, today};
use crate::form::format_date;

fn key_at(s: &AppState, i: usize) -> Option<&GameKey> {
    let sess = s.session()?;
    sess.rows.get(i).and_then(|id| sess.repo.key(*id))
}

pub fn view(s: &AppState, sess: &Session, k: &Kit, overlays: &mut Vec<El>) -> El {
    let total = sess.repo.keys().len();
    let shown = sess.rows.len();
    let selected = sess.table.selected_len();
    let subtitle = match (shown == total, selected) {
        (true, 0) => format!("{total} keys"),
        (true, n) => format!("{total} keys · {n} selected"),
        (false, 0) => format!("{shown} of {total} keys"),
        (false, n) => format!("{shown} of {total} keys · {n} selected"),
    };
    let header = row()
        .key("library-header")
        .w_full()
        .center_y()
        .gap(10.0)
        .child(
            col()
                .gap(4.0)
                .child(
                    text("My Library")
                        .font(title_style())
                        .color(k.c.text_strong),
                )
                .child(muted(k, subtitle).key("library-count")),
        )
        .child(spacer())
        .child(
            el().key("steam-match-all").child(
                Button::new("Match on Steam")
                    .variant(ButtonVariant::Outline)
                    .disabled(s.steam_busy())
                    .help("Find Steam app ids for keys that don't have one")
                    .on_press(|s: &mut AppState| s.match_missing(&[]))
                    .build(k),
            ),
        )
        .child(
            el().key("steam-ratings").child(
                Button::new("Refresh ratings")
                    .variant(ButtonVariant::Outline)
                    .disabled(s.steam_busy())
                    .on_press(|s: &mut AppState| s.refresh_ratings(true))
                    .build(k),
            ),
        )
        .child(
            el().key("prices-refresh").child(
                Button::new("Refresh prices")
                    .variant(ButtonVariant::Outline)
                    .disabled(s.steam_busy())
                    .help("Current store prices from IsThereAnyDeal / gg.deals (keys in Settings → Prices)")
                    .on_press(|s: &mut AppState| s.refresh_prices(true))
                    .build(k),
            ),
        )
        .child(
            Button::new("Import")
                .variant(ButtonVariant::Outline)
                .on_press(|s: &mut AppState| s.go(Page::Import))
                .build(k),
        )
        .child(primary(k, "add-key", "+ Add key", true, |s, _| s.new_key()));

    let search = TextInput::new(&sess.search)
        .placeholder("Search games, bundles, notes, keys…")
        .leading_icon(k.glyph.search)
        .clear_button()
        .width(Some(360.0))
        .on_edit(|s: &mut AppState| sess_edit(s, |x| &mut x.search))
        .on_change(|s: &mut AppState, _: &mut Tx| {
            if let Some(x) = s.session_mut() {
                x.filters_changed();
            }
        })
        .on_action(|s: &mut AppState, _: &mut Tx| {
            if let Some(x) = s.session_mut() {
                x.search.replace_text("");
                x.filters_changed();
            }
        })
        .build(k)
        .key("search");

    let mut chips = hstack().key("status-filters").gap(6.0).child(chip(
        k,
        "filter-all",
        "All",
        sess.statuses.is_empty(),
        |s| s.clear_status_filter(),
    ));
    for st in KeyStatus::ALL {
        chips = chips.child(chip(
            k,
            format!("filter-{}", st.label().to_lowercase()),
            st.label(),
            sess.statuses.contains(&st),
            move |s| s.toggle_status_filter(st),
        ));
    }
    let toggles = hstack()
        .gap(6.0)
        .child(chip(
            k,
            "filter-hide-gone",
            "Hide traded & redeemed",
            sess.hide_gone,
            |s| s.toggle_hide_gone(),
        ))
        .child(chip(
            k,
            "filter-deadline",
            "Deadline ≤ 30 days",
            sess.deadline_soon,
            |s| s.toggle_deadline_soon(),
        ))
        .child(chip(
            k,
            "filter-no-steam",
            "No Steam id",
            sess.missing_app_id,
            |s| s.toggle_missing_app_id(),
        ));
    let filters = row()
        .w_full()
        .gap(6.0)
        .center_y()
        .flex_wrap()
        .child(chips)
        .child(el().w(12.0))
        .child(toggles);
    let toolbar = col().w_full().gap(12.0).child(search).child(filters);

    let mut page = col()
        .key("library")
        .w_full()
        .h_full()
        .px(32.0)
        .pt(28.0)
        .pb(20.0)
        .gap(18.0)
        .child(header)
        .child(toolbar);
    if selected > 0 {
        page = page.child(bulk_bar(sess, k, overlays));
    }
    if total == 0 {
        page = page.child(empty_state(k));
    } else {
        page = page.child(table(s, sess, k));
    }
    if let Some(id) = s.hover_card_key()
        && let Some(key) = sess.repo.key(id)
    {
        overlays.push(super::card::hover_card(
            s,
            sess,
            key,
            &format!("name-{id}"),
            k,
        ));
    }
    page
}

fn bulk_bar(sess: &Session, k: &Kit, overlays: &mut Vec<El>) -> El {
    let n = sess.table.selected_len();
    let labels: Vec<&str> = KeyStatus::ALL.iter().map(|s| s.label()).collect();
    let status = Select::new("bulk-status", &labels)
        .placeholder("Set status")
        .open(sess.bulk_status_open)
        .width(150.0)
        .on_toggle(|s: &mut AppState| {
            if let Some(x) = s.session_mut() {
                x.bulk_status_open = !x.bulk_status_open;
            }
        })
        .on_close(|s: &mut AppState| {
            if let Some(x) = s.session_mut() {
                x.bulk_status_open = false;
            }
        })
        .on_choose(|s: &mut AppState, v: &str, _: &mut Tx| {
            let ids = s.session().map(Session::selected_ids).unwrap_or_default();
            if let Some(st) = KeyStatus::ALL.into_iter().find(|st| st.label() == v) {
                s.set_status(&ids, st);
            }
        })
        .build(k, overlays);
    row()
        .key("bulk-bar")
        .w_full()
        .h(48.0)
        .px(14.0)
        .gap(10.0)
        .center_y()
        .radius(k.radius.lg)
        .fill(k.c.surface_raised)
        .border(1.0, k.c.border_weak)
        .child(
            text(format!("{n} selected"))
                .font(k.ty.ui_medium)
                .color(k.c.text_strong),
        )
        .child(spacer())
        .child(
            el().key("bulk-steam").child(
                Button::new("Match on Steam")
                    .variant(ButtonVariant::Neutral)
                    .on_press(|s: &mut AppState| {
                        let ids = s.session().map(Session::selected_ids).unwrap_or_default();
                        s.match_missing(&ids);
                    })
                    .build(k),
            ),
        )
        .child(primary(k, "bulk-cart", "Add to cart", true, |s, _| {
            let ids = s.session().map(Session::selected_ids).unwrap_or_default();
            s.add_to_cart(&ids);
        }))
        .child(
            el().key("bulk-copy").child(
                Button::new("Copy \"Name: Key\"")
                    .variant(ButtonVariant::Neutral)
                    .shortcut(super::shortcut("Ctrl+Shift+C"))
                    .on_press_tx(|s: &mut AppState, tx: &mut Tx| {
                        let ids = s.session().map(Session::selected_ids).unwrap_or_default();
                        s.copy_name_keys(&ids, tx);
                    })
                    .build(k),
            ),
        )
        .child(status)
        .child(
            el().key("bulk-delete").child(
                Button::new("Delete")
                    .variant(ButtonVariant::Danger)
                    .on_press(|s: &mut AppState| {
                        let ids = s.session().map(Session::selected_ids).unwrap_or_default();
                        s.ask_delete(ids);
                    })
                    .build(k),
            ),
        )
        .child(
            Button::new("Clear")
                .variant(ButtonVariant::GhostMuted)
                .on_press(|s: &mut AppState| {
                    if let Some(x) = s.session_mut() {
                        x.table.clear_selection();
                    }
                })
                .build(k),
        )
}

fn empty_state(k: &Kit) -> El {
    col()
        .key("library-empty")
        .w_full()
        .grow(1.0)
        .center()
        .gap(14.0)
        .child(
            text("No keys yet")
                .font(rune::TextStyle::new(20.0, 650.0))
                .color(k.c.text_strong),
        )
        .child(muted(
            k,
            "Import a spreadsheet or text list, or add a key by hand.",
        ))
        .child(
            row()
                .gap(10.0)
                .child(primary(k, "empty-import", "Import keys", true, |s, _| {
                    s.go(Page::Import)
                }))
                .child(
                    Button::new("Add a key")
                        .variant(ButtonVariant::Outline)
                        .on_press(|s: &mut AppState| s.new_key())
                        .build(k),
                ),
        )
}

/// The game name as a link to its Steam page. Only the text is the link, so a press on the rest
/// of the cell still selects the row.
fn name_link(k: &Kit, key: &GameKey, url: String) -> El {
    row()
        .key(format!("name-link-{}", key.id))
        .role("link")
        .label(format!("{} on Steam", key.game_name))
        .min_w(0.0)
        .max_w(10_000.0)
        .h(22.0)
        .pr(4.0)
        .gap(5.0)
        .radius(5.0)
        .center_y()
        .hover(k.c.bg_accent.alpha(0.16))
        .cursor(rune::Cursor::Pointer)
        .help("Open on Steam")
        .on_press_tx(move |_: &mut AppState, tx: &mut Tx| {
            let _ = tx.open_url(&url);
        })
        .child(
            text(key.game_name.clone())
                .font(k.ty.ui)
                .color(k.c.text_strong)
                .truncate()
                .min_w(0.0)
                .a11y_hidden(),
        )
        .child(
            rune::icon(&super::EXTERNAL, 12.0)
                .color(k.c.text_accent)
                .a11y_hidden(),
        )
}

/// The name cell: a Steam link when the game has an app id. Resting the pointer on it opens the
/// details card (see `card::hover_card`).
fn name_cell(k: &Kit, key: &GameKey) -> El {
    let id = key.id;
    let inner = match key.steam_url() {
        Some(url) => name_link(k, key, url),
        None => text(key.game_name.clone())
            .font(k.ty.ui)
            .color(k.c.text_strong)
            .truncate()
            .min_w(0.0),
    };
    row()
        .key(format!("name-{id}"))
        .min_w(0.0)
        .center_y()
        .on_hover_change(move |s: &mut AppState, inside, tx: &mut Tx| {
            s.hover_key(id, inside, tx.clock());
        })
        .child(inner)
}

/// Best current price and its store; hover lists every store, press opens the best deal.
fn deal_cell(k: &Kit, id: Uuid, p: &keykeeper_core::GamePrices) -> El {
    let Some((best, shop)) = p.best() else {
        return text("—")
            .font(k.ty.text_small)
            .color(k.c.text_faint)
            .help("No current prices found");
    };
    let at_low = p
        .history_low
        .as_ref()
        .is_some_and(|l| best.cents <= l.cents);
    let rows = super::price_help(p);
    let mut e = row()
        .key(format!("deal-{id}"))
        .min_w(0.0)
        .h(22.0)
        .px(4.0)
        .gap(6.0)
        .radius(5.0)
        .center_y()
        .child(
            text(best.format())
                .font(k.ty.mono)
                .color(if at_low {
                    k.c.success.icon
                } else {
                    k.c.text_strong
                })
                .a11y_hidden(),
        )
        .child(
            text(shop.unwrap_or("best store").to_string())
                .font(k.ty.text_small)
                .color(k.c.text_muted)
                .truncate()
                .min_w(0.0)
                .a11y_hidden(),
        );
    let mut rows = rows.into_iter();
    if let Some(first) = rows.next() {
        e = e.help(first);
    }
    for r in rows {
        e = e.help_row(r);
    }
    e = e.label(format!("Best price {}", best.format()));
    if let Some(url) = super::price_url(p) {
        e = e
            .role("link")
            .hover(k.c.bg_accent.alpha(0.16))
            .cursor(rune::Cursor::Pointer)
            .on_press_tx(move |_: &mut AppState, tx: &mut Tx| {
                let _ = tx.open_url(&url);
            });
    }
    e
}

fn parse_id(id: &str) -> Option<Uuid> {
    Uuid::parse_str(id).ok()
}

fn table(state: &AppState, sess: &Session, k: &Kit) -> El {
    let kc = k.clone();
    let fmt = sess.date_format();
    let today = today();
    let s_view = Table::new(
        table_state,
        table_state_mut,
        |s: &AppState| s.session().map_or(0, |x| x.rows.len()),
        |s: &AppState, i| {
            Cow::Borrowed(
                s.session()
                    .and_then(|x| x.row_ids.get(i))
                    .map_or("", String::as_str),
            )
        },
    )
    .name("Keys")
    .column(
        TableColumn::new("name", "Game")
            .flex(2.2)
            .min(180.0)
            .sortable(),
    )
    .column(
        TableColumn::new("bundle", "Bundle")
            .flex(1.5)
            .min(120.0)
            .sortable(),
    )
    .column(TableColumn::new("key", "Key").width(200.0).min(150.0))
    .column(TableColumn::new("status", "Status").width(120.0).sortable())
    .column(
        TableColumn::new("platform", "Platform")
            .width(100.0)
            .sortable(),
    )
    .column(
        TableColumn::new("deadline", "Deadline")
            .width(110.0)
            .align(Align::End)
            .sortable(),
    )
    .column(
        TableColumn::new("rating", "Rating")
            .width(80.0)
            .align(Align::End)
            .sortable(),
    )
    .column(TableColumn::new("deals", "Deals").width(150.0).sortable())
    .column(TableColumn::new("steam", "Steam").width(90.0))
    .column(
        TableColumn::new("added", "Added")
            .width(110.0)
            .align(Align::End)
            .sortable(),
    )
    .cell(move |s: &AppState, i, col| {
        let Some(key) = key_at(s, i) else {
            return Cell::text("");
        };
        match col {
            "name" => Cell::el(name_cell(&kc, key)),
            "deals" => match s.session().and_then(|x| x.repo.key_prices(key)) {
                Some(p) => Cell::el(deal_cell(&kc, key.id, p)),
                None => Cell::text(""),
            },
            "bundle" => Cell::text(key.bundle.as_deref().unwrap_or("")).muted(),
            "key" => Cell::text(key.key.masked()).mono(),
            "platform" => Cell::text(key.platform.label()).muted(),
            "rating" => match key.steam.as_ref().and_then(|m| m.rating.as_ref()) {
                Some(r) => {
                    let c = match r.percent {
                        80.. => kc.c.success.icon,
                        60..80 => kc.c.text_base,
                        40..60 => kc.c.warning.icon,
                        _ => kc.c.danger.icon,
                    };
                    Cell::text(format!("{}%", r.percent)).mono().color(c)
                }
                None => Cell::text(if key.steam_app_id.is_some() {
                    "–"
                } else {
                    ""
                })
                .muted(),
            },
            "steam" => match key.steam.as_ref().map(|m| m.ownership) {
                Some(keykeeper_core::Ownership::Owned) => {
                    Cell::text("Owned").color(kc.c.success.icon)
                }
                Some(keykeeper_core::Ownership::Wishlisted) => {
                    Cell::text("Wishlist").color(kc.c.text_accent)
                }
                Some(keykeeper_core::Ownership::NotOwned) => Cell::text(""),
                _ => Cell::text(""),
            },
            "deadline" => {
                let label = format_date(key.redemption_deadline, fmt);
                match key.redemption_deadline {
                    Some(d) if !key.status.is_gone() && d < today => {
                        Cell::text(label).color(kc.c.danger.icon)
                    }
                    Some(d) if !key.status.is_gone() && (d - today).get_days() <= 30 => {
                        Cell::text(label).color(kc.c.warning.icon)
                    }
                    _ => Cell::text(label).muted(),
                }
            }
            "added" => Cell::text(format_date(
                Some(
                    key.created_at
                        .to_zoned_datetime(jiff::tz::TimeZone::system())
                        .date(),
                ),
                fmt,
            ))
            .muted(),
            _ => {
                let (bg, fg) = status_colors(&kc, key.status);
                Cell::el(
                    row().child(
                        el().h(20.0).px(8.0).radius(10.0).center().fill(bg).child(
                            text(key.status.label())
                                .font(kc.ty.text_xsmall_medium)
                                .color(fg),
                        ),
                    ),
                )
            }
        }
    })
    .on_sort(|s: &mut AppState, col, dir, _| s.sort_by(col, dir))
    .on_select(|s: &mut AppState, id, _| {
        if let Some(id) = parse_id(id) {
            s.open_key(id);
        }
    })
    .on_open(|s: &mut AppState, id, tx| {
        if let Some(id) = parse_id(id) {
            s.open_key(id);
            tx.focus("form-name", true);
        }
    })
    .row(|s: &AppState, i, row_el| {
        let Some(id) = s.session().and_then(|x| x.rows.get(i).copied()) else {
            return row_el;
        };
        row_el.on_secondary_press(move |s: &mut AppState, _, tx| s.copy_key(id, tx))
    })
    .zebra(true)
    .build(state, k)
    .grow(1.0)
    .min_h(0.0);
    col()
        .key("table-frame")
        .w_full()
        .grow(1.0)
        .min_h(0.0)
        .radius(k.radius.lg)
        .fill(k.c.bg_base)
        .border(1.0, k.c.border_weak)
        .clip_rounded(k.radius.lg)
        .child(s_view)
}
