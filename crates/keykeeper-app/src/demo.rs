//! Sample data for screenshots (`--scene`) and UI tests. Never touches the user's vault: it
//! writes a throwaway vault in the temp dir with insecure fast KDF parameters.

use jiff::civil::date;
use keykeeper_core::{
    GameKey, KdfParams, KeyStatus, Platform, Trade, TradeStatus, Vault, VaultData,
};

use crate::app::AppState;

pub const SAMPLE_CSV: &str = "Website,Bundle,Game Name,Key,Redeemed,Redeem/Trade Date,Trade Link,Redemption Deadline
Humble,Humble Conquer COVID-19 Bundle,Undertale,UNDR7-TALE0-00001,Redeemed,8/20/2025,https://lestrades.com/offer/100002/,None
Humble,In Case You Missed It: Gems of 2022,\"Haiku, the Robot\",HA1KU-R0B0T-00002,X,1/7/2026,https://lestrades.com/offer/100001/,
Fanatical,Mystery Bundle,Celeste,QQ7RT-PP2MN-ZZ9XY,,,,12/31/2026
Fanatical,Mystery Bundle,Hades,HD5S2-77KLM-NB3VC,maybe,,,
Humble,Humble Choice,Outer Wilds,,,,,
";

/// name, key, bundle, source, status, Steam app id, redemption deadline (y, m, d)
type DemoRow = (
    &'static str,
    &'static str,
    &'static str,
    &'static str,
    KeyStatus,
    Option<u32>,
    Option<(i16, i8, i8)>,
);

