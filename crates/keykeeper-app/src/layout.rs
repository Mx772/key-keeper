//! How the library table looks: columns (shown, widths, order), sort, row density and grouping
//! by bundle. All of it is remembered in `config.toml` (see `config::LibraryLayout`).

use std::collections::HashSet;

use keykeeper_core::query::natural_cmp;
use keykeeper_core::{LabelField, label_key};
use keykeeper_core::{Sort, SortDir, SortField};
use uuid::Uuid;

use crate::app::{AppState, Session};
use crate::config::{Density, LibraryLayout, LibraryView};

/// Every library column but the game's name (which always shows): id and title.
pub const COLUMNS: [(&str, &str); 10] = [
    ("bundle", "Bundle"),
    ("source", "Source"),
    ("key", "Key"),
    ("status", "Status"),
    ("platform", "Platform"),
    ("deadline", "Deadline"),
    ("rating", "Rating"),
    ("deals", "Deals"),
    ("steam", "Steam"),
    ("added", "Added"),
];

/// Seconds to wait after the last layout change (a column drag) before writing config.toml.
const SAVE_AFTER: f64 = 1.0;

/// What a table row shows: a key, or a bundle's header when grouping.
#[derive(Clone, Debug, PartialEq)]
pub enum DisplayRow {
    Group {
        name: String,
        count: usize,
        collapsed: bool,
    },
    Key(Uuid),
}

pub const NO_BUNDLE: &str = "No bundle";
pub const NO_SOURCE: &str = "No source";
const GROUP_PREFIX: &str = "group:";

pub fn group_row_id(name: &str) -> String {
    format!("{GROUP_PREFIX}{name}")
}

pub fn parse_group_id(id: &str) -> Option<&str> {
    id.strip_prefix(GROUP_PREFIX)
}

pub fn sort_to_string(s: Sort) -> String {
    let col = match s.field {
        SortField::Name => "name",
        SortField::Bundle => "bundle",
        SortField::Status => "status",
        SortField::Platform => "platform",
        SortField::Deadline => "deadline",
        SortField::Source => "source",
        SortField::Rating => "rating",
        SortField::Price => "deals",
        _ => "added",
    };
    let dir = if s.dir == SortDir::Desc {
        "desc"
    } else {
        "asc"
    };
    format!("{col}:{dir}")
}

/// `"name:asc"` → the column and direction.
pub fn parse_sort(s: &str) -> Option<(String, rune_kit::SortDir)> {
    let (col, dir) = s.split_once(':')?;
    let dir = match dir {
        "asc" => rune_kit::SortDir::Ascending,
        "desc" => rune_kit::SortDir::Descending,
        _ => return None,
    };
    let known = col == "name" || COLUMNS.iter().any(|c| c.0 == col);
    known.then(|| (col.to_string(), dir))
}

impl Session {
    /// What the table groups by, if anything.
    pub fn group_field(&self) -> Option<LabelField> {
        if self.group_by_source {
            Some(LabelField::Source)
        } else if self.group_by_bundle {
            Some(LabelField::Bundle)
        } else {
            None
        }
    }

