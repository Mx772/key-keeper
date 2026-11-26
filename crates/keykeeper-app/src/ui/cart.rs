//! The cart / checkout page, plus pieces shared with the Trades page.

use keykeeper_core::{Trade, TradeSite, TradeStatus};
use rune::{El, ScrollSpec, TextEdit, Tx, col, el, row, text};
use rune_kit::{
    Button, ButtonVariant, Checkbox, IconButton, Kit, Select, SelectAppearance, TextArea, TextInput,
};

use super::{card, field, muted, primary, spacer, title_style, trade_edit};
use crate::app::{AppState, Page, Session};
use crate::trade_actions::TradeForm;

const SITES: [&str; 4] = ["Lestrades", "SteamTrades", "Reddit", "Other"];

fn site_from_label(v: &str) -> TradeSite {
    match v {
        "Lestrades" => TradeSite::Lestrades,
        "SteamTrades" => TradeSite::SteamTrades,
        "Reddit" => TradeSite::Reddit,
        _ => TradeSite::Other(String::new()),
    }
}

fn site_label(s: &TradeSite) -> &'static str {
    match s {
        TradeSite::Lestrades => "Lestrades",
        TradeSite::SteamTrades => "SteamTrades",
        TradeSite::Reddit => "Reddit",
        TradeSite::Other(_) => "Other",
    }
}

pub fn trade_status_pill(k: &Kit, s: TradeStatus) -> El {
    let (label, bg, fg) = match s {
        TradeStatus::Draft => ("In cart", k.c.info.bg, k.c.info.fg),
        TradeStatus::Pending => ("Pending", k.c.warning.bg, k.c.warning.fg),
        TradeStatus::Completed => ("Completed", k.c.success.bg, k.c.success.fg),
        TradeStatus::Cancelled => ("Cancelled", k.c.surface_raised, k.c.text_muted),
    };
    el().h(20.0)
        .px(8.0)
        .radius(10.0)
        .center()
        .fill(bg)
        .child(text(label).font(k.ty.text_xsmall_medium).color(fg))
}

fn input(
    k: &Kit,
    key: &'static str,
    edit: &TextEdit,
    placeholder: &str,
    access: fn(&mut TradeForm) -> &mut TextEdit,
) -> El {
    TextInput::new(edit)
        .placeholder(placeholder.to_string())
        .width(None)
        .on_edit(move |s: &mut AppState| trade_edit(s, access))
        .build(k)
        .key(key)
}

/// Partner, links, what I receive, notes — editable for any trade.
pub fn details_form(
    form: &TradeForm,
    show_add_received: bool,
    k: &Kit,
    overlays: &mut Vec<El>,
) -> El {
    let site = Select::new("trade-site", &SITES)
        .current(Some(site_label(&form.site)))
        .open(form.site_open)
        .appearance(SelectAppearance::Field)
        .fill()
        .on_toggle(|s: &mut AppState| s.toggle_trade_site_open())
        .on_close(|s: &mut AppState| s.close_trade_site())
        .on_choose(|s: &mut AppState, v: &str, _: &mut Tx| s.set_trade_site(site_from_label(v)))
        .build(k, overlays);
    let mut c = col()
        .key("trade-details")
        .w_full()
        .gap(14.0)
        .child(field(k, "Site", site, None))
        .child(field(
            k,
            "Trade link",
            input(
                k,
                "trade-url",
                &form.trade_url,
                "https://lestrades.com/offer/…",
                |f| &mut f.trade_url,
            ),
            None,
        ))
        .child(field(
            k,
            "Partner",
            input(k, "trade-partner", &form.partner, "Their username", |f| {
                &mut f.partner
            }),
            None,
        ))
        .child(field(
            k,
            "Partner profile",
            input(
                k,
                "trade-partner-url",
                &form.partner_url,
                "https://…",
                |f| &mut f.partner_url,
            ),
            None,
        ))
        .child(field(
            k,
            "What you receive",
            TextArea::new(&form.received)
                .placeholder(
                    "One game per line, optionally with its key:\nTunic: XXXXX-XXXXX-XXXXX",
                )
                .rows(4)
                .width(None)
                .on_edit(|s: &mut AppState| trade_edit(s, |f| &mut f.received))
                .build(k)
                .key("trade-received"),
            None,
        ))
        .child(field(
            k,
            "Notes",
            TextArea::new(&form.notes)
                .rows(2)
                .width(None)
                .on_edit(|s: &mut AppState| trade_edit(s, |f| &mut f.notes))
                .build(k)
                .key("trade-notes"),
            None,
        ));
    if show_add_received {
        c = c.child(
            el().key("trade-add-received").child(
                Checkbox::new(form.add_received)
                    .label("Add received games to my library on completion")
                    .on_toggle(|s: &mut AppState| s.toggle_add_received())
                    .build(k),
            ),
        );
    }
    c
}

/// Read-only monospace text block (checkout lines, reputation text).
pub fn text_block(k: &Kit, key: &str, content: String) -> El {
    col()
        .key(key.to_string())
        .w_full()
        .padding(14.0)
        .radius(k.radius.md)
        .fill(k.c.page)
        .border(1.0, k.c.border_weak)
        .child(
            text(if content.is_empty() {
                " ".to_string()
            } else {
                content
            })
            .font(k.ty.mono)
            .color(k.c.text_base)
            .wrap()
            .w_full(),
        )
}

