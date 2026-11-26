//! The update banner (top of every screen, lock screen included), the "What's new" dialog, and
//! the Settings › Updates card.

use rune::{El, ScrollSpec, col, el, row, text};
use rune_kit::{Button, ButtonVariant, Checkbox, Dialog, Kit};

use super::{card, hstack, muted, primary};
use crate::app::AppState;
use crate::update_actions::{CheckStatus, Phase, Updates};

fn quiet(k: &Kit, key: &str, label: &str, press: fn(&mut AppState)) -> El {
    el().key(key.to_string()).child(
        Button::new(label)
            .variant(ButtonVariant::GhostMuted)
            .on_press(move |s: &mut AppState| press(s))
            .build(k),
    )
}

fn outline(k: &Kit, key: &str, label: &str, press: fn(&mut AppState)) -> El {
    el().key(key.to_string()).child(
        Button::new(label)
            .variant(ButtonVariant::Outline)
            .on_press(move |s: &mut AppState| press(s))
            .build(k),
    )
}

fn mib(n: u64) -> String {
    format!("{:.1} MB", n as f64 / 1_000_000.0)
}

/// A strip above the page while an update is on offer, downloading or failed.
pub fn banner(u: &Updates, k: &Kit) -> Option<El> {
    if !u.banner_visible() {
        return None;
    }
    let version = u
        .available
        .as_ref()
        .map(|r| r.version.to_string())
        .unwrap_or_default();
    let (title, detail, actions, error): (String, Option<String>, El, bool) = match &u.phase {
        Phase::Idle => (
            format!("KeyKeeper {version} is available"),
            Some(format!("You have {}.", u.current)),
            hstack()
                .gap(6.0)
                .child(quiet(k, "update-skip", "Skip this version", |s| {
                    s.skip_update()
                }))
                .child(quiet(k, "update-later", "Later", |s| s.update_later()))
                .child(outline(k, "update-notes", "What's new", |s| {
                    s.show_update_notes(true)
                }))
                .child(primary(k, "update-install", "Update", true, |s, _| {
                    s.start_update()
                })),
            false,
        ),
        Phase::Downloading { .. } => {
            let (done, total) = u.progress().unwrap_or((0, None));
            let detail = match total {
                Some(t) => format!("{} of {}", mib(done), mib(t)),
                None => mib(done),
            };
            let frac = total.map_or(0.0, |t| (done as f32 / t as f32).clamp(0.0, 1.0));
            let bar = el()
                .key("update-progress")
                .role("progressbar")
                .label(format!("{:.0}%", frac * 100.0))
                .w(180.0)
                .h(6.0)
                .radius(3.0)
                .fill(k.c.surface_raised)
                .child(
                    el().h_full()
                        .w(180.0 * frac)
                        .radius(3.0)
                        .fill(k.c.bg_accent),
                );
            (
                format!("Downloading KeyKeeper {version}…"),
                Some(format!(
                    "{detail}. Your vault is saved; KeyKeeper restarts when the installer runs."
                )),
                bar,
                false,
            )
        }
        Phase::Installing => (
            format!("Installing KeyKeeper {version}…"),
            None,
            el(),
            false,
        ),
        Phase::Failed(e) => (
            "The update didn't install".to_string(),
            Some(e.clone()),
            hstack()
                .gap(6.0)
                .child(quiet(k, "update-dismiss", "Dismiss", |s| {
                    s.dismiss_update_error()
                }))
                .child(primary(k, "update-retry", "Try again", true, |s, _| {
                    s.dismiss_update_error();
                    s.start_update();
                })),
            true,
        ),
    };
    let mut words = col().gap(2.0).grow(1.0).min_w(0.0).child(
        text(title)
            .key("update-title")
            .font(k.ty.ui_medium)
            .color(if error {
                k.c.danger.icon
            } else {
                k.c.text_strong
            }),
    );
    if let Some(d) = detail {
        words = words.child(muted(k, d).key("update-detail").wrap());
    }
    Some(
        row()
            .key("update-banner")
            .role("region")
            .label("Update available")
            .w_full()
            .px(20.0)
            .py(10.0)
            .gap(16.0)
            .center_y()
            .fill(if error { k.c.danger.bg } else { k.c.info.bg })
            .border(1.0, k.c.border_weak)
            .child(words)
            .child(actions),
    )
}

