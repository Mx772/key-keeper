//! Steam features in the app: find app ids, ratings, wishlist / library ownership.
//! Network work runs on one background thread at a time; results come back through `poll`.

use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};

use jiff::{SignedDuration, Timestamp};
use keykeeper_core::{GameKey, Ownership, Secret, SteamMeta, SteamRating};
use keykeeper_steam::{
    AppMatch, MatchOutcome, Partial, Progress, Review, SteamApi, SteamError, best_match,
};
use rune::TextEdit;
use uuid::Uuid;

use crate::app::AppState;

/// Ratings older than this are refreshed by "Refresh ratings".
pub const RATING_MAX_AGE_DAYS: i64 = 7;
/// A library-wide "Match on Steam" skips keys searched this recently without an exact match.
pub const MATCH_RETRY_DAYS: i64 = 14;

/// Keys "Match on Steam" couldn't match exactly, for the user to pick the right game.
pub struct MatchReview {
    pub rows: Vec<ReviewRow>,
    /// The row whose dropdown is open (it uses `AppState::match_combo`).
    pub open: Option<usize>,
}

pub struct ReviewRow {
    pub key_ids: Vec<Uuid>,
    pub name: String,
    /// What to search Steam for, starting with the key's name.
    pub term: TextEdit,
    pub matches: Vec<AppMatch>,
    pub chosen: Option<u32>,
}

pub enum JobDone {
    Search {
        key_id: Uuid,
        result: Result<Vec<AppMatch>, SteamError>,
    },
    Match {
        results: Vec<(Vec<Uuid>, String, MatchOutcome)>,
        error: Option<SteamError>,
        skipped: usize,
    },
    /// A search from the match review: row `row`, for `term`.
    ReviewSearch {
        row: usize,
        term: String,
        result: Result<Vec<AppMatch>, SteamError>,
    },
    Ratings {
        result: Result<Partial<Review>, SteamError>,
    },
    Ownership {
        steam_id: Result<u64, SteamError>,
        wishlist: Option<Result<Vec<u32>, SteamError>>,
        owned: Option<Result<Vec<u32>, SteamError>>,
    },
    Prices(crate::price_actions::PricesDone),
    /// Several results from one job, applied in order.
    Many(Vec<JobDone>),
}

/// A running job's progress, shared with its thread: items done and what it's doing now.
#[derive(Default)]
pub struct JobProgress {
    done: AtomicUsize,
    note: Mutex<Option<String>>,
}

impl JobProgress {
    pub fn done(&self) -> usize {
        self.done.load(Ordering::Relaxed)
    }

    /// Start counting again (a job with several passes over the same items).
    pub fn restart(&self) {
        self.done.store(0, Ordering::Relaxed);
    }

    pub fn current_note(&self) -> Option<String> {
        self.note.lock().ok()?.clone()
    }
}

impl Progress for JobProgress {
    fn advance(&self, n: usize) {
        self.done.fetch_add(n, Ordering::Relaxed);
    }

    fn note(&self, what: &str) {
        if let Ok(mut n) = self.note.lock() {
            *n = Some(what.to_string());
        }
    }
}

/// The job currently running, for progress display.
pub struct Job {
    pub label: &'static str,
    pub progress: Arc<JobProgress>,
    pub total: usize,
}

