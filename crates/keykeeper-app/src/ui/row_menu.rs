//! The right-click menu on library rows (and grid cards).

use keykeeper_core::KeyStatus;
use rune::{El, PointerPress, Tx};
use rune_kit::{Kit, MenuEntry, MenuItem, context_menu};
use uuid::Uuid;

use super::shortcut;
use crate::app::{AppState, Session};

fn item(label: &str, f: impl Fn(&mut AppState, &mut Tx) + 'static) -> MenuItem {
    MenuItem::new(label.to_string()).on_select_tx(move |s: &mut AppState, tx: &mut Tx| {
        s.close_row_menu();
        f(s, tx);
    })
}

/// Wraps `area` (the table or grid) with the row menu when it's open.
pub fn wrap(sess: &Session, area: El, k: &Kit, overlays: &mut Vec<El>) -> El {
    let Some(menu) = &sess.row_menu else {
        return context_menu("row-menu", area, None).build(k, overlays);
    };
    let ids = menu.ids.clone();
    let keys: Vec<_> = ids.iter().filter_map(|id| sess.repo.key(*id)).collect();
    if keys.is_empty() {
        return context_menu("row-menu", area, None).build(k, overlays);
    }
    // Opened by keyboard: under the cursor row's name.
    let at = menu.at.or_else(|| {
        let first = ids.first()?;
        rune::memo::layout(&format!("name-{first}")).map(|r| PointerPress {
            x: r[0] + 24.0,
            y: r[1] + r[3],
        })
    });
    let at = at.unwrap_or(PointerPress { x: 420.0, y: 260.0 });
    let n = keys.len();
    let single = (n == 1).then(|| keys[0]);

    let mut m = context_menu("row-menu", area, Some(at))
        .label(if n == 1 {
            format!("{} actions", keys[0].game_name)
        } else {
            format!("{n} keys")
        })
        .min_width(230.0)
        .on_close(|s: &mut AppState| s.close_row_menu());
    if n > 1 {
        m = m.group_label(format!("{n} keys selected")).item(
            item(&format!("Edit {n} keys…"), |s, _| s.open_bulk_edit()).key("menu-bulk-edit"),
        );
    }

    if let Some(k0) = single {
        let id = k0.id;
        m = m.item(
            item("Edit", move |s, tx| {
                s.open_key(id);
                tx.focus("form-name", true);
            })
            .key("menu-edit")
            .shortcut("F2"),
        );
        m = m.item(
            item("Copy key", move |s, tx| s.copy_key(id, tx))
                .key("menu-copy")
                .shortcut(shortcut("Ctrl+C")),
        );
    }
    {
        let ids = ids.clone();
        m = m.item(
            item("Copy \"Name: Key\"", move |s, tx| {
                s.copy_name_keys(&ids, tx)
            })
            .key("menu-copy-lines")
            .shortcut(shortcut("Ctrl+Shift+C")),
        );
    }
    m = m.separator();

    let tradable: Vec<Uuid> = keys
        .iter()
        .filter(|k| k.status.can_trade())
        .map(|k| k.id)
        .collect();
    let in_cart: Vec<Uuid> = ids.iter().copied().filter(|id| sess.in_cart(*id)).collect();
    if !tradable.is_empty() {
        m = m.item(
            item(
                &if tradable.len() == n {
                    "Add to cart".to_string()
                } else {
                    format!("Add {} to cart", tradable.len())
                },
                move |s, _| s.add_to_cart(&tradable),
            )
            .key("menu-cart")
            .shortcut(shortcut("Ctrl+B")),
        );
    }
    if !in_cart.is_empty() {
        m = m.item(
            item("Remove from cart", move |s, _| {
                for id in &in_cart {
                    s.remove_from_cart(*id);
                }
            })
            .key("menu-uncart"),
        );
    }
    let redeemable: Vec<Uuid> = keys
        .iter()
        .filter(|k| k.status.can_trade() || sess.in_cart(k.id))
        .map(|k| k.id)
        .collect();
    if !redeemable.is_empty() {
        let label = match single {
            Some(k0) if k0.steam_register_url().is_some() => "Redeem on Steam",
            _ => "Mark redeemed by me",
        };
        m = m.item(
            item(label, move |s, tx| s.redeem_self(&redeemable, true, tx)).key("menu-redeem"),
        );
    }

    let current = single.map(|k| k.status);
    let mut statuses = Vec::new();
    for st in KeyStatus::ALL {
        let ids = ids.clone();
        statuses.push(MenuEntry::item(
            item(st.label(), move |s, _| s.set_status(&ids, st))
                .key(format!("menu-status-{}", st.label().to_lowercase()))
                .checked(current == Some(st)),
        ));
    }
    m = m.item(
        MenuItem::new("Set status")
            .key("menu-status")
            .submenu(statuses, menu.status_open)
            .on_submenu(|s: &mut AppState, open| s.set_row_menu_status_open(open)),
    );
    m = m.separator();

    if let Some(k0) = single {
        let mut any = false;
        for (slug, label, url) in [
            ("steam", "Open on Steam", k0.steam_url()),
            ("gg", "Open on gg.deals", k0.gg_deals_url()),
            ("itad", "Open on IsThereAnyDeal", k0.itad_url()),
            ("page", "Open key page", k0.key_url.clone()),
        ] {
            if let Some(url) = url {
                any = true;
                m = m.item(
                    item(label, move |_, tx| {
                        let _ = tx.open_url(&url);
                    })
                    .key(format!("menu-link-{slug}")),
                );
            }
        }
        if let Some(t) = k0.trade_id.and_then(|t| sess.repo.trade(t)) {
            let tid = t.id;
            any = true;
            m = m.item(item("Show trade", move |s, _| s.select_trade(tid)).key("menu-show-trade"));
            if let Some(url) = t.trade_url.clone() {
                m = m.item(
                    item("Open trade link", move |_, tx| {
                        let _ = tx.open_url(&url);
                    })
                    .key("menu-link-trade"),
                );
            }
        }
        if k0.steam_app_id.is_none() {
            let id = k0.id;
            any = true;
            m = m.item(item("Find on Steam", move |s, _| s.find_on_steam(id)).key("menu-find"));
        }
        if any {
            m = m.separator();
        }
    } else if keys.iter().any(|k| k.steam_app_id.is_none()) {
        let ids = ids.clone();
        m = m
            .item(item("Match on Steam", move |s, _| s.match_missing(&ids)).key("menu-match"))
            .separator();
    }
    if keys.iter().any(|k| k.steam_app_id.is_some()) {
        let ids = ids.clone();
        m = m
            .item(
                item("Refresh prices & ratings", move |s, _| {
                    s.refresh_selected(&ids)
                })
                .key("menu-refresh"),
            )
            .separator();
    }

    m.item(
        item(
            &if n == 1 {
                "Delete…".to_string()
            } else {
                format!("Delete {n} keys…")
            },
            move |s, _| s.ask_delete(ids.clone()),
        )
        .key("menu-delete")
        .shortcut("Del")
        .danger(true),
    )
    .build(k, overlays)
}
