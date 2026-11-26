//! The right-hand pane for editing (or adding) one key.

use keykeeper_core::{KeyStatus, Platform};
use rune::{El, TextEdit, Tx, col, el, row, text};
use rune_kit::{
    Button, ButtonVariant, IconButton, Kit, Select, SelectAppearance, TextArea, TextInput,
};

use super::{field, form_edit, heading_style, muted, primary, spacer};
use crate::app::{AppState, Session};
use crate::form::{KeyForm, date_hint};

const PLATFORMS: [Platform; 6] = [
    Platform::Steam,
    Platform::Gog,
    Platform::Epic,
    Platform::Ea,
    Platform::Ubisoft,
    Platform::Battlenet,
];

fn input(
    k: &Kit,
    key: &'static str,
    edit: &TextEdit,
    placeholder: &str,
    access: fn(&mut KeyForm) -> &mut TextEdit,
) -> El {
    TextInput::new(edit)
        .placeholder(placeholder.to_string())
        .width(None)
        .on_edit(move |s: &mut AppState| form_edit(s, access))
        .on_submit(|s: &mut AppState, _: &mut Tx| s.save_form())
        .build(k)
        .key(key)
}

fn set_form(s: &mut AppState, f: impl FnOnce(&mut KeyForm)) {
    if let Some(form) = s.session_mut().and_then(|x| x.form.as_mut()) {
        f(form);
    }
}

fn toggle_select(name: &'static str) -> impl Fn(&mut AppState) + Clone {
    move |s: &mut AppState| {
        set_form(s, |f| {
            f.open_select = if f.open_select == Some(name) {
                None
            } else {
                Some(name)
            }
        })
    }
}

