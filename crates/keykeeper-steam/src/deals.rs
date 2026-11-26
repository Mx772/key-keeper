//! Store prices by Steam app id.
//!
//! - IsThereAnyDeal (API v2, spec: github.com/IsThereAnyDeal/API `dist/openapi.json`):
//!   `POST /lookup/id/shop/61/v1` maps `"app/<id>"` (shop 61 is Steam) to ITAD game ids, then
//!   `POST /games/prices/v3?country=XX` (≤ 200 ids) gives every store's current price, cheapest
//!   first per store, and the all-time low. The key goes in the `key` query parameter.
//! - gg.deals (docs: gg.deals/api/prices/, 2026-10): `GET api.gg.deals/v1/prices/by-steam-app-id/
//!   ?ids=…&key=…&region=xx` (≤ 100 ids) gives the cheapest retail and keyshop prices as strings,
//!   `null` for unknown ids. Limits: 100 games a minute and 1000 an hour, each id counting once
//!   (invalid requests count too); errors are `{"success": false, "data": {"message", …}}`.

use serde_json::Value;

use crate::{Result, SteamError};

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Price {
    pub cents: u64,
    pub currency: String,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ShopDeal {
    pub shop: String,
    pub price: Price,
    pub regular: Option<Price>,
    pub cut: u8,
    pub url: Option<String>,
}

/// IsThereAnyDeal's prices for one Steam app.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ItadPrices {
    pub app_id: u32,
    pub deals: Vec<ShopDeal>,
    pub history_low: Option<Price>,
}

/// gg.deals' prices for one Steam app.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct GgPrices {
    pub app_id: u32,
    pub url: Option<String>,
    pub retail: Option<Price>,
    pub keyshops: Option<Price>,
}

pub const ITAD_STEAM_SHOP: u32 = 61;
pub const ITAD_LOOKUP_BATCH: usize = 200;
pub const ITAD_PRICES_BATCH: usize = 200;
pub const GG_DEALS_BATCH: usize = 100;

/// What a gg.deals fetch got. `seen` lists every id it answered for (found or not), so ids it
/// never reached (rate limit) can be retried later; `error` says why it stopped early.
#[derive(Clone, Debug, PartialEq, Eq, Default)]
pub struct GgBatch {
    pub prices: Vec<GgPrices>,
    pub seen: Vec<u32>,
    pub error: Option<SteamError>,
}

/// A gg.deals error response (any status) as an error with their message.
pub fn gg_deals_error(status: u16, body: &str) -> SteamError {
    let msg = serde_json::from_str::<Value>(body).ok().and_then(|v| {
        v.pointer("/data/message")
            .and_then(Value::as_str)
            .map(str::to_string)
    });
    match (status, msg) {
        (401 | 403, None) => SteamError::BadServiceKey("gg.deals"),
        (_, Some(message)) => SteamError::Rejected {
            name: "gg.deals",
            message,
        },
        (s, None) => SteamError::Service {
            name: "gg.deals",
            status: s,
        },
    }
}

fn parse_err(e: impl std::fmt::Display) -> SteamError {
    SteamError::Parse(e.to_string())
}

/// Map HTTP auth failures to "key rejected" and other statuses to the service's name.
pub(crate) fn service_err(name: &'static str, e: SteamError) -> SteamError {
    match e {
        SteamError::Status(401 | 403) => SteamError::BadServiceKey(name),
        SteamError::Status(s) => SteamError::Service { name, status: s },
        e => e,
    }
}

/// `{"app/220": "uuid" | null, …}` → (app id, ITAD id) pairs.
pub fn parse_itad_lookup(body: &str) -> Result<Vec<(u32, String)>> {
    let v: Value = serde_json::from_str(body).map_err(parse_err)?;
    let obj = v
        .as_object()
        .ok_or_else(|| parse_err("lookup response is not an object"))?;
    Ok(obj
        .iter()
        .filter_map(|(k, v)| {
            let app = k.strip_prefix("app/")?.parse().ok()?;
            Some((app, v.as_str()?.to_string()))
        })
        .collect())
}

