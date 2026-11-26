//! Steam features in the app: find app ids, ratings, wishlist / library ownership.
//! Network work runs on one background thread at a time; results come back through `poll`.

use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};

use jiff::{SignedDuration, Timestamp};
use keykeeper_core::{GameKey, Ownership, Secret, SteamMeta, SteamRating};
use keykeeper_steam::{AppMatch, MatchOutcome, Review, SteamApi, SteamError, best_match};
use rune::TextEdit;
use uuid::Uuid;

use crate::app::AppState;

/// Ratings older than this are refreshed by "Refresh ratings".
pub const RATING_MAX_AGE_DAYS: i64 = 7;

pub enum JobDone {
    Search {
        key_id: Uuid,
        result: Result<Vec<AppMatch>, SteamError>,
    },
    Match {
        results: Vec<(Uuid, MatchOutcome)>,
        error: Option<SteamError>,
    },
    Ratings {
        result: Result<Vec<Review>, SteamError>,
    },
    Ownership {
        steam_id: Result<u64, SteamError>,
        wishlist: Option<Result<Vec<u32>, SteamError>>,
        owned: Option<Result<Vec<u32>, SteamError>>,
    },
    Prices(crate::price_actions::PricesDone),
}

/// The job currently running, for progress display.
pub struct Job {
    pub label: &'static str,
    pub done: Arc<AtomicUsize>,
    pub total: usize,
}

impl Job {
    pub fn progress(&self) -> String {
        let d = self.done.load(Ordering::Relaxed).min(self.total);
        if self.total > 1 {
            format!("{} {d}/{}…", self.label, self.total)
        } else {
            format!("{}…", self.label)
        }
    }
}

#[derive(Default)]
pub struct SteamUi {
    pub profile: TextEdit,
    pub api_key: TextEdit,
    /// Last ownership check summary.
    pub status: Option<(bool, String)>,
    /// Prices: two-letter country and the IsThereAnyDeal / gg.deals API keys.
    pub country: TextEdit,
    pub itad_key: TextEdit,
    pub gg_key: TextEdit,
    /// Last price fetch summary.
    pub prices_status: Option<(bool, String)>,
}

/// Candidates for one key when a search wasn't conclusive.
pub struct Candidates {
    pub key_id: Uuid,
    pub matches: Vec<AppMatch>,
}

pub type JobSlot = Arc<Mutex<Vec<JobDone>>>;

fn meta_mut(k: &mut GameKey) -> &mut SteamMeta {
    k.steam.get_or_insert_with(|| SteamMeta {
        rating: None,
        ownership: Ownership::Unknown,
        fetched_at: Timestamp::now(),
    })
}

impl AppState {
    pub fn steam_busy(&self) -> bool {
        self.job.is_some()
    }

    pub(crate) fn spawn_job(
        &mut self,
        label: &'static str,
        total: usize,
        work: impl FnOnce(&dyn SteamApi, &AtomicUsize) -> JobDone + Send + 'static,
    ) {
        if self.job.is_some() {
            self.toast("A Steam task is already running");
            return;
        }
        let done = Arc::new(AtomicUsize::new(0));
        let (api, slot, waker, counter) = (
            self.steam.clone(),
            self.jobs.clone(),
            self.waker.clone(),
            done.clone(),
        );
        let spawned = std::thread::Builder::new()
            .name("keykeeper-steam".into())
            .spawn(move || {
                let result = work(api.as_ref(), &counter);
                if let Ok(mut s) = slot.lock() {
                    s.push(result);
                }
                if let Some(w) = waker {
                    w();
                }
            });
        match spawned {
            Ok(_) => self.job = Some(Job { label, done, total }),
            Err(e) => self.toast_error("Couldn't start the Steam task", e.to_string()),
        }
    }

    /// Search Steam for one key's game; an exact match is applied, otherwise candidates are shown.
    pub fn find_on_steam(&mut self, key_id: Uuid) {
        let Some(name) = self
            .session()
            .and_then(|s| s.repo.key(key_id))
            .map(|k| k.game_name.clone())
        else {
            return;
        };
        if let Some(s) = self.session_mut() {
            s.candidates = None;
        }
        self.spawn_job("Searching Steam", 1, move |api, n| {
            let result = api.search(&name);
            n.fetch_add(1, Ordering::Relaxed);
            JobDone::Search { key_id, result }
        });
    }

    /// Match every key in `ids` that has no Steam app id (all such keys when `ids` is empty).
    pub fn match_missing(&mut self, ids: &[Uuid]) {
        let Some(s) = self.session() else { return };
        let todo: Vec<(Uuid, String)> = s
            .repo
            .keys()
            .iter()
            .filter(|k| k.steam_app_id.is_none() && (ids.is_empty() || ids.contains(&k.id)))
            .map(|k| (k.id, k.game_name.clone()))
            .collect();
        if todo.is_empty() {
            self.toast("Every key already has a Steam app id");
            return;
        }
        let total = todo.len();
        self.spawn_job("Matching on Steam", total, move |api, n| {
            let mut results = Vec::new();
            let mut error = None;
            for (id, name) in todo {
                match api.search(&name) {
                    Ok(found) => results.push((id, best_match(&name, &found))),
                    Err(e @ (SteamError::RateLimited | SteamError::Network(_))) => {
                        error = Some(e);
                        break;
                    }
                    Err(_) => results.push((id, MatchOutcome::None)),
                }
                n.fetch_add(1, Ordering::Relaxed);
            }
            JobDone::Match { results, error }
        });
    }