pub fn view(
    s: &AppState,
    sess: &Session,
    form: &KeyForm,
    busy: bool,
    k: &Kit,
    overlays: &mut Vec<El>,
) -> El {
    let fmt = sess.date_format();
    let existing = form.target.and_then(|id| sess.repo.key(id));
    let err = |f: &str| form.error_for(f, fmt);
    let can_save =
        form.errors(fmt).is_empty() && existing.is_none_or(|k| form.differs_from(k, fmt));

    let header = row()
        .w_full()
        .center_y()
        .child(
            text(if existing.is_some() {
                "Edit key"
            } else {
                "New key"
            })
            .font(heading_style())
            .color(k.c.text_strong),
        )
        .child(spacer())
        .child(
            el().key("form-close").child(
                IconButton::new(k.glyph.close)
                    .label("Close")
                    .on_press(|s: &mut AppState| s.close_form())
                    .build(k),
            ),
        );

    // Key field: hidden by default, with reveal and copy.
    let reveal = IconButton::new(if form.reveal_key {
        k.glyph.eye_off
    } else {
        k.glyph.eye
    })
    .label(if form.reveal_key {
        "Hide key"
    } else {
        "Show key"
    })
    .on_press(|s: &mut AppState| set_form(s, |f| f.reveal_key = !f.reveal_key))
    .build(k);
    let copy = IconButton::new(k.glyph.copy)
        .label("Copy key")
        .on_press_tx(|s: &mut AppState, tx: &mut Tx| {
            match s
                .session()
                .and_then(|x| x.form.as_ref())
                .and_then(|f| f.target)
            {
                Some(id) => s.copy_key(id, tx),
                None => {
                    let text = s
                        .session()
                        .and_then(|x| x.form.as_ref())
                        .map(|f| f.key.text.clone())
                        .unwrap_or_default();
                    let _ = tx.copy_secret(text, std::time::Duration::from_secs(30));
                }
            }
        })
        .build(k);
    let key_input = TextInput::new(&form.key)
        .placeholder("XXXXX-XXXXX-XXXXX")
        .secure()
        .revealed(form.reveal_key)
        .width(None)
        .on_edit(|s: &mut AppState| form_edit(s, |f| &mut f.key))
        .on_submit(|s: &mut AppState, _: &mut Tx| s.save_form())
        .build(k)
        .key("form-key")
        .grow(1.0);
    let key_row = row()
        .w_full()
        .gap(4.0)
        .center_y()
        .child(key_input)
        .child(reveal)
        .child(copy);

    let status_labels: Vec<&str> = KeyStatus::ALL.iter().map(|s| s.label()).collect();
    let status = Select::new("form-status", &status_labels)
        .current(Some(form.status.label()))
        .open(form.open_select == Some("status"))
        .on_toggle(toggle_select("status"))
        .on_close(|s: &mut AppState| set_form(s, |f| f.open_select = None))
        .on_choose(|s: &mut AppState, v: &str, _: &mut Tx| {
            let st = KeyStatus::ALL.into_iter().find(|st| st.label() == v);
            set_form(s, |f| {
                if let Some(st) = st {
                    f.status = st;
                }
                f.open_select = None;
            });
        })
        .appearance(SelectAppearance::Field)
        .fill()
        .build(k, overlays);

    let mut platform_labels: Vec<String> =
        PLATFORMS.iter().map(|p| p.label().to_string()).collect();
    if let Platform::Other(o) = &form.platform {
        platform_labels.push(o.clone());
    }
    let platform = Select::new("form-platform", &platform_labels)
        .current(Some(form.platform.label()))
        .open(form.open_select == Some("platform"))
        .on_toggle(toggle_select("platform"))
        .on_close(|s: &mut AppState| set_form(s, |f| f.open_select = None))
        .on_choose(|s: &mut AppState, v: &str, _: &mut Tx| {
            let p = Platform::parse(v);
            set_form(s, |f| {
                f.platform = p;
                f.open_select = None;
            });
        })
        .appearance(SelectAppearance::Field)
        .fill()
        .build(k, overlays);

    let date_ph = date_hint(fmt);
    let fields = col()
        .key("form-fields")
        .w_full()
        .gap(14.0)
        .child(field(
            k,
            "Game",
            input(k, "form-name", &form.name, "Game name", |f| &mut f.name),
            err("name"),
        ))
        .child(field(k, "Key", key_row, err("key")))
        .child(
            row()
                .w_full()
                .gap(10.0)
                .child(field(k, "Status", status, None).grow(1.0))
                .child(field(k, "Platform", platform, None).grow(1.0)),
        )
        .child(field(
            k,
            "Bundle",
            input(k, "form-bundle", &form.bundle, "e.g. Humble Choice", |f| {
                &mut f.bundle
            }),
            None,
        ))
        .child(field(
            k,
            "Source",
            input(
                k,
                "form-source",
                &form.source,
                "Humble, Fanatical, Trade: …",
                |f| &mut f.source,
            ),
            None,
        ))
        .child(
            row()
                .w_full()
                .gap(10.0)
                .child(
                    field(
                        k,
                        "Redeem / trade date",
                        input(k, "form-status-date", &form.status_date, date_ph, |f| {
                            &mut f.status_date
                        }),
                        err("status_date"),
                    )
                    .grow(1.0),
                )
                .child(
                    field(
                        k,
                        "Redemption deadline",
                        input(k, "form-deadline", &form.deadline, date_ph, |f| {
                            &mut f.deadline
                        }),
                        err("deadline"),
                    )
                    .grow(1.0),
                ),
        )
        .child(field(
            k,
            "Steam app id or store link",
            steam_row(sess, form, busy, k),
            err("app_id"),
        ))
        .children(candidates(sess, form, k))
        .child(field(
            k,
            "Key page link",
            input(k, "form-key-url", &form.key_url, "https://…", |f| {
                &mut f.key_url
            }),
            err("key_url"),
        ))
        .child(field(
            k,
            "Tags",
            input(k, "form-tags", &form.tags, "comma separated", |f| {
                &mut f.tags
            }),
            None,
        ))
        .child(field(
            k,
            "Notes",
            TextArea::new(&form.notes)
                .auto_grow(4, 10)
                .scroll_bar(true)
                .width(None)
                .on_edit(|s: &mut AppState| form_edit(s, |f| &mut f.notes))
                .build(k)
                .key("form-notes"),
            None,
        ));

    let mut body = col().w_full().gap(18.0);
    if let Some(key) = existing.filter(|k| k.steam_app_id.is_some()) {
        body = body.child(
            super::card::art_fill(
                s,
                key,
                (356.0 * super::card::HEADER_RATIO).round(),
                k.radius.lg,
            )
            .key("form-art"),
        );
    }
    if let Some(r) = existing.and_then(|key| redeem_row(sess, key, k)) {
        body = body.child(r);
    }
    body = body.child(fields);
    if let Some(key) = existing {
        let mut links = row().w_full().gap(8.0).flex_wrap().center_y();
        let mut any = false;
        for (slug, label, url) in [
            ("steam", "Steam", key.steam_url()),
            ("gg-deals", "gg.deals", key.gg_deals_url()),
            ("itad", "IsThereAnyDeal", key.itad_url()),
            ("key-page", "Key page", key.key_url.clone()),
        ] {
            if let Some(url) = url {
                any = true;
                links = links.child(super::link_pill(k, format!("link-{slug}"), label, url));
            }
        }
        if let Some(p) = sess.repo.key_prices(key) {
            body = body.child(prices_box(p, k));
        }
        if any {
            body = body.child(field(k, "Links", links, None));
        } else {
            body = body.child(
                muted(
                    k,
                    "Add a Steam app id to get Steam, gg.deals and IsThereAnyDeal links.",
                )
                .wrap(),
            );
        }
        if let Some(t) = key.trade_id.and_then(|t| sess.repo.trade(t)) {
            let what = match (&t.trade_url, t.partner_name.is_empty()) {
                (Some(u), _) => u.clone(),
                (None, false) => t.partner_name.clone(),
                (None, true) => "a trade".into(),
            };
            body = body.child(field(
                k,
                "Trade",
                muted(k, format!("{} · {}", t.site.label(), what)).wrap(),
                None,
            ));
        }
    }

    let in_cart = existing.is_some_and(|k| {
        k.trade_id.is_some() && sess.repo.cart().is_some_and(|c| Some(c.id) == k.trade_id)
    });
    let cart_button = match existing {
        Some(key) if key.status.can_trade() => {
            let id = key.id;
            Some(
                el().key("form-cart").child(
                    Button::new("Add to cart")
                        .variant(ButtonVariant::Outline)
                        .on_press(move |s: &mut AppState| s.add_to_cart(&[id]))
                        .build(k),
                ),
            )
        }
        Some(key) if in_cart => {
            let id = key.id;
            Some(
                el().key("form-uncart").child(
                    Button::new("Remove from cart")
                        .variant(ButtonVariant::GhostMuted)
                        .on_press(move |s: &mut AppState| s.remove_from_cart(id))
                        .build(k),
                ),
            )
        }
        _ => None,
    };
    let footer = match existing {
        Some(_) => row()
            .key("form-actions")
            .w_full()
            .gap(8.0)
            .children(cart_button)
            .child(
                el().key("form-delete").child(
                    Button::new("Delete")
                        .variant(ButtonVariant::Danger)
                        .on_press(|s: &mut AppState| {
                            let id = s
                                .session()
                                .and_then(|x| x.form.as_ref())
                                .and_then(|f| f.target);
                            s.ask_delete(id.into_iter().collect());
                        })
                        .build(k),
                ),
            )
            .child(spacer())
            .child(
                Button::new("Revert")
                    .variant(ButtonVariant::Neutral)
                    .disabled(!can_save)
                    .on_press(|s: &mut AppState| {
                        if let Some(id) = s
                            .session()
                            .and_then(|x| x.form.as_ref())
                            .and_then(|f| f.target)
                        {
                            s.open_key(id);
                        }
                    })
                    .build(k),
            )
            .child(primary(k, "form-save", "Save", can_save, |s, _| {
                s.save_form()
            })),
        None => row()
            .key("form-actions")
            .w_full()
            .gap(8.0)
            .child(spacer())
            .child(
                Button::new("Cancel")
                    .variant(ButtonVariant::Neutral)
                    .on_press(|s: &mut AppState| s.close_form())
                    .build(k),
            )
            .child(primary(k, "form-save", "Add key", can_save, |s, _| {
                s.save_form()
            })),
    };

    col()
        .key("detail")
        .w(400.0)
        .h_full()
        .fill(k.c.bg_base)
        .border(1.0, k.c.border_weak)
        .child(col().w_full().px(22.0).pt(22.0).pb(12.0).child(header))
        .child(
            col()
                .key("detail-scroll")
                .w_full()
                .grow(1.0)
                .min_h(0.0)
                .px(22.0)
                .pb(16.0)
                .scroll(rune::ScrollSpec::default())
                .child(body),
        )
        .child(
            col()
                .w_full()
                .px(22.0)
                .py(14.0)
                .border(1.0, k.c.border_weak)
                .child(footer),
        )
}

