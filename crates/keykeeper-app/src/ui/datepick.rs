//! A small month calendar for the editor's date fields. Typing a date still works.

use jiff::ToSpan;
use jiff::civil::Date;
use keykeeper_core::DateFormat;
use keykeeper_core::import::values::parse_date;
use rune::{El, TextEdit, col, el, row, text};
use rune_kit::{IconButton, Kit, Placement, Popover};

use crate::app::{AppState, today};
use crate::form::{KeyForm, format_date};

pub static CALENDAR: rune::Icon = rune::Icon {
    name: "kk-calendar",
    view: 16.0,
    body: r#"<rect x="2.5" y="3.5" width="11" height="10" rx="1.6" stroke="currentColor" stroke-width="1.3" fill="none"/><path d="M2.5 6.5H13.5M5.5 2V4.5M10.5 2V4.5" stroke="currentColor" stroke-width="1.3" stroke-linecap="round"/>"#,
};

static CHEV_LEFT: rune::Icon = rune::Icon {
    name: "kk-chev-left",
    view: 16.0,
    body: r#"<path d="M10 3.5L5.5 8L10 12.5" stroke="currentColor" stroke-width="1.5" fill="none" stroke-linecap="round" stroke-linejoin="round"/>"#,
};

static CHEV_RIGHT: rune::Icon = rune::Icon {
    name: "kk-chev-right",
    view: 16.0,
    body: r#"<path d="M6 3.5L10.5 8L6 12.5" stroke="currentColor" stroke-width="1.5" fill="none" stroke-linecap="round" stroke-linejoin="round"/>"#,
};

/// Which date field's calendar is open, and the month it shows.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct DatePick {
    pub field: &'static str,
    pub month: Date,
}

fn edit_for<'a>(f: &'a mut KeyForm, field: &str) -> Option<&'a mut TextEdit> {
    match field {
        "deadline" => Some(&mut f.deadline),
        "status_date" => Some(&mut f.status_date),
        _ => None,
    }
}

fn first_of_month(d: Date) -> Date {
    d.first_of_month()
}

impl AppState {
    pub fn toggle_date_pick(&mut self, field: &'static str) {
        let Some(s) = self.session_mut() else { return };
        let fmt = s.date_format();
        let Some(f) = s.form.as_mut() else { return };
        if f.date_pick.is_some_and(|p| p.field == field) {
            f.date_pick = None;
            return;
        }
        let current = edit_for(f, field)
            .and_then(|e| parse_date(&e.text, fmt).ok().flatten())
            .unwrap_or_else(today);
        f.date_pick = Some(DatePick {
            field,
            month: first_of_month(current),
        });
    }

    pub fn close_date_pick(&mut self) {
        if let Some(f) = self.session_mut().and_then(|s| s.form.as_mut()) {
            f.date_pick = None;
        }
    }

    pub fn date_pick_month(&mut self, months: i32) {
        if let Some(p) = self
            .session_mut()
            .and_then(|s| s.form.as_mut())
            .and_then(|f| f.date_pick.as_mut())
            && let Ok(m) = p.month.checked_add(months.months())
        {
            p.month = first_of_month(m);
        }
    }

    /// Put `date` (or nothing) in the picked field and close the calendar.
    pub fn pick_date(&mut self, date: Option<Date>) {
        let Some(s) = self.session_mut() else { return };
        let fmt = s.date_format();
        let Some(f) = s.form.as_mut() else { return };
        let Some(p) = f.date_pick.take() else { return };
        if let Some(e) = edit_for(f, p.field) {
            e.replace_text(format_date(date, fmt));
        }
    }
}

const WEEKDAYS: [&str; 7] = ["Mo", "Tu", "We", "Th", "Fr", "Sa", "Su"];

/// The calendar button for `field`, with its popover when open.
pub fn button(
    k: &Kit,
    form: &KeyForm,
    field: &'static str,
    fmt: DateFormat,
    overlays: &mut Vec<El>,
) -> El {
    let open = form.date_pick.filter(|p| p.field == field);
    let trigger = el().key(format!("pick-{field}")).child(
        IconButton::new(&CALENDAR)
            .label("Pick a date")
            .on_press(move |s: &mut AppState| s.toggle_date_pick(field))
            .build(k),
    );
    let mut pop = Popover::new(format!("datepick-{field}"))
        .trigger(trigger)
        .label("Pick a date")
        .open(open.is_some())
        .placement(Placement::BottomEnd)
        .on_close(|s: &mut AppState| s.close_date_pick());
    if let Some(p) = open {
        let selected = match field {
            "deadline" => parse_date(&form.deadline.text, fmt).ok().flatten(),
            _ => parse_date(&form.status_date.text, fmt).ok().flatten(),
        };
        pop = pop.child(calendar(k, p.month, selected));
    }
    pop.build(k, overlays)
}

