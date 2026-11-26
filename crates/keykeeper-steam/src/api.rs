//! Steam endpoints: request building and response parsing.

use serde::Deserialize;
use serde_json::Value;

use crate::deals::{
    self, GG_DEALS_BATCH, GgBatch, ITAD_LOOKUP_BATCH, ITAD_PRICES_BATCH, ITAD_STEAM_SHOP,
    ItadBatch, ItadPrices, service_err,
};
use crate::details::{StoreDetails, parse_store_details, parse_tag_list};
use crate::http::Http;
use crate::{Partial, Progress, Result, SteamError};

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct AppMatch {
    pub app_id: u32,
    pub name: String,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Review {
    pub app_id: u32,
    /// Steam's label, e.g. "Very Positive".
    pub desc: String,
    pub percent: u8,
    pub total: u32,
}

/// What the user typed for "my Steam account".
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ProfileRef {
    Id(u64),
    Vanity(String),
}

/// Accepts a SteamID64, `steamcommunity.com/profiles/<id>`, `steamcommunity.com/id/<name>`,
/// or a bare custom-URL name.
pub fn parse_profile_input(input: &str) -> Option<ProfileRef> {
    let t = input.trim().trim_end_matches('/');
    if t.is_empty() {
        return None;
    }
    let after = |marker: &str| {
        t.split(marker)
            .nth(1)
            .map(|r| r.split(['/', '?', '#']).next().unwrap_or("").to_string())
    };
    if let Some(id) = after("/profiles/") {
        return id.parse().ok().map(ProfileRef::Id);
    }
    if let Some(name) = after("/id/") {
        return (!name.is_empty()).then_some(ProfileRef::Vanity(name));
    }
    if t.len() == 17 && t.starts_with("7656") && t.chars().all(|c| c.is_ascii_digit()) {
        return t.parse().ok().map(ProfileRef::Id);
    }
    if t.contains(['/', ' ', '.']) {
        return None;
    }
    Some(ProfileRef::Vanity(t.to_string()))
}

pub trait SteamApi: Send + Sync {
    fn search(&self, term: &str) -> Result<Vec<AppMatch>>;
    /// Review summaries for many apps (batched internally). Apps Steam doesn't know are omitted.
    /// A batch failing after earlier ones succeeded keeps what they got (see [`Partial`]).
    fn reviews(&self, app_ids: &[u32], progress: &dyn Progress) -> Result<Partial<Review>>;
    fn resolve_profile(&self, input: &str) -> Result<u64>;
    /// App ids on a public wishlist (empty if private or empty).
    fn wishlist(&self, steam_id: u64) -> Result<Vec<u32>>;
    fn owned_games(&self, api_key: &str, steam_id: u64) -> Result<Vec<u32>>;
    /// IsThereAnyDeal prices for Steam apps in `country` (two letters). Apps ITAD doesn't know
    /// are omitted from `prices` but listed in `seen`.
    fn itad_prices(
        &self,
        _api_key: &str,
        _country: &str,
        _app_ids: &[u32],
        _progress: &dyn Progress,
    ) -> Result<ItadBatch> {
        Err(SteamError::Unsupported("IsThereAnyDeal"))
    }
    /// gg.deals prices for Steam apps in `region` (gg.deals' lowercase region code).
    fn gg_deals_prices(
        &self,
        _api_key: &str,
        _region: &str,
        _app_ids: &[u32],
        _progress: &dyn Progress,
    ) -> Result<GgBatch> {
        Err(SteamError::Unsupported("gg.deals"))
    }
    /// Store details for many apps (batched internally), each with its top tags' names.
    /// Apps Steam doesn't know are omitted. A failing batch keeps what earlier ones got.
    fn store_details(&self, _app_ids: &[u32]) -> Result<Partial<(StoreDetails, Vec<String>)>> {
        Err(SteamError::Unsupported("store details"))
    }
    /// An image from Steam's CDN (https, `*.steamstatic.com` only).
    fn download_image(&self, _url: &str) -> Result<Vec<u8>> {
        Err(SteamError::Unsupported("downloads"))
    }
}

/// Images bigger than this are refused.
pub const IMAGE_LIMIT: usize = 4 * 1024 * 1024;

/// Whether `url` is an https URL on Steam's image CDN.
pub fn is_steam_cdn(url: &str) -> bool {
    let Some(rest) = url.strip_prefix("https://") else {
        return false;
    };
    let host = rest.split(['/', '?', '#']).next().unwrap_or("");
    !host.contains(['@', ':']) && (host == "steamstatic.com" || host.ends_with(".steamstatic.com"))
}

/// gg.deals allows 100 games a minute: wait this long between batches of 100.
pub const GG_DEALS_PAUSE: std::time::Duration = std::time::Duration::from_secs(61);

pub struct Steam<H: Http> {
    http: H,
    gg_pause: std::time::Duration,
    /// Tag id to name, fetched once.
    tags: std::sync::Mutex<Option<std::collections::HashMap<u32, String>>>,
}

impl<H: Http> Steam<H> {
    pub fn new(http: H) -> Steam<H> {
        Steam {
            http,
            gg_pause: GG_DEALS_PAUSE,
            tags: std::sync::Mutex::new(None),
        }
    }

    /// Change the pause between gg.deals batches (tests use zero).
    pub fn with_gg_pause(mut self, pause: std::time::Duration) -> Steam<H> {
        self.gg_pause = pause;
        self
    }
}

const REVIEW_BATCH: usize = 50;

impl<H: Http> SteamApi for Steam<H> {
    fn search(&self, term: &str) -> Result<Vec<AppMatch>> {
        let body = self.http.get(
            "https://store.steampowered.com/api/storesearch/",
            &[("term", term), ("cc", "us"), ("l", "en")],
        )?;
        parse_search(&body)
    }

    fn reviews(&self, app_ids: &[u32], progress: &dyn Progress) -> Result<Partial<Review>> {
        let mut out = Partial::default();
        for chunk in app_ids.chunks(REVIEW_BATCH) {
            let ids: Vec<Value> = chunk
                .iter()
                .map(|id| serde_json::json!({ "appid": id }))
                .collect();
            let input = serde_json::json!({
                "ids": ids,
                "context": { "language": "english", "country_code": "US" },
                "data_request": { "include_reviews": true }
            })
            .to_string();
            let step = self
                .http
                .get(
                    "https://api.steampowered.com/IStoreBrowseService/GetItems/v1/",
                    &[("input_json", &input)],
                )
                .and_then(|body| parse_reviews(&body));
            match step {
                Ok(r) => out.items.extend(r),
                Err(e) => {
                    log::warn!("reviews: stopped after {} apps: {e}", out.items.len());
                    if out.items.is_empty() {
                        return Err(e);
                    }
                    out.error = Some(e);
                    break;
                }
            }
            progress.advance(chunk.len());
        }
        Ok(out)
    }

    fn resolve_profile(&self, input: &str) -> Result<u64> {
        match parse_profile_input(input) {
            Some(ProfileRef::Id(id)) => Ok(id),
            Some(ProfileRef::Vanity(name)) => {
                let body = self.http.get(
                    &format!("https://steamcommunity.com/id/{name}/"),
                    &[("xml", "1")],
                )?;
                parse_profile_xml(&body).ok_or(SteamError::ProfileNotFound(name))
            }
            None => Err(SteamError::ProfileNotFound(input.trim().to_string())),
        }
    }

    fn wishlist(&self, steam_id: u64) -> Result<Vec<u32>> {
        let id = steam_id.to_string();
        let body = self.http.get(
            "https://api.steampowered.com/IWishlistService/GetWishlist/v1/",
            &[("steamid", &id)],
        )?;
        parse_wishlist(&body)
    }

    fn owned_games(&self, api_key: &str, steam_id: u64) -> Result<Vec<u32>> {
        let id = steam_id.to_string();
        let res = self.http.get(
            "https://api.steampowered.com/IPlayerService/GetOwnedGames/v1/",
            &[
                ("key", api_key),
                ("steamid", &id),
                ("include_played_free_games", "1"),
            ],
        );
        match res {
            Err(SteamError::Status(401 | 403)) => Err(SteamError::BadApiKey),
            r => parse_owned(&r?),
        }
    }

    fn itad_prices(
        &self,
        api_key: &str,
        country: &str,
        app_ids: &[u32],
        progress: &dyn Progress,
    ) -> Result<ItadBatch> {
        let itad = |e| service_err("IsThereAnyDeal", e);
        let mut out = ItadBatch::default();
        // Stop at the first failure; what's done is kept and the rest is retried next time.
        let stop = |out: &mut ItadBatch, e: SteamError| -> Result<()> {
            log::warn!("IsThereAnyDeal: stopped after {} apps: {e}", out.seen.len());
            if out.seen.is_empty() {
                return Err(e);
            }
            out.error = Some(e);
            Ok(())
        };
        progress.note("IsThereAnyDeal: looking up games");
        let mut ids: Vec<(u32, String)> = Vec::new();
        for chunk in app_ids.chunks(ITAD_LOOKUP_BATCH) {
            let body: Vec<String> = chunk.iter().map(|a| format!("app/{a}")).collect();
            let body =
                serde_json::to_string(&body).map_err(|e| SteamError::Parse(e.to_string()))?;
            let step = self
                .http
                .post_json(
                    &format!("https://api.isthereanydeal.com/lookup/id/shop/{ITAD_STEAM_SHOP}/v1"),
                    &[("key", api_key)],
                    &body,
                )
                .map_err(itad)
                .and_then(|res| deals::parse_itad_lookup(&res));
            match step {
                Ok(found) => {
                    // Apps ITAD doesn't know are done: there's nothing to price.
                    let unknown: Vec<u32> = chunk
                        .iter()
                        .copied()
                        .filter(|a| !found.iter().any(|(f, _)| f == a))
                        .collect();
                    progress.advance(unknown.len());
                    out.seen.extend(unknown);
                    ids.extend(found);
                }
                Err(e) => {
                    // Nothing looked up in this batch: price what earlier batches found.
                    if ids.is_empty() {
                        stop(&mut out, e)?;
                    } else {
                        out.error = Some(e);
                    }
                    break;
                }
            }
        }
        progress.note("IsThereAnyDeal: fetching prices");
        for chunk in ids.chunks(ITAD_PRICES_BATCH) {
            let body: Vec<&str> = chunk.iter().map(|(_, g)| g.as_str()).collect();
            let body =
                serde_json::to_string(&body).map_err(|e| SteamError::Parse(e.to_string()))?;
            let step = self
                .http
                .post_json(
                    "https://api.isthereanydeal.com/games/prices/v3",
                    &[("key", api_key), ("country", country)],
                    &body,
                )
                .map_err(itad)
                .and_then(|res| deals::parse_itad_prices(&res));
            let games = match step {
                Ok(g) => g,
                Err(e) => {
                    stop(&mut out, e)?;
                    break;
                }
            };
            for (gid, deals, history_low) in games {
                for (app_id, _) in chunk.iter().filter(|(_, g)| *g == gid) {
                    out.prices.push(ItadPrices {
                        app_id: *app_id,
                        deals: deals.clone(),
                        history_low: history_low.clone(),
                    });
                }
            }
            out.seen.extend(chunk.iter().map(|(a, _)| *a));
            progress.advance(chunk.len());
        }
        Ok(out)
    }

    fn store_details(&self, app_ids: &[u32]) -> Result<Partial<(StoreDetails, Vec<String>)>> {
        let mut found = Vec::new();
        let mut error = None;
        for chunk in app_ids.chunks(REVIEW_BATCH) {
            let ids: Vec<Value> = chunk
                .iter()
                .map(|id| serde_json::json!({ "appid": id }))
                .collect();
            let input = serde_json::json!({
                "ids": ids,
                "context": { "language": "english", "country_code": "US" },
                "data_request": {
                    "include_basic_info": true,
                    "include_release": true,
                    "include_platforms": true,
                    "include_assets": true,
                    "include_tag_count": crate::details::TAGS_KEPT + 2
                }
            })
            .to_string();
            let step = self
                .http
                .get(
                    "https://api.steampowered.com/IStoreBrowseService/GetItems/v1/",
                    &[("input_json", &input)],
                )
                .and_then(|body| parse_store_details(&body));
            match step {
                Ok(d) => found.extend(d),
                Err(e) => {
                    log::warn!("store details: stopped after {} apps: {e}", found.len());
                    if found.is_empty() {
                        return Err(e);
                    }
                    error = Some(e);
                    break;
                }
            }
        }
        let need_tags = found.iter().any(|d| !d.tag_ids.is_empty());
        let mut names = self
            .tags
            .lock()
            .map_err(|_| SteamError::Parse("tag cache".into()))?;
        if need_tags && names.is_none() {
            // Tags are a nicety: without names the details still show.
            if let Ok(body) = self.http.get(
                "https://api.steampowered.com/IStoreService/GetTagList/v1/",
                &[("language", "english")],
            ) {
                *names = parse_tag_list(&body).ok();
            }
        }
        let items = found
            .into_iter()
            .map(|d| {
                let tags = d
                    .tag_ids
                    .iter()
                    .filter_map(|id| names.as_ref()?.get(id).cloned())
                    .collect();
                (d, tags)
            })
            .collect();
        Ok(Partial { items, error })
    }

    fn download_image(&self, url: &str) -> Result<Vec<u8>> {
        if !is_steam_cdn(url) {
            return Err(SteamError::Unsupported("images from outside Steam's CDN"));
        }
        self.http.get_bytes(url, IMAGE_LIMIT)
    }

    fn gg_deals_prices(
        &self,
        api_key: &str,
        region: &str,
        app_ids: &[u32],
        progress: &dyn Progress,
    ) -> Result<GgBatch> {
        let mut out = GgBatch::default();
        for (i, chunk) in app_ids.chunks(GG_DEALS_BATCH).enumerate() {
            if i > 0 {
                // gg.deals allows 100 games a minute.
                progress.note(&format!(
                    "gg.deals: waiting {} s (100 games a minute)",
                    self.gg_pause.as_secs()
                ));
                log::info!(
                    "gg.deals: {} of {} done, pausing {} s for its rate limit",
                    out.seen.len(),
                    app_ids.len(),
                    self.gg_pause.as_secs()
                );
                std::thread::sleep(self.gg_pause);
            }
            progress.note("gg.deals: fetching prices");
            let ids: Vec<String> = chunk.iter().map(u32::to_string).collect();
            let ids = ids.join(",");
            let step = self
                .http
                .get_with_status(
                    "https://api.gg.deals/v1/prices/by-steam-app-id/",
                    &[("ids", &ids), ("key", api_key), ("region", region)],
                )
                .and_then(|(status, body)| match status {
                    200..=299 => deals::parse_gg_deals(&body),
                    s => Err(deals::gg_deals_error(s, &body)),
                });
            match step {
                Ok((prices, seen)) => {
                    out.prices.extend(prices);
                    out.seen.extend(seen);
                    progress.advance(chunk.len());
                }
                // Nothing fetched yet: the whole request failed.
                Err(e) if out.seen.is_empty() => return Err(e),
                // Keep what earlier batches got; the rest is retried next time.
                Err(e) => {
                    log::warn!("gg.deals: stopped after {} apps: {e}", out.seen.len());
                    out.error = Some(e);
                    break;
                }
            }
        }
        Ok(out)
    }
}

fn parse_err(e: impl std::fmt::Display) -> SteamError {
    SteamError::Parse(e.to_string())
}

pub fn parse_search(body: &str) -> Result<Vec<AppMatch>> {
    #[derive(Deserialize)]
    struct Item {
        #[serde(default, rename = "type")]
        kind: String,
        name: String,
        id: u32,
    }
    #[derive(Deserialize)]
    struct Resp {
        #[serde(default)]
        items: Vec<Item>,
    }
    let r: Resp = serde_json::from_str(body).map_err(parse_err)?;
    Ok(r.items
        .into_iter()
        .filter(|i| i.kind.is_empty() || i.kind == "app")
        .map(|i| AppMatch {
            app_id: i.id,
            name: i.name,
        })
        .collect())
}

pub fn parse_reviews(body: &str) -> Result<Vec<Review>> {
    let v: Value = serde_json::from_str(body).map_err(parse_err)?;
    let items = v
        .pointer("/response/store_items")
        .and_then(Value::as_array)
        .cloned()
        .unwrap_or_default();
    let mut out = Vec::new();
    for it in items {
        let app_id = it
            .get("appid")
            .or_else(|| it.get("id"))
            .and_then(Value::as_u64);
        let s = it.pointer("/reviews/summary_filtered");
        let (Some(app_id), Some(s)) = (app_id, s) else {
            continue;
        };
        let Ok(app_id) = u32::try_from(app_id) else {
            continue;
        };
        let total = s.get("review_count").and_then(Value::as_u64).unwrap_or(0);
        if total == 0 {
            continue;
        }
        out.push(Review {
            app_id,
            desc: s
                .get("review_score_label")
                .and_then(Value::as_str)
                .unwrap_or("")
                .to_string(),
            percent: s
                .get("percent_positive")
                .and_then(Value::as_u64)
                .map_or(0, |p| p.min(100) as u8),
            total: u32::try_from(total).unwrap_or(u32::MAX),
        });
    }
    Ok(out)
}

pub fn parse_profile_xml(body: &str) -> Option<u64> {
    let start = body.find("<steamID64>")? + "<steamID64>".len();
    let end = start + body[start..].find("</steamID64>")?;
    body[start..end].trim().parse().ok()
}

pub fn parse_wishlist(body: &str) -> Result<Vec<u32>> {
    let v: Value = serde_json::from_str(body).map_err(parse_err)?;
    let items = v
        .pointer("/response/items")
        .and_then(Value::as_array)
        .cloned()
        .unwrap_or_default();
    Ok(items
        .iter()
        .filter_map(|i| i.get("appid").and_then(Value::as_u64))
        .filter_map(|a| u32::try_from(a).ok())
        .collect())
}

pub fn parse_owned(body: &str) -> Result<Vec<u32>> {
    let v: Value = serde_json::from_str(body).map_err(parse_err)?;
    let games = v
        .pointer("/response/games")
        .and_then(Value::as_array)
        .cloned()
        .unwrap_or_default();
    Ok(games
        .iter()
        .filter_map(|g| g.get("appid").and_then(Value::as_u64))
        .filter_map(|a| u32::try_from(a).ok())
        .collect())
}

#[cfg(test)]
mod tests {
    use std::sync::Mutex;

    use super::*;
    use crate::NoProgress;

    // Recorded from the live endpoints on 2026-11-25 (trimmed).
    const SEARCH: &str = r#"{"total":1,"items":[{"type":"app","name":"Haiku, the Robot","id":1231880,"price":{"currency":"USD","initial":1999,"final":679},"tiny_image":"https:\/\/shared.akamai.steamstatic.com\/x.jpg","metascore":"","platforms":{"windows":true,"mac":true,"linux":false},"streamingvideo":false,"controller_support":"full"}]}"#;
    const REVIEWS: &str = r#"{"response":{"store_items":[{"item_type":0,"id":1231880,"success":1,"visible":true,"name":"Haiku, the Robot","appid":1231880,"type":0,"reviews":{"summary_filtered":{"review_count":2602,"percent_positive":90,"review_score":8,"review_score_label":"Very Positive"},"summary_language_specific":{"review_count":1638,"percent_positive":91,"review_score":8,"review_score_label":"Very Positive"}}},{"item_type":0,"id":1,"success":2}]}}"#;
    const PROFILE: &str = "<?xml version=\"1.0\" encoding=\"UTF-8\" standalone=\"yes\"?><profile>\n\t<steamID64>76561197960287930</steamID64>\n\t<steamID><![CDATA[Rabscuttle]]></steamID>";
    const WISHLIST: &str = r#"{"response":{"items":[{"appid":1145360,"priority":1,"date_added":1700000000},{"appid":632470,"priority":2,"date_added":1700000001}]}}"#;
    const WISHLIST_PRIVATE: &str = r#"{"response":{}}"#;
    const OWNED: &str = r#"{"response":{"game_count":2,"games":[{"appid":391540,"playtime_forever":10},{"appid":367520,"playtime_forever":0}]}}"#;

    #[test]
    fn parsers() {
        assert_eq!(
            parse_search(SEARCH).unwrap(),
            vec![AppMatch {
                app_id: 1231880,
                name: "Haiku, the Robot".into()
            }]
        );
        assert_eq!(
            parse_reviews(REVIEWS).unwrap(),
            vec![Review {
                app_id: 1231880,
                desc: "Very Positive".into(),
                percent: 90,
                total: 2602
            }]
        );
        assert_eq!(parse_profile_xml(PROFILE), Some(76561197960287930));
        assert_eq!(parse_profile_xml("<html>not found</html>"), None);
        assert_eq!(parse_wishlist(WISHLIST).unwrap(), vec![1145360, 632470]);
        assert!(parse_wishlist(WISHLIST_PRIVATE).unwrap().is_empty());
        assert_eq!(parse_owned(OWNED).unwrap(), vec![391540, 367520]);
        assert!(matches!(parse_search("<html>"), Err(SteamError::Parse(_))));
    }

    #[test]
    fn profile_inputs() {
        assert_eq!(
            parse_profile_input("76561197960287930"),
            Some(ProfileRef::Id(76561197960287930))
        );
        assert_eq!(
            parse_profile_input("https://steamcommunity.com/profiles/76561197960287930/"),
            Some(ProfileRef::Id(76561197960287930))
        );
        assert_eq!(
            parse_profile_input("https://steamcommunity.com/id/gabelogannewell"),
            Some(ProfileRef::Vanity("gabelogannewell".into()))
        );
        assert_eq!(
            parse_profile_input(" gabelogannewell "),
            Some(ProfileRef::Vanity("gabelogannewell".into()))
        );
        assert_eq!(parse_profile_input(""), None);
        assert_eq!(parse_profile_input("https://example.com/foo"), None);
        assert_eq!(
            parse_profile_input("https://steamcommunity.com/profiles/abc"),
            None
        );
    }

    /// A recorded request: URL and query pairs.
    type Call = (String, Vec<(String, String)>);

    /// Records requests and answers from a function.
    struct Fake {
        calls: Mutex<Vec<Call>>,
        answer: fn(&str) -> Result<String>,
    }

    impl Http for Fake {
        fn get(&self, url: &str, query: &[(&str, &str)]) -> Result<String> {
            if let Ok(mut c) = self.calls.lock() {
                c.push((
                    url.to_string(),
                    query
                        .iter()
                        .map(|(k, v)| (k.to_string(), v.to_string()))
                        .collect(),
                ));
            }
            (self.answer)(url)
        }
        fn post_json(&self, url: &str, query: &[(&str, &str)], body: &str) -> Result<String> {
            let mut q = query.to_vec();
            q.push(("body", body));
            self.get(url, &q)
        }
    }

    fn fake(answer: fn(&str) -> Result<String>) -> Steam<Fake> {
        Steam::new(Fake {
            calls: Mutex::new(Vec::new()),
            answer,
        })
        .with_gg_pause(std::time::Duration::ZERO)
    }

    #[test]
    fn store_details_names_tags_once_and_images_stay_on_steams_cdn() {
        let s = fake(|url| {
            Ok(if url.contains("GetTagList") {
                r#"{"response":{"tags":[{"tagid":1628,"name":"Metroidvania"},{"tagid":21,"name":"Adventure"}]}}"#.into()
            } else {
                r#"{"response":{"store_items":[{"id":5,"appid":5,"success":1,"name":"Five","tags":[{"tagid":21,"weight":9},{"tagid":1628,"weight":10},{"tagid":77,"weight":1}]}]}}"#.into()
            })
        });
        let got = s.store_details(&[5]).unwrap().items;
        assert_eq!(got.len(), 1);
        assert_eq!(got[0].0.app_id, 5);
        assert_eq!(
            got[0].1,
            vec!["Metroidvania".to_string(), "Adventure".to_string()]
        );
        s.store_details(&[5]).unwrap();
        let calls = s.http.calls.lock().unwrap();
        assert_eq!(
            calls.iter().filter(|c| c.0.contains("GetTagList")).count(),
            1,
            "tag names are fetched once"
        );
        drop(calls);
        assert!(matches!(
            s.download_image("https://evil.example/header.jpg"),
            Err(SteamError::Unsupported(_))
        ));
        assert!(is_steam_cdn(
            "https://shared.akamai.steamstatic.com/store_item_assets/steam/apps/5/header.jpg"
        ));
        assert!(is_steam_cdn("https://steamstatic.com/x"));
        assert!(!is_steam_cdn("http://cdn.akamai.steamstatic.com/x"));
        assert!(!is_steam_cdn("https://steamstatic.com.evil.example/x"));
        assert!(!is_steam_cdn("https://user@cdn.steamstatic.com/x"));
        assert!(!is_steam_cdn("https://notsteamstatic.com/x"));
    }

    #[test]
    fn gg_deals_batches_by_hundred_and_keeps_partial_results() {
        // Second batch is rate-limited: the first batch's results are kept.
        let s = fake(|_| {
            use std::sync::atomic::{AtomicUsize, Ordering};
            static N: AtomicUsize = AtomicUsize::new(0);
            if N.fetch_add(1, Ordering::Relaxed) == 0 {
                Ok(r#"{"success":true,"data":{"1":{"url":"u","prices":{"currentRetail":"1.00","currentKeyshops":null,"currency":"USD"}},"2":null}}"#.into())
            } else {
                Err(SteamError::Status(429))
            }
        });
        let ids: Vec<u32> = (1..=150).collect();
        let got = s.gg_deals_prices("k", "us", &ids, &NoProgress).unwrap();
        assert_eq!(s.http.calls.lock().unwrap().len(), 2);
        assert_eq!(got.prices.len(), 1);
        assert_eq!(got.seen, vec![1, 2]);
        assert!(got.error.is_some());
        let first = &s.http.calls.lock().unwrap()[0].1;
        assert_eq!(first[0].1.split(',').count(), 100);
    }

    #[test]
    fn reviews_are_batched_by_fifty() {
        let s = fake(|_| Ok(REVIEWS.to_string()));
        let ids: Vec<u32> = (1..=120).collect();
        let got = s.reviews(&ids, &NoProgress).unwrap().items;
        let calls = s.http.calls.lock().unwrap();
        assert_eq!(calls.len(), 3);
        assert!(calls[0].1[0].1.contains("\"include_reviews\":true"));
        assert_eq!(got.len(), 3, "one review per (fake) response");
    }

    #[test]
    fn reviews_keep_earlier_batches_when_one_fails() {
        let s = fake(|_| {
            use std::sync::atomic::{AtomicUsize, Ordering};
            static N: AtomicUsize = AtomicUsize::new(0);
            if N.fetch_add(1, Ordering::Relaxed) == 0 {
                Ok(REVIEWS.to_string())
            } else {
                Err(SteamError::RateLimited)
            }
        });
        let ids: Vec<u32> = (1..=120).collect();
        let got = s.reviews(&ids, &NoProgress).unwrap();
        assert_eq!(got.items.len(), 1);
        assert_eq!(got.error, Some(SteamError::RateLimited));
        // Nothing fetched at all is a plain error.
        let s = fake(|_| Err(SteamError::RateLimited));
        assert_eq!(s.reviews(&ids, &NoProgress), Err(SteamError::RateLimited));
    }

    #[test]
    fn progress_counts_games_and_notes_the_gg_deals_pause() {
        struct Rec(Mutex<(usize, Vec<String>)>);
        impl Progress for Rec {
            fn advance(&self, n: usize) {
                self.0.lock().unwrap().0 += n;
            }
            fn note(&self, what: &str) {
                self.0.lock().unwrap().1.push(what.to_string());
            }
        }
        let s = fake(|_| Ok(r#"{"success":true,"data":{}}"#.into()));
        let rec = Rec(Mutex::new((0, Vec::new())));
        let ids: Vec<u32> = (1..=250).collect();
        s.gg_deals_prices("k", "us", &ids, &rec).unwrap();
        let (n, notes) = rec.0.into_inner().unwrap();
        assert_eq!(n, 250);
        assert!(notes.iter().any(|m| m.contains("waiting")), "{notes:?}");
    }

    #[test]
    fn resolve_profile_by_vanity_and_id() {
        let s = fake(|url| {
            if url.contains("/id/gabelogannewell") {
                Ok(PROFILE.to_string())
            } else {
                Ok("<html/>".to_string())
            }
        });
        assert_eq!(
            s.resolve_profile("gabelogannewell").unwrap(),
            76561197960287930
        );
        assert_eq!(
            s.resolve_profile("76561197960287930").unwrap(),
            76561197960287930
        );
        assert_eq!(
            s.resolve_profile("nobody-here"),
            Err(SteamError::ProfileNotFound("nobody-here".into()))
        );
        assert_eq!(
            s.http.calls.lock().unwrap().len(),
            2,
            "a SteamID64 needs no request"
        );
    }

    #[test]
    fn owned_games_maps_auth_errors() {
        let s = fake(|_| Err(SteamError::Status(403)));
        assert_eq!(s.owned_games("bad", 1), Err(SteamError::BadApiKey));
        let s = fake(|_| Ok(OWNED.to_string()));
        assert_eq!(s.owned_games("key", 1).unwrap(), vec![391540, 367520]);
        let q = &s.http.calls.lock().unwrap()[0].1;
        assert!(q.contains(&("key".to_string(), "key".to_string())));
    }

    #[test]
    fn itad_prices_look_up_then_price() {
        let s = fake(|url| {
            Ok(if url.contains("/lookup/") {
                r#"{"app/220":"g1","app/400":null}"#.to_string()
            } else {
                r#"[{"id":"g1","historyLow":{"all":null},"deals":[{"shop":{"id":61,"name":"Steam"},"price":{"amount":1.0,"amountInt":100,"currency":"USD"},"cut":0,"url":"u"}]}]"#.to_string()
            })
        });
        let got = s.itad_prices("k", "GB", &[220, 400], &NoProgress).unwrap();
        assert_eq!(got.seen, vec![400, 220]);
        let got = got.prices;
        assert_eq!(got.len(), 1);
        assert_eq!(got[0].app_id, 220);
        assert_eq!(got[0].deals[0].price.cents, 100);
        let calls = s.http.calls.lock().unwrap();
        assert!(calls[0].0.ends_with("/lookup/id/shop/61/v1"));
        assert!(
            calls[0]
                .1
                .contains(&("body".into(), r#"["app/220","app/400"]"#.into()))
        );
        assert!(calls[1].1.contains(&("country".into(), "GB".into())));
        assert!(calls[1].1.contains(&("body".into(), r#"["g1"]"#.into())));
    }

    #[test]
    fn price_services_map_auth_errors() {
        let s = fake(|_| Err(SteamError::Status(403)));
        assert_eq!(
            s.itad_prices("bad", "US", &[1], &NoProgress),
            Err(SteamError::BadServiceKey("IsThereAnyDeal"))
        );
        assert_eq!(
            s.gg_deals_prices("bad", "us", &[1], &NoProgress),
            Err(SteamError::BadServiceKey("gg.deals"))
        );
    }

    /// Real price APIs with your keys: `ITAD_KEY=... GG_DEALS_KEY=... cargo test -p keykeeper-steam
    /// live_prices -- --ignored --nocapture`. A missing variable skips that service.
    #[test]
    #[ignore = "network"]
    fn live_prices() {
        let s = Steam::new(crate::UreqHttp::new());
        if let Ok(k) = std::env::var("ITAD_KEY") {
            let p = s
                .itad_prices(&k, "US", &[400, 1145360], &NoProgress)
                .unwrap()
                .prices;
            println!("ITAD: {p:#?}");
            assert!(!p.is_empty() && p.iter().all(|g| !g.deals.is_empty()));
        }
        if let Ok(k) = std::env::var("GG_DEALS_KEY") {
            let p = s
                .gg_deals_prices(&k, "us", &[400, 1145360], &NoProgress)
                .unwrap();
            println!("gg.deals: {p:#?}");
            assert!(p.prices.iter().any(|g| g.retail.is_some()) && p.error.is_none());
        }
    }

    /// Hits the real Steam endpoints. `cargo test -p keykeeper-steam -- --ignored`
    #[test]
    #[ignore = "network"]
    fn live_endpoints() {
        let s = Steam::new(crate::UreqHttp::new());
        let found = s.search("Haiku, the Robot").unwrap();
        assert!(found.iter().any(|m| m.app_id == 1231880), "{found:?}");
        let r = s.reviews(&[1231880, 391540], &NoProgress).unwrap().items;
        assert_eq!(r.len(), 2, "{r:?}");
        assert!(r.iter().all(|r| r.percent > 50 && r.total > 100));
        assert_eq!(
            s.resolve_profile("gabelogannewell").unwrap(),
            76561197960287930
        );
        assert!(s.wishlist(76561197960287930).is_ok());
        let d = s.store_details(&[1231880, 367520]).unwrap().items;
        assert_eq!(d.len(), 2, "{d:?}");
        let (hk, tags) = d.iter().find(|(d, _)| d.app_id == 367520).unwrap();
        assert!(
            !hk.description.is_empty() && !tags.is_empty(),
            "{hk:?} {tags:?}"
        );
        let url = hk.header_url.clone().unwrap();
        let img = s.download_image(&url).unwrap();
        assert!(
            img.len() > 10_000 && img.starts_with(&[0xFF, 0xD8]),
            "a JPEG from {url}"
        );
    }
}