/// "Redeem on Steam" (opens the activation page with the key filled in) or, for other platforms,
/// "Mark redeemed by me". Both date the key today. Only for keys still in hand or in the cart.
fn redeem_row(sess: &Session, key: &keykeeper_core::GameKey, k: &Kit) -> Option<El> {
    let in_cart = key.status == KeyStatus::Reserved
        && key.trade_id.is_some()
        && sess.repo.cart().map(|c| c.id) == key.trade_id;
    if !(key.status.can_trade() || in_cart) {
        return None;
    }
    let id = key.id;
    let steam = key.steam_register_url().is_some();
    let (label, hint) = if steam {
        (
            "Redeem on Steam",
            "Opens Steam's activation page with this key filled in and marks it redeemed today.",
        )
    } else {
        (
            "Mark redeemed by me",
            "Marks this key redeemed on your account, dated today.",
        )
    };
    let button = el()
        .key("form-redeem")
        .role("button")
        .label(label)
        .focusable(true)
        .h(36.0)
        .px(16.0)
        .gap(8.0)
        .radius(k.radius.md)
        .center_y()
        .fill(k.c.success.bg)
        .border(1.0, k.c.success.icon.alpha(0.5))
        .hover(k.c.success.bg.mix(k.c.success.icon, 0.18))
        .cursor(rune::Cursor::Pointer)
        .on_press_tx(move |s: &mut AppState, tx: &mut Tx| s.redeem_self(&[id], true, tx))
        .child(
            rune::icon(&super::KEY, 15.0)
                .color(k.c.success.fg)
                .a11y_hidden(),
        )
        .child(
            text(label)
                .font(k.ty.ui_medium)
                .color(k.c.success.fg)
                .a11y_hidden(),
        );
    Some(
        col()
            .key("form-redeem-row")
            .w_full()
            .gap(6.0)
            .child(row().child(button))
            .child(muted(k, hint).wrap()),
    )
}

