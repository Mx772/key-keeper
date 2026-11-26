//! URL builders for store / price sites, derived from a Steam app id.

pub fn steam_store(app_id: u32) -> String {
    format!("https://store.steampowered.com/app/{app_id}/")
}

pub fn gg_deals(app_id: u32) -> String {
    format!("https://gg.deals/steam/app/{app_id}/")
}

/// ITAD's redirect-by-Steam-id form. To be verified against the live site in Phase 5.
pub fn itad(app_id: u32) -> String {
    format!("https://isthereanydeal.com/steam/app/{app_id}/")
}

/// Steam's key activation page with `key` prefilled.
pub fn steam_register(key: &str) -> String {
    let mut enc = String::new();
    for b in key.trim().bytes() {
        if b.is_ascii_alphanumeric() || b == b'-' {
            enc.push(b as char);
        } else {
            enc.push_str(&format!("%{b:02X}"));
        }
    }
    format!("https://store.steampowered.com/account/registerkey?key={enc}")
}

/// Lowercased host of an `http(s)://` URL, without a leading `www.`. No URL crate needed for this.
pub fn host(url: &str) -> Option<String> {
    let rest = url
        .trim()
        .strip_prefix("https://")
        .or_else(|| url.trim().strip_prefix("http://"))?;
    let host = rest
        .split(['/', '?', '#'])
        .next()?
        .split('@')
        .next_back()?
        .split(':')
        .next()?;
    if host.is_empty() {
        return None;
    }
    let host = host.to_ascii_lowercase();
    Some(
        host.strip_prefix("www.")
            .map(str::to_string)
            .unwrap_or(host),
    )
}

/// Steam app id from a store URL like `https://store.steampowered.com/app/972660/Spiritfarer/`.
pub fn steam_app_id_from_url(url: &str) -> Option<u32> {
    let h = host(url)?;
    if h != "store.steampowered.com" && h != "steamcommunity.com" && h != "gg.deals" {
        return None;
    }
    let mut parts = url.split('/');
    parts.find(|p| *p == "app")?;
    parts.next()?.parse().ok()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn hosts() {
        assert_eq!(
            host("https://www.Lestrades.com/offer/1").as_deref(),
            Some("lestrades.com")
        );
        assert_eq!(
            host("http://user@example.com:8080/x").as_deref(),
            Some("example.com")
        );
        assert_eq!(host("ftp://x"), None);
        assert_eq!(host("https:///x"), None);
    }

    #[test]
    fn register_url_encodes_the_key() {
        assert_eq!(
            steam_register(" AAAAA-BBBBB-CCCCC "),
            "https://store.steampowered.com/account/registerkey?key=AAAAA-BBBBB-CCCCC"
        );
        assert_eq!(
            steam_register("A B&C"),
            "https://store.steampowered.com/account/registerkey?key=A%20B%26C"
        );
    }

    #[test]
    fn app_ids() {
        assert_eq!(
            steam_app_id_from_url("https://store.steampowered.com/app/972660/Spiritfarer/"),
            Some(972660)
        );
        assert_eq!(
            steam_app_id_from_url("https://gg.deals/steam/app/972660/"),
            Some(972660)
        );
        assert_eq!(steam_app_id_from_url("https://example.com/app/1/"), None);
        assert_eq!(
            steam_app_id_from_url("https://store.steampowered.com/bundle/1/"),
            None
        );
    }
}