impl Job {
    pub fn progress(&self) -> String {
        let d = self.progress.done().min(self.total);
        let mut out = if self.total > 1 {
            format!("{} {d}/{}", self.label, self.total)
        } else {
            self.label.to_string()
        };
        if let Some(n) = self.progress.current_note() {
            out.push_str(" · ");
            out.push_str(&n);
        }
        out.push('…');
        out
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
        work: impl FnOnce(&dyn SteamApi, &JobProgress) -> JobDone + Send + 'static,
    ) {
        if self.job.is_some() {
            self.toast("A Steam task is already running");
            return;
        }
        let progress = Arc::new(JobProgress::default());
        let (api, slot, waker, counter) = (
            self.steam.clone(),
            self.jobs.clone(),
            self.waker.clone(),
            progress.clone(),
        );
        log::info!(
            "{label}: started ({total} item{})",
            if total == 1 { "" } else { "s" }
        );
        let spawned = std::thread::Builder::new()
            .name("keykeeper-steam".into())
            .spawn(move || {
                let started = std::time::Instant::now();
                let result = work(api.as_ref(), &counter);
                log::info!(
                    "{label}: finished in {:.1} s ({} done)",
                    started.elapsed().as_secs_f64(),
                    counter.done()
                );
                if let Ok(mut s) = slot.lock() {
                    s.push(result);
                }
                if let Some(w) = waker {
                    w();
                }
            });
        match spawned {
            Ok(_) => {
                self.job = Some(Job {
                    label,
                    progress,
                    total,
                })
            }
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
            n.advance(1);
            JobDone::Search { key_id, result }
        });
    }

    /// Match every key in `ids` that has no Steam app id (all such keys when `ids` is empty).
    /// Keys with the same game name share one search. Library-wide, keys searched in the last
    /// [`MATCH_RETRY_DAYS`] without an exact match are skipped; picking them retries them.
    pub fn match_missing(&mut self, ids: &[Uuid]) {
        let Some(s) = self.session() else { return };
        let recent = Timestamp::now() - SignedDuration::from_hours(24 * MATCH_RETRY_DAYS);
        let mut skipped = 0;
        let mut todo: Vec<(Vec<Uuid>, String)> = Vec::new();
        let mut by_title: std::collections::HashMap<String, usize> = Default::default();
        for k in s.repo.keys() {
            if k.steam_app_id.is_some() || !(ids.is_empty() || ids.contains(&k.id)) {
                continue;
            }
            if ids.is_empty() && k.steam_searched_at.is_some_and(|t| t > recent) {
                skipped += 1;
                continue;
            }
            let norm = keykeeper_steam::normalize_title(&k.game_name);
            match by_title.get(&norm) {
                Some(i) => todo[*i].0.push(k.id),
                None => {
                    by_title.insert(norm, todo.len());
                    todo.push((vec![k.id], k.game_name.clone()));
                }
            }
        }
        if todo.is_empty() {
            if skipped > 0 {
                self.toast_error(
                    format!("Skipped {skipped} keys searched recently without an exact match"),
                    "Select them in the library and use Match on Steam to search again.",
                );
            } else {
                self.toast("Every key already has a Steam app id");
            }
            return;
        }
        let total = todo.len();
        self.spawn_job("Matching on Steam", total, move |api, n| {
            let mut results = Vec::new();
            let mut error = None;
            for (ids, name) in todo {
                match api.search(&name) {
                    Ok(found) => results.push((ids, name.clone(), best_match(&name, &found))),
                    Err(e @ (SteamError::RateLimited | SteamError::Network(_))) => {
                        error = Some(e);
                        break;
                    }
                    Err(e) => {
                        log::warn!("search for a key failed: {e}");
                        results.push((ids, name, MatchOutcome::None));
                    }
                }
                n.advance(1);
            }
            JobDone::Match {
                results,
                error,
                skipped,
            }
        });
    }

    /// Search Steam again for a review row, with its (edited) search term.
    pub fn review_search(&mut self, row: usize) {
        let Some(term) = self
            .session()
            .and_then(|s| s.match_review.as_ref())
            .and_then(|r| r.rows.get(row))
            .map(|r| r.term.text.trim().to_string())
            .filter(|t| !t.is_empty())
        else {
            return;
        };
        self.match_combo.close();
        if let Some(r) = self.session_mut().and_then(|s| s.match_review.as_mut()) {
            r.open = None;
        }
        self.spawn_job("Searching Steam", 1, move |api, n| {
            let result = api.search(&term);
            n.advance(1);
            JobDone::ReviewSearch { row, term, result }
        });
    }

    pub fn review_choose(&mut self, row: usize, choice: &str) {
        self.match_combo.close();
        if let Some(r) = self.session_mut().and_then(|s| s.match_review.as_mut()) {
            r.open = None;
            if let Some(x) = r.rows.get_mut(row) {
                x.chosen = choice.parse().ok();
            }
        }
    }

    /// The combobox state for review row `row`, making it the open one.
    pub fn review_combo(&mut self, row: usize) -> &mut rune_kit::ComboboxState {
        let switched = match self.session_mut().and_then(|s| s.match_review.as_mut()) {
            Some(r) if r.open != Some(row) => {
                r.open = Some(row);
                true
            }
            _ => false,
        };
        if switched {
            self.match_combo.close();
        }
        &mut self.match_combo
    }

    /// Give every row with a chosen game its app id, as one undo step.
    pub fn apply_review(&mut self) {
        self.match_combo.close();
        let Some(s) = self.session_mut() else { return };
        let Some(review) = s.match_review.take() else {
            return;
        };
        let fmt = s.date_format();
        let mut keys = Vec::new();
        for row in &review.rows {
            let Some(app) = row.chosen else { continue };
            for id in &row.key_ids {
                if let Some(k) = s.repo.key(*id) {
                    let mut k = k.clone();
                    k.steam_app_id = Some(app);
                    k.steam_searched_at = None;
                    keys.push(k);
                }
            }
        }
        let n = keys.len();
        if n == 0 {
            return;
        }
        s.repo
            .apply_changes("Match on Steam (picked)", keys, Vec::new(), &[]);
        refresh_open_form(s, fmt);
        self.toast(format!("Matched {n} key{}", if n == 1 { "" } else { "s" }));
        self.refresh_ratings(false);
    }

    pub fn close_review(&mut self) {
        self.match_combo.close();
        if let Some(s) = self.session_mut() {
            s.match_review = None;
        }
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
            let result = api.reviews(&apps, n);
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
            n.advance(1);
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
            JobDone::Many(list) => {
                for d in list {
                    self.apply_job(d);
                }
                return;
            }
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
            JobDone::Match {
                results,
                error,
                skipped,
            } => {
                let now = Timestamp::now();
                let mut keys = Vec::new();
                let mut matched = 0;
                let mut rows = Vec::new();
                for (ids, name, outcome) in results {
                    let (app, matches) = match outcome {
                        MatchOutcome::Exact(m) => (Some(m.app_id), Vec::new()),
                        MatchOutcome::Ambiguous(m) => (None, m),
                        MatchOutcome::None => (None, Vec::new()),
                    };
                    let mut row_ids = Vec::new();
                    for id in &ids {
                        let Some(k) = s.repo.key(*id).filter(|k| k.steam_app_id.is_none()) else {
                            continue;
                        };
                        let mut k = k.clone();
                        match app {
                            Some(a) => {
                                k.steam_app_id = Some(a);
                                k.steam_searched_at = None;
                                matched += 1;
                            }
                            None => {
                                k.steam_searched_at = Some(now);
                                row_ids.push(k.id);
                            }
                        }
                        keys.push(k);
                    }
                    if app.is_none() && !row_ids.is_empty() {
                        rows.push(ReviewRow {
                            key_ids: row_ids,
                            term: TextEdit::new(name.clone()),
                            name,
                            chosen: None,
                            matches,
                        });
                    }
                }
                let unsure = rows.iter().map(|r| r.key_ids.len()).sum::<usize>();
                s.repo
                    .apply_changes("Match on Steam", keys, Vec::new(), &[]);
                refresh_open_form(s, fmt);
                log::info!(
                    "Match on Steam: {matched} matched, {unsure} to pick, {skipped} skipped"
                );
                let mut msg = format!(
                    "Matched {matched} key{}",
                    if matched == 1 { "" } else { "s" }
                );
                if unsure > 0 {
                    msg.push_str(&format!(", {unsure} to pick by hand"));
                }
                if skipped > 0 {
                    msg.push_str(&format!(", {skipped} skipped (searched recently)"));
                }
                if !rows.is_empty() {
                    s.match_review = Some(MatchReview { rows, open: None });
                }
                match error {
                    Some(e) => self.toast_error(
                        msg,
                        format!("Stopped early: {e}. Run Match on Steam again later for the rest."),
                    ),
                    None => self.toast(msg),
                }
                if matched > 0 {
                    self.refresh_ratings(false);
                }
            }
            JobDone::ReviewSearch { row, term, result } => match result {
                Err(e) => self.toast_error("Steam search failed", e.to_string()),
                Ok(found) => {
                    if let Some(r) = s.match_review.as_mut().and_then(|r| r.rows.get_mut(row)) {
                        r.chosen = match best_match(&term, &found) {
                            MatchOutcome::Exact(m) => Some(m.app_id),
                            _ => None,
                        };
                        r.matches = found;
                        if r.matches.is_empty() {
                            self.toast(format!("No Steam results for \"{term}\""));
                        }
                    }
                }
            },
            JobDone::Ratings { result } => match result {
                Err(e) => self.toast_error("Couldn't fetch ratings", e.to_string()),
                Ok(Partial {
                    items: reviews,
                    error,
                }) => {
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
                    let msg = format!(
                        "Updated ratings for {n} key{}",
                        if n == 1 { "" } else { "s" }
                    );
                    match error {
                        // Keys not reached stay stale, so the next refresh fetches them.
                        Some(e) => self.toast_error(
                            msg,
                            format!("Stopped early: {e}. Refresh ratings later for the rest."),
                        ),
                        None => self.toast(msg),
                    }
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
            JobDone::Prices(_) | JobDone::Many(_) => {}
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
