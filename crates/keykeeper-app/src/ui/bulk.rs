//! The bulk edit pane: shown in place of the key editor when editing several keys.

use keykeeper_core::{KeyStatus, Platform};
use rune::{El, TextEdit, Tx, col, el, row, text};
use rune_kit::{
    Button, ButtonVariant, Checkbox, IconButton, Kit, Select, SelectAppearance, TextInput,
};

use super::{field, heading_style, muted, primary, spacer};
use crate::app::{AppState, Screen, Session};
use crate::bulk::{BulkForm, CONFIRM_OVER, common};
use crate::form::date_hint;

const LEAVE: &str = "Leave as is";

const PLATFORMS: [Platform; 6] = [
    Platform::Steam,
    Platform::Gog,
    Platform::Epic,
    Platform::Ea,
    Platform::Ubisoft,
    Platform::Battlenet,
];

fn bulk_edit<'a>(
    s: &'a mut AppState,
    f: impl FnOnce(&'a mut BulkForm) -> &'a mut TextEdit,
) -> &'a mut TextEdit {
    let AppState {
        screen, scratch, ..
    } = s;
    match screen {
        Screen::Unlocked(x) => match x.bulk.as_mut() {
            Some(b) => f(b),
            None => scratch,
        },
        Screen::Locked(_) => scratch,
    }
}

fn input(
    k: &Kit,
    key: &'static str,
    edit: &TextEdit,
    placeholder: String,
    access: fn(&mut BulkForm) -> &mut TextEdit,
) -> El {
    TextInput::new(edit)
        .placeholder(placeholder)
        .width(None)
        .on_edit(move |s: &mut AppState| bulk_edit(s, access))
        .on_submit(|s: &mut AppState, _: &mut Tx| s.apply_bulk_edit())
        .build(k)
        .key(key)
}

fn toggle(name: &'static str) -> impl Fn(&mut AppState) + Clone {
    move |s: &mut AppState| {
        s.with_bulk(|b| {
            b.open_select = if b.open_select == Some(name) {
                None
            } else {
                Some(name)
            }
        })
    }
}

/// "Currently: Humble Choice" or "Currently mixed".
fn now_hint<T: ToString>(v: Option<T>) -> String {
    match v {
        Some(v) => {
            let v = v.to_string();
            if v.is_empty() {
                "Currently empty — type to set".into()
            } else {
                format!("Currently \"{v}\" — type to replace")
            }
        }
        None => "Mixed — type to set them all".into(),
    }
}

