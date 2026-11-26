//! Trades list. The full trade flow (cart, checkout, rep) arrives in Phase 4; for now this
//! shows trades created by imports.

use rune::{El, ScrollSpec, col, el, row, text};
use rune_kit::Kit;

use super::{muted, title_style};
use crate::app::Session;

pub fn view(sess: &Session, k: &Kit) -> El {
    let trades = sess.repo.trades();
    let mut list = col().key("trades-list").w_full().gap(8.0);
    for t in trades.iter().rev() {
        let partner = if t.partner_name.is_empty() {
            "Unknown partner".to_string()
        } else {
            t.partner_name.clone()
        };
        list = list.child(
            row()
                .w_full()
                .h(56.0)
                .px(16.0)
                .gap(16.0)
                .center_y()
                .radius(k.radius.lg)
                .fill(k.c.bg_base)
                .border(1.0, k.c.border_weak)
                .child(
                    col()
                        .gap(2.0)
                        .grow(1.0)
                        .child(text(partner).font(k.ty.ui_medium).color(k.c.text_strong))
                        .child(muted(k, t.trade_url.clone().unwrap_or_default()).truncate()),
                )
                .child(muted(k, t.site.label().to_string()))
                .child(muted(
                    k,
                    format!(
                        "{} key{}",
                        t.key_ids.len(),
                        if t.key_ids.len() == 1 { "" } else { "s" }
                    ),
                ))
                .child(el().w(90.0).child(muted(k, format!("{:?}", t.status)))),
        );
    }
    let body = if trades.is_empty() {
        col().w_full().grow(1.0).center().gap(10.0).child(text("No trades yet").font(rune::TextStyle::new(20.0, 650.0)).color(k.c.text_strong)).child(muted(k, "The cart and checkout flow is coming next. Trades from imported spreadsheets appear here."))
    } else {
        col()
            .w_full()
            .grow(1.0)
            .min_h(0.0)
            .scroll(ScrollSpec::default())
            .child(list)
    };
    col()
        .key("trades")
        .w_full()
        .h_full()
        .px(32.0)
        .pt(28.0)
        .pb(20.0)
        .gap(20.0)
        .child(
            col()
                .gap(4.0)
                .child(text("Trades").font(title_style()).color(k.c.text_strong))
                .child(muted(k, format!("{} trades", trades.len()))),
        )
        .child(body)
}