/// The current store prices for this key's game, cheapest first; each row opens that deal.
fn prices_box(p: &keykeeper_core::GamePrices, k: &Kit) -> El {
    let mut list = col().key("form-prices").w_full().gap(2.0);
    for (i, shop) in p.shops.iter().take(8).enumerate() {
        let mut r = row()
            .key(format!("price-{i}"))
            .w_full()
            .h(28.0)
            .px(10.0)
            .gap(8.0)
            .radius(k.radius.sm)
            .center_y()
            .fill(if i == 0 { k.c.success.bg } else { k.c.page })
            .child(
                text(shop.shop.clone())
                    .font(k.ty.ui)
                    .color(k.c.text_base)
                    .grow(1.0)
                    .truncate(),
            );
        if shop.cut > 0 {
            r = r.child(muted(k, format!("−{}%", shop.cut)));
        }
        r = r.child(
            text(shop.price.format())
                .font(k.ty.mono)
                .color(k.c.text_strong),
        );
        if let Some(url) = shop.url.clone() {
            r = r
                .role("link")
                .hover(k.c.surface_hover)
                .cursor(rune::Cursor::Pointer)
                .help(format!("Open {} deal", shop.shop))
                .on_press_tx(move |_: &mut AppState, tx: &mut Tx| {
                    let _ = tx.open_url(&url);
                });
        }
        list = list.child(r);
    }
    if p.shops.len() > 8 {
        list = list.child(muted(k, format!("…and {} more stores", p.shops.len() - 8)));
    }
    let mut extra = Vec::new();
    if p.shops.is_empty()
        && let Some(r) = &p.retail_low
    {
        extra.push(format!("Lowest store price {}", r.format()));
    }
    if let Some(l) = &p.history_low {
        extra.push(format!("All-time low {}", l.format()));
    }
    if let Some(ks) = &p.keyshop_low {
        extra.push(format!("Keyshops {}", ks.format()));
    }
    if p.shops.is_empty() && extra.is_empty() {
        extra.push("No current prices found".into());
    }
    if !extra.is_empty() {
        list = list.child(muted(k, extra.join(" · ")).wrap());
    }
    field(k, &format!("Prices ({})", p.country), list, None)
}

