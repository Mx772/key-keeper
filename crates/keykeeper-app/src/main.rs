//! Phase 0 spike: prove RUNE builds on Windows and that `rune-kit::Table` handles a realistic
//! key library (5,000 rows) smoothly, in dark mode with the opencode theme.
//!
//! `cargo run -p keykeeper-app` opens the window;
//! `cargo run -p keykeeper-app -- --snap target/spike.png --theme dark --size 1100x640` renders headless.

mod theme;

use std::borrow::Cow;

use rune::{App, AppSpec, Cx, El, Tx, col, el, row, text};
use rune_kit::{Cell, Kit, SortDir, Table, TableColumn, TableState};
use theme::ThemeId;

const ROWS: usize = 5_000;

struct SpikeKey {
    id: String,
    name: String,
    bundle: &'static str,
    key: String,
    status: &'static str,
    rating: u8,
}

struct State {
    rows: Vec<SpikeKey>,
    table: TableState,
    theme: ThemeId,
}

const GAMES: [&str; 8] = [
    "Haiku, the Robot",
    "Undertale",
    "Spiritfarer",
    "Hades",
    "Celeste",
    "Outer Wilds",
    "Inscryption",
    "Dredge",
];
const BUNDLES: [&str; 5] = [
    "Gems of 2022",
    "Conquer COVID-19",
    "Humble Choice",
    "Fanatical Mystery",
    "Indie Gala",
];
const STATUSES: [&str; 6] = [
    "Available",
    "Reserved",
    "Traded",
    "Redeemed",
    "Gifted",
    "Unknown",
];

fn fake_key(r: u64) -> String {
    const A: &[u8] = b"ABCDEFGHJKLMNPQRSTUVWXYZ0123456789";
    let mut x = r;
    let mut s = String::with_capacity(17);
    for i in 0..15 {
        if i > 0 && i % 5 == 0 {
            s.push('-');
        }
        s.push(A[(x % A.len() as u64) as usize] as char);
        x = x.rotate_left(7) ^ 0x9E37_79B9_7F4A_7C15;
    }
    s
}

fn state() -> State {
    let mut x: u64 = 0x2545_f491_4f6c_dd1d;
    let mut next = move || {
        x ^= x << 13;
        x ^= x >> 7;
        x ^= x << 17;
        x
    };
    let rows = (0..ROWS)
        .map(|i| {
            let r = next();
            SpikeKey {
                id: format!("k{i}"),
                name: format!("{} #{i}", GAMES[(r % 8) as usize]),
                bundle: BUNDLES[(r >> 4) as usize % 5],
                key: fake_key(r),
                status: STATUSES[(r >> 9) as usize % 6],
                rating: 60 + ((r >> 20) % 40) as u8,
            }
        })
        .collect();
    State {
        rows,
        table: TableState::new().multi_select(true),
        theme: ThemeId::default(),
    }
}

/// Spike-only theme picker; the real one lives in Settings (Phase 3).
fn theme_picker(s: &State, k: &Kit) -> El {
    let mut bar = row()
        .key("themes")
        .gap(6.0)
        .center_y()
        .child(text("Theme").font(k.ty.text_small).color(k.c.text_muted));
    for t in ThemeId::ALL {
        let on = s.theme == t;
        bar = bar.child(
            el().key(format!("theme-{}", t.slug()))
                .role("radio")
                .label(t.label())
                .checked(on)
                .focusable(true)
                .px(10.0)
                .h(26.0)
                .center_y()
                .radius(13.0)
                .fill(if on {
                    k.c.bg_accent
                } else {
                    k.c.surface_raised
                })
                .child(text(t.label()).font(k.ty.text_small).color(if on {
                    k.c.text_contrast
                } else {
                    k.c.text_base
                }))
                .on_press_tx(move |s: &mut State, tx: &mut Tx| {
                    s.theme = t;
                    tx.set_color_scheme(t.color_scheme());
                }),
        );
    }
    bar
}

fn sort(s: &mut State, col: &str, dir: SortDir) {
    let by = |a: &SpikeKey, b: &SpikeKey| match col {
        "name" => rune_kit::natural_cmp(&a.name, &b.name),
        "bundle" => a.bundle.cmp(b.bundle),
        "status" => a.status.cmp(b.status),
        "rating" => a.rating.cmp(&b.rating),
        _ => a.key.cmp(&b.key),
    };
    s.rows.sort_by(|a, b| dir.apply(by(a, b)));
}

fn masked(key: &str) -> String {
    key.split('-')
        .enumerate()
        .map(|(i, p)| {
            if i == 0 {
                p.to_string()
            } else {
                "•••••".into()
            }
        })
        .collect::<Vec<_>>()
        .join("-")
}

