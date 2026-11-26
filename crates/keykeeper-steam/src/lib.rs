//! Steam integration for KeyKeeper (PLAN §8).
//!
//! Endpoints (verified 2026-11-25):
//! - App search: `store.steampowered.com/api/storesearch` (no key).
//! - Review summaries: `api.steampowered.com/IStoreBrowseService/GetItems/v1` with
//!   `include_reviews` (no key, batched). The older `/appreviews/` endpoint now redirects.
//! - Profile → SteamID64: `steamcommunity.com/id/<vanity>?xml=1` (no key).
//! - Wishlist: `IWishlistService/GetWishlist/v1` (no key; public wishlists only).
//! - Owned games: `IPlayerService/GetOwnedGames/v1` (**needs a Web API key**, public profile).
//!
//! All network access goes through the [`Http`] trait so tests use recorded fixtures.

pub mod api;
pub mod http;
pub mod matching;

pub use api::{AppMatch, ProfileRef, Review, Steam, SteamApi, parse_profile_input};
pub use http::{Http, UreqHttp};
pub use matching::{MatchOutcome, best_match, normalize_title};

#[derive(Clone, Debug, PartialEq, Eq, thiserror::Error)]
pub enum SteamError {
    #[error("network error: {0}")]
    Network(String),
    #[error("Steam returned HTTP {0}")]
    Status(u16),
    #[error("Steam is rate-limiting requests; try again in a few minutes")]
    RateLimited,
    #[error("unexpected response from Steam: {0}")]
    Parse(String),
    #[error("couldn't find a Steam profile for '{0}'")]
    ProfileNotFound(String),
    #[error("the Steam Web API key was rejected")]
    BadApiKey,
}

pub type Result<T> = std::result::Result<T, SteamError>;
