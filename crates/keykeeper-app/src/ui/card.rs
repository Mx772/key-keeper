//! Game art and the game details hover card.

use jiff::civil::Date;
use keykeeper_core::{GameKey, Ownership};
use rune::{Align, Color, El, Side, col, el, row, text};
use rune_kit::Kit;

use super::{muted, status_pill};
use crate::app::{AppState, Session, today};
use crate::form::format_date;

/// Steam header art's shape (460×215).
pub const HEADER_RATIO: f32 = 215.0 / 460.0;

/// Pairs of colours for placeholder art, picked by the game's name so each game keeps its look.
const PLACEHOLDER: [(&str, &str); 8] = [
    ("#3b5bdb", "#15aabf"),
    ("#7048e8", "#d6336c"),
    ("#2b8a3e", "#74b816"),
    ("#e8590c", "#f08c00"),
    ("#1864ab", "#5f3dc4"),
    ("#c2255c", "#f76707"),
    ("#0b7285", "#2f9e44"),
    ("#495057", "#868e96"),
];

fn hex(s: &str) -> Color {
    rune::color::hex(s).unwrap_or(Color::BLACK)
}

fn name_hash(name: &str) -> u64 {
    // FNV-1a: stable across runs and platforms.
    name.bytes().fold(0xcbf2_9ce4_8422_2325, |h, b| {
        (h ^ u64::from(b)).wrapping_mul(0x0000_0100_0000_01b3)
    })
}

/// Up to two letters from the name's words: "Hollow Knight" → "HK".
pub fn initials(name: &str) -> String {
    let words: Vec<&str> = name
        .split(|c: char| !c.is_alphanumeric())
        .filter(|w| !w.is_empty())
        .filter(|w| !matches!(w.to_lowercase().as_str(), "the" | "of" | "a" | "an" | "and"))
        .collect();
    let mut out: String = words
        .iter()
        .take(2)
        .filter_map(|w| w.chars().next())
        .flat_map(char::to_uppercase)
        .collect();
    if out.is_empty() {
        out = name.chars().take(1).flat_map(char::to_uppercase).collect();
    }
    out
}

/// A game's header art at `w` × `h`, or a coloured placeholder with its initials.
pub fn art(s: &AppState, key: &GameKey, w: f32, h: f32, radius: [f32; 4]) -> El {
    let image = key.steam_app_id.and_then(|a| s.info.art(a));
    let base = el().w(w).h(h).corners(radius).clip_corners(radius);
    match image {
        Some(img) => base
            .fill(Color::BLACK)
            .pixels_fit(img.pixels(), rune::SurfaceFit::Fill)
            .role("image")
            .label(format!("{} artwork", key.game_name)),
        None => placeholder(&key.game_name, base, h),
    }
}

/// Like [`art`] but as wide as its parent.
pub fn art_fill(s: &AppState, key: &GameKey, h: f32, radius: f32) -> El {
    let image = key.steam_app_id.and_then(|a| s.info.art(a));
    let base = el().w_full().h(h).radius(radius).clip_rounded(radius);
    match image {
        Some(img) => base
            .fill(Color::BLACK)
            .pixels_fit(img.pixels(), rune::SurfaceFit::Fill)
            .role("image")
            .label(format!("{} artwork", key.game_name)),
        None => placeholder(&key.game_name, base, h),
    }
}

fn placeholder(name: &str, base: El, h: f32) -> El {
    let (a, b) = PLACEHOLDER[(name_hash(name) % PLACEHOLDER.len() as u64) as usize];
    let size = (h * 0.36).clamp(14.0, 56.0);
    base.fill(hex(a))
        .linear_gradient(2.2, [(0.0, hex(a)), (1.0, hex(b).mix(Color::BLACK, 0.15))])
        .center()
        .a11y_hidden()
        .child(
            text(initials(name))
                .font(rune::TextStyle::new(size, 800.0).tracking(1.0))
                .color(Color::WHITE.alpha(0.92)),
        )
}