    /// Fetch review scores for keys with an app id; `force` ignores the 7-day cache.
    pub fn refresh_ratings(&mut self, force: bool) {
        let Some(s) = self.session() else { return };
        let stale = Timestamp::now() - SignedDuration::from_hours(24 * RATING_MAX_AGE_DAYS);
        let mut apps: Vec<u32> = s
            .repo
            .keys()
            .iter()
            .filter(|k| {
                force
                    || k.steam
                        .as_ref()
                        .is_none_or(|m| m.rating.is_none() || m.fetched_at < stale)
            })
            .filter_map(|k| k.steam_app_id)
            .collect();
        apps.sort_unstable();
        apps.dedup();
        if apps.is_empty() {
            if force {
                self.toast("No keys have a Steam app id yet — use Match on Steam first");
            }
            return;
        }
        let total = apps.len();
        self.spawn_job("Fetching ratings", total, move |api, n| {
            let result = api.reviews(&apps);
            n.store(total, Ordering::Relaxed);
            JobDone::Ratings { result }
        });
    }

    /// Save the Steam profile / API key from the settings form into the vault.
    pub fn save_steam_settings(&mut self) {
        let Some(s) = self.session_mut() else { return };
        let profile = s.steam_ui.profile.text.trim().to_string();
        let key = s.steam_ui.api_key.text.trim().to_string();
        let mut settings = s.settings().clone();
        settings.steam_id = Some(profile).filter(|p| !p.is_empty());
        s.repo.set_settings(settings);
        let mut api = s.repo.api_keys().clone();
        api.steam = Some(Secret::new(key)).filter(|k| !k.is_empty());
        s.repo.set_api_keys(api);
    }

    /// Resolve the profile, read the public wishlist and (with an API key) the owned games.
    pub fn check_ownership(&mut self) {
        self.save_steam_settings();
        let Some(s) = self.session() else { return };
        let Some(profile) = s.settings().steam_id.clone() else {
            if let Some(s) = self.session_mut() {
                s.steam_ui.status =
                    Some((false, "Enter your SteamID64 or profile link first".into()));
            }
            return;
        };
        let key = s
            .repo
            .api_keys()
            .steam
            .as_ref()
            .map(|k| k.expose().to_string());
        self.spawn_job("Checking wishlist & library", 1, move |api, n| {
            let steam_id = api.resolve_profile(&profile);
            let (wishlist, owned) = match &steam_id {
                Ok(id) => (
                    Some(api.wishlist(*id)),
                    key.map(|k| api.owned_games(&k, *id)),
                ),
                Err(_) => (None, None),
            };
            n.fetch_add(1, Ordering::Relaxed);
            JobDone::Ownership {
                steam_id,
                wishlist,
                owned,
            }
        });
    }

    pub fn choose_candidate(&mut self, key_id: Uuid, app_id: u32) {
        let Some(s) = self.session_mut() else { return };
        s.candidates = None;
        let _ = s.repo.update_key(key_id, |k| k.steam_app_id = Some(app_id));
        if let Some(f) = s.form.as_mut()
            && f.target == Some(key_id)
        {
            f.app_id = TextEdit::new(app_id.to_string());
        }
        self.refresh_ratings(false);
    }

    pub fn dismiss_candidates(&mut self) {
        if let Some(s) = self.session_mut() {
            s.candidates = None;
        }
    }

    /// Apply finished background results. Returns whether anything changed.
    pub fn finish_jobs(&mut self) -> bool {
        let done: Vec<JobDone> = self
            .jobs
            .lock()
            .map(|mut j| std::mem::take(&mut *j))
            .unwrap_or_default();
        if done.is_empty() {
            return false;
        }
        self.job = None;
        for d in done {
            self.apply_job(d);
        }
        true
    }

