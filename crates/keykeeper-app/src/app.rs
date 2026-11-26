//! Application state and every action the UI can take. Views only read this and call these
//! methods, so behaviour is testable headlessly (see `tests.rs`).

use std::path::PathBuf;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use jiff::civil::Date;
use keykeeper_core::import::{self, Field, ImportOptions, ImportPreview, Table};
use keykeeper_core::{
    DateFormat, KdfParams, KeyStatus, Query, Repo, Settings, Sort, SortDir, SortField, Vault,
    VaultData, VaultError, export,
};
use rune::{FileDialogOptions, KeyEvent, TextEdit, Tx};
use rune_kit::TableState;
use uuid::Uuid;
use zeroize::Zeroizing;

use crate::config::Config;
use crate::form::KeyForm;
use crate::steam_actions::{Candidates, Job, JobSlot, SteamUi};
use crate::theme::ThemeId;
use crate::trade_actions::{TemplatesUi, TradeForm};
use crate::update_actions::Updates;
use keykeeper_steam::SteamApi;

/// Unsaved changes reach disk this long after the last edit.
pub const AUTOSAVE_DELAY: f64 = 1.5;
const TOAST_SECS: f64 = 3.5;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Page {
    Library,
    Cart,
    Trades,
    Import,
    Settings,
}

impl Page {
    pub const ALL: [Page; 5] = [
        Page::Library,
        Page::Cart,
        Page::Trades,
        Page::Import,
        Page::Settings,
    ];
    pub fn label(self) -> &'static str {
        match self {
            Page::Library => "Library",
            Page::Cart => "Cart",
            Page::Trades => "Trades",
            Page::Import => "Import",
            Page::Settings => "Settings",
        }
    }
    pub fn slug(self) -> &'static str {
        match self {
            Page::Library => "library",
            Page::Cart => "cart",
            Page::Trades => "trades",
            Page::Import => "import",
            Page::Settings => "settings",
        }
    }
}