/// "in 12 days", "tomorrow", "today", "3 days ago".
pub fn relative_days(d: Date, today: Date) -> String {
    let n = (d - today).get_days();
    match n {
        0 => "today".into(),
        1 => "tomorrow".into(),
        -1 => "yesterday".into(),
        2.. => format!("in {n} days"),
        _ => format!("{} days ago", -n),
    }
}

fn fact(k: &Kit, label: &str, value: El) -> El {
    row()
        .w_full()
        .gap(10.0)
        .center_y()
        .child(
            text(label.to_string())
                .font(k.ty.text_small)
                .color(k.c.text_faint)
                .w(78.0),
        )
        .child(value.grow(1.0).min_w(0.0))
}

fn value(k: &Kit, s: impl Into<String>, color: Color) -> El {
    text(s.into())
        .font(k.ty.text_small_medium)
        .color(color)
        .wrap()
}

fn tag(k: &Kit, s: &str) -> El {
    el().h(20.0)
        .px(8.0)
        .radius(10.0)
        .center()
        .fill(k.c.text_base.alpha(0.08))
        .child(
            text(s.to_string())
                .font(k.ty.text_xsmall_medium)
                .color(k.c.text_muted),
        )
}

pub const CARD_W: f32 = 360.0;

/// The details card for `key`, built for the overlay layer and anchored to `anchor`.
pub fn hover_card(s: &AppState, sess: &Session, key: &GameKey, anchor: &str, k: &Kit) -> El {
    let info = sess.repo.key_info(key);
    let title = info
        .map(|i| i.name.as_str())
        .filter(|n| !n.is_empty())
        .unwrap_or(&key.game_name)
        .to_string();
    let mut byline = Vec::new();
    if let Some(i) = info {
        if let Some(d) = i.developers.first() {
            byline.push(d.clone());
        }
        if let Some(t) = i.released {
            byline.push(
                t.to_zoned_datetime(jiff::tz::TimeZone::UTC)
                    .year()
                    .to_string(),
            );
        }
    }
    let mut body = col().w_full().px(16.0).pt(12.0).pb(14.0).gap(10.0).child(
        col()
            .gap(3.0)
            .child(
                text(title)
                    .font(rune::TextStyle::new(16.0, 650.0).lh(21.0))
                    .color(k.c.text_strong)
                    .wrap(),
            )
            .children((!byline.is_empty()).then(|| muted(k, byline.join(" · ")))),
    );
    if let Some(i) = info {
        if !i.tags.is_empty() {
            body = body.child(
                row()
                    .gap(5.0)
                    .flex_wrap()
                    .children(i.tags.iter().take(4).map(|t| tag(k, t))),
            );
        }
        if !i.description.is_empty() {
            body = body.child(
                text(i.description.clone())
                    .font(k.ty.text_small)
                    .color(k.c.text_base)
                    .wrap()
                    .max_lines(4),
            );
        }
    }

    let mut facts = col().w_full().gap(6.0);
    facts = facts.child(fact(k, "Status", row().child(status_pill(k, key.status))));
    if let Some(r) = key.steam.as_ref().and_then(|m| m.rating.as_ref()) {
        let c = match r.percent {
            80.. => k.c.success.icon,
            60..80 => k.c.text_base,
            40..60 => k.c.warning.icon,
            _ => k.c.danger.icon,
        };
        facts = facts.child(fact(
            k,
            "Reviews",
            value(
                k,
                format!("{} · {}% of {}", r.desc, r.percent, thousands(r.total)),
                c,
            ),
        ));
    }
    if let Some(d) = info.and_then(|i| i.deck.short_label()) {
        facts = facts.child(fact(k, "Steam Deck", value(k, d, k.c.text_base)));
    }
    match key.steam.as_ref().map(|m| m.ownership) {
        Some(Ownership::Owned) => {
            facts = facts.child(fact(
                k,
                "On Steam",
                value(k, "You own this", k.c.success.icon),
            ))
        }
        Some(Ownership::Wishlisted) => {
            facts = facts.child(fact(
                k,
                "On Steam",
                value(k, "On your wishlist", k.c.text_accent),
            ))
        }
        _ => {}
    }
    if let Some(p) = sess.repo.key_prices(key)
        && let Some((best, shop)) = p.best()
    {
        let mut v = format!("{} at {}", best.format(), shop.unwrap_or("best store"));
        if let Some(l) = &p.history_low {
            v.push_str(&format!(" · low {}", l.format()));
        }
        facts = facts.child(fact(k, "Best price", value(k, v, k.c.text_base)));
    }
    if let Some(st) = sess.repo.key_prices(key).and_then(|p| p.steam()) {
        let mut v = st.price.format();
        if st.cut > 0 {
            v.push_str(&format!(" (−{}%)", st.cut));
        }
        facts = facts.child(fact(k, "Steam price", value(k, v, k.c.text_base)));
    }
    if let Some(ks) = sess
        .repo
        .key_prices(key)
        .and_then(|p| p.keyshop_low.as_ref())
    {
        facts = facts.child(fact(k, "Keyshops", value(k, ks.format(), k.c.text_muted)));
    }
    if let Some(d) = key.redemption_deadline
        && !key.status.is_gone()
    {
        let t = today();
        let c = if d < t {
            k.c.danger.icon
        } else if (d - t).get_days() <= 30 {
            k.c.warning.icon
        } else {
            k.c.text_base
        };
        facts = facts.child(fact(
            k,
            "Redeem by",
            value(
                k,
                format!(
                    "{} ({})",
                    format_date(Some(d), sess.date_format()),
                    relative_days(d, t)
                ),
                c,
            ),
        ));
    }
    let from: Vec<&str> = [key.bundle.as_deref(), key.source.as_deref()]
        .into_iter()
        .flatten()
        .filter(|x| !x.is_empty())
        .collect();
    if !from.is_empty() {
        facts = facts.child(fact(k, "From", value(k, from.join(" · "), k.c.text_base)));
    }
    body = body
        .child(el().w_full().h(1.0).fill(k.c.border_weak))
        .child(facts);
    if info.is_none() && key.steam_app_id.is_some() && s.fetch_allowed() {
        body = body.child(muted(k, "Fetching store details…").key("card-loading"));
    }

    let r = k.radius.lg;
    let mut card = col()
        .key("game-card")
        .role("tooltip")
        .label(format!("{} details", key.game_name))
        .w(CARD_W)
        .radius(r)
        .fill(k.c.surface_raised)
        .border(1.0, k.c.border_weak)
        .clip_rounded(r);
    if key.steam_app_id.is_some() {
        card = card.child(art(
            s,
            key,
            CARD_W,
            (CARD_W * HEADER_RATIO).round(),
            [r, r, 0.0, 0.0],
        ));
    }
    card = card.child(body);
    k.elevate(card, &k.elevation.md)
        .anchored(anchor.to_string(), Side::Bottom, Align::Start, 6.0)
}

