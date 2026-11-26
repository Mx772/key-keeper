//! Cell-level parsing for imports: null tokens, dates, statuses, ids.

use jiff::civil::Date;

use crate::links;
use crate::model::{DateFormat, KeyStatus};

/// `""`, `None`, `N/A`, `-`, … all mean "no value".
pub fn is_null(s: &str) -> bool {
    matches!(
        s.trim().to_lowercase().as_str(),
        "" | "none" | "n/a" | "na" | "null" | "nil" | "-" | "—" | "–"
    )
}

/// Trim, undo spreadsheet formula-escaping (`'=x` → `=x`, see `export::sanitize_cell`), and map
/// null tokens to `""`.
pub fn clean(s: &str) -> &str {
    let t = s.trim();
    let t = match t.strip_prefix('\'') {
        Some(rest) if rest.starts_with(['=', '+', '-', '@']) => rest,
        _ => t,
    };
    if is_null(t) { "" } else { t }
}

/// Parse a date cell. ISO `YYYY-MM-DD` is always accepted; `a/b/yyyy` (also with `.` or `-`)
/// follows `format`. Two-digit years mean 20xx. A trailing time (`1/7/2026 13:45`) is ignored.
pub fn parse_date(s: &str, format: DateFormat) -> Result<Option<Date>, String> {
    let t = clean(s);
    if t.is_empty() {
        return Ok(None);
    }
    let first = t.split([' ', 'T']).next().unwrap_or(t);
    let parts: Vec<&str> = first.split(['/', '.', '-']).collect();
    let nums: Option<Vec<i32>> = parts.iter().map(|p| p.parse::<i32>().ok()).collect();
    let (Some(nums), 3) = (nums, parts.len()) else {
        return Err(format!("'{t}' is not a date"));
    };
    let (y, m, d) = if parts[0].len() == 4 {
        (nums[0], nums[1], nums[2])
    } else {
        let y = if parts[2].len() <= 2 {
            2000 + nums[2]
        } else {
            nums[2]
        };
        match format {
            DateFormat::MonthDayYear => (y, nums[0], nums[1]),
            DateFormat::DayMonthYear => (y, nums[1], nums[0]),
        }
    };
    let (Ok(y), Ok(m), Ok(d)) = (i16::try_from(y), i8::try_from(m), i8::try_from(d)) else {
        return Err(format!("'{t}' is not a valid date"));
    };
    Date::new(y, m, d)
        .map(Some)
        .map_err(|_| format!("'{t}' is not a valid date"))
}

/// How status cells map to [`KeyStatus`]. Defaults: X/Yes/Redeemed + trade link = Traded, without a link = Redeemed, blank/No = Available.
#[derive(Clone, Debug, PartialEq)]
pub struct StatusRules {
    /// A "done" mark (`X`, `Yes`, `Redeemed`, `1`, …) on a row that has a trade link.
    pub done_with_link: KeyStatus,
    /// A "done" mark without a trade link.
    pub done_without_link: KeyStatus,
    /// An empty cell, or `No` / `False` / `0`.
    pub not_done: KeyStatus,
    /// Exact, case-insensitive overrides checked first, e.g. `("sold", Traded)`.
    pub custom: Vec<(String, KeyStatus)>,
}

impl Default for StatusRules {
    fn default() -> Self {
        StatusRules {
            done_with_link: KeyStatus::Traded,
            done_without_link: KeyStatus::Redeemed,
            not_done: KeyStatus::Available,
            custom: Vec::new(),
        }
    }
}

const DONE: &[&str] = &[
    "x",
    "✓",
    "✔",
    "yes",
    "y",
    "true",
    "1",
    "done",
    "redeemed",
    "used",
    "activated",
    "claimed",
];
const NOT_DONE: &[&str] = &[
    "no",
    "n",
    "false",
    "0",
    "unused",
    "unredeemed",
    "not redeemed",
];

impl StatusRules {
    /// Returns the status and, for unrecognised text, a warning.
    pub fn map(&self, raw: &str, has_trade_link: bool) -> (KeyStatus, Option<String>) {
        let t = clean(raw).to_lowercase();
        if let Some((_, s)) = self
            .custom
            .iter()
            .find(|(k, _)| k.trim().eq_ignore_ascii_case(&t))
        {
            return (*s, None);
        }
        if t.is_empty() || NOT_DONE.contains(&t.as_str()) {
            // No status column value, but a trade link says it went somewhere.
            return (
                if t.is_empty() && has_trade_link {
                    self.done_with_link
                } else {
                    self.not_done
                },
                None,
            );
        }
        if DONE.contains(&t.as_str()) {
            return (
                if has_trade_link {
                    self.done_with_link
                } else {
                    self.done_without_link
                },
                None,
            );
        }
        match t.as_str() {
            "available" => (KeyStatus::Available, None),
            "reserved" | "pending" | "in trade" => (KeyStatus::Reserved, None),
            "traded" | "sold" => (KeyStatus::Traded, None),
            "gifted" | "gift" | "given" => (KeyStatus::Gifted, None),
            "unknown" | "?" => (KeyStatus::Unknown, None),
            _ => (
                KeyStatus::Unknown,
                Some(format!(
                    "unrecognised status '{}', set to Unknown",
                    clean(raw)
                )),
            ),
        }
    }
}

