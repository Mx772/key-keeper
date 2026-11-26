//! "My Library": search, filters, bulk actions and the key table.

use std::borrow::Cow;

use keykeeper_core::{GameKey, KeyStatus};
use rune::{Align, El, Tx, col, el, row, text};
use rune_kit::{
    Button, ButtonVariant, Cell, Kit, Menu, MenuEntry, MenuItem, Select, Table, TableColumn,
    TextInput, context_menu,
};
use uuid::Uuid;

use super::{
    chip, hstack, muted, primary, sess_edit, spacer, status_colors, table_state, table_state_mut,
    title_style,
};
use crate::app::{AppState, Page, Session, today};
use crate::config::{Density, LibraryView};
use crate::form::format_date;
use crate::layout::{COLUMNS, DisplayRow, parse_group_id};

fn key_at(s: &AppState, i: usize) -> Option<&GameKey> {
    let sess = s.session()?;
    sess.display_key(i).and_then(|id| sess.repo.key(id))
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
        .flex_wrap()
        .gap(10.0)
        .child(
            col()
                .gap(6.0)
                .child(
                    text("My Library")
                        .font(title_style())
                        .color(k.c.text_strong),
                )
                .child(
                    row()
                        .gap(10.0)
                        .center_y()
                        .flex_wrap()
                        .child(muted(k, subtitle).key("library-count"))
                        .children(stats_strip(sess, k)),
                ),
        )
        .child(spacer())
        .child(
            hstack()
                .key("library-actions")
                .child(steam_menu(s, sess, k, overlays))
                .child(
                    Button::new("Import")
                        .variant(ButtonVariant::Outline)
                        .on_press(|s: &mut AppState| s.go(Page::Import))
                        .build(k),
                )
                .child(primary(k, "add-key", "+ Add key", true, |s, _| s.new_key())),
        );

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

    let mut chips: Vec<El> = vec![chip(
        k,
        "filter-all",
        "All",
        sess.statuses.is_empty(),
        |s| s.clear_status_filter(),
    )];
    for st in KeyStatus::ALL {
        chips.push(chip(
            k,
            format!("filter-{}", st.label().to_lowercase()),
            st.label(),
            sess.statuses.contains(&st),
            move |s| s.toggle_status_filter(st),
        ));
    }
    let toggles: Vec<El> = vec![
        chip(
            k,
            "filter-hide-gone",
            "Hide traded & redeemed",
            sess.hide_gone,
            |s| s.toggle_hide_gone(),
        ),
        chip(
            k,
            "filter-deadline",
            "Deadline ≤ 30 days",
            sess.deadline_soon,
            |s| s.toggle_deadline_soon(),
        ),
        chip(
            k,
            "filter-no-steam",
            "No Steam id",
            sess.missing_app_id,
            |s| s.toggle_missing_app_id(),
        ),
            chip(
                k,
                "filter-fodder",
                "Spares I own",
                sess.trade_fodder,
                |s| s.toggle_trade_fodder(),
            )
            .help("Keys still in hand for games you already own on Steam (Settings › Steam › Check library)"),
            chip(
                k,
                "filter-dupes",
                "Duplicates",
                sess.duplicates,
                |s| s.toggle_duplicates(),
            )
            .help("Games you have more than one key for"),
    ];
    // One wrapping row of chips, so narrow windows (or an open editor) never cut one off.
    let filters = row()
        .key("status-filters")
        .w_full()
        .gap(6.0)
        .center_y()
        .flex_wrap()
        .children(chips)
        .child(el().w(10.0))
        .children(toggles);
    let search_row = row()
        .w_full()
        .gap(10.0)
        .center_y()
        .child(search)
        .child(spacer())
        .child(view_menu(s, sess, k, overlays))
        .child(
            el().key("copy-have-list").child(
                Button::new("Copy have list")
                    .variant(ButtonVariant::Ghost)
                    .help("Copy the names of the tradable keys in view, for a trading site's \"have\" list (no keys)")
                    .on_press_tx(|s: &mut AppState, tx: &mut Tx| s.copy_have_list(tx))
                    .build(k),
            ),
        );
    let toolbar = col().w_full().gap(12.0).child(search_row).child(filters);

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
    } else if shown == 0 {
        page = page.child(no_matches(k));
    } else {
        let area = if s.config.library.view == LibraryView::Grid {
            super::row_menu::wrap(sess, super::grid::view(s, sess, k), k, overlays)
        } else {
            table(s, sess, k, overlays)
        };
        page = page.child(area.w_full().min_w(0.0).grow(1.0).min_h(0.0));
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

/// A small clickable figure in the stats strip.
fn stat(k: &Kit, key: &str, label: String, color: rune::Color, on: bool, help: &str) -> El {
    row()
        .key(key.to_string())
        .role("button")
        .label(label.clone())
        .focusable(true)
        .h(22.0)
        .px(9.0)
        .radius(11.0)
        .center_y()
        .fill(if on {
            color.alpha(0.22)
        } else {
            color.alpha(0.10)
        })
        .hover(color.alpha(0.22))
        .cursor(rune::Cursor::Pointer)
        .help(help.to_string())
        .child(
            text(label)
                .font(k.ty.text_xsmall_medium)
                .color(color)
                .a11y_hidden(),
        )
}

/// "12 to trade · 2 in cart · ~$143 value · 3 expiring", each one a shortcut to its filter.
fn stats_strip(sess: &Session, k: &Kit) -> Option<El> {
    let st = sess.stats(today());
    if st.total == 0 {
        return None;
    }
    let mut r = row().key("library-stats").gap(6.0).center_y();
    r = r.child(
        stat(
            k,
            "stat-tradable",
            format!("{} to trade", st.tradable),
            k.c.success.icon,
            sess.statuses == [KeyStatus::Available],
            "Keys still in hand (Available or Unknown). Click to show Available keys.",
        )
        .on_press(|s: &mut AppState| s.only_status(KeyStatus::Available)),
    );
    if st.in_cart > 0 {
        r = r.child(
            stat(
                k,
                "stat-cart",
                format!("{} in cart", st.in_cart),
                k.c.info.icon,
                false,
                "Go to the cart",
            )
            .on_press(|s: &mut AppState| s.go(Page::Cart)),
        );
    }
    if let Some(v) = &st.value
        && st.priced > 0
    {
        r = r.child(
            stat(
                k,
                "stat-value",
                format!("~{} value", v.format()),
                k.c.text_accent,
                false,
                "The cheapest current store price of each key still in hand, added up. Click to sort by price.",
            )
            .on_press(|s: &mut AppState| s.sort_by("deals", rune_kit::SortDir::Descending)),
        );
    }
    if st.expiring > 0 {
        let label = if st.expired > 0 && st.expiring > st.expired {
            format!(
                "{} expiring · {} past deadline",
                st.expiring - st.expired,
                st.expired
            )
        } else if st.expired > 0 {
            format!("{} past deadline", st.expired)
        } else {
            format!("{} expiring soon", st.expiring)
        };
        r = r.child(
            stat(
                k,
                "stat-expiring",
                label,
                if st.expired > 0 {
                    k.c.danger.icon
                } else {
                    k.c.warning.icon
                },
                sess.deadline_soon,
                "Keys you still hold whose redemption deadline is within 30 days or already past.",
            )
            .on_press(|s: &mut AppState| s.toggle_deadline_soon()),
        );
    }
    Some(r)
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
        .min_h(48.0)
        .px(14.0)
        .py(7.0)
        .gap(8.0)
        .center_y()
        .flex_wrap()
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
            el().key("bulk-edit-open").child(
                Button::new("Edit…")
                    .variant(ButtonVariant::Neutral)
                    .disabled(n < 2)
                    .help("Change status, bundle, tags or deadline for all of them")
                    .on_press(|s: &mut AppState| s.open_bulk_edit())
                    .build(k),
            ),
        )
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

/// The Steam actions, folded into one menu so the header fits beside the editor.
fn steam_menu(s: &AppState, sess: &Session, k: &Kit, overlays: &mut Vec<El>) -> El {
    let busy = s.steam_busy();
    let trigger = el().key("steam-menu-button").child(
        Button::new("Steam")
            .variant(ButtonVariant::Outline)
            .icon(k.glyph.chevron_down)
            .help("Match games, refresh ratings and prices")
            .on_press(|s: &mut AppState| {
                s.hover = None;
                if let Some(x) = s.session_mut() {
                    x.steam_menu = !x.steam_menu;
                }
            })
            .build(k),
    );
    let close = |s: &mut AppState| {
        if let Some(x) = s.session_mut() {
            x.steam_menu = false;
        }
    };
    Menu::new("steam-menu")
        .trigger(trigger)
        .open(sess.steam_menu)
        .label("Steam actions")
        .min_width(250.0)
        .on_close(close)
        .item(
            MenuItem::new("Match on Steam")
                .key("steam-match-all")
                .description("Find app ids for keys without one")
                .disabled(busy)
                .on_select(move |s: &mut AppState| {
                    close(s);
                    s.match_missing(&[]);
                }),
        )
        .item(
            MenuItem::new("Refresh ratings")
                .key("steam-ratings")
                .disabled(busy)
                .on_select(move |s: &mut AppState| {
                    close(s);
                    s.refresh_ratings(true);
                }),
        )
        .item(
            MenuItem::new("Refresh prices")
                .key("prices-refresh")
                .description("IsThereAnyDeal / gg.deals (keys in Settings › Prices)")
                .disabled(busy)
                .on_select(move |s: &mut AppState| {
                    close(s);
                    s.refresh_prices(true);
                }),
        )
        .separator()
        .item(
            MenuItem::new("Wishlist & library check…")
                .key("steam-settings")
                .on_select(move |s: &mut AppState| {
                    close(s);
                    s.go(Page::Settings);
                }),
        )
        .build(k, overlays)
}

/// Keys exist, but none match the search and filters.
fn no_matches(k: &Kit) -> El {
    col()
        .key("library-no-matches")
        .w_full()
        .grow(1.0)
        .center()
        .gap(12.0)
        .child(
            rune::icon(k.glyph.search, 34.0)
                .color(k.c.text_faint)
                .a11y_hidden(),
        )
        .child(
            text("No keys match")
                .font(rune::TextStyle::new(20.0, 650.0))
                .color(k.c.text_strong),
        )
        .child(muted(k, "Try another search, or clear the filters."))
        .child(primary(
            k,
            "clear-filters",
            "Clear search and filters",
            true,
            |s, _| s.clear_all_filters(),
        ))
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
        .children(at_low.then(|| {
            el().key(format!("atl-{id}"))
                .h(16.0)
                .px(5.0)
                .radius(4.0)
                .center()
                .fill(k.c.success.icon.alpha(0.18))
                .child(
                    text("LOW")
                        .font(rune::TextStyle::new(9.5, 700.0).tracking(0.6))
                        .color(k.c.success.icon)
                        .a11y_hidden(),
                )
        }))
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
    e = e.label(if at_low {
        format!("Best price {}, its all-time low", best.format())
    } else {
        format!("Best price {}", best.format())
    });
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

/// Near deadlines read as "in 12 days" / "3 days ago" (the date on hover); far ones as dates.
fn deadline_cell(
    k: &Kit,
    key: &GameKey,
    fmt: keykeeper_core::DateFormat,
    today: jiff::civil::Date,
) -> Cell {
    let Some(d) = key.redemption_deadline else {
        return Cell::text("");
    };
    let date = format_date(Some(d), fmt);
    if key.status.is_gone() {
        return Cell::text(date).muted();
    }
    let days = (d - today).get_days();
    let (label, color) = match days {
        ..0 => (super::card::relative_days(d, today), k.c.danger.icon),
        0..=30 => (super::card::relative_days(d, today), k.c.warning.icon),
        31..=60 => (super::card::relative_days(d, today), k.c.text_base),
        _ => return Cell::text(date).muted(),
    };
    Cell::el(
        row().w_full().justify_end().child(
            text(label)
                .font(k.ty.text_small_medium)
                .color(color)
                .help(format!("Redeem by {date}")),
        ),
    )
}

fn parse_id(id: &str) -> Option<Uuid> {
    Uuid::parse_str(id).ok()
}

fn table(state: &AppState, sess: &Session, k: &Kit, overlays: &mut Vec<El>) -> El {
    let kc = k.clone();
    let row_height = match state.config.library.density {
        Density::Comfortable => 32.0,
        Density::Compact => 25.0,
    };
    let group_fill = k.c.text_base.alpha(0.05);
    let fmt = sess.date_format();
    let today = today();
    let s_view = Table::new(
        table_state,
        table_state_mut,
        |s: &AppState| s.session().map_or(0, |x| x.display.len()),
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
            .min(170.0)
            .sortable(),
    )
    .column(
        TableColumn::new("bundle", "Bundle")
            .flex(1.3)
            .min(110.0)
            .sortable(),
    )
    .column(TableColumn::new("key", "Key").width(170.0).min(140.0))
    .column(TableColumn::new("status", "Status").width(120.0).sortable())
    .column(
        TableColumn::new("platform", "Platform")
            .width(90.0)
            .sortable(),
    )
    .column(
        TableColumn::new("deadline", "Deadline")
            .width(100.0)
            .align(Align::End)
            .sortable(),
    )
    .column(
        TableColumn::new("rating", "Rating")
            .width(70.0)
            .align(Align::End)
            .sortable(),
    )
    .column(TableColumn::new("deals", "Deals").width(140.0).sortable())
    .column(TableColumn::new("steam", "Steam").width(80.0))
    .column(
        TableColumn::new("added", "Added")
            .width(100.0)
            .align(Align::End)
            .sortable(),
    )
    .style(rune_kit::TableStyle {
        row_height,
        ..rune_kit::TableStyle::from_kit(k)
    })
    // Columns wider than the window scroll sideways instead of being cut off.
    .scroll_wide(true)
    .cell(move |s: &AppState, i, col| {
        let Some(key) = key_at(s, i) else {
            return match s.session().and_then(|x| x.display.get(i)) {
                Some(DisplayRow::Group {
                    name,
                    count,
                    collapsed,
                }) if col == "name" => Cell::el(group_cell(&kc, name, *count, *collapsed)),
                _ => Cell::text(""),
            };
        };
        match col {
            "name" => Cell::el(name_cell(&kc, key)),
            "deals" => match s.session().and_then(|x| x.repo.key_prices(key)) {
                Some(p) => Cell::el(deal_cell(&kc, key.id, p)),
                None => Cell::text(""),
            },
            "bundle" => match key.bundle.as_deref() {
                // Long names are cut off: the whole name on hover.
                Some(b) if b.chars().count() > 14 => Cell::el(
                    text(b.to_string())
                        .font(kc.ty.text_small)
                        .color(kc.c.text_muted)
                        .truncate()
                        .min_w(0.0)
                        .help(b.to_string()),
                ),
                b => Cell::text(b.unwrap_or("")).muted(),
            },
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
            "deadline" => deadline_cell(&kc, key, fmt, today),
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
    .on_select(|s: &mut AppState, id, tx| {
        if let Some(name) = parse_group_id(id) {
            // A click on a bundle header folds it; arrowing onto it doesn't.
            if tx.physical_key().is_none() {
                s.toggle_group(name);
            }
        } else if let Some(id) = parse_id(id) {
            s.open_key(id);
        }
    })
    .on_open(|s: &mut AppState, id, tx| {
        if let Some(name) = parse_group_id(id) {
            s.toggle_group(name);
        } else if let Some(id) = parse_id(id) {
            s.open_key(id);
            tx.focus("form-name", true);
        }
    })
    .on_header_secondary_press(|s: &mut AppState, _, p, _| {
        s.hover = None;
        if let Some(x) = s.session_mut() {
            x.header_menu = Some(p);
        }
    })
    .row(move |s: &AppState, i, row_el| {
        let Some(x) = s.session() else { return row_el };
        match x.display.get(i) {
            Some(DisplayRow::Key(id)) => {
                let id = *id;
                row_el
                    .on_secondary_press(move |s: &mut AppState, p, _| s.open_row_menu(id, Some(p)))
            }
            Some(DisplayRow::Group { name, .. }) => {
                let name = name.clone();
                let _ = name;
                row_el.fill(group_fill).cursor(rune::Cursor::Pointer)
            }
            None => row_el,
        }
    })
    .zebra(true)
    .build(state, k)
    .grow(1.0)
    .min_h(0.0)
    .min_w(0.0);
    let frame = column_menu(sess, frame_inner(k, s_view), k, overlays)
        .w_full()
        .min_w(0.0)
        .grow(1.0)
        .min_h(0.0);
    super::row_menu::wrap(sess, frame, k, overlays)
        .w_full()
        .min_w(0.0)
}

fn frame_inner(k: &Kit, s_view: El) -> El {
    col()
        .key("table-frame")
        // Before the table's own keys: the menu keys.
        .on_key_preview(|s: &mut AppState, ev, tx: &mut Tx| s.table_key_preview(ev, tx))
        .w_full()
        .min_w(0.0)
        .grow(1.0)
        .min_h(0.0)
        .radius(k.radius.lg)
        .fill(k.c.bg_base)
        .border(1.0, k.c.border_weak)
        .clip_rounded(k.radius.lg)
        .child(s_view)
}

/// A bundle's header row when grouping: fold arrow, name and how many keys.
fn group_cell(k: &Kit, name: &str, count: usize, collapsed: bool) -> El {
    row()
        .key(format!("group-{name}"))
        .label(format!(
            "{name}, {count} keys, {}",
            if collapsed { "collapsed" } else { "expanded" }
        ))
        .cursor(rune::Cursor::Pointer)
        .gap(8.0)
        .center_y()
        .min_w(0.0)
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
            el().h(18.0)
                .min_w(22.0)
                .px(6.0)
                .radius(9.0)
                .center()
                .fill(k.c.text_base.alpha(0.08))
                .child(
                    text(count.to_string())
                        .font(k.ty.text_xsmall_medium)
                        .color(k.c.text_muted),
                ),
        )
        .child(
            text(name.to_string())
                .font(k.ty.text_small_medium)
                .color(k.c.text_strong)
                .truncate()
                .min_w(0.0)
                .help(name.to_string()),
        )
}

/// Column checkboxes, shared by the View menu and the header's right-click menu.
fn column_entries(sess: &Session) -> Vec<MenuEntry> {
    let mut v: Vec<MenuEntry> = COLUMNS
        .iter()
        .map(|(id, title)| {
            let id = id.to_string();
            MenuEntry::item(
                MenuItem::new(title.to_string())
                    .key(format!("col-{id}"))
                    .checked(!sess.table.is_column_hidden(&id))
                    .on_select(move |s: &mut AppState| s.toggle_column(&id)),
            )
        })
        .collect();
    v.push(MenuEntry::Separator);
    v.push(MenuEntry::item(
        MenuItem::new("Reset columns")
            .key("col-reset")
            .on_select(|s: &mut AppState| s.reset_columns()),
    ));
    v
}

/// Right-click on a column header: which columns show.
fn column_menu(sess: &Session, area: El, k: &Kit, overlays: &mut Vec<El>) -> El {
    let mut m = context_menu("header-menu", area, sess.header_menu)
        .label("Columns")
        .min_width(200.0)
        .on_close(|s: &mut AppState| s.close_view_menu());
    if sess.header_menu.is_some() {
        m = m.group_label("Show columns");
        for e in column_entries(sess) {
            m = match e {
                MenuEntry::Item(i) => m.item(*i),
                MenuEntry::Separator => m.separator(),
                MenuEntry::Label(l) => m.group_label(l),
            };
        }
    }
    m.build(k, overlays)
}

/// The View menu: table or grid, row density, grouping and columns.
fn view_menu(s: &AppState, sess: &Session, k: &Kit, overlays: &mut Vec<El>) -> El {
    let lay = &s.config.library;
    let trigger = el().key("view-menu-button").child(
        Button::new("View")
            .variant(ButtonVariant::Ghost)
            .icon(k.glyph.chevron_down)
            .on_press(|s: &mut AppState| s.toggle_view_menu())
            .build(k),
    );
    let grid = lay.view == LibraryView::Grid;
    let compact = lay.density == Density::Compact;
    let mut m = Menu::new("view-menu")
        .trigger(trigger)
        .open(sess.view_menu)
        .label("View")
        .min_width(220.0)
        .on_close(|s: &mut AppState| s.close_view_menu())
        .group_label("Layout")
        .item(
            MenuItem::new("Table")
                .key("view-table")
                .checked(!grid)
                .on_select(|s: &mut AppState| s.set_library_view(LibraryView::Table)),
        )
        .item(
            MenuItem::new("Grid with artwork")
                .key("view-grid")
                .checked(grid)
                .on_select(|s: &mut AppState| s.set_library_view(LibraryView::Grid)),
        )
        .separator()
        .group_label("Rows")
        .item(
            MenuItem::new("Comfortable")
                .key("density-comfortable")
                .checked(!compact)
                .on_select(|s: &mut AppState| s.set_density(Density::Comfortable)),
        )
        .item(
            MenuItem::new("Compact")
                .key("density-compact")
                .checked(compact)
                .on_select(|s: &mut AppState| s.set_density(Density::Compact)),
        )
        .separator()
        .item(
            MenuItem::new("Group by bundle")
                .key("group-by-bundle")
                .checked(sess.group_by_bundle)
                .on_select(|s: &mut AppState| s.toggle_group_by_bundle()),
        );
    if sess.group_by_bundle {
        m = m
            .item(
                MenuItem::new("Collapse all bundles")
                    .key("groups-collapse")
                    .on_select(|s: &mut AppState| s.collapse_all_groups(true)),
            )
            .item(
                MenuItem::new("Expand all bundles")
                    .key("groups-expand")
                    .on_select(|s: &mut AppState| s.collapse_all_groups(false)),
            );
    }
    if !grid {
        m = m.separator().item(
            MenuItem::new("Columns")
                .key("view-columns")
                .submenu(column_entries(sess), sess.view_columns_open)
                .on_submenu(|s: &mut AppState, open| {
                    if let Some(x) = s.session_mut() {
                        x.view_columns_open = open;
                    }
                }),
        );
    }
    m.build(k, overlays)
}