#[derive(Clone, Debug)]
pub struct Toast {
    pub id: u64,
    pub title: String,
    pub detail: Option<String>,
    pub error: bool,
    pub until: f64,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum LockMode {
    Unlock,
    Create,
}

pub struct LockUi {
    pub mode: LockMode,
    pub path: PathBuf,
    pub password: TextEdit,
    pub confirm: TextEdit,
    pub error: Option<String>,
    pub busy: bool,
}

impl LockUi {
    pub fn for_path(path: PathBuf) -> LockUi {
        let mode = if path.exists() {
            LockMode::Unlock
        } else {
            LockMode::Create
        };
        LockUi {
            mode,
            path,
            password: TextEdit::default(),
            confirm: TextEdit::default(),
            error: None,
            busy: false,
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Confirm {
    DeleteKeys(Vec<Uuid>),
    ExportPlain(ExportKind),
    CancelTrade(Uuid),
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ExportKind {
    Csv,
    Json,
}

pub struct ImportUi {
    pub paste: TextEdit,
    pub source: Option<String>,
    pub table: Option<Table>,
    pub preview: Option<ImportPreview>,
    pub date_format: DateFormat,
    pub include_duplicates: bool,
    /// Column whose mapping dropdown is open.
    pub open_column: Option<usize>,
    pub date_select_open: bool,
}

impl ImportUi {
    fn new(date_format: DateFormat) -> ImportUi {
        ImportUi {
            paste: TextEdit::default(),
            source: None,
            table: None,
            preview: None,
            date_format,
            include_duplicates: false,
            open_column: None,
            date_select_open: false,
        }
    }
}

#[derive(Default)]
pub struct SettingsUi {
    pub old_pw: TextEdit,
    pub new_pw: TextEdit,
    pub confirm_pw: TextEdit,
    /// `(ok, message)` under the password form.
    pub pw_message: Option<(bool, String)>,
    pub open_select: Option<&'static str>,
}

pub struct Session {
    pub vault: Vault,
    pub repo: Repo,
    pub page: Page,
    // library view
    pub search: TextEdit,
    pub statuses: Vec<KeyStatus>,
    pub hide_gone: bool,
    pub deadline_soon: bool,
    pub sort: Sort,
    pub query_rev: u64,
    pub rows: Vec<Uuid>,
    pub row_ids: Vec<String>,
    rows_stamp: Option<(u64, u64)>,
    pub table: TableState,
    pub form: Option<KeyForm>,
    pub confirm: Option<Confirm>,
    pub bulk_status_open: bool,
    /// The keyboard shortcuts overlay.
    pub shortcuts_open: bool,
    // other pages
    pub import: ImportUi,
    pub settings_ui: SettingsUi,
    pub trade_form: Option<TradeForm>,
    pub selected_trade: Option<Uuid>,
    pub templates_ui: TemplatesUi,
    pub candidates: Option<Candidates>,
    pub steam_ui: SteamUi,
    pub missing_app_id: bool,
    // persistence
    seen_gen: u64,
    dirty_since: Option<f64>,
    pub save_error: Option<String>,
}

impl Session {
    pub fn new(vault: Vault, data: VaultData) -> Session {
        let date_format = data.settings.date_format;
        let templates = data.templates.clone();
        let steam_ui = SteamUi {
            profile: TextEdit::new(data.settings.steam_id.clone().unwrap_or_default()),
            api_key: TextEdit::new(
                data.api_keys
                    .steam
                    .as_ref()
                    .map(|k| k.expose().to_string())
                    .unwrap_or_default(),
            ),
            ..SteamUi::default()
        };
        let repo = Repo::new(data);
        let gen_ = repo.generation();
        Session {
            vault,
            repo,
            page: Page::Library,
            search: TextEdit::default(),
            statuses: Vec::new(),
            hide_gone: false,
            deadline_soon: false,
            sort: Sort::default(),
            query_rev: 0,
            rows: Vec::new(),
            row_ids: Vec::new(),
            rows_stamp: None,
            table: TableState::new().key("library-table").multi_select(true),
            form: None,
            confirm: None,
            bulk_status_open: false,
            shortcuts_open: false,
            import: ImportUi::new(date_format),
            settings_ui: SettingsUi::default(),
            trade_form: None,
            selected_trade: None,
            templates_ui: TemplatesUi::from(&templates),
            candidates: None,
            steam_ui,
            missing_app_id: false,
            seen_gen: gen_,
            dirty_since: None,
            save_error: None,
        }
    }

    pub fn settings(&self) -> &Settings {
        self.repo.settings()
    }

    pub fn date_format(&self) -> DateFormat {
        self.repo.settings().date_format
    }

    pub fn query(&self) -> Query {
        let statuses = if self.statuses.is_empty() && self.hide_gone {
            KeyStatus::ALL
                .into_iter()
                .filter(|s| !s.is_gone())
                .collect()
        } else if self.hide_gone {
            self.statuses
                .iter()
                .copied()
                .filter(|s| !s.is_gone())
                .collect()
        } else {
            self.statuses.clone()
        };
        Query {
            text: self.search.text.clone(),
            statuses,
            deadline_within_days: self.deadline_soon.then_some(30),
            missing_app_id: self.missing_app_id,
            sort: self.sort,
            ..Query::default()
        }
    }

    /// Recompute the visible rows if data or filters changed. Returns whether they did.
    pub fn refresh_rows(&mut self, today: Date) -> bool {
        let stamp = (self.repo.generation(), self.query_rev);
        if self.rows_stamp == Some(stamp) {
            return false;
        }
        self.rows = self.repo.query(&self.query(), today);
        self.row_ids = self.rows.iter().map(Uuid::to_string).collect();
        self.rows_stamp = Some(stamp);
        true
    }

    pub fn filters_changed(&mut self) {
        self.query_rev += 1;
    }

    pub fn selected_ids(&self) -> Vec<Uuid> {
        self.table
            .selected()
            .filter_map(|s| Uuid::parse_str(s).ok())
            .filter(|id| self.repo.key(*id).is_some())
            .collect()
    }

    pub fn is_dirty(&self) -> bool {
        self.repo.is_dirty()
    }

    /// Encrypt and write now.
    pub fn save(&mut self) -> Result<(), VaultError> {
        let gen_ = self.repo.generation();
        match self.vault.save(self.repo.data()) {
            Ok(()) => {
                self.repo.mark_saved(gen_);
                self.dirty_since = None;
                self.save_error = None;
                Ok(())
            }
            Err(e) => {
                self.save_error = Some(e.to_string());
                Err(e)
            }
        }
    }
}

pub enum Screen {
    Locked(Box<LockUi>),
    Unlocked(Box<Session>),
}

/// The opened vault, its data, and the pre-upgrade backup written on the way (if any).
type UnlockResult = Result<(Vault, VaultData, Option<PathBuf>), VaultError>;

pub struct AppState {
    pub config: Config,
    pub config_path: Option<PathBuf>,
    pub theme: ThemeId,
    pub kdf: KdfParams,
    pub screen: Screen,
    pub toasts: Vec<Toast>,
    next_toast: u64,
    pub now: f64,
    pub last_activity: f64,
    pending: Arc<Mutex<Option<UnlockResult>>>,
    pub waker: Option<rune::Waker>,
    /// Fallback target for text-field accessors when their screen isn't showing.
    pub scratch: TextEdit,
    pub scratch_table: TableState,
    /// Steam client (a fake in tests).
    pub steam: Arc<dyn SteamApi>,
    pub(crate) jobs: JobSlot,
    pub job: Option<Job>,
    /// Update checks and installs (the source is a fake in tests).
    pub updates: Updates,
}

pub fn today() -> Date {
    jiff::ZonedDateTime::now().date()
}

impl AppState {
    pub fn new(config: Config, config_path: Option<PathBuf>, kdf: KdfParams) -> AppState {
        let theme = config.theme();
        let lock = LockUi::for_path(config.vault_path());
        AppState {
            config,
            config_path,
            theme,
            kdf,
            screen: Screen::Locked(Box::new(lock)),
            toasts: Vec::new(),
            next_toast: 0,
            now: 0.0,
            last_activity: 0.0,
            pending: Arc::new(Mutex::new(None)),
            waker: None,
            scratch: TextEdit::default(),
            scratch_table: TableState::new(),
            steam: Arc::new(keykeeper_steam::Steam::new(keykeeper_steam::UreqHttp::new())),
            jobs: Arc::new(Mutex::new(Vec::new())),
            job: None,
            updates: Updates::new(crate::update::real_source()),
        }
    }

    /// The real app: config from the user's config dir, real Argon2 cost.
    pub fn load() -> AppState {
        let path = Config::default_path();
        let config = path.as_deref().map(Config::load).unwrap_or_default();
        AppState::new(config, path, KdfParams::DEFAULT)
    }

    pub fn session(&self) -> Option<&Session> {
        match &self.screen {
            Screen::Unlocked(s) => Some(s),
            Screen::Locked(_) => None,
        }
    }

    pub fn session_mut(&mut self) -> Option<&mut Session> {
        match &mut self.screen {
            Screen::Unlocked(s) => Some(s),
            Screen::Locked(_) => None,
        }
    }

    pub fn lock_ui_mut(&mut self) -> Option<&mut LockUi> {
        match &mut self.screen {
            Screen::Locked(l) => Some(&mut **l),
            Screen::Unlocked(_) => None,
        }
    }

    pub fn is_unlocked(&self) -> bool {
        matches!(self.screen, Screen::Unlocked(_))
    }

    pub(crate) fn save_config(&mut self) {
        if let Some(p) = &self.config_path
            && let Err(e) = self.config.save(p)
        {
            self.toast_error("Couldn't save preferences", e.to_string());
        }
    }

    // ── toasts ──

    pub fn toast(&mut self, title: impl Into<String>) {
        self.push_toast(title.into(), None, false);
    }

    pub fn toast_detail(&mut self, title: impl Into<String>, detail: impl Into<String>) {
        self.push_toast(title.into(), Some(detail.into()), false);
    }

    pub fn toast_error(&mut self, title: impl Into<String>, detail: impl Into<String>) {
        self.push_toast(title.into(), Some(detail.into()), true);
    }

    fn push_toast(&mut self, title: String, detail: Option<String>, error: bool) {
        self.next_toast += 1;
        let until = self.now + if error { TOAST_SECS * 2.0 } else { TOAST_SECS };
        self.toasts.push(Toast {
            id: self.next_toast,
            title,
            detail,
            error,
            until,
        });
        if self.toasts.len() > 4 {
            self.toasts.remove(0);
        }
    }

    pub fn dismiss_toast(&mut self, id: u64) {
        self.toasts.retain(|t| t.id != id);
    }

    // ── theme ──

    pub fn set_theme(&mut self, theme: ThemeId, tx: &mut Tx) {
        self.theme = theme;
        tx.set_color_scheme(theme.color_scheme());
        self.config.theme = Some(theme.slug().to_string());
        self.save_config();
    }

    // ── lock / unlock ──

    pub fn set_lock_mode(&mut self, mode: LockMode) {
        if let Some(l) = self.lock_ui_mut() {
            l.mode = mode;
            l.error = None;
        }
    }

    pub fn set_vault_path(&mut self, path: PathBuf, mode: LockMode) {
        if let Some(l) = self.lock_ui_mut() {
            l.path = path.clone();
            l.mode = mode;
            l.error = None;
        }
        self.config.vault_path = Some(path);
        self.save_config();
    }

    /// Validate the lock form and derive keys on a background thread (Argon2 takes ~0.3 s).
    pub fn start_unlock(&mut self) {
        let kdf = self.kdf;
        let pending = self.pending.clone();
        let waker = self.waker.clone();
        let Some(l) = self.lock_ui_mut() else { return };
        if l.busy {
            return;
        }
        let password = Zeroizing::new(l.password.text.clone());
        if password.is_empty() {
            l.error = Some("Enter your password".into());
            return;
        }
        if l.mode == LockMode::Create {
            if password.chars().count() < 8 {
                l.error = Some("Use at least 8 characters".into());
                return;
            }
            if *password != l.confirm.text {
                l.error = Some("Passwords don't match".into());
                return;
            }
        }
        l.busy = true;
        l.error = None;
        let (path, mode) = (l.path.clone(), l.mode);
        let work = move || {
            let result = match mode {
                LockMode::Unlock => Vault::open(&path, &password).and_then(|(vault, mut data)| {
                    // Before anything is saved: keep the file as the previous version left it.
                    let backup = keykeeper_core::upgrade::backup_before_upgrade(
                        &vault,
                        &mut data,
                        env!("CARGO_PKG_VERSION"),
                    )?;
                    Ok((vault, data, backup))
                }),
                LockMode::Create => {
                    if let Some(dir) = path.parent()
                        && let Err(e) = std::fs::create_dir_all(dir)
                    {
                        Err(VaultError::Io(e))
                    } else {
                        let data = VaultData {
                            last_app_version: Some(env!("CARGO_PKG_VERSION").to_string()),
                            ..VaultData::default()
                        };
                        Vault::create(&path, &password, kdf, &data).map(|v| (v, data, None))
                    }
                }
            };
            if let Ok(mut slot) = pending.lock() {
                *slot = Some(result);
            }
            if let Some(w) = waker {
                w();
            }
        };
        if std::thread::Builder::new()
            .name("keykeeper-unlock".into())
            .spawn(work)
            .is_err()
            && let Some(l) = self.lock_ui_mut()
        {
            l.busy = false;
            l.error = Some("Couldn't start the unlock task".into());
        }
    }

    /// Picks up a finished unlock. Returns whether anything changed.
    fn finish_unlock(&mut self) -> bool {
        let Some(result) = self.pending.lock().ok().and_then(|mut s| s.take()) else {
            return false;
        };
        match result {
            Ok((vault, data, backup)) => {
                let path = vault.path().to_path_buf();
                self.open_session(vault, data);
                if let Some(b) = backup {
                    let name = b.file_name().map(|n| n.to_string_lossy().to_string());
                    self.toast_detail(
                        "Saved a copy of your vault from before this update",
                        name.unwrap_or_default(),
                    );
                }
                if self.config.vault_path.as_ref() != Some(&path) {
                    self.config.vault_path = Some(path);
                    self.save_config();
                }
            }
            Err(e) => {
                if let Some(l) = self.lock_ui_mut() {
                    l.busy = false;
                    l.error = Some(match e {
                        VaultError::BadPassword => "Wrong password".into(),
                        e => e.to_string(),
                    });
                    l.password.replace_text("");
                    l.confirm.replace_text("");
                }
            }
        }
        true
    }

    pub fn open_session(&mut self, vault: Vault, data: VaultData) {
        let mut session = Session::new(vault, data);
        session.refresh_rows(today());
        self.screen = Screen::Unlocked(Box::new(session));
        self.last_activity = self.now;
    }

    /// Save (if needed), forget all decrypted data, clear a copied key, and show the lock screen.
    pub fn lock(&mut self, tx: &mut Tx) {
        let path = match &mut self.screen {
            Screen::Unlocked(s) => {
                s.sync_trade_form();
                if s.is_dirty() && s.save().is_err() {
                    let err = s.save_error.clone().unwrap_or_default();
                    self.toast_error("Couldn't save before locking", err);
                    return;
                }
                s.vault.path().to_path_buf()
            }
            Screen::Locked(_) => return,
        };
        tx.clear_secret_clipboard();
        self.screen = Screen::Locked(Box::new(LockUi::for_path(path)));
        self.toasts.clear();
    }

    // ── per-frame upkeep ──

    /// Runs after handlers and on wakes: background results, row refresh, autosave,
    /// auto-lock, toast expiry. Returns whether the view needs to re-run.
    pub fn poll(&mut self, now: f64, tx: &mut Tx) -> bool {
        self.now = now;
        let mut changed = self.finish_unlock();
        changed |= self.finish_jobs();
        changed |= self.poll_updates(now, tx);
        if self.job.is_some() {
            tx.wake_after(Duration::from_millis(250));
            changed = true; // progress text
        }

        let n = self.toasts.len();
        self.toasts.retain(|t| t.until > now);
        changed |= self.toasts.len() != n;
        if let Some(next) = self.toasts.iter().map(|t| t.until).reduce(f64::min) {
            tx.wake_after(Duration::from_secs_f64((next - now).max(0.0)));
        }

        if self.last_activity <= 0.0 {
            // A session opened before the first poll (demo scenes) starts its idle clock here.
            self.last_activity = now;
        }
        let idle = now - self.last_activity;
        let mut lock_now = false;
        let mut save_failed = None;
        if let Screen::Unlocked(s) = &mut self.screen {
            changed |= s.refresh_rows(today());
            changed |= s.refresh_trade_form();
            let gen_ = s.repo.generation();
            if gen_ != s.seen_gen {
                s.seen_gen = gen_;
                s.dirty_since = Some(now);
            }
            if s.is_dirty() {
                let since = *s.dirty_since.get_or_insert(now);
                let due = since + AUTOSAVE_DELAY;
                if now >= due {
                    if let Err(e) = s.save() {
                        save_failed = Some(e.to_string());
                        s.dirty_since = Some(now + 10.0); // retry later, don't spin
                    }
                    changed = true;
                } else {
                    tx.wake_after(Duration::from_secs_f64(due - now));
                }
            }
            let mins = s.settings().auto_lock_minutes;
            if mins > 0 {
                let limit = f64::from(mins) * 60.0;
                if idle >= limit {
                    lock_now = true;
                } else {
                    tx.wake_after(Duration::from_secs_f64(limit - idle));
                }
            }
        }
        if let Some(e) = save_failed {
            self.toast_error("Autosave failed", e);
        }
        if lock_now {
            self.lock(tx);
            self.toast("Locked after inactivity");
            changed = true;
        }
        changed
    }

    pub fn touch(&mut self, tx: &Tx) {
        self.last_activity = tx.clock().max(self.now);
    }

    /// Window closing: flush unsaved changes and clear a copied key.
    pub fn on_close(&mut self, tx: &mut Tx) {
        if let Screen::Unlocked(s) = &mut self.screen {
            s.sync_trade_form();
            if s.is_dirty() {
                let _ = s.save();
            }
        }
        tx.clear_secret_clipboard();
    }

    // ── navigation ──

    pub fn go(&mut self, page: Page) {
        if let Some(s) = self.session_mut() {
            s.sync_trade_form();
            s.page = page;
            s.bulk_status_open = false;
            s.refresh_trade_form();
        }
    }

    // ── clipboard ──

    fn copy_secret_text(&mut self, text: String, tx: &mut Tx) {
        let secs = self
            .session()
            .map_or(30, |s| s.settings().clipboard_clear_secs);
        if secs == 0 {
            tx.copy(text);
        } else {
            let _ = tx.copy_secret(text, Duration::from_secs(u64::from(secs)));
        }
    }

    pub fn copy_key(&mut self, id: Uuid, tx: &mut Tx) {
        let Some(k) = self.session().and_then(|s| s.repo.key(id)) else {
            return;
        };
        let (text, name) = (k.key.expose().to_string(), k.game_name.clone());
        self.copy_secret_text(text, tx);
        self.toast(format!("Copied key for {name}"));
    }

    /// `Name: KEY` lines for the selected keys (Phase 4 makes the format a template).
    pub fn copy_name_keys(&mut self, ids: &[Uuid], tx: &mut Tx) {
        let Some(s) = self.session() else { return };
        let lines: Vec<String> = ids
            .iter()
            .filter_map(|id| s.repo.key(*id))
            .map(|k| format!("{}: {}", k.game_name, k.key.expose()))
            .collect();
        if lines.is_empty() {
            return;
        }
        let n = lines.len();
        self.copy_secret_text(lines.join("\n"), tx);
        self.toast(if n == 1 {
            "Copied 1 key".to_string()
        } else {
            format!("Copied {n} keys")
        });
    }

    // ── key editing ──

    pub fn open_key(&mut self, id: Uuid) {
        if let Some(s) = self.session_mut()
            && let Some(k) = s.repo.key(id)
        {
            s.form = Some(KeyForm::from_key(k, s.date_format()));
        }
    }

    pub fn new_key(&mut self) {
        if let Some(s) = self.session_mut() {
            s.page = Page::Library;
            s.form = Some(KeyForm::new_key());
        }
    }

    pub fn close_form(&mut self) {
        if let Some(s) = self.session_mut() {
            s.form = None;
        }
    }

    /// Apply the open form: add a new key or update the existing one (one undo step).
    pub fn save_form(&mut self) {
        let Some(s) = self.session_mut() else { return };
        let Some(form) = s.form.as_ref() else { return };
        let fmt = s.date_format();
        if !form.errors(fmt).is_empty() {
            return;
        }
        let message = match form.target {
            None => {
                let k = form.to_new_key(fmt);
                if let Some(existing) = s.repo.find_by_key(k.key.expose()) {
                    let name = existing.game_name.clone();
                    self.toast_error(
                        "Duplicate key",
                        format!("This key is already saved for {name}"),
                    );
                    return;
                }
                let name = k.game_name.clone();
                let id = s.repo.add_key(k);
                s.form = s.repo.key(id).map(|k| KeyForm::from_key(k, fmt));
                s.table.select(id.to_string());
                format!("Added {name}")
            }
            Some(id) => {
                let form = form.clone();
                match s.repo.update_key(id, |k| form.apply(k, fmt)) {
                    Ok(true) => {
                        s.form = s.repo.key(id).map(|k| KeyForm::from_key(k, fmt));
                        "Saved".to_string()
                    }
                    Ok(false) => return,
                    Err(e) => {
                        self.toast_error("Couldn't save", e.to_string());
                        return;
                    }
                }
            }
        };
        self.toast(message);
    }

    pub fn ask_delete(&mut self, ids: Vec<Uuid>) {
        if let Some(s) = self.session_mut()
            && !ids.is_empty()
        {
            s.confirm = Some(Confirm::DeleteKeys(ids));
        }
    }

    pub fn show_shortcuts(&mut self, open: bool) {
        if let Some(s) = self.session_mut() {
            s.shortcuts_open = open;
        }
    }

    fn toggle_shortcuts(&mut self) {
        let open = self.session().is_some_and(|s| s.shortcuts_open);
        self.show_shortcuts(!open);
    }

    pub fn cancel_confirm(&mut self) {
        if let Some(s) = self.session_mut() {
            s.confirm = None;
        }
    }

    pub fn confirm(&mut self, tx: &mut Tx) {
        let Some(c) = self.session_mut().and_then(|s| s.confirm.take()) else {
            return;
        };
        match c {
            Confirm::DeleteKeys(ids) => {
                let Some(s) = self.session_mut() else { return };
                let n = s.repo.delete_keys(
                    &ids,
                    if ids.len() == 1 {
                        "Delete key"
                    } else {
                        "Delete keys"
                    },
                );
                s.table.clear_selection();
                if s.form
                    .as_ref()
                    .and_then(|f| f.target)
                    .is_some_and(|t| ids.contains(&t))
                {
                    s.form = None;
                }
                self.toast(format!(
                    "Deleted {n} key{} — {} to undo",
                    if n == 1 { "" } else { "s" },
                    crate::ui::shortcut("Ctrl+Z"),
                ));
            }
            Confirm::ExportPlain(kind) => self.export_plain(kind, tx),
            Confirm::CancelTrade(id) => self.cancel_trade_now(id),
        }
    }

    pub fn set_status(&mut self, ids: &[Uuid], status: KeyStatus) {
        let Some(s) = self.session_mut() else { return };
        let fmt = s.date_format();
        let today = today();
        let res = s
            .repo
            .update_keys(ids, &format!("Mark {}", status.label()), |k| {
                if k.status != status {
                    k.status = status;
                    if status.is_gone() && k.status_date.is_none() {
                        k.status_date = Some(today);
                    }
                }
            });
        s.bulk_status_open = false;
        if let Some(f) = s.form.as_mut()
            && let Some(t) = f.target
            && ids.contains(&t)
            && let Some(k) = s.repo.key(t)
        {
            *f = KeyForm::from_key(k, fmt);
        }
        if let Ok(n) = res
            && n > 0
        {
            self.toast(format!("Marked {n} as {}", status.label()));
        }
    }

    pub fn undo(&mut self) {
        let Some(s) = self.session_mut() else { return };
        let fmt = s.date_format();
        if let Some(label) = s.repo.undo() {
            refresh_form(s, fmt);
            self.toast(format!("Undid: {label}"));
        }
    }

    pub fn redo(&mut self) {
        let Some(s) = self.session_mut() else { return };
        let fmt = s.date_format();
        if let Some(label) = s.repo.redo() {
            refresh_form(s, fmt);
            self.toast(format!("Redid: {label}"));
        }
    }

    // ── library filters ──

    pub fn toggle_status_filter(&mut self, status: KeyStatus) {
        if let Some(s) = self.session_mut() {
            if let Some(i) = s.statuses.iter().position(|x| *x == status) {
                s.statuses.remove(i);
            } else {
                s.statuses.push(status);
            }
            s.filters_changed();
        }
    }

    pub fn clear_status_filter(&mut self) {
        if let Some(s) = self.session_mut() {
            s.statuses.clear();
            s.filters_changed();
        }
    }

    pub fn toggle_hide_gone(&mut self) {
        if let Some(s) = self.session_mut() {
            s.hide_gone = !s.hide_gone;
            s.filters_changed();
        }
    }

    pub fn toggle_missing_app_id(&mut self) {
        if let Some(s) = self.session_mut() {
            s.missing_app_id = !s.missing_app_id;
            s.filters_changed();
        }
    }

    pub fn toggle_deadline_soon(&mut self) {
        if let Some(s) = self.session_mut() {
            s.deadline_soon = !s.deadline_soon;
            s.filters_changed();
        }
    }

    pub fn sort_by(&mut self, column: &str, dir: rune_kit::SortDir) {
        let field = match column {
            "name" => SortField::Name,
            "bundle" => SortField::Bundle,
            "status" => SortField::Status,
            "platform" => SortField::Platform,
            "deadline" => SortField::Deadline,
            "source" => SortField::Source,
            "rating" => SortField::Rating,
            _ => SortField::Added,
        };
        let dir = if matches!(dir, rune_kit::SortDir::Descending) {
            SortDir::Desc
        } else {
            SortDir::Asc
        };
        if let Some(s) = self.session_mut() {
            s.sort = Sort { field, dir };
            s.filters_changed();
        }
    }

    // ── settings ──

    pub fn update_settings(&mut self, f: impl FnOnce(&mut Settings)) {
        if let Some(s) = self.session_mut() {
            let mut settings = s.settings().clone();
            f(&mut settings);
            let fmt_changed = settings.date_format != s.date_format();
            s.repo.set_settings(settings);
            if fmt_changed {
                s.import.date_format = s.date_format();
                refresh_form(s, s.date_format());
                s.filters_changed();
            }
            s.settings_ui.open_select = None;
        }
    }

    pub fn change_password(&mut self) {
        let kdf = self.kdf;
        let Some(s) = self.session_mut() else { return };
        let ui = &mut s.settings_ui;
        let (old, new, confirm) = (
            Zeroizing::new(ui.old_pw.text.clone()),
            Zeroizing::new(ui.new_pw.text.clone()),
            ui.confirm_pw.text.clone(),
        );
        let fail = |ui: &mut SettingsUi, m: &str| ui.pw_message = Some((false, m.to_string()));
        if new.chars().count() < 8 {
            return fail(ui, "Use at least 8 characters");
        }
        if *new != confirm {
            return fail(ui, "New passwords don't match");
        }
        if !s.vault.verify_password(&old) {
            return fail(&mut s.settings_ui, "Current password is wrong");
        }
        let gen_ = s.repo.generation();
        match s.vault.change_password(&new, kdf, s.repo.data()) {
            Ok(()) => {
                s.repo.mark_saved(gen_);
                let ui = &mut s.settings_ui;
                for e in [&mut ui.old_pw, &mut ui.new_pw, &mut ui.confirm_pw] {
                    e.replace_text("");
                }
                ui.pw_message =
                    Some((true, "Password changed. Older backups were removed.".into()));
            }
            Err(e) => s.settings_ui.pw_message = Some((false, e.to_string())),
        }
    }

    // ── import ──

    pub fn import_text(&mut self, source: impl Into<String>, text: &str) {
        let Some(s) = self.session_mut() else { return };
        let table = import::parse(text);
        s.import.source = Some(source.into());
        s.import.table = Some(table);
        s.import.open_column = None;
        rebuild_preview(s);
    }

    pub fn import_paste(&mut self) {
        let Some(text) = self.session().map(|s| s.import.paste.text.clone()) else {
            return;
        };
        if text.trim().is_empty() {
            return;
        }
        self.import_text("Pasted text", &text);
    }

    pub fn import_pick_file(&mut self, tx: &mut Tx) {
        let options = FileDialogOptions {
            title: "Import keys".into(),
            ..Default::default()
        }
        .filter("Spreadsheets and text", &["csv", "tsv", "txt"])
        .filter("All files", &["*"]);
        tx.open_file(options, |s: &mut AppState, result, _| match result {
            Ok(Some(path)) => match std::fs::read(&path) {
                Ok(bytes) => {
                    let text = String::from_utf8_lossy(&bytes).into_owned();
                    let name = path
                        .file_name()
                        .map(|n| n.to_string_lossy().into_owned())
                        .unwrap_or_default();
                    s.import_text(name, &text);
                }
                Err(e) => s.toast_error("Couldn't read the file", e.to_string()),
            },
            Ok(None) => {}
            Err(e) => s.toast_error("Couldn't open a file dialog", e.to_string()),
        });
    }

    pub fn set_column_mapping(&mut self, column: usize, field: Option<Field>) {
        let Some(s) = self.session_mut() else { return };
        if let Some(t) = s.import.table.as_mut() {
            if let Some(f) = field {
                for m in t.mapping.iter_mut() {
                    if *m == Some(f) {
                        *m = None; // a field maps from one column only
                    }
                }
            }
            if let Some(slot) = t.mapping.get_mut(column) {
                *slot = field;
            }
        }
        s.import.open_column = None;
        rebuild_preview(s);
    }

    pub fn set_import_date_format(&mut self, fmt: DateFormat) {
        if let Some(s) = self.session_mut() {
            s.import.date_format = fmt;
            s.import.date_select_open = false;
            rebuild_preview(s);
        }
    }

    pub fn reset_import(&mut self) {
        if let Some(s) = self.session_mut() {
            let fmt = s.date_format();
            s.import = ImportUi::new(fmt);
        }
    }

    pub fn commit_import(&mut self) {
        let Some(s) = self.session_mut() else { return };
        let Some(p) = s.import.preview.as_ref() else {
            return;
        };
        let summary = import::commit(&mut s.repo, p, s.import.include_duplicates);
        let fmt = s.date_format();
        s.import = ImportUi::new(fmt);
        s.page = Page::Library;
        let mut msg = format!(
            "Imported {} key{}",
            summary.added,
            if summary.added == 1 { "" } else { "s" }
        );
        if summary.skipped_duplicates > 0 {
            msg.push_str(&format!(
                ", skipped {} duplicate{}",
                summary.skipped_duplicates,
                if summary.skipped_duplicates == 1 {
                    ""
                } else {
                    "s"
                }
            ));
        }
        self.toast(msg);
    }

    // ── export ──

    pub fn ask_export(&mut self, kind: ExportKind) {
        if let Some(s) = self.session_mut() {
            s.confirm = Some(Confirm::ExportPlain(kind));
        }
    }

    fn export_plain(&mut self, kind: ExportKind, tx: &mut Tx) {
        let (name, ext, label) = match kind {
            ExportKind::Csv => ("keykeeper-export.csv", "csv", "CSV"),
            ExportKind::Json => ("keykeeper-export.json", "json", "JSON"),
        };
        let options = FileDialogOptions {
            title: format!("Export {label} (unencrypted)"),
            file_name: Some(name.into()),
            ..Default::default()
        }
        .filter(label, &[ext]);
        tx.save_file(options, move |s: &mut AppState, result, _| {
            let Ok(Some(path)) = result else { return };
            let Some(sess) = s.session() else { return };
            let ids: Vec<Uuid> = sess.rows.clone();
            let text = match kind {
                ExportKind::Csv => export::to_csv(&sess.repo, &ids),
                ExportKind::Json => export::to_json(&sess.repo, &ids),
            };
            match std::fs::write(&path, text) {
                Ok(()) => s.toast(format!("Exported {} keys to {}", ids.len(), path.display())),
                Err(e) => s.toast_error("Export failed", e.to_string()),
            }
        });
    }

    pub fn export_backup(&mut self, tx: &mut Tx) {
        let date = today();
        let options = FileDialogOptions {
            title: "Save encrypted backup".into(),
            file_name: Some(format!("keykeeper-backup-{date}.kkvault")),
            ..Default::default()
        }
        .filter("KeyKeeper vault", &["kkvault"]);
        tx.save_file(options, |s: &mut AppState, result, _| {
            let Ok(Some(path)) = result else { return };
            let Some(sess) = s.session() else { return };
            match export::encrypted_backup(&sess.vault, &sess.repo, &path) {
                Ok(()) => s.toast("Encrypted backup saved"),
                Err(e) => s.toast_error("Backup failed", e.to_string()),
            }
        });
    }

    // ── keyboard ──

    /// App-wide shortcuts. Text fields get keys first, so these only see unclaimed ones.
    pub fn on_key(&mut self, e: &KeyEvent, tx: &mut Tx) -> bool {
        self.touch(tx);
        if !self.is_unlocked() {
            return false;
        }
        let ctrl = e.mods.cmd || e.mods.ctrl;
        match &e.key {
            rune::Key::Command(c) if ctrl => match c.to_ascii_lowercase() {
                'f' => {
                    self.go(Page::Library);
                    tx.focus("search", true);
                    true
                }
                'n' => {
                    self.new_key();
                    tx.focus("form-name", true);
                    true
                }
                'i' => {
                    self.go(Page::Import);
                    true
                }
                'l' => {
                    self.lock(tx);
                    true
                }
                'z' if e.mods.shift => {
                    self.redo();
                    true
                }
                'z' => {
                    self.undo();
                    true
                }
                'y' => {
                    self.redo();
                    true
                }
                's' => {
                    self.save_form();
                    true
                }
                'c' if e.mods.shift => {
                    let ids = self
                        .session()
                        .map(Session::selected_ids)
                        .unwrap_or_default();
                    self.copy_name_keys(&ids, tx);
                    true
                }
                '/' => {
                    self.toggle_shortcuts();
                    true
                }
                _ => false,
            },
            rune::Key::Function(1) => {
                self.toggle_shortcuts();
                true
            }
            rune::Key::Delete => {
                let ids = self
                    .session()
                    .filter(|s| s.page == Page::Library)
                    .map(Session::selected_ids)
                    .unwrap_or_default();
                if ids.is_empty() {
                    return false;
                }
                self.ask_delete(ids);
                true
            }
            rune::Key::Escape => {
                let Some(s) = self.session_mut() else {
                    return false;
                };
                if std::mem::take(&mut s.shortcuts_open)
                    || s.confirm.take().is_some()
                    || s.bulk_status_open
                    || s.form.take().is_some()
                {
                    s.bulk_status_open = false;
                    return true;
                }
                false
            }
            _ => false,
        }
    }
}

fn refresh_form(s: &mut Session, fmt: DateFormat) {
    if let Some(f) = &s.form
        && let Some(id) = f.target
    {
        s.form = s.repo.key(id).map(|k| KeyForm::from_key(k, fmt));
    }
}

fn rebuild_preview(s: &mut Session) {
    let opts = ImportOptions::new(s.import.date_format);
    s.import.preview = s
        .import
        .table
        .as_ref()
        .map(|t| import::preview(t, &opts, &s.repo));
}
