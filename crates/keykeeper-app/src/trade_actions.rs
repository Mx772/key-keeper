//! Trade actions on `AppState`: cart, checkout, send/complete/cancel, reputation text, and the
//! template editor. The core lifecycle lives in `keykeeper_core::trade`.

use std::time::Duration;

use keykeeper_core::template::{self, KEY_VARS, TRADE_VARS, TemplateError};
use keykeeper_core::trade::{parse_received, received_text};
use keykeeper_core::{Templates, Trade, TradeSite, TradeStatus};
use rune::{TextEdit, Tx};
use uuid::Uuid;

use crate::app::{AppState, Confirm, Page, Session, today};

/// Editable buffers for one trade's details (partner, links, what I receive, notes).
#[derive(Clone)]
pub struct TradeForm {
    pub trade_id: Uuid,
    pub site: TradeSite,
    pub partner: TextEdit,
    pub partner_url: TextEdit,
    pub trade_url: TextEdit,
    pub received: TextEdit,
    pub notes: TextEdit,
    pub add_received: bool,
    pub site_open: bool,
    pub show_keys: bool,
}

fn opt(e: &TextEdit) -> Option<String> {
    Some(e.text.trim())
        .filter(|s| !s.is_empty())
        .map(str::to_string)
}

impl TradeForm {
    pub fn from_trade(t: &Trade, add_received_default: bool) -> TradeForm {
        TradeForm {
            trade_id: t.id,
            site: t.site.clone(),
            partner: TextEdit::new(t.partner_name.clone()),
            partner_url: TextEdit::new(t.partner_profile_url.clone().unwrap_or_default()),
            trade_url: TextEdit::new(t.trade_url.clone().unwrap_or_default()),
            received: TextEdit::new(received_text(&t.received)),
            notes: TextEdit::new(t.notes.clone()),
            add_received: if t.status == TradeStatus::Draft {
                add_received_default
            } else {
                t.add_received_to_library
            },
            site_open: false,
            show_keys: false,
        }
    }

    /// `t` with this form's details written in. A recognised trade link sets the site.
    pub fn apply(&self, t: &Trade) -> Trade {
        let trade_url = opt(&self.trade_url);
        let site = match trade_url.as_deref().and_then(TradeSite::from_url) {
            Some(s @ (TradeSite::Lestrades | TradeSite::SteamTrades | TradeSite::Reddit))
                if self.site == t.site =>
            {
                s
            }
            _ => self.site.clone(),
        };
        Trade {
            site,
            partner_name: self.partner.text.trim().to_string(),
            partner_profile_url: opt(&self.partner_url),
            trade_url,
            received: parse_received(&self.received.text),
            notes: self.notes.text.clone(),
            add_received_to_library: self.add_received,
            ..t.clone()
        }
    }
}

/// Buffers for the template editor in Settings.
#[derive(Clone)]
pub struct TemplatesUi {
    pub checkout: TextEdit,
    pub rep: TextEdit,
}

impl TemplatesUi {
    pub fn from(t: &Templates) -> TemplatesUi {
        TemplatesUi {
            checkout: TextEdit::new(t.checkout_line.clone()),
            rep: TextEdit::new(t.rep.clone()),
        }
    }

    pub fn checkout_errors(&self) -> Vec<TemplateError> {
        template::check(&self.checkout.text, &[KEY_VARS, TRADE_VARS])
    }

    pub fn rep_errors(&self) -> Vec<TemplateError> {
        template::check(&self.rep.text, &[TRADE_VARS])
    }

    pub fn to_templates(&self) -> Templates {
        Templates {
            checkout_line: self.checkout.text.clone(),
            rep: self.rep.text.clone(),
        }
    }
}