fn view(s: &State, cx: &mut Cx<State>) -> El {
    let k = s.theme.kit(cx.system_dark());
    let kc = k.clone();
    let table = Table::new(
        |s: &State| &s.table,
        |s: &mut State| &mut s.table,
        |s: &State| s.rows.len(),
        |s: &State, i| Cow::Borrowed(s.rows[i].id.as_str()),
    )
    .name("Keys")
    .column(
        TableColumn::new("name", "Game")
            .flex(2.0)
            .min(180.0)
            .sortable(),
    )
    .column(
        TableColumn::new("bundle", "Bundle")
            .flex(1.2)
            .min(120.0)
            .sortable(),
    )
    .column(TableColumn::new("key", "Key").width(190.0).min(150.0))
    .column(TableColumn::new("status", "Status").width(110.0).sortable())
    .column(TableColumn::new("rating", "Rating").width(80.0).sortable())
    .cell(move |s: &State, i, col| {
        let r = &s.rows[i];
        match col {
            "name" => Cell::text(&r.name),
            "bundle" => Cell::text(r.bundle).muted(),
            "key" => Cell::text(masked(&r.key)).mono(),
            "rating" => Cell::text(format!("{}%", r.rating)).mono(),
            _ => {
                let dot = match r.status {
                    "Available" => kc.c.success.icon,
                    "Traded" | "Redeemed" | "Gifted" => kc.c.icon_faint,
                    _ => kc.c.warning.icon,
                };
                Cell::el(
                    row()
                        .gap(6.0)
                        .center_y()
                        .child(el().size(6.0, 6.0).radius(3.0).fill(dot))
                        .child(text(r.status).font(kc.ty.text_small).color(kc.c.text_muted)),
                )
            }
        }
    })
    .on_sort(|s: &mut State, col, dir, _| sort(s, col, dir))
    .zebra(true)
    .build(s, k)
    .grow(1.0)
    .min_h(0.0);
    let status = text(format!(
        "{} keys · {} selected",
        s.rows.len(),
        s.table.selected_len()
    ))
    .font(k.ty.text_small)
    .color(k.c.text_muted);
    col()
        .w_full()
        .h_full()
        .fill(k.c.page)
        .padding(12.0)
        .gap(8.0)
        .child(theme_picker(s, k))
        .child(
            col()
                .key("frame")
                .w_full()
                .grow(1.0)
                .min_h(0.0)
                .radius(k.radius.md)
                .fill(k.c.bg_base)
                .clip_rounded(k.radius.md)
                .child(table),
        )
        .child(row().key("status").w_full().child(status))
}

fn main() {
    let spec = AppSpec::<State> {
        scenes: &[
            ("midnight", |s| s.theme = ThemeId::Midnight),
            ("amethyst", |s| s.theme = ThemeId::Amethyst),
            ("forest", |s| s.theme = ThemeId::Forest),
            ("opencode", |s| s.theme = ThemeId::Opencode),
            ("graphite", |s| s.theme = ThemeId::Graphite),
            ("light", |s| s.theme = ThemeId::Light),
        ],
        ..AppSpec::new("KeyKeeper", (1100.0, 640.0), state, view)
    };
    App::new(spec)
        .color_scheme(ThemeId::default().color_scheme())
        .run();
}

#[cfg(test)]
mod tests {
    use super::*;
    use rune::{Env, Runtime};

    #[test]
    fn spike_renders_5k_rows_headlessly() {
        let env = Env {
            system_dark: true,
            scale: 1.0,
        };
        let mut rt = Runtime::new(state(), view, (1100.0, 640.0), env);
        rt.frame(0.0);
        assert_eq!(rt.state.rows.len(), ROWS);
        assert!(rt.find("status").is_some(), "status bar is laid out");
    }

    #[test]
    fn clicking_a_theme_pill_switches_theme() {
        let env = Env {
            system_dark: true,
            scale: 1.0,
        };
        let mut rt = Runtime::new(state(), view, (1100.0, 640.0), env);
        rt.frame(0.0);
        assert_eq!(rt.state.theme, ThemeId::Midnight);
        let (x, y) = rt.find("theme-forest").expect("forest pill");
        rt.pointer_down(x, y);
        rt.pointer_up(x, y);
        rt.frame(1.0 / 60.0);
        assert_eq!(rt.state.theme, ThemeId::Forest);
    }

    #[test]
    fn sorting_by_rating_orders_rows() {
        let mut s = state();
        sort(&mut s, "rating", SortDir::Descending);
        assert!(s.rows.windows(2).all(|w| w[0].rating >= w[1].rating));
    }

    #[test]
    fn keys_are_masked_after_first_group() {
        assert_eq!(masked("ABCDE-FGHIJ-KLMNO"), "ABCDE-•••••-•••••");
    }
}
