//! Store details and artwork for the hover card, grid view and editor header.
//!
//! One long-lived background thread answers requests in order: store details (batched, one
//! `GetItems` call per 50 games) and header images (from the encrypted cache next to the vault,
//! else Steam's CDN when fetching is allowed). Results come back through `poll`, like Steam jobs,
//! but never block them or show progress: this is quiet, best-effort work.

use std::collections::{HashMap, HashSet};
use std::sync::mpsc::{Receiver, Sender, channel};
use std::sync::{Arc, Mutex};

use jiff::{SignedDuration, Timestamp};
use keykeeper_core::{AppInfo, BlobCache, DeckCompat};
use keykeeper_steam::{SteamApi, SteamError, StoreDetails};

use crate::app::AppState;
use uuid::Uuid;

/// Details older than this are fetched again when next needed.
pub const INFO_MAX_AGE_DAYS: i64 = 30;

/// How a game's header image is doing.
pub enum Art {
    Loading,
    Ready(rune::Image),
    /// No image (fetching off and nothing cached, the CDN had none, or it didn't decode).
    Missing,
}

enum Req {
    Details(Vec<u32>),
    Art {
        app_id: u32,
        url: String,
        fetch: bool,
    },
}

pub enum Done {
    Details {
        asked: Vec<u32>,
        result: Result<keykeeper_steam::Partial<(StoreDetails, Vec<String>)>, SteamError>,
    },
    Art {
        app_id: u32,
        bytes: Option<Vec<u8>>,
    },
}

type Slot = Arc<Mutex<Vec<Done>>>;

#[derive(Default)]
pub struct GameInfo {
    tx: Option<Sender<Req>>,
    slot: Slot,
    /// App ids whose details were asked for this session (so a failure isn't retried per hover).
    asked: HashSet<u32>,
    art: HashMap<u32, Art>,
    /// Art wanted while the game's details (with its header URL) are still on their way.
    art_after_details: HashSet<u32>,
    images: Option<rune::ImageCache>,
    cache: Option<BlobCache>,
}

impl GameInfo {
    /// A vault was unlocked: cache artwork next to it.
    pub fn attach(&mut self, cache: BlobCache) {
        self.reset();
        self.cache = Some(cache);
    }

    /// The vault was locked: drop everything decrypted and stop the worker.
    pub fn reset(&mut self) {
        self.tx = None;
        self.asked.clear();
        self.art.clear();
        self.art_after_details.clear();
        self.cache = None;
        self.slot = Slot::default();
    }

    pub fn art(&self, app_id: u32) -> Option<&rune::Image> {
        match self.art.get(&app_id) {
            Some(Art::Ready(img)) if img.is_ready() && img.error().is_none() => Some(img),
            _ => None,
        }
    }

    pub fn cache(&self) -> Option<&BlobCache> {
        self.cache.as_ref()
    }

    /// Forget loaded artwork (after clearing the cache).
    pub fn forget_art(&mut self) {
        self.art.clear();
    }

    fn send(&mut self, req: Req, api: &Arc<dyn SteamApi>, waker: Option<&rune::Waker>) {
        if self.tx.is_none() {
            let (tx, rx) = channel();
            let (api, slot, cache, waker) = (
                api.clone(),
                self.slot.clone(),
                self.cache.clone(),
                waker.cloned(),
            );
            let spawned = std::thread::Builder::new()
                .name("keykeeper-info".into())
                .spawn(move || worker(rx, api.as_ref(), cache, slot, waker));
            if spawned.is_err() {
                return;
            }
            self.tx = Some(tx);
        }
        if let Some(tx) = &self.tx
            && tx.send(req).is_err()
        {
            self.tx = None;
        }
    }
}

fn worker(
    rx: Receiver<Req>,
    api: &dyn SteamApi,
    cache: Option<BlobCache>,
    slot: Slot,
    waker: Option<rune::Waker>,
) {
    while let Ok(req) = rx.recv() {
        let done = match req {
            Req::Details(asked) => {
                let result = api.store_details(&asked);
                Done::Details { asked, result }
            }
            Req::Art { app_id, url, fetch } => {
                let id = format!("art-{app_id}");
                let mut bytes = cache.as_ref().and_then(|c| c.get(&id));
                if bytes.is_none() && fetch {
                    bytes = api.download_image(&url).ok();
                    if let (Some(b), Some(c)) = (&bytes, &cache) {
                        let _ = c.put(&id, b);
                    }
                }
                Done::Art { app_id, bytes }
            }
        };
        match slot.lock() {
            Ok(mut s) => s.push(done),
            Err(_) => return,
        }
        if let Some(w) = &waker {
            w();
        }
    }
}

