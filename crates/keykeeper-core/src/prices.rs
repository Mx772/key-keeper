//! Store prices per Steam app id, from IsThereAnyDeal (per-store deals) and gg.deals (retail and
//! keyshop lows). Stored in the vault as a cache keyed by app id, so keys of the same game share it.
//! Serde names are part of the file format.

use jiff::Timestamp;
use serde::{Deserialize, Serialize};

/// An amount in minor units (cents) and its ISO 4217 currency.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Money {
    pub cents: u64,
    pub currency: String,
}

impl Money {
    pub fn new(cents: u64, currency: impl Into<String>) -> Money {
        Money {
            cents,
            currency: currency.into(),
        }
    }

    /// `$9.99`, `€9.99`, `£9.99`, or `9.99 PLN` for currencies without a common symbol.
    pub fn format(&self) -> String {
        let n = format!("{}.{:02}", self.cents / 100, self.cents % 100);
        match self.currency.as_str() {
            "USD" => format!("${n}"),
            "EUR" => format!("€{n}"),
            "GBP" => format!("£{n}"),
            "CAD" => format!("CA${n}"),
            "AUD" => format!("A${n}"),
            "BRL" => format!("R${n}"),
            "JPY" => format!("¥{}", self.cents / 100),
            c => format!("{n} {c}"),
        }
    }
}

/// One store's current offer.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct ShopPrice {
    pub shop: String,
    pub price: Money,
    #[serde(default)]
    pub regular: Option<Money>,
    /// Percent off the regular price.
    #[serde(default)]
    pub cut: u8,
    #[serde(default)]
    pub url: Option<String>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct GamePrices {
    /// Current offers, cheapest first (IsThereAnyDeal).
    #[serde(default)]
    pub shops: Vec<ShopPrice>,
    /// All-time low across stores (IsThereAnyDeal).
    #[serde(default)]
    pub history_low: Option<Money>,
    /// gg.deals: cheapest official store and cheapest keyshop right now, and the game's page.
    #[serde(default)]
    pub retail_low: Option<Money>,
    #[serde(default)]
    pub keyshop_low: Option<Money>,
    #[serde(default)]
    pub gg_deals_url: Option<String>,
    /// The country the prices are for (e.g. "US").
    pub country: String,
    pub fetched_at: Timestamp,
}

impl GamePrices {
    pub fn empty(country: impl Into<String>, fetched_at: Timestamp) -> GamePrices {
        GamePrices {
            shops: Vec::new(),
            history_low: None,
            retail_low: None,
            keyshop_low: None,
            gg_deals_url: None,
            country: country.into(),
            fetched_at,
        }
    }

    /// The cheapest current store offer: ITAD's best deal, else gg.deals' retail low.
    pub fn best(&self) -> Option<(&Money, Option<&str>)> {
        match self.shops.first() {
            Some(s) => Some((&s.price, Some(s.shop.as_str()))),
            None => self.retail_low.as_ref().map(|m| (m, None)),
        }
    }

    /// Steam's own current offer, when IsThereAnyDeal lists it.
    pub fn steam(&self) -> Option<&ShopPrice> {
        self.shops
            .iter()
            .find(|s| s.shop.eq_ignore_ascii_case("steam"))
    }

    /// Current offers from official stores other than Steam, cheapest first.
    pub fn other_shops(&self) -> impl Iterator<Item = &ShopPrice> {
        self.shops
            .iter()
            .filter(|s| !s.shop.eq_ignore_ascii_case("steam"))
    }

    /// Sort key for the library's Deals column: the best current price in cents.
    pub fn best_cents(&self) -> Option<u64> {
        self.best().map(|(m, _)| m.cents)
    }

    pub fn sort_shops(&mut self) {
        self.shops.sort_by(|a, b| {
            a.price
                .cents
                .cmp(&b.price.cents)
                .then_with(|| a.shop.cmp(&b.shop))
        });
    }
}

/// gg.deals takes a lowercase region; these are the ones it serves. Others fall back to "us".
pub const GG_DEALS_REGIONS: [&str; 19] = [
    "au", "be", "br", "ca", "ch", "de", "dk", "es", "eu", "fi", "fr", "gb", "ie", "it", "nl", "no",
    "pl", "se", "us",
];

/// Euro countries gg.deals doesn't list by name use its "eu" region.
const EURO: [&str; 9] = ["AT", "PT", "GR", "SK", "SI", "LU", "LT", "LV", "EE"];

/// The gg.deals region for an ISO country code.
pub fn gg_deals_region(country: &str) -> &'static str {
    let c = country.trim().to_ascii_uppercase();
    if c == "UK" {
        return "gb";
    }
    if EURO.contains(&c.as_str()) {
        return "eu";
    }
    let lower = c.to_ascii_lowercase();
    GG_DEALS_REGIONS
        .iter()
        .find(|r| **r == lower)
        .copied()
        .unwrap_or("us")
}

/// A two-letter country code, uppercased; `None` if it isn't one.
pub fn normalize_country(s: &str) -> Option<String> {
    let t = s.trim();
    (t.len() == 2 && t.chars().all(|c| c.is_ascii_alphabetic())).then(|| t.to_ascii_uppercase())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn money_formats() {
        assert_eq!(Money::new(999, "USD").format(), "$9.99");
        assert_eq!(Money::new(5, "EUR").format(), "€0.05");
        assert_eq!(Money::new(12345, "PLN").format(), "123.45 PLN");
    }

    #[test]
    fn regions() {
        assert_eq!(gg_deals_region("US"), "us");
        assert_eq!(gg_deals_region("uk"), "gb");
        assert_eq!(gg_deals_region("AT"), "eu");
        assert_eq!(gg_deals_region("DE"), "de");
        assert_eq!(gg_deals_region("JP"), "us");
        assert_eq!(normalize_country(" gb "), Some("GB".into()));
        assert_eq!(normalize_country("USA"), None);
    }

    #[test]
    fn best_prefers_itad_then_gg() {
        let mut p = GamePrices::empty("US", Timestamp::UNIX_EPOCH);
        assert_eq!(p.best_cents(), None);
        p.retail_low = Some(Money::new(700, "USD"));
        assert_eq!(p.best(), Some((&Money::new(700, "USD"), None)));
        p.shops = vec![
            ShopPrice {
                shop: "Steam".into(),
                price: Money::new(999, "USD"),
                regular: None,
                cut: 0,
                url: None,
            },
            ShopPrice {
                shop: "Fanatical".into(),
                price: Money::new(499, "USD"),
                regular: None,
                cut: 50,
                url: None,
            },
        ];
        p.sort_shops();
        assert_eq!(p.best(), Some((&Money::new(499, "USD"), Some("Fanatical"))));
        assert_eq!(p.steam().map(|s| s.price.cents), Some(999));
        assert_eq!(
            p.other_shops().map(|s| s.shop.as_str()).collect::<Vec<_>>(),
            ["Fanatical"]
        );
    }
}