pub fn sample_data() -> VaultData {
    let rows: [DemoRow; 18] = [
        (
            "Undertale",
            "UNDR7-TALE0-00001",
            "Humble Conquer COVID-19 Bundle",
            "Humble",
            KeyStatus::Traded,
            Some(391540),
            None,
        ),
        (
            "Haiku, the Robot",
            "HA1KU-R0B0T-00002",
            "In Case You Missed It: Gems of 2022",
            "Humble",
            KeyStatus::Available,
            Some(1231880),
            Some((2026, 12, 15)),
        ),
        (
            "Spiritfarer",
            "SP1R1-T4RER-00001",
            "Humble Choice March",
            "Humble",
            KeyStatus::Available,
            Some(972660),
            None,
        ),
        (
            "Celeste",
            "QQ7RT-PP2MN-ZZ9XY",
            "Fanatical Mystery",
            "Fanatical",
            KeyStatus::Reserved,
            Some(504230),
            Some((2026, 12, 2)),
        ),
        (
            "Hades",
            "HD5S2-77KLM-NB3VC",
            "Humble Choice",
            "Humble",
            KeyStatus::Available,
            Some(1145360),
            None,
        ),
        (
            "Outer Wilds",
            "OW7LD-SS2N3-QQ8PL",
            "Humble Choice",
            "Humble",
            KeyStatus::Redeemed,
            Some(753640),
            None,
        ),
        (
            "Inscryption",
            "1NSCR-YPT10-N0002",
            "Fanatical Build Your Own",
            "Fanatical",
            KeyStatus::Available,
            Some(1092790),
            None,
        ),
        (
            "Dredge",
            "AB12C-DE34F-GH56I",
            "In Case You Missed It: Gems of 2022",
            "Humble",
            KeyStatus::Unknown,
            Some(1562430),
            Some((2026, 11, 20)),
        ),
        (
            "Disco Elysium",
            "D1SC0-ELYS1-UM999",
            "Humble Choice",
            "Humble",
            KeyStatus::Gifted,
            Some(632470),
            None,
        ),
        (
            "Return of the Obra Dinn",
            "0BRA9-D1NN7-RTRN4",
            "Humble Indie Bundle",
            "Humble",
            KeyStatus::Available,
            Some(653530),
            None,
        ),
        (
            "Slay the Spire",
            "SL4YS-P1RE7-77777",
            "Fanatical Mystery",
            "Fanatical",
            KeyStatus::Available,
            Some(646570),
            None,
        ),
        (
            "Hollow Knight",
            "H0LL0-WKN1G-HT321",
            "Humble Choice",
            "Humble",
            KeyStatus::Traded,
            Some(367520),
            None,
        ),
        (
            "Tunic",
            "TUN1C-F0X99-AAAA1",
            "Humble Choice",
            "Humble",
            KeyStatus::Available,
            Some(553420),
            Some((2027, 3, 1)),
        ),
        (
            "A Short Hike",
            "SH0RT-H1KE2-BBBB2",
            "Itch.io Bundle for Racial Justice",
            "Itch",
            KeyStatus::Available,
            Some(1055540),
            None,
        ),
        (
            "Cult of the Lamb",
            "CULT0-FLAMB-CCCC3",
            "Fanatical Mystery",
            "Fanatical",
            KeyStatus::Available,
            Some(1313140),
            None,
        ),
        (
            "Death's Door",
            "DEATH-SD00R-DDDD4",
            "Humble Choice",
            "Humble",
            KeyStatus::Available,
            Some(894020),
            None,
        ),
        (
            "Stardew Valley",
            "ST4RD-EWV4L-EEEE5",
            "",
            "GOG",
            KeyStatus::Available,
            None,
            None,
        ),
        (
            "Pentiment",
            "PENT1-MENT7-FFFF6",
            "Humble Choice",
            "Humble",
            KeyStatus::Unknown,
            Some(1205520),
            None,
        ),
    ];
    let mut data = VaultData::default();
    let mut trade = Trade::new();
    trade.partner_name = "lumen".into();
    trade.trade_url = Some("https://lestrades.com/offer/100001/".into());
    trade.status = TradeStatus::Completed;
    trade.completed_at = jiff::Timestamp::from_second(1_767_787_200).ok(); // 2026-01-07
    let base =
        jiff::Timestamp::from_second(1_767_225_600).unwrap_or_else(|_| jiff::Timestamp::now()); // 2026-01-01
    for (i, (name, code, bundle, source, status, app, deadline)) in rows.into_iter().enumerate() {
        let mut k = GameKey::new(name, code);
        k.bundle = Some(bundle.to_string()).filter(|b| !b.is_empty());
        k.source = Some(source.to_string());
        k.status = status;
        k.steam_app_id = app;
        k.redemption_deadline = deadline.and_then(|(y, m, d)| jiff::civil::Date::new(y, m, d).ok());
        k.created_at = base
            .checked_add(jiff::SignedDuration::from_hours(i as i64 * 30))
            .unwrap_or(base);
        if let Some(app) = app {
            // Plausible-looking demo metadata so screenshots show ratings and ownership.
            let percent = [
                94u8, 97, 90, 96, 98, 95, 92, 89, 93, 98, 97, 97, 90, 97, 92, 89, 96, 81,
            ][i % 18];
            let label = if percent >= 95 {
                "Overwhelmingly Positive"
            } else if percent >= 80 {
                "Very Positive"
            } else {
                "Mostly Positive"
            };
            let ownership = match app {
                1145360 | 632470 => keykeeper_core::Ownership::Wishlisted,
                391540 | 367520 => keykeeper_core::Ownership::Owned,
                _ => keykeeper_core::Ownership::NotOwned,
            };
            k.steam = Some(keykeeper_core::SteamMeta {
                rating: Some(keykeeper_core::SteamRating {
                    desc: label.into(),
                    percent,
                    total: 1_000 + (app % 90_000),
                }),
                ownership,
                fetched_at: base,
            });
        }
        if name == "Stardew Valley" {
            k.platform = Platform::Gog;
        }
        if status == KeyStatus::Traded {
            k.status_date = Some(date(2026, 1, 7));
            k.trade_id = Some(trade.id);
            trade.key_ids.push(k.id);
        }
        data.keys.push(k);
    }
    data.trades.push(trade);
    data
}

/// Replace the screen with an unlocked demo session.
pub fn unlock(s: &mut AppState, data: VaultData) {
    // Unique per call: tests run in parallel within one process.
    static NEXT: std::sync::atomic::AtomicU32 = std::sync::atomic::AtomicU32::new(0);
    let n = NEXT.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
    let path =
        std::env::temp_dir().join(format!("keykeeper-demo-{}-{n}.kkvault", std::process::id()));
    let _ = std::fs::remove_file(&path);
    if let Ok(v) = Vault::create(
        &path,
        "demo-password",
        KdfParams::INSECURE_FAST_FOR_TESTS,
        &data,
    ) {
        s.open_session(v, data);
    }
}
