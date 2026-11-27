//! Store prices in the app: fetch from IsThereAnyDeal and/or gg.deals (whichever has a key) on the
//! shared background job, and merge the results into the vault's price cache.

use jiff::{SignedDuration, Timestamp};
use keykeeper_core::prices::{gg_deals_region, normalize_country};
use keykeeper_core::{GamePrices, Money, PriceScope, Secret, ShopPrice};
use keykeeper_steam::{GgBatch, ItadBatch, Price, Progress, SteamError};
use uuid::Uuid;

use crate::app::AppState;
use crate::steam_actions::JobDone;

/// Prices older than this are refetched by a plain "Refresh prices".
pub const PRICE_MAX_AGE_HOURS: i64 = 12;

pub struct PricesDone {
    pub country: String,
    pub apps: Vec<u32>,
    pub itad: Option<Result<ItadBatch, SteamError>>,
    pub gg: Option<Result<GgBatch, SteamError>>,
}

fn money(p: &Price) -> Money {
    Money::new(p.cents, p.currency.clone())
}

impl AppState {
    /// Save the price country and API keys from Settings into the vault. Returns an error
    /// message for a bad country code (the keys are still saved).
    pub fn save_price_settings(&mut self) -> Option<String> {
        let s = self.session_mut()?;
        let ui = &s.steam_ui;
        let country = normalize_country(&ui.country.text);
        let itad = ui.itad_key.text.trim().to_string();
        let gg = ui.gg_key.text.trim().to_string();
        let mut api = s.repo.api_keys().clone();
        api.itad = Some(Secret::new(itad)).filter(|k| !k.is_empty());
        api.gg_deals = Some(Secret::new(gg)).filter(|k| !k.is_empty());
        s.repo.set_api_keys(api);
        match country {
            Some(c) => {
                let mut st = s.settings().clone();
                st.price_country = c.clone();
                s.repo.set_settings(st);
                s.steam_ui.country.replace_text(&c);
                None
            }
            None => Some("Use a two-letter country code, like US, GB or DE".into()),
        }
    }

    /// Refresh ratings and (with a price API key) prices for just the keys in `ids`, ignoring
    /// the caches: the library's right-click "Refresh prices & ratings".
    pub fn refresh_selected(&mut self, ids: &[Uuid]) {
        let _ = self.save_price_settings();
        let Some(s) = self.session() else { return };
        let mut apps: Vec<u32> = s
            .repo
            .keys()
            .iter()
            .filter(|k| ids.contains(&k.id))
            .filter_map(|k| k.steam_app_id)
            .collect();
        apps.sort_unstable();
        apps.dedup();
        if apps.is_empty() {
            self.toast("None of these keys has a Steam app id yet — use Match on Steam first");
            return;
        }
        let api_keys = s.repo.api_keys();
        let itad = api_keys.itad.as_ref().map(|k| k.expose().to_string());
        let gg = api_keys.gg_deals.as_ref().map(|k| k.expose().to_string());
        let country = s.settings().price_country.clone();
        let label = if apps.len() == 1 {
            "Refreshing game"
        } else {
            "Refreshing games"
        };
        self.spawn_job(label, apps.len(), move |api, n| {
            n.note("Steam ratings");
            let ratings = JobDone::Ratings {
                result: api.reviews(&apps, n),
            };
            if itad.is_none() && gg.is_none() {
                return ratings;
            }
            n.restart();
            let itad = itad.map(|k| api.itad_prices(&k, &country, &apps, n));
            if gg.is_some() {
                n.restart();
            }
            let gg = gg.map(|k| api.gg_deals_prices(&k, gg_deals_region(&country), &apps, n));
            JobDone::Many(vec![
                ratings,
                JobDone::Prices(PricesDone {
                    country,
                    apps,
                    itad,
                    gg,
                }),
            ])
        });
    }

    /// Fetch prices for keys with a Steam app id. `force` ignores the cache.
    pub fn refresh_prices(&mut self, force: bool) {
        self.refresh_prices_for(&[], force);
    }

    /// Fetch prices for the keys in `ids` (every key when empty) that are in the settings'
    /// [`PriceScope`]. `force` ignores the cache.
    pub fn refresh_prices_for(&mut self, ids: &[Uuid], force: bool) {
        if let Some(err) = self.save_price_settings() {
            if let Some(s) = self.session_mut() {
                s.steam_ui.prices_status = Some((false, err.clone()));
            }
            self.toast_error("Couldn't fetch prices", err);
            return;
        }
        let Some(s) = self.session() else { return };
        let api = s.repo.api_keys();
        let itad = api.itad.as_ref().map(|k| k.expose().to_string());
        let gg = api.gg_deals.as_ref().map(|k| k.expose().to_string());
        if itad.is_none() && gg.is_none() {
            let msg = "Add an IsThereAnyDeal or gg.deals API key in Settings → Prices";
            if let Some(s) = self.session_mut() {
                s.steam_ui.prices_status = Some((false, msg.into()));
            }
            self.toast_error("No price API key", msg);
            return;
        }
        let country = s.settings().price_country.clone();
        let scope = s.settings().price_scope;
        let stale = Timestamp::now() - SignedDuration::from_hours(PRICE_MAX_AGE_HOURS);
        let mut apps: Vec<u32> = s
            .repo
            .keys()
            .iter()
            .filter(|k| ids.is_empty() || ids.contains(&k.id))
            .filter(|k| scope.includes(k.status))
            .filter_map(|k| k.steam_app_id)
            .filter(|a| {
                force
                    || s.repo
                        .prices(*a)
                        .is_none_or(|p| p.fetched_at < stale || p.country != country)
            })
            .collect();
        apps.sort_unstable();
        apps.dedup();
        if apps.is_empty() {
            if force {
                self.toast(format!(
                    "No {} have a Steam app id yet — use Match on Steam first",
                    match scope {
                        PriceScope::All => "keys",
                        PriceScope::InHand => "keys you still have",
                        PriceScope::Unredeemed => "unredeemed keys",
                    }
                ));
            } else {
                self.toast("Prices are up to date");
            }
            return;
        }
        let total = apps.len();
        if let Some(s) = self.session_mut() {
            s.steam_ui.prices_status = None;
        }
        self.spawn_job("Fetching prices", total, move |api, n| {
            let itad = itad.map(|k| api.itad_prices(&k, &country, &apps, n));
            if gg.is_some() {
                // Count again for gg.deals: the status bar shows one service at a time.
                n.restart();
            }
            let gg = gg.map(|k| api.gg_deals_prices(&k, gg_deals_region(&country), &apps, n));
            JobDone::Prices(PricesDone {
                country,
                apps,
                itad,
                gg,
            })
        });
    }

