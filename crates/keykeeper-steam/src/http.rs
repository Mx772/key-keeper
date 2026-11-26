//! Blocking HTTP with polite rate limiting. Call from background threads only.
//!
//! Every API host gets its own minimum gap between requests (Steam's store search is stricter
//! than the rest). A 429 waits (honouring `Retry-After`) and retries twice, and slows that host
//! down for the rest of the session. Requests are logged with `log` (host and path only: query
//! strings carry API keys).

use std::collections::HashMap;
use std::sync::Mutex;
use std::time::{Duration, Instant};

use crate::{Result, SteamError};

pub trait Http: Send + Sync {
    /// GET `url` with `query` parameters (encoded by the implementation); returns the body.
    fn get(&self, url: &str, query: &[(&str, &str)]) -> Result<String>;
    /// POST a JSON `body` to `url` with `query` parameters; returns the response body.
    fn post_json(&self, url: &str, query: &[(&str, &str)], body: &str) -> Result<String>;
    /// GET raw bytes (images). Fails on non-2xx statuses and bodies over `limit` bytes.
    fn get_bytes(&self, _url: &str, _limit: usize) -> Result<Vec<u8>> {
        Err(SteamError::Unsupported("downloads"))
    }
    /// GET without status handling or retries: the status and body, also for errors (APIs that
    /// explain a 4xx in its body). Only network failures are `Err`.
    fn get_with_status(&self, url: &str, query: &[(&str, &str)]) -> Result<(u16, String)> {
        match self.get(url, query) {
            Ok(body) => Ok((200, body)),
            Err(SteamError::Status(s)) => Ok((s, String::new())),
            Err(SteamError::RateLimited) => Ok((429, String::new())),
            Err(e) => Err(e),
        }
    }
}

/// The gap between requests to most APIs.
pub const DEFAULT_INTERVAL: Duration = Duration::from_millis(1100);
/// Steam's store search allows roughly 200 requests in 5 minutes.
pub const SEARCH_INTERVAL: Duration = Duration::from_millis(1600);
/// The slowest a host is paced after repeated 429s.
const MAX_INTERVAL: Duration = Duration::from_secs(6);
/// Waits after a 429 without a usable `Retry-After`: first retry, second retry.
const RETRY_WAITS: [Duration; 2] = [Duration::from_secs(30), Duration::from_secs(60)];
/// The longest `Retry-After` honoured.
const MAX_RETRY_AFTER: Duration = Duration::from_secs(120);

/// `host/path` of `url`, without scheme or query: safe to log and the key for pacing.
pub fn log_target(url: &str) -> &str {
    let rest = url.split_once("://").map_or(url, |(_, r)| r);
    rest.split(['?', '#']).next().unwrap_or(rest)
}

fn host(url: &str) -> &str {
    log_target(url).split('/').next().unwrap_or("")
}

/// The pacing bucket for `url`: store search separately from the rest of its host.
fn bucket(url: &str) -> String {
    let t = log_target(url);
    if t.contains("/api/storesearch") {
        "steam-search".into()
    } else {
        host(url).to_string()
    }
}

/// Removes anything that looks like a credential (`key=…`, `api_key=…`, `token=…`) from text
/// such as an error message that quotes a URL.
pub fn redact(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    let mut rest = text;
    while let Some(i) = rest.find('=') {
        let (before, after) = rest.split_at(i);
        let name_start = before
            .rfind(|c: char| !(c.is_ascii_alphanumeric() || c == '_'))
            .map_or(0, |p| p + 1);
        let name = before[name_start..].to_ascii_lowercase();
        out.push_str(before);
        out.push('=');
        let value = &after[1..];
        if name.ends_with("key") || name.contains("token") || name == "secret" {
            let end = value
                .find(|c: char| c == '&' || c == '"' || c == '\'' || c.is_whitespace())
                .unwrap_or(value.len());
            out.push_str("***");
            rest = &value[end..];
        } else {
            rest = value;
        }
    }
    out.push_str(rest);
    out
}

