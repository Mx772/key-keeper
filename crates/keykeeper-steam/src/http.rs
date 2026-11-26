//! Blocking HTTP with polite rate limiting. Call from background threads only.

use std::sync::Mutex;
use std::time::{Duration, Instant};

use crate::{Result, SteamError};

pub trait Http: Send + Sync {
    /// GET `url` with `query` parameters (encoded by the implementation); returns the body.
    fn get(&self, url: &str, query: &[(&str, &str)]) -> Result<String>;
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

    fn once(&self, url: &str, query: &[(&str, &str)]) -> Result<(u16, String)> {
        self.throttle();
        let mut req = self.agent.get(url);
        for (k, v) in query {
            req = req.query(*k, *v);
        }
        let mut resp = req.call().map_err(|e| SteamError::Network(e.to_string()))?;
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

impl Http for UreqHttp {
    fn get(&self, url: &str, query: &[(&str, &str)]) -> Result<String> {
        let (mut status, mut body) = self.once(url, query)?;
        if status == 429 {
            std::thread::sleep(self.retry_after);
            (status, body) = self.once(url, query)?;
        }
        match status {
            200..=299 => Ok(body),
            429 => Err(SteamError::RateLimited),
            s => Err(SteamError::Status(s)),
        }
    }
}
