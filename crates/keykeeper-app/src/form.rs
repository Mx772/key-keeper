//! The key editor's form model: one [`TextEdit`] per text field, converted to and from a
//! [`GameKey`]. Kept free of view code so it can be unit-tested.

use jiff::civil::Date;
use keykeeper_core::import::values::{parse_app_id, parse_date};
use keykeeper_core::{DateFormat, GameKey, KeyStatus, Platform, Secret};
use rune::TextEdit;
use uuid::Uuid;

#[derive(Clone)]
pub struct KeyForm {
    /// `None` while adding a new key.
    pub target: Option<Uuid>,
    pub name: TextEdit,
    pub key: TextEdit,
    pub status: KeyStatus,
    pub platform: Platform,
    pub bundle: TextEdit,
    pub source: TextEdit,
    pub key_url: TextEdit,
    pub app_id: TextEdit,
    pub status_date: TextEdit,
    pub deadline: TextEdit,
    pub tags: TextEdit,
    pub notes: TextEdit,
    pub reveal_key: bool,
    /// Which dropdown is open, if any.
    pub open_select: Option<&'static str>,
    /// The open date calendar, if any.
    pub date_pick: Option<crate::ui::DatePick>,
}

pub fn format_date(d: Option<Date>, fmt: DateFormat) -> String {
    match (d, fmt) {
        (None, _) => String::new(),
        (Some(d), DateFormat::MonthDayYear) => format!("{}/{}/{}", d.month(), d.day(), d.year()),
        (Some(d), DateFormat::DayMonthYear) => format!("{}/{}/{}", d.day(), d.month(), d.year()),
    }
}

pub fn date_hint(fmt: DateFormat) -> &'static str {
    match fmt {
        DateFormat::MonthDayYear => "M/D/YYYY",
        DateFormat::DayMonthYear => "D/M/YYYY",
    }
}

fn opt(s: &TextEdit) -> Option<String> {
    Some(s.text.trim())
        .filter(|t| !t.is_empty())
        .map(str::to_string)
}

impl KeyForm {
    pub fn new_key() -> KeyForm {
        KeyForm {
            target: None,
            name: TextEdit::default(),
            key: TextEdit::default(),
            status: KeyStatus::Available,
            platform: Platform::Steam,
            bundle: TextEdit::default(),
            source: TextEdit::default(),
            key_url: TextEdit::default(),
            app_id: TextEdit::default(),
            status_date: TextEdit::default(),
            deadline: TextEdit::default(),
            tags: TextEdit::default(),
            notes: TextEdit::default(),
            reveal_key: true,
            open_select: None,
            date_pick: None,
        }
    }

    pub fn from_key(k: &GameKey, fmt: DateFormat) -> KeyForm {
        KeyForm {
            target: Some(k.id),
            name: TextEdit::new(k.game_name.clone()),
            key: TextEdit::new(k.key.expose()),
            status: k.status,
            platform: k.platform.clone(),
            bundle: TextEdit::new(k.bundle.clone().unwrap_or_default()),
            source: TextEdit::new(k.source.clone().unwrap_or_default()),
            key_url: TextEdit::new(k.key_url.clone().unwrap_or_default()),
            app_id: TextEdit::new(k.steam_app_id.map(|i| i.to_string()).unwrap_or_default()),
            status_date: TextEdit::new(format_date(k.status_date, fmt)),
            deadline: TextEdit::new(format_date(k.redemption_deadline, fmt)),
            tags: TextEdit::new(k.tags.join(", ")),
            notes: TextEdit::new(k.notes.clone()),
            reveal_key: false,
            open_select: None,
            date_pick: None,
        }
    }

    /// Field-level problems, as `(field, message)`. Empty means the form can be applied.
    pub fn errors(&self, fmt: DateFormat) -> Vec<(&'static str, String)> {
        let mut e = Vec::new();
        if self.name.text.trim().is_empty() {
            e.push(("name", "Game name is required".to_string()));
        }
        // "Other" entries (software, website redemptions) may have only a link.
        if self.key.text.trim().is_empty() && !matches!(self.platform, Platform::Other(_)) {
            e.push((
                "key",
                "Key is required (or set the platform to Other)".to_string(),
            ));
        }
        if let Some(t) = opt(&self.app_id)
            && parse_app_id(&t).is_none()
        {
            e.push(("app_id", "Enter a Steam app id or store link".to_string()));
        }
        if let Some(t) = opt(&self.key_url)
            && keykeeper_core::links::host(&t).is_none()
        {
            e.push(("key_url", "Enter a full http(s) link".to_string()));
        }
        for (field, edit) in [
            ("status_date", &self.status_date),
            ("deadline", &self.deadline),
        ] {
            if let Err(msg) = parse_date(&edit.text, fmt) {
                e.push((
                    field,
                    format!("{msg} (use {} or YYYY-MM-DD)", date_hint(fmt)),
                ));
            }
        }
        e
    }

