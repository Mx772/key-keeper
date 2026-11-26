//! The domain model (`docs/PLAN.md` §4). Every type here is stored inside the encrypted vault, so
//! serde names are part of the file format: **do not rename variants/fields without a migration.**

use jiff::Timestamp;
use jiff::civil::Date;
use serde::{Deserialize, Serialize};
use uuid::Uuid;

use crate::links;
use crate::secret::Secret;

// ── Key status ────────────────────────────────────────────────────────────────────────────────

/// Where a key is in its life. `Available` / `Traded` / `Unknown` are the "traded? false / true /
/// not sure" states; the rest exist so a reserved or self-used key is never mislabelled.
#[derive(
    Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord, Default, Serialize, Deserialize,
)]
#[serde(rename_all = "snake_case")]
pub enum KeyStatus {
    #[default]
    Available,
    /// In the cart or an open trade; cannot be added to another trade.
    Reserved,
    Traded,
    /// Activated on my own account.
    Redeemed,
    Gifted,
    Unknown,
}

impl KeyStatus {
    pub const ALL: [KeyStatus; 6] = [
        KeyStatus::Available,
        KeyStatus::Reserved,
        KeyStatus::Traded,
        KeyStatus::Redeemed,
        KeyStatus::Gifted,
        KeyStatus::Unknown,
    ];

    pub fn label(self) -> &'static str {
        match self {
            KeyStatus::Available => "Available",
            KeyStatus::Reserved => "Reserved",
            KeyStatus::Traded => "Traded",
            KeyStatus::Redeemed => "Redeemed",
            KeyStatus::Gifted => "Gifted",
            KeyStatus::Unknown => "Unknown",
        }
    }

    /// The key has left my hands and can't be traded again.
    pub fn is_gone(self) -> bool {
        matches!(
            self,
            KeyStatus::Traded | KeyStatus::Redeemed | KeyStatus::Gifted
        )
    }

    /// The key can go into the cart. `Unknown` is allowed (the user is unsure) but the UI warns.
    pub fn can_trade(self) -> bool {
        matches!(self, KeyStatus::Available | KeyStatus::Unknown)
    }
}

// ── Platform ──────────────────────────────────────────────────────────────────────────────────

#[derive(Clone, Debug, PartialEq, Eq, Hash, Default, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Platform {
    #[default]
    Steam,
    Gog,
    Epic,
    Ea,
    Ubisoft,
    Battlenet,
    Other(String),
}

impl Platform {
    pub fn label(&self) -> &str {
        match self {
            Platform::Steam => "Steam",
            Platform::Gog => "GOG",
            Platform::Epic => "Epic",
            Platform::Ea => "EA",
            Platform::Ubisoft => "Ubisoft",
            Platform::Battlenet => "Battle.net",
            Platform::Other(s) => s,
        }
    }

    /// Lenient parse for imports. Empty input means Steam (the common case).
    pub fn parse(s: &str) -> Platform {
        let t = s.trim();
        match t.to_ascii_lowercase().replace([' ', '.', '-'], "").as_str() {
            "" | "steam" => Platform::Steam,
            "gog" | "gogcom" | "goggalaxy" => Platform::Gog,
            "epic" | "epicgames" | "egs" => Platform::Epic,
            "ea" | "origin" | "eaapp" => Platform::Ea,
            "ubisoft" | "uplay" | "ubisoftconnect" => Platform::Ubisoft,
            "battlenet" | "blizzard" => Platform::Battlenet,
            _ => Platform::Other(t.to_string()),
        }
    }
}