pub fn to_app_info(d: &StoreDetails, tags: Vec<String>, now: Timestamp) -> AppInfo {
    AppInfo {
        name: d.name.clone(),
        description: d.description.clone(),
        developers: d.developers.clone(),
        tags,
        released: d.released.and_then(|s| Timestamp::from_second(s).ok()),
        deck: DeckCompat::from_category(d.deck_category),
        header_url: d.header_url.clone(),
        capsule_url: d.capsule_url.clone(),
        fetched_at: now,
    }
}

impl AppState {
    pub fn fetch_allowed(&self) -> bool {
        !self.offline && self.config.fetch_store_info()
    }

    /// The pointer entered (`inside`) or left key `id`'s name. The card opens after the
    /// configured delay (see [`Self::poll_hover`]).
    pub fn hover_key(&mut self, id: Uuid, inside: bool, now: f64) {
        if !inside {
            if self.hover.is_some_and(|h| h.key == id) {
                self.hover = None;
            }
            return;
        }
        if !self.config.hover_cards() || self.hover.is_some_and(|h| h.key == id) {
            return;
        }
        self.hover = Some(crate::app::Hover {
            key: id,
            since: now,
            open: false,
        });
        if let Some(app) = self
            .session()
            .and_then(|s| s.repo.key(id))
            .and_then(|k| k.steam_app_id)
        {
            self.want_info(app);
        }
    }

    /// Opens the hover card once the pointer has rested long enough.
    pub fn poll_hover(&mut self, now: f64, tx: &mut rune::Tx) -> bool {
        let delay = self.config.hover_delay_secs();
        let Some(h) = self.hover.as_mut() else {
            return false;
        };
        if h.open {
            return false;
        }
        let due = h.since + delay;
        if now >= due {
            h.open = true;
            true
        } else {
            tx.wake_after(std::time::Duration::from_secs_f64(due - now));
            false
        }
    }

    /// The key whose hover card is showing.
    pub fn hover_card_key(&self) -> Option<Uuid> {
        self.hover.filter(|h| h.open).map(|h| h.key)
    }

    /// Make sure store details and artwork for `app_id` are on their way (or here).
    pub fn want_info(&mut self, app_id: u32) {
        self.want_details(&[app_id]);
        self.want_art(app_id);
    }

    /// Ask for details of `app_ids` that have none (or stale ones); when any is missing, every
    /// game in the library that lacks details rides along in the same batched request.
    pub fn want_details(&mut self, app_ids: &[u32]) {
        if !self.fetch_allowed() {
            return;
        }
        let Some(s) = self.session() else { return };
        let stale = Timestamp::now() - SignedDuration::from_hours(24 * INFO_MAX_AGE_DAYS);
        let needs = |app: u32| {
            !self.info.asked.contains(&app)
                && s.repo.app_info(app).is_none_or(|i| i.fetched_at < stale)
        };
        if !app_ids.iter().any(|a| needs(*a)) {
            return;
        }
        let mut batch: Vec<u32> = app_ids.iter().copied().filter(|a| needs(*a)).collect();
        batch.extend(
            s.repo
                .keys()
                .iter()
                .filter_map(|k| k.steam_app_id)
                .filter(|a| needs(*a)),
        );
        batch.sort_unstable();
        batch.dedup();
        self.info.asked.extend(batch.iter().copied());
        let (api, waker) = (self.steam.clone(), self.waker.clone());
        self.info.send(Req::Details(batch), &api, waker.as_ref());
    }

    /// In the grid, artwork for the cards in view (the first few hundred).
    pub fn want_grid_art(&mut self) {
        let Some(s) = self.session() else { return };
        if self.config.library.view != crate::config::LibraryView::Grid
            || s.page != crate::app::Page::Library
        {
            return;
        }
        let apps: Vec<u32> = s
            .rows
            .iter()
            .take(400)
            .filter_map(|id| s.repo.key(*id).and_then(|k| k.steam_app_id))
            .filter(|a| !self.info.art.contains_key(a))
            .collect();
        if apps.is_empty() {
            return;
        }
        self.want_details(&apps);
        for a in apps {
            self.want_art(a);
        }
    }