struct Pace {
    interval: Duration,
    next: Instant,
}

/// `ureq` with per-host pacing and retries after a 429.
pub struct UreqHttp {
    agent: ureq::Agent,
    retry_waits: [Duration; 2],
    pace: Mutex<HashMap<String, Pace>>,
}

impl UreqHttp {
    pub fn new() -> UreqHttp {
        let config = ureq::Agent::config_builder()
            .timeout_global(Some(Duration::from_secs(20)))
            .user_agent(concat!(
                "KeyKeeper/",
                env!("CARGO_PKG_VERSION"),
                " (desktop key manager)"
            ))
            .http_status_as_error(false)
            .build();
        UreqHttp {
            agent: config.into(),
            retry_waits: RETRY_WAITS,
            pace: Mutex::new(HashMap::new()),
        }
    }

    /// Wait for this request's turn on its host. The lock is only held to book the slot, so a
    /// slow host doesn't hold up requests to another.
    fn throttle(&self, url: &str) {
        let key = bucket(url);
        let wait = {
            let Ok(mut pace) = self.pace.lock() else {
                return;
            };
            let now = Instant::now();
            let p = pace.entry(key).or_insert_with(|| Pace {
                interval: if url.contains("/api/storesearch") {
                    SEARCH_INTERVAL
                } else {
                    DEFAULT_INTERVAL
                },
                next: now,
            });
            let at = p.next.max(now);
            p.next = at + p.interval;
            at - now
        };
        if !wait.is_zero() {
            std::thread::sleep(wait);
        }
    }

    /// After a 429: space this host's requests further apart for the rest of the session.
    fn slow_down(&self, url: &str) {
        if let Ok(mut pace) = self.pace.lock()
            && let Some(p) = pace.get_mut(&bucket(url))
        {
            let slower = (p.interval * 3 / 2).min(MAX_INTERVAL);
            if slower != p.interval {
                log::warn!(
                    "{}: rate-limited; now {} ms between requests",
                    host(url),
                    slower.as_millis()
                );
            }
            p.interval = slower;
        }
    }

    fn once(
        &self,
        url: &str,
        query: &[(&str, &str)],
        body: Option<&str>,
    ) -> Result<(u16, String, Option<Duration>)> {
        self.throttle(url);
        let started = Instant::now();
        let method = if body.is_some() { "POST" } else { "GET" };
        let sent = match body {
            None => {
                let mut req = self.agent.get(url);
                for (k, v) in query {
                    req = req.query(*k, *v);
                }
                req.call()
            }
            Some(b) => {
                let mut req = self
                    .agent
                    .post(url)
                    .header("Content-Type", "application/json");
                for (k, v) in query {
                    req = req.query(*k, *v);
                }
                req.send(b)
            }
        };
        let mut resp = match sent {
            Ok(r) => r,
            Err(e) => {
                let msg = redact(&e.to_string());
                log::warn!("{method} {} failed: {msg}", log_target(url));
                return Err(SteamError::Network(msg));
            }
        };
        let status = resp.status().as_u16();
        let retry_after = resp
            .headers()
            .get("retry-after")
            .and_then(|v| v.to_str().ok())
            .and_then(|v| v.trim().parse::<u64>().ok())
            .map(Duration::from_secs);
        let body = resp.body_mut().read_to_string().map_err(|e| {
            let msg = redact(&e.to_string());
            log::warn!("{method} {} body: {msg}", log_target(url));
            SteamError::Network(msg)
        })?;
        let ms = started.elapsed().as_millis();
        if (200..300).contains(&status) {
            log::debug!("{method} {} → {status} ({ms} ms)", log_target(url));
        } else {
            log::warn!("{method} {} → {status} ({ms} ms)", log_target(url));
        }
        Ok((status, body, retry_after))
    }