/// The checkout text card for `trade` with Copy / Show keys.
pub fn checkout_card(sess: &Session, trade: &Trade, show_keys: bool, k: &Kit) -> El {
    let id = trade.id;
    let rendered = sess.repo.checkout_text(trade, !show_keys);
    let body = match rendered {
        Ok(t) => text_block(k, "checkout-text", t),
        Err(e) => text(format!(
            "Checkout template error: {e} — fix it in Settings → Templates"
        ))
        .font(k.ty.text_small)
        .color(k.c.danger.icon)
        .wrap(),
    };
    card(
        k,
        "checkout",
        "Checkout text",
        col()
            .w_full()
            .gap(12.0)
            .child(
                muted(
                    k,
                    "Paste this into the trade on the site. Format it in Settings → Templates.",
                )
                .wrap(),
            )
            .child(body)
            .child(
                row()
                    .w_full()
                    .gap(8.0)
                    .center_y()
                    .child(
                        Checkbox::new(show_keys)
                            .label("Show keys")
                            .on_toggle(|s: &mut AppState| s.toggle_show_keys())
                            .build(k),
                    )
                    .child(spacer())
                    .child(primary(
                        k,
                        "copy-checkout",
                        "Copy all",
                        true,
                        move |s, tx| s.copy_checkout(id, tx),
                    )),
            ),
    )
}

pub fn view(sess: &Session, k: &Kit, overlays: &mut Vec<El>) -> El {
    let header = |sub: String| {
        col()
            .gap(4.0)
            .child(text("Cart").font(title_style()).color(k.c.text_strong))
            .child(muted(k, sub).key("cart-count"))
    };
    let page = col()
        .key("cart")
        .w_full()
        .h_full()
        .px(32.0)
        .pt(28.0)
        .pb(20.0)
        .gap(20.0);
    let cart = sess.repo.cart();
    let keys = cart.map(|t| sess.repo.trade_keys(t)).unwrap_or_default();
    let (Some(cart), Some(form)) = (cart.filter(|_| !keys.is_empty()), sess.trade_form.as_ref())
    else {
        return page.child(header("Empty".into())).child(
            col()
                .key("cart-empty")
                .w_full()
                .grow(1.0)
                .center()
                .gap(14.0)
                .child(
                    text("Your cart is empty")
                        .font(rune::TextStyle::new(20.0, 650.0))
                        .color(k.c.text_strong),
                )
                .child(muted(
                    k,
                    "Select keys in the Library and choose \"Add to cart\".",
                ))
                .child(primary(
                    k,
                    "cart-to-library",
                    "Go to Library",
                    true,
                    |s, _| s.go(Page::Library),
                )),
        );
    };

    let mut items = col().key("cart-items").w_full().gap(4.0);
    for key in &keys {
        let id = key.id;
        items = items.child(
            row()
                .key(format!("cart-item-{id}"))
                .w_full()
                .h(38.0)
                .px(12.0)
                .gap(12.0)
                .center_y()
                .radius(k.radius.sm)
                .fill(k.c.page)
                .child(
                    text(key.game_name.clone())
                        .font(k.ty.ui_medium)
                        .color(k.c.text_strong)
                        .grow(1.0)
                        .truncate(),
                )
                .child(muted(k, key.platform.label().to_string()))
                .child(
                    text(key.key.masked())
                        .font(k.ty.mono)
                        .color(k.c.text_muted)
                        .w(190.0)
                        .truncate(),
                )
                .child(
                    el().key(format!("cart-remove-{id}")).child(
                        IconButton::new(k.glyph.close)
                            .label(format!("Remove {}", key.game_name))
                            .on_press(move |s: &mut AppState| s.remove_from_cart(id))
                            .build(k),
                    ),
                ),
        );
    }
    let items_card = card(
        k,
        "cart-keys",
        "Keys you give",
        col().w_full().gap(10.0).child(items).child(
            row().child(spacer()).child(
                el().key("cart-clear").child(
                    Button::new("Clear cart")
                        .variant(ButtonVariant::GhostMuted)
                        .on_press(|s: &mut AppState| s.clear_cart())
                        .build(k),
                ),
            ),
        ),
    );

    let cart_id = cart.id;
    let actions = row()
        .key("cart-actions")
        .w_full()
        .gap(8.0)
        .child(spacer())
        .child(
            el().key("cart-send").child(
                Button::new("Mark as sent")
                    .variant(ButtonVariant::Outline)
                    .on_press(|s: &mut AppState| s.send_cart())
                    .build(k),
            ),
        )
        .child(primary(
            k,
            "cart-complete",
            "Complete trade",
            true,
            move |s, _| s.complete_trade(cart_id),
        ));
    let details = card(
        k,
        "cart-trade",
        "Trade details",
        col()
            .w_full()
            .gap(16.0)
            .child(details_form(form, true, k, overlays))
            .child(actions),
    );

    let n = keys.len();
    page.child(header(format!(
        "{n} key{} ready to trade",
        if n == 1 { "" } else { "s" }
    )))
    .child(
        row()
            .w_full()
            .grow(1.0)
            .min_h(0.0)
            .gap(24.0)
            .child(
                col()
                    .grow(1.0)
                    .w(0.0)
                    .h_full()
                    .gap(22.0)
                    .scroll(ScrollSpec::default())
                    .child(items_card)
                    .child(checkout_card(sess, cart, form.show_keys, k)),
            )
            .child(
                col()
                    .w(400.0)
                    .h_full()
                    .scroll(ScrollSpec::default())
                    .child(details),
            ),
    )
}
