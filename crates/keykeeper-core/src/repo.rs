//! [`Repo`]: the unlocked, in-memory vault contents, with undo/redo and dirty tracking.
//! The GUI mutates data only through these methods.

use std::collections::{HashMap, HashSet};

use jiff::Timestamp;
use uuid::Uuid;

use crate::data::VaultData;
use crate::error::RepoError;
use crate::model::{ApiKeys, GameKey, Settings, Trade};
use crate::secret::normalize_key;

const UNDO_LIMIT: usize = 100;

#[derive(Clone, Debug)]
enum Edit {
    Key {
        id: Uuid,
        before: Option<Box<GameKey>>,
        after: Option<Box<GameKey>>,
    },
    Trade {
        id: Uuid,
        before: Option<Box<Trade>>,
        after: Option<Box<Trade>>,
    },
}

#[derive(Clone, Debug)]
struct Change {
    label: String,
    edits: Vec<Edit>,
}

pub struct Repo {
    data: VaultData,
    key_index: HashMap<Uuid, usize>,
    undo: Vec<Change>,
    redo: Vec<Change>,
    generation: u64,
    saved_generation: u64,
}

impl Repo {
    pub fn new(data: VaultData) -> Repo {
        let mut r = Repo {
            data,
            key_index: HashMap::new(),
            undo: Vec::new(),
            redo: Vec::new(),
            generation: 0,
            saved_generation: 0,
        };
        r.reindex();
        r
    }

    // ── reads ──

    pub fn data(&self) -> &VaultData {
        &self.data
    }

    pub fn into_data(self) -> VaultData {
        self.data
    }

    pub fn keys(&self) -> &[GameKey] {
        &self.data.keys
    }

    pub fn key(&self, id: Uuid) -> Option<&GameKey> {
        self.key_index.get(&id).map(|&i| &self.data.keys[i])
    }

    pub fn trades(&self) -> &[Trade] {
        &self.data.trades
    }

    pub fn trade(&self, id: Uuid) -> Option<&Trade> {
        self.data.trades.iter().find(|t| t.id == id)
    }

    pub fn settings(&self) -> &Settings {
        &self.data.settings
    }

    pub fn api_keys(&self) -> &ApiKeys {
        &self.data.api_keys
    }

    /// The existing key with the same code, ignoring case, spaces and dashes.
    pub fn find_by_key(&self, code: &str) -> Option<&GameKey> {
        let n = normalize_key(code);
        if n.is_empty() {
            return None;
        }
        self.data.keys.iter().find(|k| k.key.normalized() == n)
    }

    // ── dirty tracking ──

    /// Increments on every change, including undo/redo. The app autosaves when it moves.
    pub fn generation(&self) -> u64 {
        self.generation
    }

    pub fn is_dirty(&self) -> bool {
        self.generation != self.saved_generation
    }

    /// Record that the state at `generation` reached disk.
    pub fn mark_saved(&mut self, generation: u64) {
        self.saved_generation = generation;
    }

    // ── key writes ──

    pub fn add_key(&mut self, key: GameKey) -> Uuid {
        let id = key.id;
        self.add_keys(vec![key], "Add key");
        id
    }

    /// One undo step for the whole batch (e.g. an import).
    pub fn add_keys(&mut self, keys: Vec<GameKey>, label: &str) -> Vec<Uuid> {
        let ids = keys.iter().map(|k| k.id).collect();
        let edits = keys
            .into_iter()
            .map(|k| Edit::Key {
                id: k.id,
                before: None,
                after: Some(Box::new(k)),
            })
            .collect();
        self.commit(label, edits);
        ids
    }

    /// Returns whether anything changed. Bumps `updated_at` only on a real change.
    pub fn update_key(
        &mut self,
        id: Uuid,
        f: impl FnOnce(&mut GameKey),
    ) -> Result<bool, RepoError> {
        let mut f = Some(f);
        self.update_keys(&[id], "Edit key", |k| {
            if let Some(f) = f.take() {
                f(k)
            }
        })
        .map(|n| n > 0)
    }