// ── Steam metadata ────────────────────────────────────────────────────────────────────────────

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct SteamRating {
    /// Steam's own summary, e.g. "Very Positive".
    pub desc: String,
    pub percent: u8,
    pub total: u32,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Default, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Ownership {
    Owned,
    Wishlisted,
    NotOwned,
    #[default]
    Unknown,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct SteamMeta {
    #[serde(default)]
    pub rating: Option<SteamRating>,
    #[serde(default)]
    pub ownership: Ownership,
    pub fetched_at: Timestamp,
}

/// Manual URLs for when the ones derived from `steam_app_id` are wrong or missing.
#[derive(Clone, Debug, PartialEq, Eq, Default, Serialize, Deserialize)]
pub struct LinkOverrides {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub steam: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub gg_deals: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub itad: Option<String>,
}

// ── GameKey ───────────────────────────────────────────────────────────────────────────────────

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct GameKey {
    pub id: Uuid,
    pub game_name: String,
    pub key: Secret,
    #[serde(default)]
    pub platform: Platform,
    #[serde(default)]
    pub status: KeyStatus,
    #[serde(default)]
    pub bundle: Option<String>,
    /// Where the key came from ("Humble", "Fanatical", "Trade: alice"). The CSV "Website" column.
    #[serde(default)]
    pub source: Option<String>,
    /// Link to the page the key was revealed on.
    #[serde(default)]
    pub key_url: Option<String>,
    #[serde(default)]
    pub steam_app_id: Option<u32>,
    #[serde(default)]
    pub links: LinkOverrides,
    #[serde(default)]
    pub steam: Option<SteamMeta>,
    #[serde(default)]
    pub trade_id: Option<Uuid>,
    /// When the status last meaningfully changed (traded / redeemed). The CSV "Redeem/Trade Date".
    #[serde(default)]
    pub status_date: Option<Date>,
    #[serde(default)]
    pub redemption_deadline: Option<Date>,
    #[serde(default)]
    pub tags: Vec<String>,
    #[serde(default)]
    pub notes: String,
    pub created_at: Timestamp,
    pub updated_at: Timestamp,
}

impl GameKey {
    pub fn new(game_name: impl Into<String>, key: impl Into<Secret>) -> GameKey {
        let now = Timestamp::now();
        GameKey {
            id: Uuid::now_v7(),
            game_name: game_name.into(),
            key: key.into(),
            platform: Platform::default(),
            status: KeyStatus::default(),
            bundle: None,
            source: None,
            key_url: None,
            steam_app_id: None,
            links: LinkOverrides::default(),
            steam: None,
            trade_id: None,
            status_date: None,
            redemption_deadline: None,
            tags: Vec::new(),
            notes: String::new(),
            created_at: now,
            updated_at: now,
        }
    }

    pub fn steam_url(&self) -> Option<String> {
        self.links
            .steam
            .clone()
            .or_else(|| self.steam_app_id.map(links::steam_store))
    }

    pub fn gg_deals_url(&self) -> Option<String> {
        self.links
            .gg_deals
            .clone()
            .or_else(|| self.steam_app_id.map(links::gg_deals))
    }

    pub fn itad_url(&self) -> Option<String> {
        self.links
            .itad
            .clone()
            .or_else(|| self.steam_app_id.map(links::itad))
    }
}

// ── Trades ────────────────────────────────────────────────────────────────────────────────────

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Default, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum TradeStatus {
    /// The cart.
    #[default]
    Draft,
    Pending,
    Completed,
    Cancelled,
}

#[derive(Clone, Debug, PartialEq, Eq, Hash, Default, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum TradeSite {
    #[default]
    Lestrades,
    SteamTrades,
    Reddit,
    Other(String),
}

impl TradeSite {
    pub fn label(&self) -> &str {
        match self {
            TradeSite::Lestrades => "Lestrades",
            TradeSite::SteamTrades => "SteamTrades",
            TradeSite::Reddit => "Reddit",
            TradeSite::Other(s) => s,
        }
    }

    /// Guess the site from a trade URL. `None` if there is no recognisable host.
    pub fn from_url(url: &str) -> Option<TradeSite> {
        let host = links::host(url)?;
        Some(match host.as_str() {
            h if h.ends_with("lestrades.com") => TradeSite::Lestrades,
            h if h.ends_with("steamtrades.com") => TradeSite::SteamTrades,
            h if h.ends_with("reddit.com") => TradeSite::Reddit,
            h => TradeSite::Other(h.to_string()),
        })
    }
}

/// Something I received in a trade. Entered by hand (lestrades has no API).
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct ReceivedItem {
    pub game_name: String,
    #[serde(default)]
    pub key: Option<Secret>,
    #[serde(default)]
    pub platform: Platform,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Trade {
    pub id: Uuid,
    #[serde(default)]
    pub status: TradeStatus,
    #[serde(default)]
    pub site: TradeSite,
    #[serde(default)]
    pub partner_name: String,
    #[serde(default)]
    pub partner_profile_url: Option<String>,
    #[serde(default)]
    pub trade_url: Option<String>,
    /// What I gave.
    #[serde(default)]
    pub key_ids: Vec<Uuid>,
    #[serde(default)]
    pub received: Vec<ReceivedItem>,
    #[serde(default)]
    pub add_received_to_library: bool,
    pub created_at: Timestamp,
    #[serde(default)]
    pub completed_at: Option<Timestamp>,
    #[serde(default)]
    pub rep_posted: bool,
    #[serde(default)]
    pub notes: String,
}

impl Trade {
    pub fn new() -> Trade {
        Trade {
            id: Uuid::now_v7(),
            status: TradeStatus::Draft,
            site: TradeSite::default(),
            partner_name: String::new(),
            partner_profile_url: None,
            trade_url: None,
            key_ids: Vec::new(),
            received: Vec::new(),
            add_received_to_library: false,
            created_at: Timestamp::now(),
            completed_at: None,
            rep_posted: false,
            notes: String::new(),
        }
    }
}

impl Default for Trade {
    fn default() -> Self {
        Trade::new()
    }
}

// ── Settings (stored inside the vault) ────────────────────────────────────────────────────────

/// How to read ambiguous slash dates (`1/7/2026`). ISO `YYYY-MM-DD` is always accepted.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum DateFormat {
    /// `M/D/YYYY`: `1/7/2026` is 7 January.
    #[default]
    MonthDayYear,
    /// `D/M/YYYY`: `1/7/2026` is 1 July.
    DayMonthYear,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct Settings {
    pub date_format: DateFormat,
    /// Default for the "Add received games to library" checkbox when completing a trade.
    pub add_received_to_library: bool,
    /// Lock after this many idle minutes; 0 disables.
    pub auto_lock_minutes: u32,
    /// Clear a copied key from the clipboard after this many seconds; 0 disables.
    pub clipboard_clear_secs: u32,
    /// Encrypted backups kept next to the vault.
    pub backups_to_keep: u32,
    pub steam_id: Option<String>,
}

impl Default for Settings {
    fn default() -> Self {
        Settings {
            date_format: DateFormat::MonthDayYear,
            add_received_to_library: true,
            auto_lock_minutes: 10,
            clipboard_clear_secs: 30,
            backups_to_keep: 5,
            steam_id: None,
        }
    }
}

/// Third-party API keys. Kept in the vault, never in `config.toml`.
#[derive(Clone, Debug, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct ApiKeys {
    pub steam: Option<Secret>,
    pub itad: Option<Secret>,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn status_serde_names_are_stable() {
        // These strings are the on-disk format.
        let names: Vec<String> = KeyStatus::ALL
            .iter()
            .map(|s| serde_json::to_string(s).unwrap())
            .collect();
        assert_eq!(
            names,
            [
                "\"available\"",
                "\"reserved\"",
                "\"traded\"",
                "\"redeemed\"",
                "\"gifted\"",
                "\"unknown\""
            ]
        );
    }

    #[test]
    fn status_rules() {
        assert!(KeyStatus::Available.can_trade());
        assert!(KeyStatus::Unknown.can_trade());
        assert!(!KeyStatus::Reserved.can_trade());
        assert!(
            KeyStatus::Traded.is_gone()
                && KeyStatus::Redeemed.is_gone()
                && KeyStatus::Gifted.is_gone()
        );
        assert!(!KeyStatus::Unknown.is_gone());
    }

    #[test]
    fn platform_parse() {
        assert_eq!(Platform::parse(""), Platform::Steam);
        assert_eq!(Platform::parse(" Steam "), Platform::Steam);
        assert_eq!(Platform::parse("Origin"), Platform::Ea);
        assert_eq!(Platform::parse("uPlay"), Platform::Ubisoft);
        assert_eq!(Platform::parse("Battle.net"), Platform::Battlenet);
        assert_eq!(
            Platform::parse("Itch.io"),
            Platform::Other("Itch.io".into())
        );
    }

    #[test]
    fn trade_site_from_url() {
        assert_eq!(
            TradeSite::from_url("https://lestrades.com/offer/100001/"),
            Some(TradeSite::Lestrades)
        );
        assert_eq!(
            TradeSite::from_url("https://www.steamtrades.com/trade/abc"),
            Some(TradeSite::SteamTrades)
        );
        assert_eq!(
            TradeSite::from_url("https://old.reddit.com/r/SteamGameSwap"),
            Some(TradeSite::Reddit)
        );
        assert_eq!(
            TradeSite::from_url("https://example.org/x"),
            Some(TradeSite::Other("example.org".into()))
        );
        assert_eq!(
            TradeSite::from_url("https://lestrad..."),
            Some(TradeSite::Other("lestrad...".into()))
        );
        assert_eq!(TradeSite::from_url("not a url"), None);
    }

    #[test]
    fn derived_links_and_overrides() {
        let mut k = GameKey::new("Spiritfarer", "AAAAA-BBBBB-CCCCC");
        assert_eq!(k.steam_url(), None);
        k.steam_app_id = Some(972660);
        assert_eq!(
            k.steam_url().as_deref(),
            Some("https://store.steampowered.com/app/972660/")
        );
        assert_eq!(
            k.gg_deals_url().as_deref(),
            Some("https://gg.deals/steam/app/972660/")
        );
        k.links.itad = Some("https://isthereanydeal.com/game/spiritfarer/info/".into());
        assert_eq!(
            k.itad_url().as_deref(),
            Some("https://isthereanydeal.com/game/spiritfarer/info/")
        );
    }

    #[test]
    fn minimal_json_fills_defaults() {
        // Forward/backward compatibility: only required fields present.
        let json = r#"{"id":"0190f2a4-0000-7000-8000-000000000000","game_name":"Undertale","key":"UNDR7-TALE0-00001",
                       "created_at":"2026-01-01T00:00:00Z","updated_at":"2026-01-01T00:00:00Z"}"#;
        let k: GameKey = serde_json::from_str(json).unwrap();
        assert_eq!(k.status, KeyStatus::Available);
        assert_eq!(k.platform, Platform::Steam);
        assert!(k.tags.is_empty());
    }

    #[test]
    fn settings_defaults() {
        let s = Settings::default();
        assert_eq!(s.date_format, DateFormat::MonthDayYear);
        assert!(s.add_received_to_library);
        let partial: Settings = serde_json::from_str(r#"{"auto_lock_minutes":3}"#).unwrap();
        assert_eq!(partial.auto_lock_minutes, 3);
        assert_eq!(partial.backups_to_keep, 5);
    }
}