/// "What's new": the release notes from the manifest.
pub fn notes_dialog(u: &Updates, k: &Kit, overlays: &mut Vec<El>) {
    let (true, Some(r)) = (u.notes_open, &u.available) else {
        return;
    };
    let notes = if r.notes.trim().is_empty() {
        "No release notes were published for this version.".to_string()
    } else {
        plain_notes(&r.notes)
    };
    let long = notes.lines().count() > 14;
    let mut desc = format!("You have {}.", u.current);
    if let Some(d) = &r.date {
        desc = format!("Released {d}. {desc}");
    }
    let mut body = col().key("update-notes-text").w_full();
    if long {
        body = body.h(320.0).scroll(ScrollSpec::default());
    }
    let body = body.child(
        text(notes)
            .font(k.ty.text_small)
            .color(k.c.text_base)
            .wrap()
            .selectable(true),
    );
    let buttons = row()
        .w_full()
        .gap(8.0)
        .justify_end()
        .child(outline(k, "update-notes-close", "Close", |s| {
            s.show_update_notes(false)
        }))
        .child(primary(
            k,
            "update-notes-install",
            "Update",
            true,
            |s, _| s.start_update(),
        ));
    Dialog::new(
        "update-notes-dialog",
        format!("What's new in KeyKeeper {}", r.version),
    )
    .open(true) // rune-kit dialogs render nothing unless opened
    .description(desc)
    .width(520.0)
    .fit()
    .child(
        col()
            .w_full()
            .px(16.0)
            .pb(16.0)
            .gap(16.0)
            .child(body)
            .child(buttons),
    )
    .on_close(|s: &mut AppState| s.show_update_notes(false))
    .build(k, overlays);
}

/// Release notes are Markdown; show them as plain text: no `#` heading marks, bullets as `•`.
pub fn plain_notes(md: &str) -> String {
    md.trim()
        .lines()
        .map(|l| {
            let t = l.trim_start();
            if t.starts_with('#') {
                t.trim_start_matches('#').trim().to_string()
            } else if let Some(rest) = t.strip_prefix("- ").or_else(|| t.strip_prefix("* ")) {
                format!("• {rest}")
            } else {
                l.to_string()
            }
        })
        .collect::<Vec<_>>()
        .join(
            "
",
        )
}

/// Settings › Updates.
pub fn settings_card(s: &AppState, k: &Kit) -> El {
    let u = &s.updates;
    let (status, ok) = match &u.status {
        CheckStatus::NotChecked => ("Not checked yet".to_string(), None),
        CheckStatus::Checking => ("Checking…".to_string(), None),
        CheckStatus::UpToDate => ("You're up to date.".to_string(), Some(true)),
        CheckStatus::Available(v) => (format!("KeyKeeper {v} is available."), Some(true)),
        CheckStatus::Skipped(v) => (
            format!("KeyKeeper {v} is available (skipped). Check now to see it again."),
            None,
        ),
        CheckStatus::Note(m) => (m.clone(), None),
        CheckStatus::Failed(m) => (m.clone(), Some(false)),
    };
    let status = text(status)
        .key("update-status")
        .font(k.ty.text_small_medium)
        .color(match ok {
            Some(true) => k.c.success.icon,
            Some(false) => k.c.danger.icon,
            None => k.c.text_muted,
        })
        .wrap();
    let auto = s.config.check_updates();
    card(
        k,
        "settings-updates",
        "Updates",
        col()
            .gap(14.0)
            .child(
                text(format!("KeyKeeper {}", u.current))
                    .key("update-current")
                    .font(k.ty.ui_medium)
                    .color(k.c.text_base),
            )
            .child(
                Checkbox::new(auto)
                    .label("Check for updates automatically")
                    .on_toggle(move |s: &mut AppState| s.set_auto_update_check(!auto))
                    .build(k)
                    .key("update-auto"),
            )
            .child(
                hstack()
                    .child(primary(k, "update-check", "Check now", !u.checking, |s, _| {
                        s.check_for_updates(true)
                    }))
                    .child(status.grow(1.0).min_w(0.0)),
            )
            .child(
                muted(
                    k,
                    "A check is one request to GitHub for the latest release's version and notes; \
                     nothing about you or your vault is sent. Updates are only installed when you \
                     click Update, and only if the download is signed with KeyKeeper's release key.",
                )
                .wrap(),
            ),
    )
}
