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

pub fn view(sess: &Session, form: &KeyForm, k: &Kit, overlays: &mut Vec<El>) -> El {
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
            input(k, "form-app-id", &form.app_id, "972660", |f| &mut f.app_id),
            err("app_id"),
        ))
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
                .rows(4)
                .width(None)
                .on_edit(|s: &mut AppState| form_edit(s, |f| &mut f.notes))
                .build(k)
                .key("form-notes"),
            None,
        ));

    let mut body = col().w_full().gap(18.0).child(fields);
    if let Some(key) = existing {
        let mut links = row().gap(6.0).center_y();
        let mut any = false;
        for (label, url) in [
            ("Steam", key.steam_url()),
            ("gg.deals", key.gg_deals_url()),
            ("IsThereAnyDeal", key.itad_url()),
            ("Key page", key.key_url.clone()),
        ] {
            if let Some(url) = url {
                any = true;
                links = links.child(
                    Button::new(label)
                        .variant(ButtonVariant::Outline)
                        .on_press_tx(move |_: &mut AppState, tx: &mut Tx| {
                            let _ = tx.open_url(&url);
                        })
                        .build(k),
                );
            }
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

    let footer = match existing {
        Some(_) => row()
            .key("form-actions")
            .w_full()
            .gap(8.0)
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