/// Sample values for template previews.
pub fn sample_var(name: &str, date: &str) -> Option<String> {
    Some(
        match name {
            "name" => "Undertale",
            "key" => "UNDR7-TALE0-00001",
            "bundle" => "Humble Conquer COVID-19 Bundle",
            "platform" => "Steam",
            "steam_url" => "https://store.steampowered.com/app/391540/",
            "notes" => "",
            "partner" => "lumen",
            "partner_url" => "https://lestrades.com/user/lumen/",
            "trade_url" => "https://lestrades.com/offer/100001/",
            "site" => "Lestrades",
            "date" => date,
            "count" => "2",
            "games" => "Undertale, Celeste",
            "received" => "Tunic, A Short Hike",
            _ => return None,
        }
        .to_string(),
    )
}

impl Session {
    /// The trade the current page edits: the cart on Cart, the selection on Trades.
    pub fn form_target(&self) -> Option<Uuid> {
        match self.page {
            Page::Cart => self.repo.cart().map(|t| t.id),
            Page::Trades => self.selected_trade,
            _ => None,
        }
    }

    /// Write the trade form's details back to its trade (no-op if unchanged or gone).
    pub fn sync_trade_form(&mut self) {
        let Some(form) = self.trade_form.as_ref() else {
            return;
        };
        let Some(t) = self.repo.trade(form.trade_id) else {
            return;
        };
        let updated = form.apply(t);
        if updated != *t {
            let _ = self.repo.update_trade_details(updated);
        }
    }

    /// Point the trade form at the page's target trade. Returns whether it changed.
    pub fn refresh_trade_form(&mut self) -> bool {
        let target = self.form_target();
        if self.trade_form.as_ref().map(|f| f.trade_id) == target {
            return false;
        }
        self.sync_trade_form();
        let default = self.settings().add_received_to_library;
        self.trade_form = target
            .and_then(|id| self.repo.trade(id))
            .map(|t| TradeForm::from_trade(t, default));
        true
    }
}

impl AppState {
    fn with_trade_form(&mut self, f: impl FnOnce(&mut TradeForm)) {
        if let Some(form) = self.session_mut().and_then(|s| s.trade_form.as_mut()) {
            f(form);
        }
    }

    pub fn set_trade_site(&mut self, site: TradeSite) {
        self.with_trade_form(|f| {
            f.site = site;
            f.site_open = false;
        });
    }

    pub fn toggle_trade_site_open(&mut self) {
        self.with_trade_form(|f| f.site_open = !f.site_open);
    }

    pub fn close_trade_site(&mut self) {
        self.with_trade_form(|f| f.site_open = false);
    }

    pub fn toggle_add_received(&mut self) {
        self.with_trade_form(|f| f.add_received = !f.add_received);
    }

    pub fn toggle_show_keys(&mut self) {
        self.with_trade_form(|f| f.show_keys = !f.show_keys);
    }

    pub fn add_to_cart(&mut self, ids: &[Uuid]) {
        let Some(s) = self.session_mut() else { return };
        let res = s.repo.add_to_cart(ids);
        s.table.clear_selection();
        let fmt = s.date_format();
        if let Some(form) = &s.form
            && let Some(t) = form.target
            && ids.contains(&t)
            && let Some(k) = s.repo.key(t)
        {
            s.form = Some(crate::form::KeyForm::from_key(k, fmt));
        }
        if res.added > 0 {
            self.cart_bump_pending = true;
        }
        let msg = match (res.added, res.skipped) {
            (0, _) => {
                "Nothing added — those keys are already traded, redeemed or reserved".to_string()
            }
            (n, 0) => format!("Added {n} to cart"),
            (n, k) => format!("Added {n} to cart, skipped {k} unavailable"),
        };
        self.toast(msg);
    }