fn price(v: Option<&Value>) -> Option<Price> {
    let v = v?;
    let currency = v.get("currency")?.as_str()?.to_string();
    let cents = match v.get("amountInt").and_then(Value::as_u64) {
        Some(c) => c,
        None => (v.get("amount")?.as_f64()? * 100.0).round().max(0.0) as u64,
    };
    Some(Price { cents, currency })
}

/// One game in an ITAD prices response: its ITAD id, deals (cheapest first), all-time low.
pub type ItadGame = (String, Vec<ShopDeal>, Option<Price>);

/// ITAD prices response → per-ITAD-id deals (cheapest first) and all-time low.
pub fn parse_itad_prices(body: &str) -> Result<Vec<ItadGame>> {
    let v: Value = serde_json::from_str(body).map_err(parse_err)?;
    let items = v
        .as_array()
        .ok_or_else(|| parse_err("prices response is not a list"))?;
    let mut out = Vec::new();
    for it in items {
        let Some(id) = it.get("id").and_then(Value::as_str) else {
            continue;
        };
        let mut deals: Vec<ShopDeal> = it
            .get("deals")
            .and_then(Value::as_array)
            .map(|ds| {
                ds.iter()
                    .filter_map(|d| {
                        Some(ShopDeal {
                            shop: d.pointer("/shop/name")?.as_str()?.to_string(),
                            price: price(d.get("price"))?,
                            regular: price(d.get("regular")),
                            cut: d
                                .get("cut")
                                .and_then(Value::as_u64)
                                .map_or(0, |c| c.min(100) as u8),
                            url: d.get("url").and_then(Value::as_str).map(str::to_string),
                        })
                    })
                    .collect()
            })
            .unwrap_or_default();
        deals.sort_by_key(|d| d.price.cents);
        // One entry per store: the cheapest.
        let mut seen = std::collections::HashSet::new();
        deals.retain(|d| seen.insert(d.shop.clone()));
        out.push((id.to_string(), deals, price(it.pointer("/historyLow/all"))));
    }
    Ok(out)
}

/// A gg.deals price: `"9.99"`, `9.99`, or null.
fn gg_price(v: Option<&Value>, currency: &str) -> Option<Price> {
    let v = v?;
    let amount = match v {
        Value::String(s) => s.trim().parse::<f64>().ok()?,
        Value::Number(n) => n.as_f64()?,
        _ => return None,
    };
    (amount >= 0.0).then(|| Price {
        cents: (amount * 100.0).round() as u64,
        currency: currency.to_string(),
    })
}

/// `{"success": true, "data": {"<appid>": {"url", "prices": {"currentRetail", "currentKeyshops",
/// "currency", …}} | null}}`. An unsuccessful response's `data.message` becomes the error.
/// Returns the prices found and every id answered for.
pub fn parse_gg_deals(body: &str) -> Result<(Vec<GgPrices>, Vec<u32>)> {
    let v: Value = serde_json::from_str(body).map_err(parse_err)?;
    if v.get("success").and_then(Value::as_bool) == Some(false) {
        return Err(gg_deals_error(200, body));
    }
    let data = v
        .get("data")
        .and_then(Value::as_object)
        .ok_or_else(|| parse_err("gg.deals response has no data"))?;
    let seen = data.keys().filter_map(|k| k.parse().ok()).collect();
    let prices = data
        .iter()
        .filter_map(|(k, g)| {
            let app_id = k.parse().ok()?;
            let prices = g.get("prices")?;
            let currency = prices
                .get("currency")
                .and_then(Value::as_str)
                .unwrap_or("USD");
            Some(GgPrices {
                app_id,
                url: g.get("url").and_then(Value::as_str).map(str::to_string),
                retail: gg_price(prices.get("currentRetail"), currency),
                keyshops: gg_price(prices.get("currentKeyshops"), currency),
            })
        })
        .collect();
    Ok((prices, seen))
}

#[cfg(test)]
mod tests {
    use super::*;

