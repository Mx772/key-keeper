//! Trades: list on the left, the selected trade on the right with its actions and the
//! steamtrades reputation generator.

use keykeeper_core::{Trade, TradeStatus};
use rune::{El, ScrollSpec, col, el, row, text};
use rune_kit::{Button, ButtonVariant, Checkbox, Kit, TextInput};

use super::cart::{checkout_card, details_form, text_block, trade_status_pill};
use super::{card, field, heading_style, hstack, muted, primary, spacer, title_style, trade_edit};
use crate::app::{AppState, Page, Session};

fn when(sess: &Session, t: &Trade) -> String {
    sess.date_format().format(keykeeper_core::trade::trade_day(
        t,
        &jiff::tz::TimeZone::system(),
    ))
}

fn title(t: &Trade) -> String {
    if t.partner_name.is_empty() {
        "Unnamed trade".into()
    } else {
        t.partner_name.clone()
    }
}

pub fn view(sess: &Session, k: &Kit, overlays: &mut Vec<El>) -> El {
    // Newest first; the draft is the cart and lives on its own page.
    let mut trades: Vec<&Trade> = sess
        .repo
        .trades()
        .iter()
        .filter(|t| t.status != TradeStatus::Draft)
        .collect();
    trades.sort_by_key(|t| std::cmp::Reverse(t.completed_at.or(t.sent_at).unwrap_or(t.created_at)));
    let pending = trades
        .iter()
        .filter(|t| t.status == TradeStatus::Pending)
        .count();
    let header = col()
        .gap(4.0)
        .child(text("Trades").font(title_style()).color(k.c.text_strong))
        .child(muted(
            k,
            format!(
                "{} trade{} · {pending} pending",
                trades.len(),
                if trades.len() == 1 { "" } else { "s" }
            ),
        ));
    let page = col()
        .key("trades")
        .w_full()
        .h_full()
        .px(32.0)
        .pt(28.0)
        .pb(20.0)
        .gap(20.0)
        .child(header);

    if trades.is_empty() {
        return page.child(
            col()
                .key("trades-empty")
                .w_full()
                .grow(1.0)
                .center()
                .gap(12.0)
                .child(
                    text("No trades yet")
                        .font(rune::TextStyle::new(20.0, 650.0))
                        .color(k.c.text_strong),
                )
                .child(muted(
                    k,
                    "Add keys to the cart from your Library, then check out here.",
                ))
                .child(primary(k, "trades-to-cart", "Open cart", true, |s, _| {
                    s.go(Page::Cart)
                })),
        );
    }

    let selected = sess
        .selected_trade
        .and_then(|id| sess.repo.trade(id))
        .filter(|t| t.status != TradeStatus::Draft);
    let mut list = col().key("trades-list").w_full().gap(6.0);
    for t in &trades {
        let id = t.id;
        let on = selected.is_some_and(|s| s.id == id);
        let keys = sess.repo.trade_keys(t).len();
        list = list.child(
            row()
                .key(format!("trade-{id}"))
                .role("option")
                .label(title(t))
                .checked(on)
                .focusable(true)
                .w_full()
                .h(58.0)
                .px(14.0)
                .gap(12.0)
                .center_y()
                .radius(k.radius.lg)
                .fill(if on { k.c.surface_raised } else { k.c.bg_base })
                .border(1.0, if on { k.c.bg_accent } else { k.c.border_weak })
                .hover(k.c.surface_hover)
                .on_press(move |s: &mut AppState| s.select_trade(id))
                .child(
                    col()
                        .grow(1.0)
                        .w(0.0)
                        .gap(2.0)
                        .child(
                            text(title(t))
                                .font(k.ty.ui_medium)
                                .color(k.c.text_strong)
                                .truncate(),
                        )
                        .child(
                            muted(
                                k,
                                format!(
                                    "{} · {} · {keys} key{}",
                                    t.site.label(),
                                    when(sess, t),
                                    if keys == 1 { "" } else { "s" }
                                ),
                            )
                            .truncate(),
                        ),
                )
                .child(trade_status_pill(k, t.status)),
        );
    }

    let detail = match (selected, &sess.trade_form) {
        (Some(t), Some(form)) if form.trade_id == t.id => detail(sess, t, form, k, overlays),
        _ => col()
            .key("trade-none")
            .w_full()
            .grow(1.0)
            .center()
            .child(muted(k, "Select a trade to see its details")),
    };
    page.child(
        row()
            .w_full()
            .grow(1.0)
            .min_h(0.0)
            .gap(24.0)
            .child(
                col()
                    .w(380.0)
                    .h_full()
                    .scroll(ScrollSpec::default())
                    .child(list),
            )
            .child(
                col()
                    .grow(1.0)
                    .w(0.0)
                    .h_full()
                    .scroll(ScrollSpec::default())
                    .child(detail),
            ),
    )
}