    /// Mark keys as redeemed on my own account today (they leave the cart if they were in it).
    /// With `open_steam`, a single Steam key also opens Steam's activation page with it filled in.
    pub fn redeem_self(&mut self, ids: &[Uuid], open_steam: bool, tx: &mut Tx) {
        let Some(s) = self.session_mut() else { return };
        let url = match ids {
            [id] if open_steam => s.repo.key(*id).and_then(|k| k.steam_register_url()),
            _ => None,
        };
        let done = s.repo.redeem_keys(ids, today());
        let fmt = s.date_format();
        if let Some(f) = s.form.as_mut()
            && let Some(t) = f.target
            && done.contains(&t)
            && let Some(k) = s.repo.key(t)
        {
            *f = crate::form::KeyForm::from_key(k, fmt);
        }
        if s.repo.cart().is_none() {
            s.trade_form = None;
        }
        let name = match done.as_slice() {
            [one] => s.repo.key(*one).map(|k| k.game_name.clone()),
            _ => None,
        };
        if done.is_empty() {
            self.toast("Nothing to redeem — those keys are already gone or in a sent trade");
            return;
        }
        if let Some(url) = url {
            let _ = tx.open_url(&url);
        }
        let undo = crate::ui::shortcut("Ctrl+Z");
        self.toast(match name {
            Some(n) => format!("Marked {n} as redeemed today — {undo} to undo"),
            None => format!(
                "Marked {} keys as redeemed today — {undo} to undo",
                done.len()
            ),
        });
    }

    pub fn ask_redeem_cart(&mut self) {
        if let Some(s) = self.session_mut()
            && s.repo.cart_len() > 0
        {
            s.confirm = Some(Confirm::RedeemCart);
        }
    }

    pub(crate) fn redeem_cart_now(&mut self) {
        let Some(s) = self.session_mut() else { return };
        s.sync_trade_form();
        let ids = s.repo.cart().map(|c| c.key_ids.clone()).unwrap_or_default();
        let done = s.repo.redeem_keys(&ids, today());
        s.trade_form = None;
        s.refresh_trade_form();
        let n = done.len();
        self.toast(format!(
            "Self-checkout: {n} key{} marked redeemed today — {} to undo",
            if n == 1 { "" } else { "s" },
            crate::ui::shortcut("Ctrl+Z")
        ));
    }

    pub fn remove_from_cart(&mut self, id: Uuid) {
        if let Some(s) = self.session_mut() {
            s.repo.remove_from_cart(&[id]);
        }
    }

    pub fn clear_cart(&mut self) {
        if let Some(s) = self.session_mut() {
            s.trade_form = None;
            let n = s.repo.clear_cart();
            self.toast(format!(
                "Cart cleared, {n} key{} released",
                if n == 1 { "" } else { "s" }
            ));
        }
    }

    pub fn select_trade(&mut self, id: Uuid) {
        if let Some(s) = self.session_mut() {
            s.page = Page::Trades;
            s.selected_trade = Some(id);
            s.refresh_trade_form();
        }
    }

    /// Cart → Pending, then show it on the Trades page.
    pub fn send_cart(&mut self) {
        let Some(s) = self.session_mut() else { return };
        s.sync_trade_form();
        let Some(id) = s.repo.cart().map(|t| t.id) else {
            return;
        };
        match s.repo.send_trade(id) {
            Ok(()) => {
                self.select_trade(id);
                self.toast("Trade marked as sent — complete it once you've received your side");
            }
            Err(e) => self.toast_error("Can't send this trade", e.to_string()),
        }
    }

    pub fn complete_trade(&mut self, id: Uuid) {
        let Some(s) = self.session_mut() else { return };
        s.sync_trade_form();
        let add = s
            .trade_form
            .as_ref()
            .filter(|f| f.trade_id == id)
            .map_or(s.settings().add_received_to_library, |f| f.add_received);
        match s.repo.complete_trade(id, today(), add) {
            Ok(done) => {
                s.trade_form = None;
                self.select_trade(id);
                let mut msg = format!(
                    "Trade completed — {} key{} marked traded",
                    done.traded,
                    if done.traded == 1 { "" } else { "s" }
                );
                if done.received_added > 0 {
                    msg.push_str(&format!(
                        ", {} received game{} added",
                        done.received_added,
                        if done.received_added == 1 { "" } else { "s" }
                    ));
                }
                self.toast(msg);
            }
            Err(e) => self.toast_error("Can't complete this trade", e.to_string()),
        }
    }

    pub fn ask_cancel_trade(&mut self, id: Uuid) {
        if let Some(s) = self.session_mut() {
            s.confirm = Some(Confirm::CancelTrade(id));
        }
    }