    fn request(&self, url: &str, query: &[(&str, &str)], send: Option<&str>) -> Result<String> {
        let (mut status, mut body, mut retry_after) = self.once(url, query, send)?;
        for fallback in self.retry_waits {
            if status != 429 {
                break;
            }
            self.slow_down(url);
            let wait = retry_after
                .filter(|d| *d <= MAX_RETRY_AFTER)
                .unwrap_or(fallback);
            log::warn!(
                "{}: HTTP 429, waiting {} s before retrying",
                host(url),
                wait.as_secs()
            );
            std::thread::sleep(wait);
            (status, body, retry_after) = self.once(url, query, send)?;
        }
        match status {
            200..=299 => Ok(body),
            429 => {
                log::error!("{}: still rate-limited after retries; giving up", host(url));
                Err(SteamError::RateLimited)
            }
            s => Err(SteamError::Status(s)),
        }
    }
}

impl Default for UreqHttp {
    fn default() -> Self {
        UreqHttp::new()
    }
}

impl Http for UreqHttp {
    fn get(&self, url: &str, query: &[(&str, &str)]) -> Result<String> {
        self.request(url, query, None)
    }

    fn post_json(&self, url: &str, query: &[(&str, &str)], body: &str) -> Result<String> {
        self.request(url, query, Some(body))
    }

    fn get_with_status(&self, url: &str, query: &[(&str, &str)]) -> Result<(u16, String)> {
        let (status, body, _) = self.once(url, query, None)?;
        if status == 429 {
            self.slow_down(url);
        }
        Ok((status, body))
    }

    /// Not throttled: CDN files, not API calls.
    fn get_bytes(&self, url: &str, limit: usize) -> Result<Vec<u8>> {
        let mut resp = self.agent.get(url).call().map_err(|e| {
            let msg = redact(&e.to_string());
            log::warn!("GET {} failed: {msg}", log_target(url));
            SteamError::Network(msg)
        })?;
        let status = resp.status().as_u16();
        if !(200..300).contains(&status) {
            log::warn!("GET {} → {status}", log_target(url));
            return Err(SteamError::Status(status));
        }
        resp.body_mut()
            .with_config()
            .limit(limit as u64)
            .read_to_vec()
            .map_err(|e| SteamError::Network(redact(&e.to_string())))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn credentials_are_redacted() {
        assert_eq!(
            redact("https://api.gg.deals/v1/prices/?ids=1,2&key=SECRET&region=us: timed out"),
            "https://api.gg.deals/v1/prices/?ids=1,2&key=***&region=us: timed out"
        );
        assert_eq!(redact("api_key=abc token=xyz"), "api_key=*** token=***");
        assert_eq!(redact("a=b c==d"), "a=b c==d");
        assert_eq!(redact("no secrets here"), "no secrets here");
    }

    #[test]
    fn targets_drop_scheme_and_query() {
        assert_eq!(
            log_target("https://api.isthereanydeal.com/games/prices/v3?key=x&country=US"),
            "api.isthereanydeal.com/games/prices/v3"
        );
        assert_eq!(
            bucket("https://store.steampowered.com/api/storesearch/?term=x"),
            "steam-search"
        );
        assert_eq!(
            bucket("https://store.steampowered.com/app/1"),
            "store.steampowered.com"
        );
    }

    #[test]
    fn pacing_books_slots_per_host() {
        let h = UreqHttp::new();
        let t = Instant::now();
        h.throttle("https://a.example/x");
        h.throttle("https://b.example/x"); // another host: no wait
        assert!(t.elapsed() < Duration::from_millis(500));
        h.slow_down("https://a.example/x");
        let pace = h.pace.lock().unwrap();
        assert_eq!(pace["a.example"].interval, DEFAULT_INTERVAL * 3 / 2);
        assert_eq!(pace["b.example"].interval, DEFAULT_INTERVAL);
    }
}