pub fn view(sess: &Session, b: &BulkForm, k: &Kit, overlays: &mut Vec<El>) -> El {
    let fmt = sess.date_format();
    let keys = sess.bulk_keys();
    let n = keys.len();

    let header = row()
        .w_full()
        .center_y()
        .child(
            col()
                .gap(2.0)
                .child(
                    text(format!("Edit {n} keys"))
                        .font(heading_style())
                        .color(k.c.text_strong),
                )
                .child(muted(k, "Fields you leave alone don't change.")),
        )
        .child(spacer())
        .child(
            el().key("bulk-close").child(
                IconButton::new(k.glyph.close)
                    .label("Close")
                    .on_press(|s: &mut AppState| s.close_bulk_edit())
                    .build(k),
            ),
        );

    let mut status_labels = vec![LEAVE];
    status_labels.extend(KeyStatus::ALL.iter().map(|s| s.label()));
    let status = Select::new("bulk-edit-status", &status_labels)
        .current(Some(b.status.map_or(LEAVE, |s| s.label())))
        .open(b.open_select == Some("status"))
        .on_toggle(toggle("status"))
        .on_close(|s: &mut AppState| s.with_bulk(|b| b.open_select = None))
        .on_choose(|s: &mut AppState, v: &str, _: &mut Tx| {
            let st = KeyStatus::ALL.into_iter().find(|st| st.label() == v);
            s.with_bulk(|b| {
                b.status = st;
                b.open_select = None;
            });
        })
        .appearance(SelectAppearance::Field)
        .fill()
        .build(k, overlays);

    let mut platform_labels = vec![LEAVE.to_string()];
    platform_labels.extend(PLATFORMS.iter().map(|p| p.label().to_string()));
    let platform = Select::new("bulk-edit-platform", &platform_labels)
        .current(Some(b.platform.as_ref().map_or(LEAVE, |p| p.label())))
        .open(b.open_select == Some("platform"))
        .on_toggle(toggle("platform"))
        .on_close(|s: &mut AppState| s.with_bulk(|b| b.open_select = None))
        .on_choose(|s: &mut AppState, v: &str, _: &mut Tx| {
            let p = (v != LEAVE).then(|| Platform::parse(v));
            s.with_bulk(|b| {
                b.platform = p;
                b.open_select = None;
            });
        })
        .appearance(SelectAppearance::Field)
        .fill()
        .build(k, overlays);

    let status_now = match common(&keys, |k| k.status) {
        Some(s) => format!("Currently {}", s.label()),
        None => "Mixed".into(),
    };
    let platform_now = match common(&keys, |k| k.platform.label().to_string()) {
        Some(p) => format!("Currently {p}"),
        None => "Mixed".into(),
    };
    let deadline_now = match common(&keys, |k| k.redemption_deadline) {
        Some(Some(d)) => format!(
            "Currently {} — type to replace",
            crate::form::format_date(Some(d), fmt)
        ),
        Some(None) => format!("None yet — {}", date_hint(fmt)),
        None => format!("Mixed — {}", date_hint(fmt)),
    };

    let deadline = col()
        .w_full()
        .gap(6.0)
        .child(if b.clear_deadline {
            muted(k, "Deadlines will be removed").key("bulk-edit-deadline")
        } else {
            input(k, "bulk-edit-deadline", &b.deadline, deadline_now, |b| {
                &mut b.deadline
            })
        })
        .child(
            Checkbox::new(b.clear_deadline)
                .label("Remove the deadline from all of them")
                .on_toggle(|s: &mut AppState| s.with_bulk(|b| b.clear_deadline = !b.clear_deadline))
                .build(k)
                .key("bulk-edit-clear-deadline"),
        );

    let fields = col()
        .key("bulk-edit-fields")
        .w_full()
        .gap(14.0)
        .child(
            row()
                .w_full()
                .gap(10.0)
                .child(field(k, "Status", status, None).grow(1.0))
                .child(field(k, "Platform", platform, None).grow(1.0)),
        )
        .child(muted(
            k,
            format!("Status: {status_now} · Platform: {platform_now}"),
        ))
        .child(field(
            k,
            "Bundle",
            input(
                k,
                "bulk-edit-bundle",
                &b.bundle,
                now_hint(common(&keys, |k| k.bundle.clone().unwrap_or_default())),
                |b| &mut b.bundle,
            ),
            None,
        ))
        .child(field(
            k,
            "Source",
            input(
                k,
                "bulk-edit-source",
                &b.source,
                now_hint(common(&keys, |k| k.source.clone().unwrap_or_default())),
                |b| &mut b.source,
            ),
            None,
        ))
        .child(field(
            k,
            "Redemption deadline",
            deadline,
            b.deadline_error(fmt),
        ))
        .child(field(
            k,
            "Add tags",
            input(
                k,
                "bulk-edit-add-tags",
                &b.add_tags,
                "comma separated".into(),
                |b| &mut b.add_tags,
            ),
            None,
        ))
        .child(field(
            k,
            "Remove tags",
            input(
                k,
                "bulk-edit-remove-tags",
                &b.remove_tags,
                "comma separated".into(),
                |b| &mut b.remove_tags,
            ),
            None,
        ));

    let mut names: Vec<&str> = keys.iter().take(6).map(|k| k.game_name.as_str()).collect();
    if n > 6 {
        names.push("…");
    }
    let mut body = col().w_full().gap(18.0).child(fields).child(field(
        k,
        "Keys",
        muted(k, names.join(", ")).wrap(),
        None,
    ));
    if n > CONFIRM_OVER {
        body = body.child(
            muted(
                k,
                format!("You'll be asked to confirm: this changes {n} keys."),
            )
            .wrap(),
        );
    }

    let can_apply = !b.is_empty() && b.deadline_error(fmt).is_none();
    let footer = row()
        .key("bulk-edit-actions")
        .w_full()
        .gap(8.0)
        .child(spacer())
        .child(
            Button::new("Cancel")
                .variant(ButtonVariant::Neutral)
                .on_press(|s: &mut AppState| s.close_bulk_edit())
                .build(k),
        )
        .child(primary(
            k,
            "bulk-edit-apply",
            format!("Apply to {n} keys"),
            can_apply,
            |s, _| s.apply_bulk_edit(),
        ));

    col()
        .key("bulk-edit")
        .w(400.0)
        .h_full()
        .fill(k.c.bg_base)
        .border(1.0, k.c.border_weak)
        .child(col().w_full().px(22.0).pt(22.0).pb(12.0).child(header))
        .child(
            col()
                .key("bulk-edit-scroll")
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
