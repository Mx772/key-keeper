//! Settings: appearance, dates, security, password, export.

use keykeeper_core::DateFormat;
use rune::{El, ScrollSpec, Tx, col, el, row, text};
use rune_kit::{Button, ButtonVariant, Checkbox, Kit, TextInput};

use super::{card, chip, field, hstack, muted, primary, sess_edit, title_style};
use crate::app::{AppState, ExportKind, Session};
use crate::theme::ThemeId;

fn option_row(k: &Kit, label: &str, control: El) -> El {
    row()
        .w_full()
        .gap(16.0)
        .center_y()
        .child(
            text(label.to_string())
                .font(k.ty.ui)
                .color(k.c.text_base)
                .w(220.0),
        )
        .child(control)
}

fn number_chips(
    k: &Kit,
    key: &str,
    values: &[u32],
    current: u32,
    fmt: fn(u32) -> String,
    set: fn(&mut keykeeper_core::Settings, u32),
) -> El {
    let mut r = hstack().gap(6.0);
    for &v in values {
        r = r.child(chip(
            k,
            format!("{key}-{v}"),
            fmt(v),
            current == v,
            move |s| s.update_settings(|st| set(st, v)),
        ));
    }
    r
}

pub fn view(s: &AppState, sess: &Session, k: &Kit, _overlays: &mut Vec<El>) -> El {
    let st = sess.settings();

    let mut themes = hstack().key("themes").gap(6.0).flex_wrap().grow(1.0).w(0.0);
    for t in ThemeId::ALL {
        let on = s.theme == t;
        themes = themes.child(
            el().key(format!("theme-{}", t.slug()))
                .role("radio")
                .label(t.label())
                .checked(on)
                .focusable(true)
                .h(30.0)
                .px(14.0)
                .radius(15.0)
                .center()
                .fill(if on {
                    k.c.bg_accent
                } else {
                    k.c.surface_raised
                })
                .border(1.0, if on { k.c.bg_accent } else { k.c.border_weak })
                .on_press_tx(move |s: &mut AppState, tx: &mut Tx| s.set_theme(t, tx))
                .child(
                    text(t.label())
                        .font(k.ty.text_small_medium)
                        .color(if on { k.c.text_contrast } else { k.c.text_base })
                        .a11y_hidden(),
                ),
        );
    }
    let appearance = card(
        k,
        "settings-appearance",
        "Appearance",
        option_row(k, "Theme", themes),
    );

    let date_chips = hstack()
        .gap(6.0)
        .child(chip(
            k,
            "date-mdy",
            "M/D/YYYY (1/7 = Jan 7)",
            st.date_format == DateFormat::MonthDayYear,
            |s| s.update_settings(|x| x.date_format = DateFormat::MonthDayYear),
        ))
        .child(chip(
            k,
            "date-dmy",
            "D/M/YYYY (1/7 = Jul 1)",
            st.date_format == DateFormat::DayMonthYear,
            |s| s.update_settings(|x| x.date_format = DateFormat::DayMonthYear),
        ));
    let data = card(
        k,
        "settings-data",
        "Dates & trades",
        col()
            .gap(14.0)
            .child(option_row(k, "Date format", date_chips))
            .child(option_row(
                k,
                "Received games",
                Checkbox::new(st.add_received_to_library)
                    .label("Add games received in trades to my library by default")
                    .on_toggle(|s: &mut AppState| {
                        s.update_settings(|x| {
                            x.add_received_to_library = !x.add_received_to_library
                        })
                    })
                    .build(k),
            )),
    );

    let minutes = |v: u32| {
        if v == 0 {
            "Never".to_string()
        } else {
            format!("{v} min")
        }
    };
    let seconds = |v: u32| {
        if v == 0 {
            "Never".to_string()
        } else {
            format!("{v} s")
        }
    };
    let count = |v: u32| {
        if v == 0 {
            "None".to_string()
        } else {
            v.to_string()
        }
    };
    let ui = &sess.settings_ui;
    let pw = |label: &str,
              key: &'static str,
              edit: &rune::TextEdit,
              access: fn(&mut Session) -> &mut rune::TextEdit| {
        field(
            k,
            label,
            TextInput::new(edit)
                .label(label.to_string())
                .secure()
                .width(Some(300.0))
                .on_edit(move |s: &mut AppState| sess_edit(s, access))
                .build(k)
                .key(key),
            None,
        )
    };
    let mut pw_form = col()
        .key("password-form")
        .gap(10.0)
        .child(pw("Current password", "pw-old", &ui.old_pw, |x| {
            &mut x.settings_ui.old_pw
        }))
        .child(pw("New password", "pw-new", &ui.new_pw, |x| {
            &mut x.settings_ui.new_pw
        }))
        .child(pw(
            "Confirm new password",
            "pw-confirm",
            &ui.confirm_pw,
            |x| &mut x.settings_ui.confirm_pw,
        ))
        .child(
            row().child(
                el().key("pw-change").child(
                    Button::new("Change password")
                        .variant(ButtonVariant::Outline)
                        .on_press(|s: &mut AppState| s.change_password())
                        .build(k),
                ),
            ),
        );
    if let Some((ok, m)) = &ui.pw_message {
        pw_form = pw_form.child(
            text(m.clone())
                .key("pw-message")
                .font(k.ty.text_small_medium)
                .color(if *ok {
                    k.c.success.icon
                } else {
                    k.c.danger.icon
                }),
        );
    }
    let security = card(
        k,
        "settings-security",
        "Security",
        col()
            .gap(14.0)
            .child(option_row(
                k,
                "Lock after inactivity",
                number_chips(
                    k,
                    "autolock",
                    &[0, 5, 10, 15, 30, 60],
                    st.auto_lock_minutes,
                    minutes,
                    |x, v| x.auto_lock_minutes = v,
                ),
            ))
            .child(option_row(
                k,
                "Clear copied keys after",
                number_chips(
                    k,
                    "clipboard",
                    &[0, 15, 30, 60, 120],
                    st.clipboard_clear_secs,
                    seconds,
                    |x, v| x.clipboard_clear_secs = v,
                ),
            ))
            .child(option_row(
                k,
                "Encrypted backups to keep",
                number_chips(
                    k,
                    "backups",
                    &[0, 3, 5, 10],
                    st.backups_to_keep,
                    count,
                    |x, v| x.backups_to_keep = v,
                ),
            ))
            .child(el().h(1.0).w_full().fill(k.c.border_weak))
            .child(option_row(k, "Change password", pw_form)),
    );

    let export = card(
        k,
        "settings-export",
        "Backup & export",
        col()
            .gap(14.0)
            .child(muted(k, format!("Vault file: {}", sess.vault.path().display())).key("vault-path").wrap())
            .child(
                hstack()
                    .child(primary(k, "export-backup", "Save encrypted backup…", true, |s, tx| s.export_backup(tx)))
                    .child(el().key("export-csv").child(Button::new("Export CSV…").variant(ButtonVariant::Outline).on_press(|s: &mut AppState| s.ask_export(ExportKind::Csv)).build(k)))
                    .child(el().key("export-json").child(Button::new("Export JSON…").variant(ButtonVariant::Outline).on_press(|s: &mut AppState| s.ask_export(ExportKind::Json)).build(k))),
            )
            .child(muted(k, "CSV and JSON exports are not encrypted and include the keys in the current library view.").wrap()),
    );

    col()
        .key("settings")
        .w_full()
        .h_full()
        .scroll(ScrollSpec::default())
        .child(
            col()
                .w_full()
                .max_w(860.0)
                .px(32.0)
                .pt(28.0)
                .pb(40.0)
                .gap(26.0)
                .child(text("Settings").font(title_style()).color(k.c.text_strong))
                .child(appearance)
                .child(data)
                .child(security)
                .child(export),
        )
}
