//! The trade lifecycle and template rendering for checkout and reputation text.
//!
//! ```text
//! Library ─add_to_cart─▶ Draft (the cart) ─send─▶ Pending ─complete─▶ Completed
//!    keys → Reserved        │                       │           keys → Traded
//!                           └─clear / cancel────────┴─▶ keys restored (Cancelled)
//! ```
//! Every operation is one undo step.

use jiff::Timestamp;
use jiff::civil::Date;
use uuid::Uuid;

use crate::import::split_line;
use crate::model::{DateFormat, GameKey, KeyStatus, Platform, ReceivedItem, Trade, TradeStatus};
use crate::repo::Repo;
use crate::secret::Secret;
use crate::template::{self, TemplateError};

/// A date with no time of day (imported or picked) is stored as midnight UTC.
pub fn day_timestamp(date: Date) -> Timestamp {
    date.to_zoned_datetime(jiff::tz::TimeZone::UTC)
        .map(|z| z.timestamp())
        .unwrap_or(Timestamp::UNIX_EPOCH)
}

/// The day `t` happened: completed, else sent, else made, in `tz`. A timestamp at exactly
/// midnight UTC is a date without a time (see [`day_timestamp`]) and reads as that day
/// everywhere, instead of the day before west of Greenwich.
pub fn trade_day(t: &Trade, tz: &jiff::tz::TimeZone) -> Date {
    let ts = t.completed_at.or(t.sent_at).unwrap_or(t.created_at);
    let utc = ts.to_zoned_datetime(jiff::tz::TimeZone::UTC);
    if utc.time() == jiff::civil::Time::midnight() {
        utc.date()
    } else {
        ts.to_zoned_datetime(tz.clone()).date()
    }
}