    pub(crate) fn apply_prices(&mut self, done: PricesDone) {
        let Some(s) = self.session_mut() else { return };
        let now = Timestamp::now();
        let (itad, itad_err) = match done.itad {
            Some(Ok(b)) => {
                let note = b.error.as_ref().map(|e| {
                    format!(
                        "IsThereAnyDeal stopped after {} of {} games ({e}); refresh later for the rest",
                        b.seen.len(),
                        done.apps.len()
                    )
                });
                (Some(b), note)
            }
            Some(Err(e)) => (None, Some(format!("IsThereAnyDeal: {e}"))),
            None => (None, None),
        };
        let (gg, gg_err) = match done.gg {
            Some(Ok(b)) => {
                let note = b.error.as_ref().map(|e| {
                    format!(
                        "gg.deals stopped after {} of {} games ({e}); refresh later for the rest",
                        b.seen.len(),
                        done.apps.len()
                    )
                });
                (Some(b), note)
            }
            Some(Err(e)) => (None, Some(format!("gg.deals: {e}"))),
            None => (None, None),
        };
        let mut priced = 0;
        let mut updates = Vec::new();
        if itad.is_some() || gg.is_some() {
            for app in &done.apps {
                // Keep what a failed service gave last time.
                let old = s.repo.prices(*app).filter(|p| p.country == done.country);
                let mut p = GamePrices::empty(done.country.clone(), now);
                match &itad {
                    Some(b) if b.seen.contains(app) => {
                        if let Some(i) = b.prices.iter().find(|i| i.app_id == *app) {
                            p.shops = i
                                .deals
                                .iter()
                                .map(|d| ShopPrice {
                                    shop: d.shop.clone(),
                                    price: money(&d.price),
                                    regular: d.regular.as_ref().map(money),
                                    cut: d.cut,
                                    url: d.url.clone(),
                                })
                                .collect();
                            p.sort_shops();
                            p.history_low = i.history_low.as_ref().map(money);
                        }
                    }
                    _ => {
                        // Not reached (or ITAD failed): keep the old prices, and when ITAD ran
                        // but stopped early, leave the game due for a refetch.
                        if itad.is_some() {
                            p.fetched_at = old.map_or(Timestamp::UNIX_EPOCH, |o| o.fetched_at);
                        }
                        if let Some(o) = old {
                            p.shops = o.shops.clone();
                            p.history_low = o.history_low.clone();
                        }
                    }
                }
                match &gg {
                    Some(b) if b.seen.contains(app) => {
                        if let Some(g) = b.prices.iter().find(|g| g.app_id == *app) {
                            p.retail_low = g.retail.as_ref().map(money);
                            p.keyshop_low = g.keyshops.as_ref().map(money);
                            p.gg_deals_url = g.url.clone();
                        }
                    }
                    _ => {
                        // Not reached this time (or gg.deals failed): keep the old prices, and
                        // when gg.deals ran but stopped early, leave the game due for a refetch.
                        if gg.is_some() {
                            let was = old.map_or(Timestamp::UNIX_EPOCH, |o| o.fetched_at);
                            p.fetched_at = p.fetched_at.min(was);
                        }
                        if let Some(o) = old {
                            p.retail_low = o.retail_low.clone();
                            p.keyshop_low = o.keyshop_low.clone();
                            p.gg_deals_url = o.gg_deals_url.clone();
                        }
                    }
                }
                if p.best().is_some() || p.keyshop_low.is_some() {
                    priced += 1;
                }
                updates.push((*app, p));
            }
            s.repo.set_prices(updates);
        }
        let mut msg = format!(
            "Prices for {priced} of {} game{} ({})",
            done.apps.len(),
            if done.apps.len() == 1 { "" } else { "s" },
            done.country
        );
        let errors: Vec<String> = [itad_err, gg_err].into_iter().flatten().collect();
        log::info!(
            "{msg}{}",
            if errors.is_empty() {
                String::new()
            } else {
                format!(" ({})", errors.join("; "))
            }
        );
        s.steam_ui.prices_status = Some((errors.is_empty(), {
            let mut m = msg.clone();
            for e in &errors {
                m.push_str(". ");
                m.push_str(e);
            }
            m
        }));
        if errors.is_empty() {
            self.toast(msg);
        } else {
            if itad.is_none() && gg.is_none() {
                msg = "Couldn't fetch prices".into();
            }
            self.toast_error(msg, errors.join("\n"));
        }
    }
}