    /// Apply `f` to each key as one undo step. All ids must exist, or nothing changes.
    /// Returns how many keys actually changed.
    pub fn update_keys(
        &mut self,
        ids: &[Uuid],
        label: &str,
        mut f: impl FnMut(&mut GameKey),
    ) -> Result<usize, RepoError> {
        if let Some(&missing) = ids.iter().find(|id| !self.key_index.contains_key(id)) {
            return Err(RepoError::KeyNotFound(missing));
        }
        let now = Timestamp::now();
        let mut edits = Vec::new();
        let mut seen = HashSet::new();
        for &id in ids {
            if !seen.insert(id) {
                continue;
            }
            let Some(before) = self.key(id).cloned() else {
                continue;
            };
            let mut after = before.clone();
            f(&mut after);
            after.id = id; // ids are immutable
            if after != before {
                after.updated_at = now;
                edits.push(Edit::Key {
                    id,
                    before: Some(Box::new(before)),
                    after: Some(Box::new(after)),
                });
            }
        }
        let n = edits.len();
        self.commit(label, edits);
        Ok(n)
    }

    /// Unknown ids are ignored. Returns how many were deleted.
    pub fn delete_keys(&mut self, ids: &[Uuid], label: &str) -> usize {
        let mut seen = HashSet::new();
        let edits: Vec<Edit> = ids
            .iter()
            .filter(|id| seen.insert(**id))
            .filter_map(|&id| {
                self.key(id).cloned().map(|k| Edit::Key {
                    id,
                    before: Some(Box::new(k)),
                    after: None,
                })
            })
            .collect();
        let n = edits.len();
        self.commit(label, edits);
        n
    }

    // ── trade writes ──

    pub fn upsert_trade(&mut self, trade: Trade, label: &str) {
        let before = self.trade(trade.id).cloned();
        if before.as_ref() == Some(&trade) {
            return;
        }
        self.commit(
            label,
            vec![Edit::Trade {
                id: trade.id,
                before: before.map(Box::new),
                after: Some(Box::new(trade)),
            }],
        );
    }

    pub fn delete_trade(&mut self, id: Uuid) -> Result<(), RepoError> {
        let before = self
            .trade(id)
            .cloned()
            .ok_or(RepoError::TradeNotFound(id))?;
        self.commit(
            "Delete trade",
            vec![Edit::Trade {
                id,
                before: Some(Box::new(before)),
                after: None,
            }],
        );
        Ok(())
    }

    // ── settings (not undoable) ──

    pub fn set_settings(&mut self, settings: Settings) {
        if self.data.settings != settings {
            self.data.settings = settings;
            self.generation += 1;
        }
    }

    pub fn set_api_keys(&mut self, api_keys: ApiKeys) {
        if self.data.api_keys != api_keys {
            self.data.api_keys = api_keys;
            self.generation += 1;
        }
    }

    // ── undo / redo ──

    pub fn can_undo(&self) -> bool {
        !self.undo.is_empty()
    }

    pub fn can_redo(&self) -> bool {
        !self.redo.is_empty()
    }

    pub fn undo_label(&self) -> Option<&str> {
        self.undo.last().map(|c| c.label.as_str())
    }

    pub fn redo_label(&self) -> Option<&str> {
        self.redo.last().map(|c| c.label.as_str())
    }

    /// Returns the label of the undone change.
    pub fn undo(&mut self) -> Option<String> {
        let change = self.undo.pop()?;
        self.apply(&change.edits, false);
        let label = change.label.clone();
        self.redo.push(change);
        Some(label)
    }

    pub fn redo(&mut self) -> Option<String> {
        let change = self.redo.pop()?;
        self.apply(&change.edits, true);
        let label = change.label.clone();
        self.undo.push(change);
        Some(label)
    }

    // ── internals ──

    fn commit(&mut self, label: &str, edits: Vec<Edit>) {
        if edits.is_empty() {
            return;
        }
        self.apply(&edits, true);
        self.redo.clear();
        self.undo.push(Change {
            label: label.to_string(),
            edits,
        });
        if self.undo.len() > UNDO_LIMIT {
            self.undo.remove(0);
        }
    }