/// A Steam app id from a number or a store/gg.deals URL.
pub fn parse_app_id(s: &str) -> Option<u32> {
    let t = clean(s);
    t.parse().ok().or_else(|| links::steam_app_id_from_url(t))
}

pub fn is_url(s: &str) -> bool {
    links::host(s).is_some()
}

#[cfg(test)]
mod tests {
    use super::*;
    use jiff::civil::date;

    const MDY: DateFormat = DateFormat::MonthDayYear;
    const DMY: DateFormat = DateFormat::DayMonthYear;

    #[test]
    fn nulls() {
        for s in ["", "  ", "None", "N/A", "na", "null", "-", "—"] {
            assert!(is_null(s), "{s:?}");
        }
        assert!(!is_null("0"));
        assert_eq!(clean(" None "), "");
        assert_eq!(clean("'=SUM(A1)"), "=SUM(A1)");
        assert_eq!(clean("'quoted"), "'quoted");
    }

    #[test]
    fn dates_follow_the_setting() {
        assert_eq!(parse_date("1/7/2026", MDY), Ok(Some(date(2026, 1, 7))));
        assert_eq!(parse_date("1/7/2026", DMY), Ok(Some(date(2026, 7, 1))));
        assert_eq!(parse_date("31.12.2026", DMY), Ok(Some(date(2026, 12, 31))));
        assert_eq!(
            parse_date("2026-01-07", DMY),
            Ok(Some(date(2026, 1, 7))),
            "ISO ignores the setting"
        );
        assert_eq!(parse_date("8/20/25", MDY), Ok(Some(date(2025, 8, 20))));
        assert_eq!(
            parse_date("1/7/2026 13:45", MDY),
            Ok(Some(date(2026, 1, 7)))
        );
        assert_eq!(parse_date("None", MDY), Ok(None));
    }

    #[test]
    fn bad_dates_are_errors() {
        assert!(parse_date("8/20/2025", DMY).is_err(), "month 20");
        assert!(parse_date("2/30/2026", MDY).is_err());
        assert!(parse_date("tomorrow", MDY).is_err());
        assert!(parse_date("1/2", MDY).is_err());
        assert!(parse_date("99999/1/1", MDY).is_err());
    }

    #[test]
    fn status_defaults() {
        let r = StatusRules::default();
        assert_eq!(r.map("Redeemed", true).0, KeyStatus::Traded);
        assert_eq!(r.map("X", true).0, KeyStatus::Traded);
        assert_eq!(r.map("x", false).0, KeyStatus::Redeemed);
        assert_eq!(r.map("", false).0, KeyStatus::Available);
        assert_eq!(
            r.map("", true).0,
            KeyStatus::Traded,
            "a trade link alone implies traded"
        );
        assert_eq!(
            r.map("No", true).0,
            KeyStatus::Available,
            "an explicit no wins over the link"
        );
        assert_eq!(r.map("Gifted", false).0, KeyStatus::Gifted);
        assert_eq!(r.map("Traded", false).0, KeyStatus::Traded);
        let (s, warn) = r.map("maybe", false);
        assert_eq!(s, KeyStatus::Unknown);
        assert!(warn.is_some());
    }

    #[test]
    fn status_custom_rules_win() {
        let r = StatusRules {
            custom: vec![
                ("X".into(), KeyStatus::Gifted),
                ("sold out".into(), KeyStatus::Traded),
            ],
            ..StatusRules::default()
        };
        assert_eq!(r.map("x", true).0, KeyStatus::Gifted);
        assert_eq!(r.map(" Sold Out ", false).0, KeyStatus::Traded);
    }

    #[test]
    fn app_ids() {
        assert_eq!(parse_app_id("972660"), Some(972660));
        assert_eq!(
            parse_app_id("https://store.steampowered.com/app/972660/Spiritfarer/"),
            Some(972660)
        );
        assert_eq!(parse_app_id("Spiritfarer"), None);
    }
}