pub fn thousands(n: u32) -> String {
    let s = n.to_string();
    let mut out = String::new();
    for (i, c) in s.chars().enumerate() {
        if i > 0 && (s.len() - i).is_multiple_of(3) {
            out.push(',');
        }
        out.push(c);
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use jiff::civil::date;

    #[test]
    fn initials_pick_words() {
        assert_eq!(initials("Hollow Knight"), "HK");
        assert_eq!(initials("The Witness"), "W");
        assert_eq!(initials("Return of the Obra Dinn"), "RO");
        assert_eq!(initials("Haiku, the Robot"), "HR");
        assert_eq!(initials("A"), "A");
        assert_eq!(initials(""), "");
    }

    #[test]
    fn relative() {
        let t = date(2026, 11, 26);
        assert_eq!(relative_days(t, t), "today");
        assert_eq!(relative_days(date(2026, 11, 27), t), "tomorrow");
        assert_eq!(relative_days(date(2026, 11, 25), t), "yesterday");
        assert_eq!(relative_days(date(2026, 12, 8), t), "in 12 days");
        assert_eq!(relative_days(date(2026, 11, 23), t), "3 days ago");
    }

    #[test]
    fn thousands_sep() {
        assert_eq!(thousands(0), "0");
        assert_eq!(thousands(999), "999");
        assert_eq!(thousands(12431), "12,431");
        assert_eq!(thousands(1234567), "1,234,567");
    }
}