    /// Rebuild `display` from `rows`: grouped by bundle or source (groups in natural order,
    /// spelled the way most of their keys spell them, case and spacing ignored; "No bundle"
    /// last, keys inside keeping their sort), or one row per key.
    pub fn rebuild_display(&mut self) {
        let Some(field) = self.group_field() else {
            self.display = self.rows.iter().copied().map(DisplayRow::Key).collect();
            self.row_ids = self.rows.iter().map(Uuid::to_string).collect();
            return;
        };
        {
            let none = match field {
                LabelField::Bundle => NO_BUNDLE,
                LabelField::Source => NO_SOURCE,
            };
            let names: std::collections::HashMap<String, String> =
                keykeeper_core::label_values(self.repo.keys(), field)
                    .into_iter()
                    .map(|(v, _)| (label_key(&v), v))
                    .collect();
            let group = |id: &Uuid| {
                self.repo.key(*id).and_then(|k| field.of(k)).map(|v| {
                    names
                        .get(&label_key(v))
                        .cloned()
                        .unwrap_or_else(|| v.trim().to_string())
                })
            };
            let mut rows: Vec<(Uuid, Option<String>)> =
                self.rows.iter().map(|id| (*id, group(id))).collect();
            rows.sort_by(|(_, a), (_, b)| match (a, b) {
                (Some(x), Some(y)) => natural_cmp(x, y),
                (Some(_), None) => std::cmp::Ordering::Less,
                (None, Some(_)) => std::cmp::Ordering::Greater,
                (None, None) => std::cmp::Ordering::Equal,
            });
            let mut display = Vec::with_capacity(rows.len() + 8);
            let mut i = 0;
            while i < rows.len() {
                let name = rows[i].1.clone().unwrap_or_else(|| none.to_string());
                let mut j = i;
                while j < rows.len() && rows[j].1.as_deref().unwrap_or(none) == name {
                    j += 1;
                }
                let collapsed = self.collapsed.contains(&name);
                display.push(DisplayRow::Group {
                    name: name.clone(),
                    count: j - i,
                    collapsed,
                });
                if !collapsed {
                    display.extend(rows[i..j].iter().map(|(id, _)| DisplayRow::Key(*id)));
                }
                i = j;
            }
            self.rows = rows.into_iter().map(|(id, _)| id).collect();
            self.display = display;
        }
        self.row_ids = self
            .display
            .iter()
            .map(|r| match r {
                DisplayRow::Group { name, .. } => group_row_id(name),
                DisplayRow::Key(id) => id.to_string(),
            })
            .collect();
    }

    /// The key on table row `i` (`None` for a group header).
    pub fn display_key(&self, i: usize) -> Option<Uuid> {
        match self.display.get(i)? {
            DisplayRow::Key(id) => Some(*id),
            DisplayRow::Group { .. } => None,
        }
    }

    /// Put a saved layout on the table.
    pub fn apply_layout(&mut self, l: &LibraryLayout) {
        for (id, _) in COLUMNS {
            self.table
                .set_column_hidden(id, l.hidden_columns.iter().any(|h| h == id));
        }
        for (id, w) in &l.column_widths {
            if w.is_finite() && *w > 20.0 {
                self.table.set_column_width(id.clone(), *w);
            }
        }
        if !l.column_order.is_empty() {
            self.table.set_column_order(l.column_order.clone());
        }
        self.group_by_bundle = l.group_by_bundle;
        self.group_by_source = l.group_by_source && !l.group_by_bundle;
        if let Some((col, dir)) = l.sort.as_deref().and_then(parse_sort) {
            self.table.set_sort(col.clone(), dir);
            self.sort = crate::app::sort_for(&col, dir);
        }
        self.filters_changed();
    }

    /// The table's current layout, to compare with and save into the config.
    pub fn read_layout(&self, mut into: LibraryLayout) -> LibraryLayout {
        into.hidden_columns = COLUMNS
            .iter()
            .filter(|(id, _)| self.table.is_column_hidden(id))
            .map(|(id, _)| id.to_string())
            .collect();
        into.column_widths = std::iter::once("name")
            .chain(COLUMNS.iter().map(|c| c.0))
            .filter_map(|id| {
                self.table
                    .column_width(id)
                    .map(|w| (id.to_string(), w.round()))
            })
            .collect();
        into.column_order = self.table.column_order().to_vec();
        into.group_by_bundle = self.group_by_bundle;
        into.group_by_source = self.group_by_source;
        into.sort = Some(sort_to_string(self.sort));
        into
    }
}

impl AppState {
    /// Notice layout changes made in the table (resizes, reorders, sorts, hidden columns) and
    /// save them once things settle. Returns nothing visible.
    pub fn poll_layout(&mut self, now: f64, tx: &mut rune::Tx) {
        if self.offline {
            return; // screenshots and tests never touch the user's config
        }
        let Some(s) = self.session() else { return };
        let current = s.read_layout(self.config.library.clone());
        if current != self.config.library {
            self.config.library = current;
            self.layout_save_at = Some(now + SAVE_AFTER);
        }
        if let Some(at) = self.layout_save_at {
            if now >= at {
                self.layout_save_at = None;
                self.save_config();
            } else {
                tx.wake_after(std::time::Duration::from_secs_f64(at - now));
            }
        }
    }

