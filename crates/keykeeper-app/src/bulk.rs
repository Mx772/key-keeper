//! Bulk edit: change several keys at once. Fields left alone don't change; the whole edit is
//! one undo step.

use keykeeper_core::import::values::parse_date;
use keykeeper_core::{DateFormat, GameKey, KeyStatus, Platform};
use rune::TextEdit;
use uuid::Uuid;

use crate::app::{AppState, Confirm, Session, today};
use crate::form::date_hint;

/// Edits touching more keys than this ask first.
pub const CONFIRM_OVER: usize = 25;

#[derive(Clone, Debug, Default)]
pub struct BulkForm {
    pub ids: Vec<Uuid>,
    pub status: Option<KeyStatus>,
    pub platform: Option<Platform>,
    pub bundle: TextEdit,
    pub source: TextEdit,
    /// A date sets every deadline; "clear" removes them.
    pub deadline: TextEdit,
    pub clear_deadline: bool,
    pub add_tags: TextEdit,
    pub remove_tags: TextEdit,
    pub open_select: Option<&'static str>,
}

fn tags(t: &TextEdit) -> Vec<String> {
    t.text
        .split([',', ';'])
        .map(str::trim)
        .filter(|t| !t.is_empty())
        .map(str::to_string)
        .collect()
}

fn text(t: &TextEdit) -> Option<String> {
    Some(t.text.trim().to_string()).filter(|s| !s.is_empty())
}

/// One value shared by every key, or `None` when they differ ("Mixed").
pub fn common<T: PartialEq + Clone>(keys: &[&GameKey], f: impl Fn(&GameKey) -> T) -> Option<T> {
    let first = f(keys.first()?);
    keys.iter().all(|k| f(k) == first).then_some(first)
}

impl BulkForm {
    pub fn new(ids: Vec<Uuid>) -> BulkForm {
        BulkForm {
            ids,
            ..BulkForm::default()
        }
    }

    pub fn deadline_error(&self, fmt: DateFormat) -> Option<String> {
        if self.clear_deadline {
            return None;
        }
        parse_date(&self.deadline.text, fmt)
            .err()
            .map(|m| format!("{m} (use {} or YYYY-MM-DD)", date_hint(fmt)))
    }

    /// Whether applying would change anything at all.
    pub fn is_empty(&self) -> bool {
        self.status.is_none()
            && self.platform.is_none()
            && text(&self.bundle).is_none()
            && text(&self.source).is_none()
            && text(&self.deadline).is_none()
            && !self.clear_deadline
            && tags(&self.add_tags).is_empty()
            && tags(&self.remove_tags).is_empty()
    }

    /// What to say before applying, if it needs a confirmation.
    pub fn needs_confirm(&self) -> bool {
        self.ids.len() > CONFIRM_OVER
            || matches!(
                self.status,
                Some(KeyStatus::Traded | KeyStatus::Redeemed | KeyStatus::Gifted)
            )
    }

    pub fn apply(&self, k: &mut GameKey, fmt: DateFormat, today: jiff::civil::Date) {
        if let Some(st) = self.status
            && k.status != st
        {
            k.status = st;
            if st.is_gone() && k.status_date.is_none() {
                k.status_date = Some(today);
            }
        }
        if let Some(p) = &self.platform {
            k.platform = p.clone();
        }
        if let Some(b) = text(&self.bundle) {
            k.bundle = Some(b);
        }
        if let Some(s) = text(&self.source) {
            k.source = Some(s);
        }
        if self.clear_deadline {
            k.redemption_deadline = None;
        } else if let Ok(Some(d)) = parse_date(&self.deadline.text, fmt) {
            k.redemption_deadline = Some(d);
        }
        let remove: Vec<String> = tags(&self.remove_tags)
            .into_iter()
            .map(|t| t.to_lowercase())
            .collect();
        k.tags.retain(|t| !remove.contains(&t.to_lowercase()));
        for t in tags(&self.add_tags) {
            if !k.tags.iter().any(|x| x.eq_ignore_ascii_case(&t)) {
                k.tags.push(t);
            }
        }
    }
}

impl Session {
    pub fn bulk_keys(&self) -> Vec<&GameKey> {
        self.bulk
            .as_ref()
            .map(|b| b.ids.iter().filter_map(|id| self.repo.key(*id)).collect())
            .unwrap_or_default()
    }
}

