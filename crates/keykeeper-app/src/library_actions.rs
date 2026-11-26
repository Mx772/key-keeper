//! Library row actions: the right-click menu and keyboard navigation.

use keykeeper_core::KeyStatus;
use rune::{PointerPress, Tx};
use uuid::Uuid;

use crate::app::{AppState, Page, Session};

/// The open row menu: the keys it acts on and where it opened (`None`: by keyboard, at the
/// cursor row).
#[derive(Clone, Debug, PartialEq)]
pub struct RowMenu {
    pub ids: Vec<Uuid>,
    pub at: Option<PointerPress>,
    pub status_open: bool,
}

impl Session {
    /// The row the keyboard cursor is on (else the only selected row).
    pub fn cursor_id(&self) -> Option<Uuid> {
        self.table
            .cursor()
            .and_then(|c| Uuid::parse_str(c).ok())
            .filter(|id| self.repo.key(*id).is_some() && self.table.is_selected(&id.to_string()))
            .or_else(|| match self.selected_ids().as_slice() {
                [one] => Some(*one),
                _ => None,
            })
    }

    /// Keys an action on row `id` applies to: the whole selection when `id` is part of it.
    pub fn action_ids(&self, id: Uuid) -> Vec<Uuid> {
        let sel = self.selected_ids();
        if sel.contains(&id) { sel } else { vec![id] }
    }

    /// Whether the key is in the open cart.
    pub fn in_cart(&self, id: Uuid) -> bool {
        self.repo.key(id).is_some_and(|k| {
            k.status == KeyStatus::Reserved
                && k.trade_id.is_some()
                && self.repo.cart().map(|c| c.id) == k.trade_id
        })
    }
}

impl AppState {
    /// Right-click on row `id`: select it unless it's already part of the selection, then open
    /// the menu for the selection.
    pub fn open_row_menu(&mut self, id: Uuid, at: Option<PointerPress>) {
        self.hover = None;
        let Some(s) = self.session_mut() else { return };
        if !s.table.is_selected(&id.to_string()) {
            s.table.select(id.to_string());
        }
        let ids = s.action_ids(id);
        s.row_menu = Some(RowMenu {
            ids,
            at,
            status_open: false,
        });
    }

    pub fn close_row_menu(&mut self) {
        if let Some(s) = self.session_mut() {
            s.row_menu = None;
        }
    }

    pub fn set_row_menu_status_open(&mut self, open: bool) {
        if let Some(m) = self.session_mut().and_then(|s| s.row_menu.as_mut()) {
            m.status_open = open;
        }
    }

    /// Move the selection `delta` rows, opening the key like a click does.
    pub fn move_cursor(&mut self, delta: isize, tx: &mut Tx) {
        let Some(s) = self.session_mut() else { return };
        if s.rows.is_empty() {
            return;
        }
        let at = s
            .cursor_id()
            .and_then(|id| s.rows.iter().position(|r| *r == id));
        let next = match at {
            None if delta >= 0 => 0,
            None => s.rows.len() - 1,
            Some(i) => i
                .saturating_add_signed(delta)
                .min(s.rows.len().saturating_sub(1)),
        };
        let id = s.rows[next];
        s.table.select(id.to_string());
        s.table.reveal(id.to_string(), tx);
        self.open_key(id);
    }

    /// Ctrl+B: put the selected keys in the cart, or take them out if they're all in it already.
    pub fn toggle_cart(&mut self) {
        let Some(s) = self.session() else { return };
        let ids = s.selected_ids();
        if ids.is_empty() {
            return;
        }
        if ids.iter().all(|id| s.in_cart(*id)) {
            let n = ids.len();
            for id in ids {
                self.remove_from_cart(id);
            }
            self.toast(if n == 1 {
                "Removed from cart".to_string()
            } else {
                format!("Removed {n} from cart")
            });
        } else {
            self.add_to_cart(&ids);
        }
    }

    /// Library keys the table leaves alone. Text fields get keys first, so typing is safe; the
    /// table itself handles arrows, Enter and type-to-jump while it has focus.
    pub(crate) fn library_key(&mut self, e: &rune::KeyEvent, tx: &mut Tx) -> bool {
        let Some(s) = self.session() else {
            return false;
        };
        if s.page != Page::Library || s.confirm.is_some() || s.shortcuts_open {
            return false;
        }
        let m = e.mods;
        let ctrl = m.ctrl || m.cmd;
        match &e.key {
            // Arrows while the table isn't focused: move and hand it the focus.
            rune::Key::Down | rune::Key::Up if !ctrl && !m.alt => {
                self.move_cursor(if e.key == rune::Key::Down { 1 } else { -1 }, tx);
                tx.focus("library-table", false);
                true
            }
            rune::Key::Command('b') if ctrl && !m.shift => {
                self.toggle_cart();
                true
            }
            rune::Key::Command('c') if !m.shift => match s.cursor_id() {
                Some(id) => {
                    self.copy_key(id, tx);
                    true
                }
                None => false,
            },
            rune::Key::Function(2) => match s.cursor_id() {
                Some(id) => {
                    self.open_key(id);
                    tx.focus("form-name", true);
                    true
                }
                None => false,
            },
            rune::Key::ContextMenu => self.keyboard_menu(),
            rune::Key::Function(10) if m.shift => self.keyboard_menu(),
            _ => false,
        }
    }

    /// While the table has focus, keys reach the app's handler only after the table's own and
    /// Rune's (Ctrl+C, Delete); library shortcuts run here first instead.
    pub fn table_key_preview(&mut self, e: &rune::KeyEvent, tx: &mut Tx) -> bool {
        let ctrl = e.mods.ctrl || e.mods.cmd;
        let ours = match e.key {
            rune::Key::Command('b' | 'c') => ctrl,
            rune::Key::Delete | rune::Key::Function(2) | rune::Key::ContextMenu => true,
            rune::Key::Function(10) => e.mods.shift,
            _ => false,
        };
        ours && self.on_key(e, tx)
    }

    fn keyboard_menu(&mut self) -> bool {
        match self.session().and_then(Session::cursor_id) {
            Some(id) => {
                self.open_row_menu(id, None);
                true
            }
            None => false,
        }
    }
}
