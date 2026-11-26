//! Update checks and installs in the app: background checks, the banner's actions, and the
//! save-download-verify-install sequence. Network work runs on a background thread and comes
//! back through `poll`, like the Steam jobs.

use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use rune::Tx;
use semver::Version;

use crate::app::{AppState, Screen};
use crate::update::{
    CHECK_INTERVAL_SECS, Release, STARTUP_DELAY_SECS, UpdateError, UpdateSource, current_version,
    should_offer,
};

/// What the Settings › Updates card says.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum CheckStatus {
    NotChecked,
    Checking,
    UpToDate,
    /// A newer version exists (it may be hidden by Later or Skip).
    Available(Version),
    /// The newest version is one the user skipped.
    Skipped(Version),
    /// Not an error: e.g. nothing published yet.
    Note(String),
    Failed(String),
}

/// Download / install progress, shown in the banner.
#[derive(Clone, Debug)]
pub enum Phase {
    Idle,
    Downloading {
        done: Arc<AtomicU64>,
        total: Arc<AtomicU64>,
    },
    Installing,
    Failed(String),
}

pub enum UpdateDone {
    Checked {
        manual: bool,
        result: Result<Release, UpdateError>,
    },
    Downloaded(Result<Vec<u8>, UpdateError>),
}

pub struct Updates {
    pub source: Arc<dyn UpdateSource>,
    pub current: Version,
    pub status: CheckStatus,
    /// The release on offer. The banner shows it unless `later` is set.
    pub available: Option<Release>,
    /// "Later": hidden until the next launch (or a manual check).
    pub later: bool,
    pub notes_open: bool,
    pub phase: Phase,
    /// App-clock time of the next automatic check.
    pub next_check: f64,
    pub checking: bool,
    slot: Arc<Mutex<Vec<UpdateDone>>>,
}

impl Updates {
    pub fn new(source: Arc<dyn UpdateSource>) -> Updates {
        Updates {
            source,
            current: current_version(),
            status: CheckStatus::NotChecked,
            available: None,
            later: false,
            notes_open: false,
            phase: Phase::Idle,
            next_check: STARTUP_DELAY_SECS,
            checking: false,
            slot: Arc::new(Mutex::new(Vec::new())),
        }
    }

    pub fn banner_visible(&self) -> bool {
        !matches!(self.phase, Phase::Idle) || (self.available.is_some() && !self.later)
    }

    /// `0.0..=1.0` while downloading, when the size is known.
    pub fn progress(&self) -> Option<(u64, Option<u64>)> {
        match &self.phase {
            Phase::Downloading { done, total } => {
                let t = total.load(Ordering::Relaxed);
                Some((done.load(Ordering::Relaxed), (t > 0).then_some(t)))
            }
            _ => None,
        }
    }
}

impl AppState {
    fn spawn_update_work(
        &mut self,
        name: &str,
        work: impl FnOnce(&dyn UpdateSource) -> UpdateDone + Send + 'static,
    ) -> bool {
        let (source, slot, waker) = (
            self.updates.source.clone(),
            self.updates.slot.clone(),
            self.waker.clone(),
        );
        std::thread::Builder::new()
            .name(name.into())
            .spawn(move || {
                let done = work(source.as_ref());
                if let Ok(mut s) = slot.lock() {
                    s.push(done);
                }
                if let Some(w) = waker {
                    w();
                }
            })
            .is_ok()
    }

    /// Ask for the latest release in the background. `manual` ("Check now") reports errors and
    /// shows a newer version even if it was skipped or put off with Later.
    pub fn check_for_updates(&mut self, manual: bool) {
        if self.updates.checking {
            return;
        }
        if manual {
            self.updates.status = CheckStatus::Checking;
        }
        let spawned =
            self.spawn_update_work("keykeeper-update-check", move |src| UpdateDone::Checked {
                manual,
                result: src.latest(),
            });
        if spawned {
            self.updates.checking = true;
        } else if manual {
            self.updates.status = CheckStatus::Failed("Couldn't start the update check".into());
        }
    }

    pub fn set_auto_update_check(&mut self, on: bool) {
        self.config.check_updates = Some(on);
        self.save_config();
        if on {
            self.updates.next_check = self.now + STARTUP_DELAY_SECS;
        }
    }

    /// "Later": hide the banner until the next launch.
    pub fn update_later(&mut self) {
        self.updates.later = true;
        self.updates.notes_open = false;
    }

    /// "Skip this version": never offer it automatically again.
    pub fn skip_update(&mut self) {
        let Some(r) = self.updates.available.take() else {
            return;
        };
        self.config.skipped_version = Some(r.version.to_string());
        self.save_config();
        self.updates.notes_open = false;
        self.updates.status = CheckStatus::Skipped(r.version);
    }

    pub fn show_update_notes(&mut self, open: bool) {
        self.updates.notes_open = open && self.updates.available.is_some();
    }

