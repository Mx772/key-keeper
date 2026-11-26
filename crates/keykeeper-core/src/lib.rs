//! KeyKeeper core: everything that is not GUI or network lives here so it can be tested headlessly.
//! Crates: `keykeeper-core` (this), `keykeeper-steam` (network), `keykeeper-app` (GUI).

pub mod appinfo;
pub mod data;
pub mod error;
pub mod export;
pub mod import;
pub mod links;
pub mod model;
pub mod prices;
pub mod query;
pub mod repo;
pub mod secret;
pub mod template;
pub mod trade;
pub mod upgrade;
pub mod vault;

pub use appinfo::{AppInfo, BlobCache, BlobCipher, DeckCompat};
pub use data::VaultData;
pub use error::{RepoError, VaultError};
pub use model::{
    ApiKeys, DateFormat, GameKey, KeyStatus, LinkOverrides, Ownership, Platform, ReceivedItem,
    Settings, SteamMeta, SteamRating, Trade, TradeSite, TradeStatus,
};
pub use prices::{GamePrices, Money, ShopPrice};
pub use query::{
    LabelField, Query, Sort, SortDir, SortField, canonical_label, game_identity, label_key,
    label_suggestions, label_values,
};
pub use repo::Repo;
pub use secret::Secret;
pub use template::Templates;
pub use trade::{CartAdd, Completed, TradeError};
pub use vault::{KdfParams, Vault, VaultKeys};
