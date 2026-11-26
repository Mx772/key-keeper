//! Blocking HTTP with polite rate limiting. Call from background threads only.

use std::sync::Mutex;
use std::time::{Duration, Instant};

use crate::{Result, SteamError};

pub trait Http: Send + Sync {
    /// GET `url` with `query` parameters (encoded by the implementation); returns the body.
    fn get(&self, url: &str, query: &[(&str, &str)]) -> Result<String>;
    /// POST a JSON `body` to `url` with `query` parameters; returns the response body.
    fn post_json(&self, url: &str, query: &[(&str, &str)], body: &str) -> Result<String>;
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

/// `ureq` with a minimum gap between requests and one retry after a 429.
pub struct UreqHttp {
    agent: ureq::Agent,
    min_interval: Duration,
    retry_after: Duration,
    last: Mutex<Option<Instant>>,
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
            min_interval: Duration::from_millis(1100),
            retry_after: Duration::from_secs(30),
            last: Mutex::new(None),
        }
    }

    fn throttle(&self) {
        let Ok(mut last) = self.last.lock() else {
            return;
        };
        if let Some(t) = *last {
            let since = t.elapsed();
            if since < self.min_interval {
                std::thread::sleep(self.min_interval - since);
            }
        }
        *last = Some(Instant::now());
    }

    fn once(&self, url: &str, query: &[(&str, &str)], body: Option<&str>) -> Result<(u16, String)> {
        self.throttle();
        let mut resp = match body {
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
        }
        .map_err(|e| SteamError::Network(e.to_string()))?;
        let status = resp.status().as_u16();
        let body = resp
            .body_mut()
            .read_to_string()
            .map_err(|e| SteamError::Network(e.to_string()))?;
        Ok((status, body))
    }
}

impl Default for UreqHttp {
    fn default() -> Self {
        UreqHttp::new()
    }
}

impl UreqHttp {
    fn request(&self, url: &str, query: &[(&str, &str)], send: Option<&str>) -> Result<String> {
        let (mut status, mut body) = self.once(url, query, send)?;
        if status == 429 {
            std::thread::sleep(self.retry_after);
            (status, body) = self.once(url, query, send)?;
        }
        match status {
            200..=299 => Ok(body),
            429 => Err(SteamError::RateLimited),
            s => Err(SteamError::Status(s)),
        }
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
        self.once(url, query, None)
    }
}