fn calendar(k: &Kit, month: Date, selected: Option<Date>) -> El {
    let now = today();
    let title = format!("{} {}", month_name(month.month()), month.year());
    let nav = |key: &str, glyph: &'static rune::Icon, label: &str, delta: i32| {
        el().key(key.to_string()).child(
            IconButton::new(glyph)
                .label(label.to_string())
                .on_press(move |s: &mut AppState| s.date_pick_month(delta))
                .build(k),
        )
    };
    let head = row()
        .w_full()
        .center_y()
        .child(nav("pick-prev", &CHEV_LEFT, "Previous month", -1))
        .child(
            text(title)
                .font(k.ty.ui_medium)
                .color(k.c.text_strong)
                .grow(1.0)
                .align(rune::Align::Center),
        )
        .child(nav("pick-next", &CHEV_RIGHT, "Next month", 1));
    let mut grid = col()
        .w_full()
        .gap(2.0)
        .child(row().w_full().children(WEEKDAYS.iter().map(|d| {
            text(d.to_string())
                .font(k.ty.text_xsmall_medium)
                .color(k.c.text_faint)
                .w(34.0)
                .align(rune::Align::Center)
        })));
    let lead = month.weekday().to_monday_zero_offset() as i64;
    let start = month.checked_sub(lead.days()).unwrap_or(month);
    for week in 0..6 {
        let mut r = row().w_full();
        for d in 0..7 {
            let day = start.checked_add((week * 7 + d).days()).unwrap_or(month);
            let in_month = day.month() == month.month();
            let is_sel = selected == Some(day);
            let is_today = day == now;
            let mut cell = el()
                .key(format!("pick-day-{day}"))
                .role("button")
                .label(day.to_string())
                .focusable(true)
                .size(34.0, 30.0)
                .radius(8.0)
                .center()
                .cursor(rune::Cursor::Pointer)
                .on_press(move |s: &mut AppState| s.pick_date(Some(day)))
                .child(
                    text(day.day().to_string())
                        .font(if is_today || is_sel {
                            k.ty.text_small_medium
                        } else {
                            k.ty.text_small
                        })
                        .color(if is_sel {
                            k.c.text_contrast
                        } else if !in_month {
                            k.c.text_faint
                        } else if is_today {
                            k.c.text_accent
                        } else {
                            k.c.text_base
                        })
                        .a11y_hidden(),
                );
            cell = if is_sel {
                cell.fill(k.c.bg_accent)
            } else {
                cell.hover(k.c.surface_hover)
            };
            if is_today && !is_sel {
                cell = cell.border(1.0, k.c.text_accent.alpha(0.6));
            }
            r = r.child(cell);
        }
        grid = grid.child(r);
        if start
            .checked_add(((week + 1) * 7).days())
            .is_ok_and(|d| d.month() != month.month() && week >= 3)
        {
            break;
        }
    }
    let foot = row()
        .w_full()
        .gap(8.0)
        .child(
            rune_kit::Button::new("Today")
                .variant(rune_kit::ButtonVariant::Ghost)
                .on_press(move |s: &mut AppState| s.pick_date(Some(now)))
                .build(k),
        )
        .child(super::spacer())
        .child(
            el().key("pick-clear").child(
                rune_kit::Button::new("Clear")
                    .variant(rune_kit::ButtonVariant::GhostMuted)
                    .on_press(|s: &mut AppState| s.pick_date(None))
                    .build(k),
            ),
        );
    col().w(238.0).gap(8.0).child(head).child(grid).child(foot)
}

fn month_name(m: i8) -> &'static str {
    [
        "January",
        "February",
        "March",
        "April",
        "May",
        "June",
        "July",
        "August",
        "September",
        "October",
        "November",
        "December",
    ]
    .get(usize::try_from(m - 1).unwrap_or(0))
    .copied()
    .unwrap_or("")
}