    fn apply_job(&mut self, done: JobDone) {
        let done = match done {
            JobDone::Prices(p) => return self.apply_prices(p),
            d => d,
        };
        let Some(s) = self.session_mut() else { return };
        let fmt = s.date_format();
        match done {
            JobDone::Search { key_id, result } => match result {
                Err(e) => self.toast_error("Steam search failed", e.to_string()),
                Ok(found) => {
                    let name = s
                        .repo
                        .key(key_id)
                        .map(|k| k.game_name.clone())
                        .unwrap_or_default();
                    match best_match(&name, &found) {
                        MatchOutcome::Exact(m) => {
                            self.choose_candidate(key_id, m.app_id);
                            self.toast(format!("Matched {name} → {}", m.name));
                        }
                        MatchOutcome::Ambiguous(matches) => {
                            s.candidates = Some(Candidates { key_id, matches })
                        }
                        MatchOutcome::None => {
                            self.toast(format!("No Steam results for \"{name}\""))
                        }
                    }
                }
            },
            JobDone::Match { results, error } => {
                let mut keys = Vec::new();
                let mut unsure = 0;
                for (id, outcome) in results {
                    match outcome {
                        MatchOutcome::Exact(m) => {
                            if let Some(k) = s.repo.key(id).filter(|k| k.steam_app_id.is_none()) {
                                let mut k = k.clone();
                                k.steam_app_id = Some(m.app_id);
                                keys.push(k);
                            }
                        }
                        _ => unsure += 1,
                    }
                }
                let matched = keys.len();
                s.repo
                    .apply_changes("Match on Steam", keys, Vec::new(), &[]);
                refresh_open_form(s, fmt);
                let mut msg = format!(
                    "Matched {matched} key{}",
                    if matched == 1 { "" } else { "s" }
                );
                if unsure > 0 {
                    msg.push_str(&format!(
                        ", {unsure} need a manual pick (filter \"No Steam id\")"
                    ));
                }
                match error {
                    Some(e) => self.toast_error(msg, e.to_string()),
                    None => self.toast(msg),
                }
                if matched > 0 {
                    self.refresh_ratings(false);
                }
            }
            JobDone::Ratings { result } => match result {
                Err(e) => self.toast_error("Couldn't fetch ratings", e.to_string()),
                Ok(reviews) => {
                    let now = Timestamp::now();
                    let keys: Vec<GameKey> = s
                        .repo
                        .keys()
                        .iter()
                        .filter_map(|k| {
                            let r = reviews.iter().find(|r| Some(r.app_id) == k.steam_app_id)?;
                            let mut k = k.clone();
                            let m = meta_mut(&mut k);
                            m.rating = Some(SteamRating {
                                desc: r.desc.clone(),
                                percent: r.percent,
                                total: r.total,
                            });
                            m.fetched_at = now;
                            Some(k)
                        })
                        .collect();
                    let n = keys.len();
                    s.repo
                        .apply_changes("Update Steam ratings", keys, Vec::new(), &[]);
                    self.toast(format!(
                        "Updated ratings for {n} key{}",
                        if n == 1 { "" } else { "s" }
                    ));
                }
            },
            JobDone::Ownership {
                steam_id,
                wishlist,
                owned,
            } => {
                let id = match steam_id {
                    Ok(id) => id,
                    Err(e) => {
                        s.steam_ui.status = Some((false, e.to_string()));
                        return;
                    }
                };
                let wish = match wishlist {
                    Some(Ok(w)) => w,
                    Some(Err(e)) => {
                        s.steam_ui.status = Some((false, format!("Wishlist: {e}")));
                        return;
                    }
                    None => Vec::new(),
                };
                let (owned, owned_note) = match owned {
                    Some(Ok(o)) => (Some(o), None),
                    Some(Err(e)) => (None, Some(e.to_string())),
                    None => (None, None),
                };
                let keys: Vec<GameKey> = s
                    .repo
                    .keys()
                    .iter()
                    .filter_map(|k| {
                        let app = k.steam_app_id?;
                        let state = match &owned {
                            Some(o) if o.contains(&app) => Ownership::Owned,
                            _ if wish.contains(&app) => Ownership::Wishlisted,
                            Some(_) => Ownership::NotOwned,
                            None => Ownership::Unknown,
                        };
                        let mut k = k.clone();
                        meta_mut(&mut k).ownership = state;
                        Some(k)
                    })
                    .collect();
                let (n_owned, n_wish) = (
                    keys.iter()
                        .filter(|k| {
                            k.steam
                                .as_ref()
                                .is_some_and(|m| m.ownership == Ownership::Owned)
                        })
                        .count(),
                    keys.iter()
                        .filter(|k| {
                            k.steam
                                .as_ref()
                                .is_some_and(|m| m.ownership == Ownership::Wishlisted)
                        })
                        .count(),
                );
                s.repo
                    .apply_changes("Check Steam wishlist & library", keys, Vec::new(), &[]);
                let mut msg = format!("SteamID {id}: {n_wish} of your keys are on your wishlist");
                match (&owned, owned_note) {
                    (Some(_), _) => msg.push_str(&format!(", {n_owned} you already own")),
                    (None, Some(e)) => msg.push_str(&format!(". Library check failed: {e}")),
                    (None, None) => msg.push_str(". Add a Web API key to also check your library."),
                }
                if wish.is_empty() {
                    msg.push_str(" (Is your wishlist public?)");
                }
                s.steam_ui.status = Some((true, msg));
            }
            JobDone::Prices(_) => {}
        }
    }
}

fn refresh_open_form(s: &mut crate::app::Session, fmt: keykeeper_core::DateFormat) {
    if let Some(f) = &s.form
        && let Some(id) = f.target
    {
        s.form = s
            .repo
            .key(id)
            .map(|k| crate::form::KeyForm::from_key(k, fmt));
    }
}
