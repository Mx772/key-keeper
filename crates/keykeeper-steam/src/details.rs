//! Store details for the hover card and grid view: description, developers, top tags, release
//! date, Steam Deck support and artwork URLs, from the same batched
//! `IStoreBrowseService/GetItems` endpoint the review scores use (no key). Tag ids are named
//! with `IStoreService/GetTagList` (no key, fetched once per session).

use std::collections::HashMap;

use serde_json::Value;

use crate::{Result, SteamError};

/// Asset paths in `GetItems` are relative to this.
pub const ASSET_BASE: &str = "https://shared.akamai.steamstatic.com/store_item_assets/";

/// How many top tags a game keeps.
pub const TAGS_KEPT: usize = 4;

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct StoreDetails {
    pub app_id: u32,
    pub name: String,
    pub description: String,
    pub developers: Vec<String>,
    /// Top tag ids, most popular first (named with [`parse_tag_list`]).
    pub tag_ids: Vec<u32>,
    /// Seconds since the epoch; `None` when unreleased or unknown.
    pub released: Option<i64>,
    /// Steam's `steam_deck_compat_category` (0 unknown, 1 unsupported, 2 playable, 3 verified).
    pub deck_category: u64,
    pub header_url: Option<String>,
    pub capsule_url: Option<String>,
}

fn str_list(v: Option<&Value>) -> Vec<String> {
    v.and_then(Value::as_array)
        .map(|a| {
            a.iter()
                .filter_map(|x| x.get("name").and_then(Value::as_str))
                .map(str::to_string)
                .collect()
        })
        .unwrap_or_default()
}

fn asset(format: Option<&str>, file: Option<&str>) -> Option<String> {
    let (format, file) = (format?, file?);
    if file.is_empty() || !format.contains("${FILENAME}") {
        return None;
    }
    Some(format!(
        "{ASSET_BASE}{}",
        format.replace("${FILENAME}", file)
    ))
}

/// `GetItems` with `include_basic_info`, `include_release`, `include_platforms`,
/// `include_assets` and `include_tag_count`. Unknown apps (`success` != 1) are omitted.
pub fn parse_store_details(body: &str) -> Result<Vec<StoreDetails>> {
    let v: Value = serde_json::from_str(body).map_err(|e| SteamError::Parse(e.to_string()))?;
    let items = v
        .pointer("/response/store_items")
        .and_then(Value::as_array)
        .cloned()
        .unwrap_or_default();
    let mut out = Vec::new();
    for it in items {
        if it.get("success").and_then(Value::as_u64) != Some(1) {
            continue;
        }
        let Some(app_id) = it
            .get("appid")
            .or_else(|| it.get("id"))
            .and_then(Value::as_u64)
            .and_then(|a| u32::try_from(a).ok())
            .filter(|a| *a != 0)
        else {
            continue;
        };
        let mut tags: Vec<(u32, u64)> = it
            .get("tags")
            .and_then(Value::as_array)
            .map(|a| {
                a.iter()
                    .filter_map(|t| {
                        let id = u32::try_from(t.get("tagid")?.as_u64()?).ok()?;
                        Some((id, t.get("weight").and_then(Value::as_u64).unwrap_or(0)))
                    })
                    .collect()
            })
            .unwrap_or_default();
        if tags.is_empty() {
            tags = it
                .get("tagids")
                .and_then(Value::as_array)
                .map(|a| {
                    a.iter()
                        .filter_map(|t| u32::try_from(t.as_u64()?).ok())
                        .map(|id| (id, 0))
                        .collect()
                })
                .unwrap_or_default();
        }
        tags.sort_by_key(|t| std::cmp::Reverse(t.1));
        let format = it
            .pointer("/assets/asset_url_format")
            .and_then(Value::as_str);
        let file = |name: &str| {
            it.pointer(&format!("/assets/{name}"))
                .and_then(Value::as_str)
        };
        out.push(StoreDetails {
            app_id,
            name: it
                .get("name")
                .and_then(Value::as_str)
                .unwrap_or("")
                .to_string(),
            description: it
                .pointer("/basic_info/short_description")
                .and_then(Value::as_str)
                .unwrap_or("")
                .trim()
                .to_string(),
            developers: str_list(it.pointer("/basic_info/developers")),
            tag_ids: tags.into_iter().take(TAGS_KEPT).map(|t| t.0).collect(),
            released: it
                .pointer("/release/steam_release_date")
                .and_then(Value::as_i64)
                .filter(|t| *t > 0),
            deck_category: it
                .pointer("/platforms/steam_deck_compat_category")
                .and_then(Value::as_u64)
                .unwrap_or(0),
            header_url: asset(format, file("header")),
            capsule_url: asset(format, file("main_capsule")),
        });
    }
    Ok(out)
}