impl AppState {
    /// Open the bulk editor for the selected keys (in the editor pane).
    pub fn open_bulk_edit(&mut self) {
        self.hover = None;
        let Some(s) = self.session_mut() else { return };
        let ids = s.selected_ids();
        if ids.len() < 2 {
            return;
        }
        s.form = None;
        s.row_menu = None;
        s.bulk = Some(BulkForm::new(ids));
    }

    pub fn close_bulk_edit(&mut self) {
        if let Some(s) = self.session_mut() {
            s.bulk = None;
        }
    }

    pub fn with_bulk(&mut self, f: impl FnOnce(&mut BulkForm)) {
        if let Some(b) = self.session_mut().and_then(|s| s.bulk.as_mut()) {
            f(b);
        }
    }

    /// Apply, asking first for big or irreversible-looking edits.
    pub fn apply_bulk_edit(&mut self) {
        let Some(s) = self.session_mut() else { return };
        let Some(b) = s.bulk.as_ref() else { return };
        if b.is_empty() || b.deadline_error(s.date_format()).is_some() {
            return;
        }
        if b.needs_confirm() {
            s.confirm = Some(Confirm::BulkEdit);
        } else {
            self.apply_bulk_now();
        }
    }

    pub(crate) fn apply_bulk_now(&mut self) {
        let Some(s) = self.session_mut() else { return };
        let Some(b) = s.bulk.take() else { return };
        let fmt = s.date_format();
        let today = today();
        let label = format!("Edit {} keys", b.ids.len());
        let res = s
            .repo
            .update_keys(&b.ids, &label, |k| b.apply(k, fmt, today));
        match res {
            Ok(n) => {
                self.toast_undo(
                    if n == 0 {
                        "Nothing changed".to_string()
                    } else {
                        format!("Updated {n} key{}", if n == 1 { "" } else { "s" })
                    },
                    n > 0,
                );
            }
            Err(e) => self.toast_error("Couldn't apply the edit", e.to_string()),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use jiff::civil::date;

    #[test]
    fn untouched_fields_stay_and_tags_merge() {
        let mut k = GameKey::new("A", "AAAAA-BBBBB-CCCCC");
        k.bundle = Some("Old".into());
        k.tags = vec!["Keep".into(), "drop".into()];
        k.redemption_deadline = Some(date(2027, 1, 1));
        let mut f = BulkForm::new(vec![k.id]);
        assert!(f.is_empty());
        f.add_tags = TextEdit::new("new, keep");
        f.remove_tags = TextEdit::new("DROP");
        f.status = Some(KeyStatus::Redeemed);
        assert!(f.needs_confirm());
        let today = date(2026, 11, 26);
        f.apply(&mut k, DateFormat::MonthDayYear, today);
        assert_eq!(k.bundle.as_deref(), Some("Old"));
        assert_eq!(k.tags, vec!["Keep".to_string(), "new".to_string()]);
        assert_eq!(k.status, KeyStatus::Redeemed);
        assert_eq!(k.status_date, Some(today));
        assert_eq!(k.redemption_deadline, Some(date(2027, 1, 1)));

        f.clear_deadline = true;
        f.apply(&mut k, DateFormat::MonthDayYear, today);
        assert_eq!(k.redemption_deadline, None);

        let mut g = BulkForm::new(vec![k.id]);
        g.deadline = TextEdit::new("13/45/2026");
        assert!(g.deadline_error(DateFormat::MonthDayYear).is_some());
        g.deadline = TextEdit::new("12/31/2026");
        assert!(g.deadline_error(DateFormat::MonthDayYear).is_none());
        g.bundle = TextEdit::new(" Humble ");
        g.apply(&mut k, DateFormat::MonthDayYear, today);
        assert_eq!(k.redemption_deadline, Some(date(2026, 12, 31)));
        assert_eq!(k.bundle.as_deref(), Some("Humble"));
    }

    #[test]
    fn common_values() {
        let a = GameKey::new("A", "A1");
        let mut b = GameKey::new("B", "B1");
        assert_eq!(common(&[&a, &b], |k| k.status), Some(a.status));
        b.status = KeyStatus::Traded;
        assert_eq!(common(&[&a, &b], |k| k.status), None);
        assert_eq!(common::<KeyStatus>(&[], |k| k.status), None);
    }
}