    // From the ITAD OpenAPI spec's examples (github.com/IsThereAnyDeal/API, 2026-11).
    const LOOKUP: &str =
        r#"{"app/220":"018d937f-012f-73b8-ab2c-898516969e6a","app/137730":null,"sub/1":"x"}"#;
    const PRICES: &str = r#"[{"id":"018d937f-012f-73b8-ab2c-898516969e6a","historyLow":{"all":{"amount":0.99,"amountInt":99,"currency":"USD"},"y1":null,"m3":null},"deals":[
        {"shop":{"id":61,"name":"Steam"},"price":{"amount":9.99,"amountInt":999,"currency":"USD"},"regular":{"amount":9.99,"amountInt":999,"currency":"USD"},"cut":0,"voucher":null,"storeLow":null,"flag":null,"drm":[],"platforms":[],"timestamp":"2024-02-11T01:47:46+01:00","expiry":null,"url":"https://itad.link/a/"},
        {"shop":{"id":35,"name":"GOG"},"price":{"amount":4.99,"amountInt":499,"currency":"USD"},"regular":{"amount":9.99,"amountInt":999,"currency":"USD"},"cut":50,"voucher":null,"storeLow":null,"flag":null,"drm":[],"platforms":[],"timestamp":"2024-02-11T01:47:46+01:00","expiry":null,"url":"https://itad.link/b/"}]}]"#;
    const GG: &str = r#"{"success":true,"data":{"400":{"title":"Portal","url":"https://gg.deals/game/portal/","prices":{"currentRetail":"9.99","currentKeyshops":"1.23","historicalRetail":"0.79","historicalKeyshops":"0.50","currency":"USD"}},"1":null,"420":{"title":"x","url":null,"prices":{"currentRetail":null,"currentKeyshops":2,"currency":"EUR"}}}}"#;

    #[test]
    fn itad_parsers() {
        assert_eq!(
            parse_itad_lookup(LOOKUP).unwrap(),
            vec![(220, "018d937f-012f-73b8-ab2c-898516969e6a".to_string())]
        );
        let p = parse_itad_prices(PRICES).unwrap();
        assert_eq!(p.len(), 1);
        let (id, deals, low) = &p[0];
        assert_eq!(id, "018d937f-012f-73b8-ab2c-898516969e6a");
        assert_eq!(deals[0].shop, "GOG", "cheapest first");
        assert_eq!(deals[0].cut, 50);
        assert_eq!(deals[1].price.cents, 999);
        assert_eq!(low.as_ref().map(|l| l.cents), Some(99));
    }

    #[test]
    fn gg_deals_parser_is_lenient() {
        let (mut g, mut seen) = parse_gg_deals(GG).unwrap();
        seen.sort_unstable();
        assert_eq!(
            seen,
            vec![1, 400, 420],
            "a null entry still counts as answered"
        );
        g.sort_by_key(|x| x.app_id);
        assert_eq!(g.len(), 2, "null entries are skipped");
        assert_eq!(g[0].app_id, 400);
        assert_eq!(g[0].retail.as_ref().map(|p| p.cents), Some(999));
        assert_eq!(g[0].keyshops.as_ref().map(|p| p.cents), Some(123));
        assert_eq!(g[1].retail, None);
        assert_eq!(
            g[1].keyshops,
            Some(Price {
                cents: 200,
                currency: "EUR".into()
            })
        );
        let err = parse_gg_deals(r#"{"success":false,"data":{"message":"Invalid key"}}"#);
        assert_eq!(
            err,
            Err(SteamError::Rejected {
                name: "gg.deals",
                message: "Invalid key".into()
            })
        );
    }

    #[test]
    fn gg_deals_errors_keep_their_message() {
        // The documented 429 body.
        let body = r#"{"success":false,"data":{"name":"Too Many Requests","message":"You can fetch prices info only for 100 games per minute. Try again later.","code":429,"status":429}}"#;
        assert_eq!(
            gg_deals_error(429, body),
            SteamError::Rejected {
                name: "gg.deals",
                message:
                    "You can fetch prices info only for 100 games per minute. Try again later."
                        .into()
            }
        );
        assert_eq!(
            gg_deals_error(403, ""),
            SteamError::BadServiceKey("gg.deals")
        );
        assert_eq!(
            gg_deals_error(400, "<html>"),
            SteamError::Service {
                name: "gg.deals",
                status: 400
            }
        );
    }
}