    pub(crate) fn cancel_trade_now(&mut self, id: Uuid) {
        let Some(s) = self.session_mut() else { return };
        s.sync_trade_form();
        match s.repo.cancel_trade(id) {
            Ok(n) => {
                s.trade_form = None;
                s.refresh_trade_form();
                self.toast(format!(
                    "Trade cancelled, {n} key{} released",
                    if n == 1 { "" } else { "s" }
                ));
            }
            Err(e) => self.toast_error("Can't cancel this trade", e.to_string()),
        }
    }

    pub fn copy_checkout(&mut self, id: Uuid, tx: &mut Tx) {
        let Some(s) = self.session_mut() else { return };
        s.sync_trade_form();
        let Some(t) = s.repo.trade(id) else { return };
        match s.repo.checkout_text(t, false) {
            Ok(text) if !text.is_empty() => {
                let n = text.lines().count();
                let secs = s.settings().clipboard_clear_secs;
                if secs == 0 {
                    tx.copy(text);
                } else {
                    let _ = tx.copy_secret(text, Duration::from_secs(u64::from(secs)));
                }
                self.toast(format!(
                    "Copied {n} line{} for checkout",
                    if n == 1 { "" } else { "s" }
                ));
            }
            Ok(_) => {}
            Err(e) => self.toast_error("Checkout template error", e.to_string()),
        }
    }

    pub fn copy_rep(&mut self, id: Uuid, tx: &mut Tx) {
        let Some(s) = self.session_mut() else { return };
        s.sync_trade_form();
        let Some(t) = s.repo.trade(id) else { return };
        match s.repo.rep_text(t) {
            Ok(text) => {
                tx.copy(text);
                self.toast("Reputation text copied — paste it on steamtrades.com");
            }
            Err(e) => self.toast_error("Reputation template error", e.to_string()),
        }
    }

    pub fn toggle_rep_posted(&mut self, id: Uuid) {
        if let Some(s) = self.session_mut()
            && let Some(posted) = s.repo.trade(id).map(|t| t.rep_posted)
        {
            let _ = s.repo.set_rep_posted(id, !posted);
        }
    }

    pub fn save_templates(&mut self) {
        let Some(s) = self.session_mut() else { return };
        if !s.templates_ui.checkout_errors().is_empty() || !s.templates_ui.rep_errors().is_empty() {
            return;
        }
        s.repo.set_templates(s.templates_ui.to_templates());
        self.toast("Templates saved");
    }

    pub fn reset_templates(&mut self) {
        if let Some(s) = self.session_mut() {
            s.templates_ui = TemplatesUi::from(&Templates::default());
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn form_apply_detects_site_from_link() {
        let t = Trade::new();
        let mut f = TradeForm::from_trade(&t, true);
        assert!(f.add_received, "drafts take the settings default");
        f.trade_url = TextEdit::new("https://www.steamtrades.com/trade/abc/");
        f.partner = TextEdit::new("  lumen ");
        f.received = TextEdit::new("Tunic: TUN1C-F0X99-AAAA1\nA Short Hike");
        let out = f.apply(&t);
        assert_eq!(out.site, TradeSite::SteamTrades);
        assert_eq!(out.partner_name, "lumen");
        assert_eq!(out.received.len(), 2);
        // An explicitly chosen site wins over detection.
        f.site = TradeSite::Reddit;
        assert_eq!(f.apply(&t).site, TradeSite::Reddit);
    }

    #[test]
    fn template_editor_validation() {
        let mut ui = TemplatesUi::from(&Templates::default());
        assert!(ui.checkout_errors().is_empty() && ui.rep_errors().is_empty());
        ui.rep = TextEdit::new("{key} leaked");
        assert_eq!(ui.rep_errors().len(), 1);
        ui.checkout = TextEdit::new("{name");
        assert_eq!(ui.checkout_errors(), vec![TemplateError::Unclosed]);
    }

    #[test]
    fn sample_vars_cover_every_variable() {
        for v in KEY_VARS.iter().chain(TRADE_VARS) {
            assert!(sample_var(v, "1/7/2026").is_some(), "{v}");
        }
    }
}