    fn apply(&mut self, edits: &[Edit], forward: bool) {
        let mut removed_keys = HashSet::new();
        for edit in edits {
            match edit {
                Edit::Key { id, before, after } => {
                    let target = if forward { after } else { before };
                    match target {
                        Some(k) => {
                            removed_keys.remove(id);
                            match self.key_index.get(id) {
                                Some(&i) => self.data.keys[i] = (**k).clone(),
                                None => {
                                    self.key_index.insert(*id, self.data.keys.len());
                                    self.data.keys.push((**k).clone());
                                }
                            }
                        }
                        None => {
                            removed_keys.insert(*id);
                        }
                    }
                }
                Edit::Trade { id, before, after } => {
                    let target = if forward { after } else { before };
                    let pos = self.data.trades.iter().position(|t| t.id == *id);
                    match (target, pos) {
                        (Some(t), Some(i)) => self.data.trades[i] = (**t).clone(),
                        (Some(t), None) => self.data.trades.push((**t).clone()),
                        (None, Some(i)) => {
                            self.data.trades.remove(i);
                        }
                        (None, None) => {}
                    }
                }
            }
        }
        if !removed_keys.is_empty() {
            self.data.keys.retain(|k| !removed_keys.contains(&k.id));
            self.reindex();
        }
        self.generation += 1;
    }