    pub fn set_density(&mut self, d: Density) {
        self.update_config(|c| c.library.density = d);
    }

    pub fn set_library_view(&mut self, v: LibraryView) {
        self.hover = None;
        self.update_config(|c| c.library.view = v);
        if let Some(s) = self.session_mut() {
            s.view_menu = false;
        }
    }

    pub fn toggle_group_by_bundle(&mut self) {
        if let Some(s) = self.session_mut() {
            s.group_by_bundle = !s.group_by_bundle;
            s.group_by_source = false;
            s.collapsed.clear();
            s.filters_changed();
        }
    }

    pub fn toggle_group_by_source(&mut self) {
        if let Some(s) = self.session_mut() {
            s.group_by_source = !s.group_by_source;
            s.group_by_bundle = false;
            s.collapsed.clear();
            s.filters_changed();
        }
    }

    /// Fold or unfold one bundle's rows.
    pub fn toggle_group(&mut self, name: &str) {
        if let Some(s) = self.session_mut() {
            if !s.collapsed.remove(name) {
                s.collapsed.insert(name.to_string());
            }
            s.filters_changed();
        }
    }

    pub fn collapse_all_groups(&mut self, collapse: bool) {
        if let Some(s) = self.session_mut() {
            if collapse {
                let names: HashSet<String> = s
                    .display
                    .iter()
                    .filter_map(|r| match r {
                        DisplayRow::Group { name, .. } => Some(name.clone()),
                        DisplayRow::Key(_) => None,
                    })
                    .collect();
                s.collapsed = names;
            } else {
                s.collapsed.clear();
            }
            s.filters_changed();
        }
    }

    pub fn toggle_column(&mut self, id: &str) {
        if let Some(s) = self.session_mut() {
            let hidden = s.table.is_column_hidden(id);
            s.table.set_column_hidden(id.to_string(), !hidden);
        }
    }

    /// Every column back, at its default width and place.
    pub fn reset_columns(&mut self) {
        if let Some(s) = self.session_mut() {
            for (id, _) in COLUMNS {
                s.table.show_column(id);
                s.table.reset_column_width(id);
            }
            s.table.reset_column_width("name");
            s.table.reset_column_order();
            s.view_menu = false;
            s.header_menu = None;
        }
    }

    pub fn toggle_view_menu(&mut self) {
        self.hover = None;
        if let Some(s) = self.session_mut() {
            s.view_menu = !s.view_menu;
            s.view_columns_open = false;
        }
    }

    pub fn close_view_menu(&mut self) {
        if let Some(s) = self.session_mut() {
            s.view_menu = false;
            s.view_columns_open = false;
            s.header_menu = None;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sorts_round_trip() {
        for (f, col) in [
            (SortField::Name, "name"),
            (SortField::Price, "deals"),
            (SortField::Deadline, "deadline"),
            (SortField::Added, "added"),
        ] {
            for d in [SortDir::Asc, SortDir::Desc] {
                let s = sort_to_string(Sort { field: f, dir: d });
                let (c, dir) = parse_sort(&s).unwrap();
                assert_eq!(c, col);
                assert_eq!(
                    crate::app::sort_for(&c, dir),
                    Sort { field: f, dir: d },
                    "{s}"
                );
            }
        }
        assert_eq!(parse_sort("nope:asc"), None);
        assert_eq!(parse_sort("name:sideways"), None);
        assert_eq!(parse_sort("garbage"), None);
    }

    #[test]
    fn group_ids() {
        assert_eq!(
            parse_group_id(&group_row_id("Humble Choice")),
            Some("Humble Choice")
        );
        assert_eq!(parse_group_id("0197-uuid"), None);
    }
}