/// `IStoreService/GetTagList`: tag id → English name.
pub fn parse_tag_list(body: &str) -> Result<HashMap<u32, String>> {
    let v: Value = serde_json::from_str(body).map_err(|e| SteamError::Parse(e.to_string()))?;
    let tags = v
        .pointer("/response/tags")
        .and_then(Value::as_array)
        .ok_or_else(|| SteamError::Parse("no tags in tag list".into()))?;
    Ok(tags
        .iter()
        .filter_map(|t| {
            let id = u32::try_from(t.get("tagid")?.as_u64()?).ok()?;
            Some((id, t.get("name")?.as_str()?.to_string()))
        })
        .collect())
}

#[cfg(test)]
mod tests {
    use super::*;

    // Recorded from the live endpoints on 2026-11-26 (trimmed).
    const ITEMS: &str = r#"{"response":{"store_items":[{"item_type":0,"id":1231880,"success":1,"visible":true,"name":"Haiku, the Robot","appid":1231880,"type":0,"tagids":[1628,21,3834,3964,4166],"basic_info":{"short_description":"Delve into the depths of a mechanical world in this cute, adventure-exploration game.","publishers":[{"name":"Mister Morris Games","creator_clan_account_id":42747636}],"developers":[{"name":"Mister Morris Games","creator_clan_account_id":42747636}]},"tags":[{"tagid":1628,"weight":681},{"tagid":21,"weight":642},{"tagid":3834,"weight":637},{"tagid":3964,"weight":630},{"tagid":4166,"weight":569}],"assets":{"asset_url_format":"steam/apps/1231880/${FILENAME}?t=1791533761","main_capsule":"capsule_616x353.jpg","small_capsule":"capsule_231x87.jpg","header":"header.jpg"},"release":{"steam_release_date":1651154399,"original_release_date":0},"platforms":{"windows":true,"mac":true,"vr_support":{},"steam_deck_compat_category":3}},{"item_type":0,"id":1,"success":15,"visible":false,"name":"","store_url_path":"app/0/","store_url_slug":"","appid":0}]}}"#;
    const HASHED: &str = r#"{"response":{"store_items":[{"id":367520,"success":1,"name":"Hollow Knight","appid":367520,"tagids":[1628,1625],"assets":{"asset_url_format":"steam/apps/367520/${FILENAME}?t=1776125684","main_capsule":"f30cc/capsule_616x353.jpg","header":"3c348/header.jpg"},"release":{"steam_release_date":0}}]}}"#;
    const TAGS: &str = r#"{"response":{"version_hash":"711684454","tags":[{"tagid":19,"name":"Action"},{"tagid":21,"name":"Adventure"},{"tagid":1628,"name":"Metroidvania"},{"tagid":1625,"name":"Platformer"}]}}"#;

    #[test]
    fn store_details() {
        let d = parse_store_details(ITEMS).unwrap();
        assert_eq!(d.len(), 1, "the unknown app is skipped");
        let h = &d[0];
        assert_eq!(h.app_id, 1231880);
        assert_eq!(h.name, "Haiku, the Robot");
        assert!(h.description.starts_with("Delve into"));
        assert_eq!(h.developers, vec!["Mister Morris Games".to_string()]);
        assert_eq!(h.tag_ids, vec![1628, 21, 3834, 3964], "top four by weight");
        assert_eq!(h.released, Some(1651154399));
        assert_eq!(h.deck_category, 3);
        assert_eq!(
            h.header_url.as_deref(),
            Some(
                "https://shared.akamai.steamstatic.com/store_item_assets/steam/apps/1231880/header.jpg?t=1791533761"
            )
        );
        assert!(
            h.capsule_url
                .as_deref()
                .is_some_and(|u| u.ends_with("1231880/capsule_616x353.jpg?t=1791533761"))
        );
    }

    #[test]
    fn hashed_asset_paths_and_tagids_fallback() {
        let d = parse_store_details(HASHED).unwrap();
        let h = &d[0];
        assert_eq!(h.tag_ids, vec![1628, 1625]);
        assert_eq!(h.released, None, "0 means unknown");
        assert_eq!(h.deck_category, 0);
        assert!(h.description.is_empty() && h.developers.is_empty());
        assert!(
            h.header_url
                .as_deref()
                .is_some_and(|u| u.ends_with("/367520/3c348/header.jpg?t=1776125684"))
        );
    }

    #[test]
    fn tag_list() {
        let t = parse_tag_list(TAGS).unwrap();
        assert_eq!(t.get(&1628).map(String::as_str), Some("Metroidvania"));
        assert_eq!(t.len(), 4);
        assert!(parse_tag_list("{}").is_err());
        assert!(parse_tag_list("nope").is_err());
        assert!(parse_store_details("nope").is_err());
        assert!(parse_store_details("{}").unwrap().is_empty());
    }
}