#[derive(Clone, Debug, PartialEq, Eq, thiserror::Error)]
pub enum TradeError {
    #[error("trade not found")]
    NotFound,
    #[error("this trade is {0:?}")]
    WrongStatus(TradeStatus),
    #[error("the trade has no keys")]
    Empty,
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct CartAdd {
    pub added: usize,
    /// Keys that weren't added: already traded/redeemed/reserved elsewhere, or missing.
    pub skipped: usize,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Completed {
    pub traded: usize,
    pub received_added: usize,
}

impl DateFormat {
    pub fn format(self, d: Date) -> String {
        match self {
            DateFormat::MonthDayYear => format!("{}/{}/{}", d.month(), d.day(), d.year()),
            DateFormat::DayMonthYear => format!("{}/{}/{}", d.day(), d.month(), d.year()),
        }
    }
}

/// Parse a "received" text box: one game per line, optionally with a key (`Game: KEY`).
pub fn parse_received(text: &str) -> Vec<ReceivedItem> {
    text.lines()
        .map(str::trim)
        .filter(|l| !l.is_empty())
        .map(|l| {
            let (name, key) = split_line(l);
            ReceivedItem {
                game_name: if name.is_empty() { l.to_string() } else { name },
                key: Some(key).filter(|k| !k.is_empty()).map(Secret::new),
                platform: Platform::Steam,
            }
        })
        .collect()
}

/// The inverse of [`parse_received`], for editing.
pub fn received_text(items: &[ReceivedItem]) -> String {
    items
        .iter()
        .map(|r| match &r.key {
            Some(k) => format!("{}: {}", r.game_name, k.expose()),
            None => r.game_name.clone(),
        })
        .collect::<Vec<_>>()
        .join("\n")
}

impl Repo {
    /// The cart: the (single) draft trade, if any.
    pub fn cart(&self) -> Option<&Trade> {
        self.trades()
            .iter()
            .find(|t| t.status == TradeStatus::Draft)
    }

    pub fn cart_len(&self) -> usize {
        self.cart().map_or(0, |t| {
            t.key_ids
                .iter()
                .filter(|id| self.key(**id).is_some())
                .count()
        })
    }

    /// Reserve tradeable keys into the cart (creating it if needed).
    pub fn add_to_cart(&mut self, ids: &[Uuid]) -> CartAdd {
        let mut cart = self.cart().cloned().unwrap_or_default();
        let mut keys = Vec::new();
        let mut res = CartAdd::default();
        for id in ids {
            match self.key(*id) {
                Some(k) if k.status.can_trade() && !cart.key_ids.contains(id) => {
                    cart.key_ids.push(*id);
                    cart.prior_status.push((*id, k.status));
                    let mut k = k.clone();
                    k.status = KeyStatus::Reserved;
                    k.trade_id = Some(cart.id);
                    keys.push(k);
                    res.added += 1;
                }
                _ => res.skipped += 1,
            }
        }
        if res.added > 0 {
            let label = if res.added == 1 {
                "Add to cart".to_string()
            } else {
                format!("Add {} to cart", res.added)
            };
            self.apply_changes(&label, keys, vec![cart], &[]);
        }
        res
    }

    /// Keys released from `trade` back to their earlier status (only if still reserved by it).
    fn released(&self, trade: &Trade, ids: &[Uuid]) -> Vec<GameKey> {
        ids.iter()
            .filter_map(|id| self.key(*id))
            .filter(|k| k.trade_id == Some(trade.id) && k.status == KeyStatus::Reserved)
            .map(|k| {
                let mut k = k.clone();
                k.status = trade
                    .prior_status
                    .iter()
                    .find(|(id, _)| *id == k.id)
                    .map_or(KeyStatus::Available, |(_, s)| *s);
                k.trade_id = None;
                k
            })
            .collect()
    }

    pub fn remove_from_cart(&mut self, ids: &[Uuid]) -> usize {
        let Some(mut cart) = self.cart().cloned() else {
            return 0;
        };
        let keys = self.released(&cart, ids);
        cart.key_ids.retain(|id| !ids.contains(id));
        cart.prior_status.retain(|(id, _)| !ids.contains(id));
        let n = keys.len();
        self.apply_changes("Remove from cart", keys, vec![cart], &[]);
        n
    }

    /// Empty the cart: release every key and drop the draft.
    pub fn clear_cart(&mut self) -> usize {
        let Some(cart) = self.cart().cloned() else {
            return 0;
        };
        let keys = self.released(&cart, &cart.key_ids);
        let n = keys.len();
        self.apply_changes("Clear cart", keys, Vec::new(), &[cart.id]);
        n
    }

    /// Mark keys as redeemed on my own account, dated `today`. Keys in the cart leave it (an
    /// emptied cart is dropped). Keys that are gone or promised in a sent trade are skipped.
    /// One undo step. Returns the ids that were redeemed.
    pub fn redeem_keys(&mut self, ids: &[Uuid], today: Date) -> Vec<Uuid> {
        let mut cart = self.cart().cloned();
        let cart_id = cart.as_ref().map(|c| c.id);
        let mut keys = Vec::new();
        for id in ids {
            let Some(k) = self.key(*id) else { continue };
            let in_cart =
                k.status == KeyStatus::Reserved && k.trade_id.is_some() && k.trade_id == cart_id;
            if !(k.status.can_trade() || in_cart) || keys.iter().any(|x: &GameKey| x.id == *id) {
                continue;
            }
            let mut k = k.clone();
            k.status = KeyStatus::Redeemed;
            k.status_date = Some(today);
            if in_cart {
                k.trade_id = None;
            }
            keys.push(k);
        }
        let redeemed: Vec<Uuid> = keys.iter().map(|k| k.id).collect();
        if redeemed.is_empty() {
            return redeemed;
        }
        let mut trades = Vec::new();
        let mut delete = Vec::new();
        if let Some(c) = cart.as_mut() {
            let before = c.key_ids.len();
            c.key_ids.retain(|id| !redeemed.contains(id));
            c.prior_status.retain(|(id, _)| !redeemed.contains(id));
            if c.key_ids.len() != before {
                if c.key_ids.iter().any(|id| self.key(*id).is_some()) {
                    trades.push(c.clone());
                } else {
                    delete.push(c.id);
                }
            }
        }
        let label = if redeemed.len() == 1 {
            "Redeem key".to_string()
        } else {
            format!("Redeem {} keys", redeemed.len())
        };
        self.apply_changes(&label, keys, trades, &delete);
        redeemed
    }

    /// Save edited details (partner, links, received, notes) of a trade.
    pub fn update_trade_details(&mut self, trade: Trade) -> Result<(), TradeError> {
        let current = self.trade(trade.id).ok_or(TradeError::NotFound)?;
        // Details only: status and membership are changed by the lifecycle methods.
        let merged = Trade {
            status: current.status,
            key_ids: current.key_ids.clone(),
            prior_status: current.prior_status.clone(),
            completed_at: current.completed_at,
            sent_at: current.sent_at,
            ..trade
        };
        self.apply_changes("Edit trade details", Vec::new(), vec![merged], &[]);
        Ok(())
    }

    /// Change the day a trade happened (as [`trade_day`] reads it): the completion date of a
    /// completed trade, otherwise the day it was sent (or made). Keys it traded away take the
    /// same date. One undo step.
    pub fn set_trade_date(&mut self, id: Uuid, date: Date) -> Result<(), TradeError> {
        let mut t = self.trade(id).cloned().ok_or(TradeError::NotFound)?;
        let at = day_timestamp(date);
        if t.completed_at.is_some() || t.status == TradeStatus::Completed {
            t.completed_at = Some(at);
        } else if t.sent_at.is_some() {
            t.sent_at = Some(at);
        } else {
            t.created_at = at;
        }
        let keys: Vec<GameKey> = t
            .key_ids
            .iter()
            .filter_map(|k| self.key(*k))
            .filter(|k| k.trade_id == Some(id) && k.status == KeyStatus::Traded)
            .map(|k| {
                let mut k = k.clone();
                k.status_date = Some(date);
                k
            })
            .collect();
        self.apply_changes("Set trade date", keys, vec![t], &[]);
        Ok(())
    }

    /// Draft → Pending.
    pub fn send_trade(&mut self, id: Uuid) -> Result<(), TradeError> {
        let mut t = self.trade(id).cloned().ok_or(TradeError::NotFound)?;
        if t.status != TradeStatus::Draft {
            return Err(TradeError::WrongStatus(t.status));
        }
        if !t.key_ids.iter().any(|k| self.key(*k).is_some()) {
            return Err(TradeError::Empty);
        }
        t.status = TradeStatus::Pending;
        t.sent_at = Some(Timestamp::now());
        self.apply_changes("Mark trade as sent", Vec::new(), vec![t], &[]);
        Ok(())
    }

    /// Draft or Pending → Completed: given keys become Traded (dated `today`), and received
    /// games are optionally added to the library (skipping keys already in it).
    pub fn complete_trade(
        &mut self,
        id: Uuid,
        today: Date,
        add_received: bool,
    ) -> Result<Completed, TradeError> {
        let mut t = self.trade(id).cloned().ok_or(TradeError::NotFound)?;
        if !matches!(t.status, TradeStatus::Draft | TradeStatus::Pending) {
            return Err(TradeError::WrongStatus(t.status));
        }
        let mut keys: Vec<GameKey> = t
            .key_ids
            .iter()
            .filter_map(|k| self.key(*k))
            .map(|k| {
                let mut k = k.clone();
                k.status = KeyStatus::Traded;
                k.status_date = Some(today);
                k.trade_id = Some(id);
                k
            })
            .collect();
        if keys.is_empty() {
            return Err(TradeError::Empty);
        }
        let traded = keys.len();
        let mut received_added = 0;
        if add_received {
            let source = if t.partner_name.trim().is_empty() {
                "Trade".to_string()
            } else {
                format!("Trade: {}", t.partner_name.trim())
            };
            for r in &t.received {
                if r.key
                    .as_ref()
                    .is_some_and(|k| self.find_by_key(k.expose()).is_some())
                {
                    continue;
                }
                let mut k = GameKey::new(r.game_name.clone(), r.key.clone().unwrap_or_default());
                k.platform = r.platform.clone();
                k.source = Some(source.clone());
                k.trade_id = Some(id);
                k.status_date = Some(today);
                if r.key.is_none() {
                    k.notes = "Key not received yet".into();
                }
                keys.push(k);
                received_added += 1;
            }
        }
        t.status = TradeStatus::Completed;
        t.completed_at = Some(Timestamp::now());
        t.add_received_to_library = add_received;
        self.apply_changes("Complete trade", keys, vec![t], &[]);
        Ok(Completed {
            traded,
            received_added,
        })
    }

    /// Pending → Cancelled (keys restored). A draft is simply cleared.
    pub fn cancel_trade(&mut self, id: Uuid) -> Result<usize, TradeError> {
        let mut t = self.trade(id).cloned().ok_or(TradeError::NotFound)?;
        match t.status {
            TradeStatus::Draft => Ok(self.clear_cart()),
            TradeStatus::Pending => {
                let keys = self.released(&t, &t.key_ids);
                let n = keys.len();
                t.status = TradeStatus::Cancelled;
                self.apply_changes("Cancel trade", keys, vec![t], &[]);
                Ok(n)
            }
            s => Err(TradeError::WrongStatus(s)),
        }
    }

    pub fn set_rep_posted(&mut self, id: Uuid, posted: bool) -> Result<(), TradeError> {
        let mut t = self.trade(id).cloned().ok_or(TradeError::NotFound)?;
        t.rep_posted = posted;
        self.apply_changes(
            if posted {
                "Mark rep posted"
            } else {
                "Mark rep not posted"
            },
            Vec::new(),
            vec![t],
            &[],
        );
        Ok(())
    }

    /// Keys given in `trade` that still exist, in trade order.
    pub fn trade_keys(&self, trade: &Trade) -> Vec<&GameKey> {
        trade
            .key_ids
            .iter()
            .filter_map(|id| self.key(*id))
            .collect()
    }

    fn trade_var(&self, trade: &Trade, fmt: DateFormat, name: &str) -> Option<String> {
        Some(match name {
            "partner" => trade.partner_name.clone(),
            "partner_url" => trade.partner_profile_url.clone().unwrap_or_default(),
            "trade_url" => trade.trade_url.clone().unwrap_or_default(),
            "site" => trade.site.label().to_string(),
            "date" => fmt.format(trade_day(trade, &jiff::tz::TimeZone::system())),
            "count" => self.trade_keys(trade).len().to_string(),
            "games" => self
                .trade_keys(trade)
                .iter()
                .map(|k| k.game_name.as_str())
                .collect::<Vec<_>>()
                .join(", "),
            "received" => trade
                .received
                .iter()
                .map(|r| r.game_name.as_str())
                .collect::<Vec<_>>()
                .join(", "),
            _ => return None,
        })
    }

    /// The checkout text: the checkout-line template for each key, one per line.
    /// With `mask`, keys are shown masked (for display); copy the unmasked text.
    pub fn checkout_text(&self, trade: &Trade, mask: bool) -> Result<String, TemplateError> {
        let tpl = &self.data().templates.checkout_line;
        let fmt = self.settings().date_format;
        let mut lines = Vec::new();
        for k in self.trade_keys(trade) {
            lines.push(template::render(tpl, |v| match v {
                "name" => Some(k.game_name.clone()),
                "key" => Some(if mask {
                    k.key.masked()
                } else {
                    k.key.expose().to_string()
                }),
                "bundle" => Some(k.bundle.clone().unwrap_or_default()),
                "platform" => Some(k.platform.label().to_string()),
                "steam_url" => Some(k.steam_url().unwrap_or_default()),
                "notes" => Some(k.notes.clone()),
                other => self.trade_var(trade, fmt, other),
            })?);
        }
        Ok(lines.join("\n"))
    }

    pub fn rep_text(&self, trade: &Trade) -> Result<String, TemplateError> {
        let fmt = self.settings().date_format;
        template::render(&self.data().templates.rep, |v| {
            self.trade_var(trade, fmt, v)
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn trade_dates_are_days_in_any_time_zone() {
        let mut repo = Repo::new(crate::VaultData::default());
        let mut t = Trade::new();
        t.status = TradeStatus::Completed;
        let mut k = GameKey::new("Tunic", "TUN1C-F0X99-AAAA1");
        k.status = KeyStatus::Traded;
        k.trade_id = Some(t.id);
        t.key_ids.push(k.id);
        repo.apply_changes("seed", vec![k.clone()], vec![t.clone()], &[]);
        let day = jiff::civil::date(2025, 3, 9);
        repo.set_trade_date(t.id, day).unwrap();
        let t = repo.trade(t.id).unwrap();
        let ny = jiff::tz::TimeZone::get("America/New_York").unwrap();
        assert_eq!(trade_day(t, &ny), day, "not the 8th in New York");
        assert_eq!(
            trade_day(t, &jiff::tz::TimeZone::get("Asia/Tokyo").unwrap()),
            day
        );
        assert_eq!(repo.key(k.id).unwrap().status_date, Some(day));
        // A real moment reads in the given zone.
        let mut live = Trade::new();
        live.created_at = "2025-03-09T03:00:00Z".parse().unwrap();
        assert_eq!(trade_day(&live, &ny), jiff::civil::date(2025, 3, 8));
    }
    use crate::data::VaultData;
    use crate::model::TradeSite;
    use jiff::civil::date;

    fn repo() -> (Repo, Vec<Uuid>) {
        let mut r = Repo::new(VaultData::default());
        let mut keys = vec![
            GameKey::new("Undertale", "UNDR7-TALE0-00001"),
            GameKey::new("Celeste", "QQ7RT-PP2MN-ZZ9XY"),
            GameKey::new("Hades", "HD5S2-77KLM-NB3VC"),
            GameKey::new("Gone", "GGGGG-11111-GGGGG"),
        ];
        keys[1].status = KeyStatus::Unknown;
        keys[3].status = KeyStatus::Traded;
        let ids = r.add_keys(keys, "seed");
        (r, ids)
    }

    fn status(r: &Repo, id: Uuid) -> KeyStatus {
        r.key(id).unwrap().status
    }

    #[test]
    fn add_to_cart_reserves_tradeable_keys_only() {
        let (mut r, ids) = repo();
        let res = r.add_to_cart(&[ids[0], ids[1], ids[3], ids[0]]);
        assert_eq!(
            res,
            CartAdd {
                added: 2,
                skipped: 2
            },
            "traded key and the repeat are skipped"
        );
        assert_eq!(r.cart_len(), 2);
        assert_eq!(status(&r, ids[0]), KeyStatus::Reserved);
        assert_eq!(r.key(ids[0]).unwrap().trade_id, r.cart().map(|t| t.id));
        assert_eq!(r.trades().len(), 1);
        assert_eq!(
            r.add_to_cart(&[ids[0]]),
            CartAdd {
                added: 0,
                skipped: 1
            },
            "already reserved"
        );
        r.undo();
        assert_eq!(status(&r, ids[0]), KeyStatus::Available);
        assert!(r.cart().is_none());
    }

    #[test]
    fn remove_and_clear_restore_prior_status() {
        let (mut r, ids) = repo();
        r.add_to_cart(&ids[..3]);
        assert_eq!(r.remove_from_cart(&[ids[1]]), 1);
        assert_eq!(
            status(&r, ids[1]),
            KeyStatus::Unknown,
            "back to what it was"
        );
        assert_eq!(r.cart_len(), 2);
        assert_eq!(r.clear_cart(), 2);
        assert!(r.cart().is_none());
        assert_eq!(status(&r, ids[0]), KeyStatus::Available);
        assert!(r.key(ids[0]).unwrap().trade_id.is_none());
    }

    #[test]
    fn redeem_keys_dates_them_and_leaves_the_cart() {
        let (mut r, ids) = repo();
        let today = date(2026, 11, 26);
        r.add_to_cart(&ids[..2]);
        let done = r.redeem_keys(&[ids[0], ids[2], ids[3]], today);
        assert_eq!(done, vec![ids[0], ids[2]], "the traded key is skipped");
        let k = r.key(ids[0]).unwrap();
        assert_eq!(
            (k.status, k.status_date, k.trade_id),
            (KeyStatus::Redeemed, Some(today), None)
        );
        assert_eq!(r.cart_len(), 1, "the other cart key stays");
        assert_eq!(r.undo_label(), Some("Redeem 2 keys"));

        // Redeeming the rest of the cart drops it.
        assert_eq!(r.redeem_keys(&[ids[1]], today), vec![ids[1]]);
        assert!(r.cart().is_none());
        r.undo();
        assert_eq!(status(&r, ids[1]), KeyStatus::Reserved);
        assert_eq!(r.cart_len(), 1);

        // A key promised in a sent trade can't be self-redeemed.
        let cart = r.cart().unwrap().id;
        r.send_trade(cart).unwrap();
        assert!(r.redeem_keys(&[ids[1]], today).is_empty());
    }

    #[test]
    fn full_flow_send_complete_with_received() {
        let (mut r, ids) = repo();
        r.add_to_cart(&ids[..2]);
        let mut t = r.cart().cloned().unwrap();
        t.partner_name = "lumen".into();
        t.trade_url = Some("https://lestrades.com/offer/100001/".into());
        t.received =
            parse_received("Tunic: TUN1C-F0X99-AAAA1\nA Short Hike\nHades: HD5S2-77KLM-NB3VC");
        r.update_trade_details(t.clone()).unwrap();
        assert_eq!(
            r.trade(t.id).unwrap().status,
            TradeStatus::Draft,
            "details can't change status"
        );

        r.send_trade(t.id).unwrap();
        assert_eq!(r.trade(t.id).unwrap().status, TradeStatus::Pending);
        assert!(r.cart().is_none(), "a sent trade is no longer the cart");
        assert_eq!(
            r.send_trade(t.id),
            Err(TradeError::WrongStatus(TradeStatus::Pending))
        );

        let done = r.complete_trade(t.id, date(2026, 1, 7), true).unwrap();
        assert_eq!(
            done,
            Completed {
                traded: 2,
                received_added: 2
            },
            "Hades' key is already in the vault"
        );
        assert_eq!(status(&r, ids[0]), KeyStatus::Traded);
        assert_eq!(r.key(ids[0]).unwrap().status_date, Some(date(2026, 1, 7)));
        let tunic = r.find_by_key("TUN1C-F0X99-AAAA1").unwrap();
        assert_eq!(tunic.source.as_deref(), Some("Trade: lumen"));
        assert_eq!(tunic.status, KeyStatus::Available);
        let hike = r
            .keys()
            .iter()
            .find(|k| k.game_name == "A Short Hike")
            .unwrap();
        assert_eq!(hike.notes, "Key not received yet");
        assert_eq!(r.trade(t.id).unwrap().status, TradeStatus::Completed);

        // One undo reverts the whole completion.
        r.undo();
        assert_eq!(r.trade(t.id).unwrap().status, TradeStatus::Pending);
        assert_eq!(status(&r, ids[0]), KeyStatus::Reserved);
        assert!(r.find_by_key("TUN1C-F0X99-AAAA1").is_none());
    }

    #[test]
    fn complete_without_received_and_cancel_paths() {
        let (mut r, ids) = repo();
        r.add_to_cart(&[ids[0]]);
        let id = r.cart().unwrap().id;
        let done = r.complete_trade(id, date(2026, 1, 1), false).unwrap();
        assert_eq!(done.received_added, 0);
        assert_eq!(
            r.cancel_trade(id),
            Err(TradeError::WrongStatus(TradeStatus::Completed))
        );

        r.add_to_cart(&[ids[1], ids[2]]);
        let id2 = r.cart().unwrap().id;
        assert_ne!(id, id2, "a new cart after completing");
        r.send_trade(id2).unwrap();
        assert_eq!(r.cancel_trade(id2), Ok(2));
        assert_eq!(r.trade(id2).unwrap().status, TradeStatus::Cancelled);
        assert_eq!(status(&r, ids[1]), KeyStatus::Unknown);
        assert_eq!(status(&r, ids[2]), KeyStatus::Available);
    }

    #[test]
    fn empty_trades_cannot_be_sent_or_completed() {
        let (mut r, ids) = repo();
        r.add_to_cart(&[ids[0]]);
        let id = r.cart().unwrap().id;
        r.delete_keys(&[ids[0]], "delete");
        assert_eq!(r.send_trade(id), Err(TradeError::Empty));
        assert_eq!(
            r.complete_trade(id, date(2026, 1, 1), true),
            Err(TradeError::Empty)
        );
        assert_eq!(r.send_trade(Uuid::nil()), Err(TradeError::NotFound));
    }

    #[test]
    fn edited_key_is_not_clobbered_on_release() {
        let (mut r, ids) = repo();
        r.add_to_cart(&[ids[0]]);
        r.update_key(ids[0], |k| k.status = KeyStatus::Gifted)
            .unwrap();
        r.clear_cart();
        assert_eq!(status(&r, ids[0]), KeyStatus::Gifted);
    }

    #[test]
    fn checkout_and_rep_text() {
        let (mut r, ids) = repo();
        r.add_to_cart(&ids[..2]);
        let mut t = r.cart().cloned().unwrap();
        t.trade_url = Some("https://lestrades.com/offer/100002/".into());
        t.partner_name = "lumen".into();
        t.site = TradeSite::Lestrades;
        r.update_trade_details(t.clone()).unwrap();
        let t = r.trade(t.id).unwrap().clone();
        assert_eq!(
            r.checkout_text(&t, false).unwrap(),
            "Undertale: UNDR7-TALE0-00001\nCeleste: QQ7RT-PP2MN-ZZ9XY"
        );
        assert_eq!(
            r.checkout_text(&t, true).unwrap(),
            "Undertale: UNDR7-•••••-•••••\nCeleste: QQ7RT-•••••-•••••"
        );
        assert!(
            r.rep_text(&t)
                .unwrap()
                .contains("this trade: https://lestrades.com/offer/100002/\nThanks")
        );

        let mut tpl = r.data().templates.clone();
        tpl.checkout_line = "{count}. {name} ({platform}) - {key}".into();
        tpl.rep = "+Rep {partner} on {site}: {games}".into();
        r.set_templates(tpl);
        assert_eq!(
            r.checkout_text(&t, false).unwrap().lines().next().unwrap(),
            "2. Undertale (Steam) - UNDR7-TALE0-00001"
        );
        assert_eq!(
            r.rep_text(&t).unwrap(),
            "+Rep lumen on Lestrades: Undertale, Celeste"
        );
    }

    #[test]
    fn received_text_round_trips() {
        let items = parse_received("  Tunic: TUN1C-F0X99-AAAA1 \n\nA Short Hike\n");
        assert_eq!(items.len(), 2);
        assert_eq!(items[0].game_name, "Tunic");
        assert_eq!(items[1].key, None);
        assert_eq!(
            received_text(&items),
            "Tunic: TUN1C-F0X99-AAAA1\nA Short Hike"
        );
    }

    #[test]
    fn date_format_helper() {
        assert_eq!(
            DateFormat::MonthDayYear.format(date(2026, 1, 7)),
            "1/7/2026"
        );
        assert_eq!(
            DateFormat::DayMonthYear.format(date(2026, 1, 7)),
            "7/1/2026"
        );
    }
}
