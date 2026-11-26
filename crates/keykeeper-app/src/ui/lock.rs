//! Create / unlock screen.

use rune::{El, FileDialogOptions, Tx, col, el, row, text};
use rune_kit::{Button, ButtonVariant, Kit, TextInput};

use super::{field, lock_edit, muted, primary, title_style};
use crate::app::{AppState, LockMode, LockUi};

/// 0–4 and a word. A rough guide only: length and character variety.
pub fn strength(pw: &str) -> (u8, &'static str) {
    if pw.is_empty() {
        return (0, "");
    }
    let n = pw.chars().count();
    let classes = [
        pw.chars().any(|c| c.is_lowercase()),
        pw.chars().any(|c| c.is_uppercase()),
        pw.chars().any(|c| c.is_ascii_digit()),
        pw.chars().any(|c| !c.is_alphanumeric()),
    ]
    .iter()
    .filter(|b| **b)
    .count();
    let score = match (n, classes) {
        (0..8, _) => 0,
        (8..12, 0..=2) => 1,
        (8..12, _) => 2,
        (12..16, 0..=1) => 2,
        (12..16, _) => 3,
        (_, 0..=1) => 3,
        _ => 4,
    };
    (
        score,
        ["Too short", "Weak", "Fair", "Good", "Strong"][score as usize],
    )
}

pub fn view(l: &LockUi, k: &Kit) -> El {
    let creating = l.mode == LockMode::Create;
    let password = TextInput::new(&l.password)
        .label(if creating { "New password" } else { "Password" })
        .placeholder(if creating {
            "At least 8 characters"
        } else {
            "Vault password"
        })
        .secure()
        .large()
        .width(None)
        .disabled(l.busy)
        .on_edit(|s: &mut AppState| lock_edit(s, |l| &mut l.password))
        .on_submit(|s: &mut AppState, _: &mut Tx| s.start_unlock())
        .build(k)
        .key("lock-password");

    let mut form = col().w_full().gap(14.0).child(field(
        k,
        if creating { "New password" } else { "Password" },
        password,
        None,
    ));
    if creating {
        let (score, word) = strength(&l.password.text);
        let bar = row()
            .key("strength")
            .w_full()
            .gap(4.0)
            .children((0..4).map(|i| {
                let on = score > i;
                let color = match score {
                    0 | 1 => k.c.danger.icon,
                    2 => k.c.warning.icon,
                    _ => k.c.success.icon,
                };
                el().h(4.0)
                    .grow(1.0)
                    .radius(2.0)
                    .fill(if on { color } else { k.c.surface_raised })
            }));
        form = form.child(col().gap(6.0).child(bar).child(muted(k, word)));
        form = form.child(field(
            k,
            "Confirm password",
            TextInput::new(&l.confirm)
                .label("Confirm password")
                .secure()
                .large()
                .width(None)
                .disabled(l.busy)
                .on_edit(|s: &mut AppState| lock_edit(s, |l| &mut l.confirm))
                .on_submit(|s: &mut AppState, _: &mut Tx| s.start_unlock())
                .build(k)
                .key("lock-confirm"),
            None,
        ));
    }
    if let Some(e) = &l.error {
        form = form.child(
            text(e.clone())
                .key("lock-error")
                .font(k.ty.text_small_medium)
                .color(k.c.danger.icon),
        );
    }
    let label = match (creating, l.busy) {
        (_, true) => "Working…",
        (true, false) => "Create vault",
        (false, false) => "Unlock",
    };
    form = form.child(primary(k, "lock-submit", label, !l.busy, |s, _| s.start_unlock()).w_full());

    if creating {
        form = form.child(
            text("Your keys are encrypted with this password. There is no way to recover it if you forget it — keep it somewhere safe.")
                .font(k.ty.text_small)
                .color(k.c.warning.icon)
                .wrap(),
        );
    }

    let path = l.path.display().to_string();
    let switch = row()
        .gap(6.0)
        .center_y()
        .child(
            Button::new("Open vault…")
                .variant(ButtonVariant::GhostMuted)
                .on_press_tx(|_: &mut AppState, tx: &mut Tx| {
                    let o = FileDialogOptions {
                        title: "Open a KeyKeeper vault".into(),
                        ..Default::default()
                    }
                    .filter("KeyKeeper vault", &["kkvault"]);
                    tx.open_file(o, |s: &mut AppState, r, _| {
                        if let Ok(Some(p)) = r {
                            s.set_vault_path(p, LockMode::Unlock);
                        }
                    });
                })
                .build(k),
        )
        .child(
            Button::new("New vault…")
                .variant(ButtonVariant::GhostMuted)
                .on_press_tx(|_: &mut AppState, tx: &mut Tx| {
                    let o = FileDialogOptions {
                        title: "Create a KeyKeeper vault".into(),
                        file_name: Some("keys.kkvault".into()),
                        ..Default::default()
                    }
                    .filter("KeyKeeper vault", &["kkvault"]);
                    tx.save_file(o, |s: &mut AppState, r, _| {
                        if let Ok(Some(p)) = r {
                            s.set_vault_path(p, LockMode::Create);
                        }
                    });
                })
                .build(k),
        );

    let card = col()
        .key("lock-card")
        .w(420.0)
        .padding(32.0)
        .gap(22.0)
        .radius(k.radius.xl)
        .fill(k.c.bg_base)
        .border(1.0, k.c.border_weak)
        .shadow(12.0, 40.0, k.c.black.alpha(0.35))
        .child(
            col()
                .gap(6.0)
                .child(
                    text("KeyKeeper")
                        .font(rune::TextStyle::new(15.0, 700.0))
                        .color(k.c.text_accent),
                )
                .child(
                    text(if creating {
                        "Create your vault"
                    } else {
                        "Unlock your vault"
                    })
                    .font(title_style())
                    .color(k.c.text_strong),
                )
                .child(muted(k, path).key("lock-path").truncate()),
        )
        .child(form)
        .child(switch);
    col()
        .key("lock")
        .w_full()
        .h_full()
        .center()
        .fill(k.c.page)
        .child(card)
}

#[cfg(test)]
mod tests {
    use super::strength;

    #[test]
    fn strength_scale() {
        assert_eq!(strength("").0, 0);
        assert_eq!(strength("abc").0, 0);
        assert_eq!(strength("abcdefgh").0, 1);
        assert_eq!(strength("abcdEF12").0, 2);
        assert_eq!(strength("correcthorsebattery").0, 3);
        assert_eq!(
            strength("correct horse battery").0,
            4,
            "long passphrases are strong"
        );
        assert_eq!(strength("Correct-Horse-9-Battery").0, 4);
    }
}