fn steam_row(sess: &Session, form: &KeyForm, busy: bool, k: &Kit) -> El {
    let mut r = row()
        .w_full()
        .gap(6.0)
        .center_y()
        .child(input(k, "form-app-id", &form.app_id, "972660", |f| &mut f.app_id).grow(1.0));
    if let Some(id) = form.target {
        r = r.child(
            el().key("form-find-steam").child(
                Button::new("Find on Steam")
                    .variant(ButtonVariant::Outline)
                    .disabled(busy)
                    .on_press(move |s: &mut AppState| s.find_on_steam(id))
                    .build(k),
            ),
        );
    }
    let meta = form
        .target
        .and_then(|id| sess.repo.key(id))
        .and_then(|key| key.steam.as_ref());
    let mut c = col().w_full().gap(6.0).child(r);
    if let Some(m) = meta {
        let mut parts = Vec::new();
        if let Some(r) = &m.rating {
            parts.push(format!(
                "{} · {}% of {} reviews",
                r.desc, r.percent, r.total
            ));
        }
        match m.ownership {
            keykeeper_core::Ownership::Owned => parts.push("You own this game".into()),
            keykeeper_core::Ownership::Wishlisted => parts.push("On your wishlist".into()),
            _ => {}
        }
        if !parts.is_empty() {
            c = c.child(muted(k, parts.join(" · ")).key("form-steam-meta"));
        }
    }
    c
}

fn candidates(sess: &Session, form: &KeyForm, k: &Kit) -> Option<El> {
    let cands = sess
        .candidates
        .as_ref()
        .filter(|c| Some(c.key_id) == form.target)?;
    let key_id = cands.key_id;
    let mut list = col().key("steam-candidates").w_full().gap(4.0).child(
        row()
            .w_full()
            .center_y()
            .child(muted(k, "Which one is it?").grow(1.0))
            .child(
                Button::new("Dismiss")
                    .variant(ButtonVariant::GhostMuted)
                    .on_press(|s: &mut AppState| s.dismiss_candidates())
                    .build(k),
            ),
    );
    for m in &cands.matches {
        let app = m.app_id;
        list = list.child(
            row()
                .key(format!("candidate-{app}"))
                .role("button")
                .label(m.name.clone())
                .focusable(true)
                .w_full()
                .h(32.0)
                .px(10.0)
                .gap(8.0)
                .center_y()
                .radius(k.radius.sm)
                .fill(k.c.page)
                .hover(k.c.surface_hover)
                .on_press(move |s: &mut AppState| s.choose_candidate(key_id, app))
                .child(
                    text(m.name.clone())
                        .font(k.ty.ui)
                        .color(k.c.text_base)
                        .grow(1.0)
                        .truncate(),
                )
                .child(muted(k, app.to_string())),
        );
    }
    Some(list)
}