    fn reindex(&mut self) {
        self.key_index = self
            .data
            .keys
            .iter()
            .enumerate()
            .map(|(i, k)| (k.id, i))
            .collect();
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::{KeyStatus, TradeStatus};

    fn repo_with(names: &[&str]) -> (Repo, Vec<Uuid>) {
        let mut r = Repo::new(VaultData::default());
        let keys: Vec<GameKey> = names
            .iter()
            .enumerate()
            .map(|(i, n)| GameKey::new(*n, format!("KEY{i:02}-AAAAA-BBBBB")))
            .collect();
        let ids = r.add_keys(keys, "seed");
        (r, ids)
    }

    #[test]
    fn add_get_and_dirty() {
        let mut r = Repo::new(VaultData::default());
        assert!(!r.is_dirty());
        let id = r.add_key(GameKey::new("Undertale", "UNDR7-TALE0-00001"));
        assert!(r.is_dirty());
        assert_eq!(r.key(id).map(|k| k.game_name.as_str()), Some("Undertale"));
        let g = r.generation();
        r.mark_saved(g);
        assert!(!r.is_dirty());
    }

    #[test]
    fn update_bumps_updated_at_only_on_change() {
        let (mut r, ids) = repo_with(&["A"]);
        let before = r.key(ids[0]).cloned().unwrap();
        let g = r.generation();
        assert_eq!(r.update_key(ids[0], |k| k.notes.clear()), Ok(false));
        assert_eq!(r.generation(), g, "no-op edits are not recorded");
        assert_eq!(r.undo_label(), Some("seed"));
        assert_eq!(
            r.update_key(ids[0], |k| k.status = KeyStatus::Traded),
            Ok(true)
        );
        let after = r.key(ids[0]).unwrap();
        assert_eq!(after.status, KeyStatus::Traded);
        assert!(after.updated_at >= before.updated_at);
    }

    #[test]
    fn update_cannot_change_id() {
        let (mut r, ids) = repo_with(&["A"]);
        r.update_key(ids[0], |k| k.id = Uuid::nil()).unwrap();
        assert!(r.key(ids[0]).is_some());
        assert!(r.key(Uuid::nil()).is_none());
    }

    #[test]
    fn update_with_missing_id_changes_nothing() {
        let (mut r, ids) = repo_with(&["A", "B"]);
        let g = r.generation();
        let err = r.update_keys(&[ids[0], Uuid::nil()], "bulk", |k| k.notes = "x".into());
        assert_eq!(err, Err(RepoError::KeyNotFound(Uuid::nil())));
        assert_eq!(r.generation(), g);
        assert_eq!(r.key(ids[0]).unwrap().notes, "");
    }

    #[test]
    fn bulk_update_is_one_undo_step() {
        let (mut r, ids) = repo_with(&["A", "B", "C"]);
        assert_eq!(
            r.update_keys(&ids, "Set status", |k| k.status = KeyStatus::Reserved),
            Ok(3)
        );
        assert!(r.keys().iter().all(|k| k.status == KeyStatus::Reserved));
        assert_eq!(r.undo().as_deref(), Some("Set status"));
        assert!(r.keys().iter().all(|k| k.status == KeyStatus::Available));
        assert_eq!(r.redo().as_deref(), Some("Set status"));
        assert!(r.keys().iter().all(|k| k.status == KeyStatus::Reserved));
    }

    #[test]
    fn delete_and_undo_restores_keys() {
        let (mut r, ids) = repo_with(&["A", "B", "C", "D"]);
        assert_eq!(
            r.delete_keys(&[ids[1], ids[3], ids[1], Uuid::nil()], "Delete"),
            2
        );
        assert_eq!(r.keys().len(), 2);
        assert!(r.key(ids[1]).is_none());
        // Index stays consistent after removal.
        assert_eq!(r.key(ids[2]).unwrap().game_name, "C");
        r.undo();
        assert_eq!(r.keys().len(), 4);
        for (id, n) in ids.iter().zip(["A", "B", "C", "D"]) {
            assert_eq!(r.key(*id).unwrap().game_name, n);
        }
    }

    #[test]
    fn new_change_clears_redo() {
        let (mut r, ids) = repo_with(&["A"]);
        r.update_key(ids[0], |k| k.notes = "1".into()).unwrap();
        r.undo();
        assert!(r.can_redo());
        r.update_key(ids[0], |k| k.notes = "2".into()).unwrap();
        assert!(!r.can_redo());
    }

    #[test]
    fn undo_history_is_bounded() {
        let (mut r, ids) = repo_with(&["A"]);
        for i in 0..(UNDO_LIMIT + 20) {
            r.update_key(ids[0], |k| k.notes = i.to_string()).unwrap();
        }
        let mut n = 0;
        while r.undo().is_some() {
            n += 1;
        }
        assert_eq!(n, UNDO_LIMIT);
    }

    #[test]
    fn undo_on_empty_history_is_none() {
        let mut r = Repo::new(VaultData::default());
        assert_eq!(r.undo(), None);
        assert_eq!(r.redo(), None);
    }

    #[test]
    fn find_by_key_normalizes() {
        let (r, ids) = repo_with(&["A"]);
        assert_eq!(
            r.find_by_key("key00 aaaaa bbbbb").map(|k| k.id),
            Some(ids[0])
        );
        assert!(r.find_by_key("").is_none());
        assert!(r.find_by_key("---").is_none());
    }

    #[test]
    fn trades_upsert_delete_undo() {
        let mut r = Repo::new(VaultData::default());
        let mut t = Trade::new();
        t.partner_name = "alice".into();
        r.upsert_trade(t.clone(), "New trade");
        assert_eq!(r.trades().len(), 1);
        let g = r.generation();
        r.upsert_trade(t.clone(), "noop");
        assert_eq!(r.generation(), g);
        t.status = TradeStatus::Completed;
        r.upsert_trade(t.clone(), "Complete");
        assert_eq!(r.trade(t.id).unwrap().status, TradeStatus::Completed);
        r.delete_trade(t.id).unwrap();
        assert!(r.trades().is_empty());
        assert_eq!(r.delete_trade(t.id), Err(RepoError::TradeNotFound(t.id)));
        r.undo();
        assert_eq!(r.trade(t.id).unwrap().status, TradeStatus::Completed);
        r.undo();
        assert_eq!(r.trade(t.id).unwrap().status, TradeStatus::Draft);
    }

    #[test]
    fn settings_change_marks_dirty_without_undo() {
        let mut r = Repo::new(VaultData::default());
        let mut s = r.settings().clone();
        s.auto_lock_minutes = 1;
        r.set_settings(s.clone());
        assert!(r.is_dirty());
        assert!(!r.can_undo());
        let g = r.generation();
        r.set_settings(s);
        assert_eq!(r.generation(), g);
    }
}