    pub fn error_for(&self, field: &str, fmt: DateFormat) -> Option<String> {
        self.errors(fmt)
            .into_iter()
            .find(|(f, _)| *f == field)
            .map(|(_, m)| m)
    }

    /// Write the form into `k`. Call only when [`KeyForm::errors`] is empty; invalid optional
    /// fields are left unchanged rather than cleared.
    pub fn apply(&self, k: &mut GameKey, fmt: DateFormat) {
        k.game_name = self.name.text.trim().to_string();
        let code = self.key.text.trim();
        if code != k.key.expose() {
            k.key = Secret::new(code);
        }
        k.status = self.status;
        k.platform = self.platform.clone();
        k.bundle = opt(&self.bundle);
        k.source = opt(&self.source);
        k.key_url = opt(&self.key_url);
        k.steam_app_id = opt(&self.app_id).and_then(|t| parse_app_id(&t));
        if let Ok(d) = parse_date(&self.status_date.text, fmt) {
            k.status_date = d;
        }
        if let Ok(d) = parse_date(&self.deadline.text, fmt) {
            k.redemption_deadline = d;
        }
        k.tags = self
            .tags
            .text
            .split([',', ';'])
            .map(str::trim)
            .filter(|t| !t.is_empty())
            .map(str::to_string)
            .collect();
        k.notes = self.notes.text.clone();
    }

    pub fn to_new_key(&self, fmt: DateFormat) -> GameKey {
        let mut k = GameKey::new(String::new(), Secret::default());
        self.apply(&mut k, fmt);
        k
    }

    /// Whether the form differs from `k` (for "unsaved changes" hints).
    pub fn differs_from(&self, k: &GameKey, fmt: DateFormat) -> bool {
        let mut probe = k.clone();
        self.apply(&mut probe, fmt);
        probe != *k
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use jiff::civil::date;

    const MDY: DateFormat = DateFormat::MonthDayYear;

    fn key() -> GameKey {
        let mut k = GameKey::new("Haiku, the Robot", "HA1KU-R0B0T-00002");
        k.status = KeyStatus::Traded;
        k.status_date = Some(date(2026, 1, 7));
        k.tags = vec!["rare".into(), "dlc".into()];
        k.steam_app_id = Some(1);
        k
    }

    #[test]
    fn round_trip_is_unchanged() {
        let k = key();
        let f = KeyForm::from_key(&k, MDY);
        assert_eq!(f.status_date.text, "1/7/2026");
        assert!(f.errors(MDY).is_empty());
        assert!(!f.differs_from(&k, MDY));
        let f = KeyForm::from_key(&k, DateFormat::DayMonthYear);
        assert_eq!(f.status_date.text, "7/1/2026");
        assert!(!f.differs_from(&k, DateFormat::DayMonthYear));
    }

    #[test]
    fn edits_apply() {
        let mut k = key();
        let mut f = KeyForm::from_key(&k, MDY);
        f.name = TextEdit::new("  Haiku  ");
        f.deadline = TextEdit::new("2026-12-31");
        f.app_id = TextEdit::new("https://store.steampowered.com/app/972660/x/");
        f.tags = TextEdit::new("a; b, ,c");
        f.status = KeyStatus::Gifted;
        assert!(f.differs_from(&k, MDY));
        f.apply(&mut k, MDY);
        assert_eq!(k.game_name, "Haiku");
        assert_eq!(k.redemption_deadline, Some(date(2026, 12, 31)));
        assert_eq!(k.steam_app_id, Some(972660));
        assert_eq!(k.tags, ["a", "b", "c"]);
        assert_eq!(k.status, KeyStatus::Gifted);
    }

    #[test]
    fn validation() {
        let mut f = KeyForm::new_key();
        let fields: Vec<&str> = f.errors(MDY).into_iter().map(|(f, _)| f).collect();
        assert_eq!(fields, ["name", "key"]);
        f.name = TextEdit::new("X");
        f.key = TextEdit::new("AAAAA-BBBBB-CCCCC");
        f.deadline = TextEdit::new("13/40/2026");
        f.app_id = TextEdit::new("nope");
        f.key_url = TextEdit::new("humble");
        let fields: Vec<&str> = f.errors(MDY).into_iter().map(|(f, _)| f).collect();
        assert_eq!(fields, ["app_id", "key_url", "deadline"]);
        assert!(f.error_for("deadline", MDY).unwrap().contains("M/D/YYYY"));
    }

    #[test]
    fn new_key_from_form() {
        let mut f = KeyForm::new_key();
        f.name = TextEdit::new("Celeste");
        f.key = TextEdit::new(" QQ7RT-PP2MN-ZZ9XY ");
        let k = f.to_new_key(MDY);
        assert_eq!(k.game_name, "Celeste");
        assert_eq!(k.key.expose(), "QQ7RT-PP2MN-ZZ9XY");
        assert_eq!(k.status, KeyStatus::Available);
    }
}
