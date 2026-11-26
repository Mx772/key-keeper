//! Keyboard shortcut labels (⌘ on macOS, Ctrl elsewhere) and the shortcuts overlay.

use rune::{El, col, row, text};
use rune_kit::{Dialog, Keybind, Kit};

use crate::app::{AppState, Session};

/// Every global shortcut, as shown in the overlay: the key combinations (alternatives) and what
/// they do. Combinations are written with `Ctrl`, which reads as ⌘ on macOS.
pub const SHORTCUTS: &[(&[&str], &str)] = &[
    (&["Ctrl+F"], "Search the library"),
    (&["Ctrl+N"], "New key"),
    (&["Ctrl+I"], "Import keys"),
    (&["Ctrl+S"], "Save the open key"),
    (&["Ctrl+Z"], "Undo"),
    (&["Ctrl+Y", "Ctrl+Shift+Z"], "Redo"),
    (&["Ctrl+Shift+C"], "Copy selected keys as \"Name: Key\""),
    (&["Delete"], "Delete selected keys"),
    (
        &["↑", "↓"],
        "Move through the library (or type a name to jump to it)",
    ),
    (&["Enter", "F2"], "Edit the selected key"),
    (&["Ctrl+C"], "Copy the selected key"),
    (
        &["Ctrl+B"],
        "Add selected keys to the cart, or take them out",
    ),
    (&["Menu", "Shift+F10"], "Actions menu for the selected keys"),
    (&["Ctrl+L"], "Lock the vault"),
    (&["Esc"], "Close a dialog or the editor"),
    (&["F1", "Ctrl+/"], "Show keyboard shortcuts"),
];

const MAC: bool = cfg!(target_os = "macos");

/// The keycaps of `combo` ("Ctrl+Shift+C") on macOS (`["⇧", "⌘", "C"]`, in Apple's modifier
/// order) or elsewhere (`["Ctrl", "Shift", "C"]`).
pub fn caps_for(mac: bool, combo: &str) -> Vec<String> {
    let parts: Vec<&str> = combo.split('+').collect();
    let Some((key, mods)) = parts.split_last() else {
        return Vec::new();
    };
    if !mac {
        return parts.iter().map(|p| (*p).to_string()).collect();
    }
    let has = |m: &str| mods.iter().any(|x| x.eq_ignore_ascii_case(m));
    let mut caps = Vec::new();
    for (name, sym) in [("Alt", "⌥"), ("Shift", "⇧"), ("Ctrl", "⌘")] {
        if has(name) {
            caps.push(sym.to_string());
        }
    }
    caps.push(match *key {
        "Delete" => "⌦".to_string(),
        "Esc" => "⎋".to_string(),
        k => k.to_string(),
    });
    caps
}

/// `combo` as one label: "⇧⌘C" on macOS, "Ctrl+Shift+C" elsewhere.
pub fn label_for(mac: bool, combo: &str) -> String {
    let caps = caps_for(mac, combo);
    if mac { caps.concat() } else { caps.join("+") }
}

/// `combo` as one label for this platform.
pub fn shortcut(combo: &str) -> String {
    label_for(MAC, combo)
}

/// The shortcuts overlay (F1 / Ctrl+/ / the sidebar's "Keyboard shortcuts").
pub fn dialog(sess: &Session, k: &Kit, overlays: &mut Vec<El>) {
    if !sess.shortcuts_open {
        return;
    }
    let mut list = col()
        .key("shortcut-list")
        .w_full()
        .px(16.0)
        .pb(14.0)
        .gap(2.0);
    for (i, (combos, what)) in SHORTCUTS.iter().enumerate() {
        let mut keys = row().gap(6.0).center_y();
        for (j, combo) in combos.iter().enumerate() {
            if j > 0 {
                keys = keys.child(text("or").font(k.ty.text_xsmall).color(k.c.text_faint));
            }
            keys = keys.child(Keybind::new(&caps_for(MAC, combo)).keep_case().build(k));
        }
        list = list.child(
            row()
                .key(format!("shortcut-{i}"))
                .w_full()
                .min_h(28.0)
                .gap(12.0)
                .center_y()
                .child(
                    text(*what)
                        .font(k.ty.text_small)
                        .color(k.c.text_base)
                        .grow(1.0),
                )
                .child(keys),
        );
    }
    Dialog::new("shortcuts", "Keyboard shortcuts")
        .open(true) // rune-kit dialogs render nothing unless opened
        .width(460.0)
        .fit()
        .child(list)
        .on_close(|s: &mut AppState| s.show_shortcuts(false))
        .build(k, overlays);
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn labels_follow_the_platform() {
        assert_eq!(label_for(false, "Ctrl+L"), "Ctrl+L");
        assert_eq!(label_for(true, "Ctrl+L"), "⌘L");
        assert_eq!(label_for(false, "Ctrl+Shift+C"), "Ctrl+Shift+C");
        assert_eq!(label_for(true, "Ctrl+Shift+C"), "⇧⌘C");
        assert_eq!(label_for(true, "Ctrl+/"), "⌘/");
        assert_eq!(label_for(true, "F1"), "F1");
        assert_eq!(caps_for(true, "Ctrl+Shift+Z"), ["⇧", "⌘", "Z"]);
        assert_eq!(caps_for(false, "Ctrl+Shift+Z"), ["Ctrl", "Shift", "Z"]);
    }
}