    /// Load `app_id`'s header art from the cache, or Steam when allowed.
    pub fn want_art(&mut self, app_id: u32) {
        if self.info.art.contains_key(&app_id) || self.info.cache.is_none() {
            return;
        }
        let Some(s) = self.session() else { return };
        if s.repo.app_info(app_id).is_none() && self.info.asked.contains(&app_id) {
            // Details are coming with the real header URL; ask for the art then.
            self.info.art_after_details.insert(app_id);
            return;
        }
        let url = s.repo.app_info(app_id).map_or_else(
            || keykeeper_core::appinfo::default_header_url(app_id),
            |i| i.header_or_default(app_id),
        );
        let fetch = self.fetch_allowed();
        self.info.art.insert(app_id, Art::Loading);
        let (api, waker) = (self.steam.clone(), self.waker.clone());
        self.info
            .send(Req::Art { app_id, url, fetch }, &api, waker.as_ref());
    }

    /// Apply finished info work. Returns whether anything changed.
    pub fn finish_info(&mut self) -> bool {
        let done: Vec<Done> = self
            .info
            .slot
            .lock()
            .map(|mut d| std::mem::take(&mut *d))
            .unwrap_or_default();
        if done.is_empty() {
            return false;
        }
        let mut art_next = Vec::new();
        for d in done {
            match d {
                Done::Details { asked, result } => {
                    art_next.extend(
                        asked
                            .iter()
                            .filter(|a| self.info.art_after_details.remove(a)),
                    );
                    match result {
                        Ok(found) => {
                            let now = Timestamp::now();
                            if found.error.is_some() {
                                // Stopped early: let a later hover ask for the ones not reached.
                                for a in &asked {
                                    if !found.items.iter().any(|(d, _)| d.app_id == *a) {
                                        self.info.asked.remove(a);
                                    }
                                }
                            }
                            let info: Vec<(u32, AppInfo)> = found
                                .items
                                .into_iter()
                                .map(|(d, tags)| (d.app_id, to_app_info(&d, tags, now)))
                                .collect();
                            if let Some(s) = self.session_mut() {
                                s.repo.set_app_info(info);
                            }
                        }
                        Err(e) => {
                            log::warn!("store details for {} apps: {e}", asked.len());
                            // Quiet: let a later hover try again (art uses the default URL now).
                            for a in asked {
                                self.info.asked.remove(&a);
                            }
                        }
                    }
                }
                Done::Art { app_id, bytes } => {
                    let art = match bytes {
                        Some(b) => {
                            if self.info.images.is_none() {
                                let waker = self
                                    .waker
                                    .clone()
                                    .unwrap_or_else(|| Arc::new(|| {}) as rune::Waker);
                                self.info.images = Some(rune::ImageCache::new(waker));
                            }
                            match &self.info.images {
                                Some(c) => Art::Ready(c.load(b)),
                                None => Art::Missing,
                            }
                        }
                        None => Art::Missing,
                    };
                    self.info.art.insert(app_id, art);
                }
            }
        }
        for app in art_next {
            self.want_art(app);
        }
        true
    }

    /// Delete the encrypted artwork cache.
    pub fn clear_art_cache(&mut self) {
        let Some(cache) = self.info.cache.clone() else {
            return;
        };
        match cache.clear() {
            Ok(bytes) => {
                self.info.forget_art();
                self.toast(format!("Cleared {} of cached artwork", human_bytes(bytes)));
            }
            Err(e) => self.toast_error("Couldn't clear the artwork cache", e.to_string()),
        }
    }
}

pub fn human_bytes(b: u64) -> String {
    match b {
        0..1_000 => format!("{b} B"),
        1_000..1_000_000 => format!("{:.0} KB", b as f64 / 1e3),
        _ => format!("{:.1} MB", b as f64 / 1e6),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn bytes() {
        assert_eq!(human_bytes(12), "12 B");
        assert_eq!(human_bytes(45_300), "45 KB");
        assert_eq!(human_bytes(2_500_000), "2.5 MB");
    }

    #[test]
    fn details_convert() {
        let d = StoreDetails {
            app_id: 7,
            name: "Seven".into(),
            description: "d".into(),
            developers: vec!["Dev".into()],
            tag_ids: vec![1],
            released: Some(1_651_154_399),
            deck_category: 2,
            header_url: Some("https://x.steamstatic.com/h.jpg".into()),
            capsule_url: None,
        };
        let i = to_app_info(&d, vec!["Indie".into()], Timestamp::UNIX_EPOCH);
        assert_eq!(i.deck, DeckCompat::Playable);
        assert_eq!(i.tags, vec!["Indie".to_string()]);
        assert_eq!(i.released.map(|t| t.as_second()), Some(1_651_154_399));
    }
}
