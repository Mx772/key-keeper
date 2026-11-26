//! Steam endpoints: request building and response parsing.

use serde::Deserialize;
use serde_json::Value;

use crate::http::Http;
use crate::{Result, SteamError};

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
    fn reviews(&self, app_ids: &[u32]) -> Result<Vec<Review>>;
    fn resolve_profile(&self, input: &str) -> Result<u64>;
    /// App ids on a public wishlist (empty if private or empty).
    fn wishlist(&self, steam_id: u64) -> Result<Vec<u32>>;
    fn owned_games(&self, api_key: &str, steam_id: u64) -> Result<Vec<u32>>;
}

pub struct Steam<H: Http> {
    http: H,
}

impl<H: Http> Steam<H> {
    pub fn new(http: H) -> Steam<H> {
        Steam { http }
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

    fn reviews(&self, app_ids: &[u32]) -> Result<Vec<Review>> {
        let mut out = Vec::new();
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
            let body = self.http.get(
                "https://api.steampowered.com/IStoreBrowseService/GetItems/v1/",
                &[("input_json", &input)],
            )?;
            out.extend(parse_reviews(&body)?);
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
    }

    fn fake(answer: fn(&str) -> Result<String>) -> Steam<Fake> {
        Steam::new(Fake {
            calls: Mutex::new(Vec::new()),
            answer,
        })
    }

    #[test]
    fn reviews_are_batched_by_fifty() {
        let s = fake(|_| Ok(REVIEWS.to_string()));
        let ids: Vec<u32> = (1..=120).collect();
        let got = s.reviews(&ids).unwrap();
        let calls = s.http.calls.lock().unwrap();
        assert_eq!(calls.len(), 3);
        assert!(calls[0].1[0].1.contains("\"include_reviews\":true"));
        assert_eq!(got.len(), 3, "one review per (fake) response");
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

    /// Hits the real Steam endpoints. `cargo test -p keykeeper-steam -- --ignored`
    #[test]
    #[ignore = "network"]
    fn live_endpoints() {
        let s = Steam::new(crate::UreqHttp::new());
        let found = s.search("Haiku, the Robot").unwrap();
        assert!(found.iter().any(|m| m.app_id == 1231880), "{found:?}");
        let r = s.reviews(&[1231880, 391540]).unwrap();
        assert_eq!(r.len(), 2, "{r:?}");
        assert!(r.iter().all(|r| r.percent > 50 && r.total > 100));
        assert_eq!(
            s.resolve_profile("gabelogannewell").unwrap(),
            76561197960287930
        );
        assert!(s.wishlist(76561197960287930).is_ok());
    }
}