fn detail(
    sess: &Session,
    t: &Trade,
    form: &crate::trade_actions::TradeForm,
    k: &Kit,
    overlays: &mut Vec<El>,
) -> El {
    let id = t.id;
    let mut c = col().key("trade-detail").w_full().gap(22.0).pb(20.0);
    c = c.child(
        row()
            .w_full()
            .gap(10.0)
            .center_y()
            .child(
                text(title(t))
                    .font(rune::TextStyle::new(20.0, 650.0))
                    .color(k.c.text_strong),
            )
            .child(trade_status_pill(k, t.status))
            .child(spacer())
            .child(muted(k, when(sess, t))),
    );

    // What happens next.
    match t.status {
        TradeStatus::Pending => {
            c = c.child(
                row()
                    .key("trade-actions")
                    .w_full()
                    .gap(8.0)
                    .center_y()
                    .child(muted(k, "Sent — complete it once you've received your side.").grow(1.0))
                    .child(
                        el().key("trade-cancel").child(
                            Button::new("Cancel trade")
                                .variant(ButtonVariant::Danger)
                                .on_press(move |s: &mut AppState| s.ask_cancel_trade(id))
                                .build(k),
                        ),
                    )
                    .child(primary(
                        k,
                        "trade-complete",
                        "Complete trade",
                        true,
                        move |s, _| s.complete_trade(id),
                    )),
            );
        }
        TradeStatus::Completed => {
            let rep = match sess.repo.rep_text(t) {
                Ok(r) => text_block(k, "rep-text", r),
                Err(e) => text(format!("Reputation template error: {e}"))
                    .font(k.ty.text_small)
                    .color(k.c.danger.icon),
            };
            c = c.child(card(
                k,
                "rep",
                "Reputation for steamtrades.com",
                col().w_full().gap(12.0).child(rep).child(
                    row()
                        .w_full()
                        .gap(8.0)
                        .center_y()
                        .child(
                            el().key("rep-posted").child(
                                Checkbox::new(t.rep_posted)
                                    .label("Posted")
                                    .on_toggle(move |s: &mut AppState| s.toggle_rep_posted(id))
                                    .build(k),
                            ),
                        )
                        .child(spacer())
                        .child(primary(
                            k,
                            "copy-rep",
                            "Copy reputation",
                            true,
                            move |s, tx| s.copy_rep(id, tx),
                        )),
                ),
            ));
        }
        TradeStatus::Cancelled => {
            c = c.child(muted(
                k,
                "This trade was cancelled and its keys were released.",
            ))
        }
        TradeStatus::Draft => {}
    }

    c = c.child(checkout_card(sess, t, form.show_keys, k));
    if t.status == TradeStatus::Completed && !t.received.is_empty() {
        let names: Vec<String> = t.received.iter().map(|r| r.game_name.clone()).collect();
        c = c.child(
            col()
                .gap(6.0)
                .child(
                    text("Received")
                        .font(heading_style())
                        .color(k.c.text_strong),
                )
                .child(
                    muted(
                        k,
                        format!(
                            "{}{}",
                            names.join(", "),
                            if t.add_received_to_library {
                                " — added to your library"
                            } else {
                                ""
                            }
                        ),
                    )
                    .wrap(),
                ),
        );
    }
    let fmt = sess.date_format();
    let label = match t.status {
        TradeStatus::Completed => "Completed on",
        TradeStatus::Pending => "Sent on",
        _ => "Date",
    };
    let date = field(
        k,
        label,
        hstack()
            .w_full()
            .child(
                TextInput::new(&form.date)
                    .placeholder(crate::form::date_hint(fmt).to_string())
                    .width(Some(180.0))
                    .on_edit(|s: &mut AppState| trade_edit(s, |f| &mut f.date))
                    .build(k)
                    .key("trade-date"),
            )
            .child(super::datepick::button(
                k,
                form.date_pick,
                &form.date,
                super::datepick::TRADE_DATE,
                fmt,
                overlays,
            ))
            .child(muted(k, "Keys traded in it get this date too.")),
        None,
    );
    c.child(card(
        k,
        "trade-edit",
        "Details",
        col().w_full().gap(14.0).child(date).child(details_form(
            form,
            t.status == TradeStatus::Pending,
            k,
            overlays,
        )),
    ))
}