    pub fn dismiss_update_error(&mut self) {
        if matches!(self.updates.phase, Phase::Failed(_)) {
            self.updates.phase = Phase::Idle;
        }
    }

    /// Save unsaved changes (the same path as locking). The vault must be on disk before an
    /// installer replaces the app.
    fn flush_for_update(&mut self) -> Result<(), String> {
        if let Screen::Unlocked(s) = &mut self.screen {
            s.sync_trade_form();
            if s.is_dirty() && s.save().is_err() {
                return Err(s.save_error.clone().unwrap_or_default());
            }
        }
        Ok(())
    }

    /// "Update": save, then download and verify in the background; `poll` installs.
    pub fn start_update(&mut self) {
        if matches!(
            self.updates.phase,
            Phase::Downloading { .. } | Phase::Installing
        ) {
            return;
        }
        let Some(release) = self.updates.available.clone() else {
            return;
        };
        self.updates.notes_open = false;
        if let Err(e) = self.flush_for_update() {
            self.updates.phase = Phase::Failed(format!(
                "Your vault couldn't be saved, so the update was not started: {e}"
            ));
            return;
        }
        let (done, total) = (Arc::new(AtomicU64::new(0)), Arc::new(AtomicU64::new(0)));
        let (d, t) = (done.clone(), total.clone());
        let spawned = self.spawn_update_work("keykeeper-update-download", move |src| {
            let progress = move |n: u64, of: Option<u64>| {
                d.store(n, Ordering::Relaxed);
                t.store(of.unwrap_or(0), Ordering::Relaxed);
            };
            UpdateDone::Downloaded(src.download(&release, &progress))
        });
        self.updates.phase = if spawned {
            Phase::Downloading { done, total }
        } else {
            Phase::Failed("Couldn't start the download".into())
        };
    }

    /// Background results, the automatic check timer, and progress redraws.
    pub(crate) fn poll_updates(&mut self, now: f64, tx: &mut Tx) -> bool {
        let done: Vec<UpdateDone> = self
            .updates
            .slot
            .lock()
            .map(|mut j| std::mem::take(&mut *j))
            .unwrap_or_default();
        let mut changed = !done.is_empty();
        for d in done {
            match d {
                UpdateDone::Checked { manual, result } => self.finish_check(manual, result),
                UpdateDone::Downloaded(result) => self.finish_download(result, tx),
            }
        }
        if self.config.check_updates() {
            if now >= self.updates.next_check && !self.updates.checking {
                self.updates.next_check = now + CHECK_INTERVAL_SECS;
                self.check_for_updates(false);
            }
            tx.wake_after(Duration::from_secs_f64(
                (self.updates.next_check - now).max(0.0),
            ));
        }
        if matches!(self.updates.phase, Phase::Downloading { .. }) {
            tx.wake_after(Duration::from_millis(250));
            changed = true; // progress
        }
        changed
    }

    fn finish_check(&mut self, manual: bool, result: Result<Release, UpdateError>) {
        self.updates.checking = false;
        let u = &mut self.updates;
        match result {
            Ok(release) => {
                let skipped = if manual {
                    None
                } else {
                    self.config.skipped_version.as_deref()
                };
                if should_offer(&u.current, &release.version, skipped) {
                    u.status = CheckStatus::Available(release.version.clone());
                    if manual {
                        u.later = false;
                    }
                    u.available = Some(release);
                } else if should_offer(&u.current, &release.version, None) {
                    u.status = CheckStatus::Skipped(release.version);
                } else {
                    u.status = CheckStatus::UpToDate;
                }
            }
            Err(e) => {
                if manual {
                    u.status = match e {
                        UpdateError::NotAvailable(m) => CheckStatus::Note(m),
                        e => CheckStatus::Failed(e.to_string()),
                    };
                } else {
                    // Automatic checks fail quietly; the next one is in a day.
                    eprintln!("keykeeper: update check: {e}");
                }
            }
        }
    }

    fn finish_download(&mut self, result: Result<Vec<u8>, UpdateError>, tx: &mut Tx) {
        let bytes = match result {
            Ok(b) => b,
            Err(e) => {
                self.updates.phase = Phase::Failed(e.to_string());
                return;
            }
        };
        let Some(release) = self.updates.available.clone() else {
            self.updates.phase = Phase::Idle;
            return;
        };
        // Edits made during the download must reach disk too.
        if let Err(e) = self.flush_for_update() {
            self.updates.phase = Phase::Failed(format!(
                "Your vault couldn't be saved, so the update was not installed: {e}"
            ));
            return;
        }
        tx.clear_secret_clipboard();
        self.updates.phase = Phase::Installing;
        // Windows: starts the installer and exits this process.
        match self.updates.source.install(&release, bytes) {
            Ok(()) => {
                if cfg!(target_os = "macos") {
                    tx.relaunch(); // the new bundle is in place
                } else {
                    tx.quit();
                }
            }
            Err(e) => self.updates.phase = Phase::Failed(e.to_string()),
        }
    }
}
