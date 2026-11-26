//! [`VaultData`]: everything that lives inside the encrypted vault, plus schema migrations.

use serde::{Deserialize, Serialize};
use zeroize::Zeroizing;

use crate::error::VaultError;
use std::collections::BTreeMap;

use crate::model::{ApiKeys, GameKey, Settings, Trade};
use crate::prices::GamePrices;
use crate::template::Templates;

/// Bump when the payload shape changes incompatibly, and add a step to [`migrate`].
pub const SCHEMA_VERSION: u32 = 1;

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct VaultData {
    pub schema_version: u32,
    #[serde(default)]
    pub keys: Vec<GameKey>,
    #[serde(default)]
    pub trades: Vec<Trade>,
    #[serde(default)]
    pub settings: Settings,
    #[serde(default)]
    pub api_keys: ApiKeys,
    #[serde(default)]
    pub templates: Templates,
    /// The newest KeyKeeper version that has opened this vault (e.g. `"0.2.0"`). `None` for
    /// vaults last opened before this was recorded. Drives the pre-upgrade backup
    /// ([`crate::upgrade`]).
    #[serde(default)]
    pub last_app_version: Option<String>,
    /// Store prices by Steam app id (a cache; refetched on demand).
    #[serde(default)]
    pub prices: BTreeMap<u32, GamePrices>,
}

impl Default for VaultData {
    fn default() -> Self {
        VaultData {
            schema_version: SCHEMA_VERSION,
            keys: Vec::new(),
            trades: Vec::new(),
            settings: Settings::default(),
            api_keys: ApiKeys::default(),
            templates: Templates::default(),
            last_app_version: None,
            prices: BTreeMap::new(),
        }
    }
}

#[derive(Deserialize)]
struct Peek {
    schema_version: u32,
}

impl VaultData {
    pub(crate) fn to_json(&self) -> Result<Zeroizing<Vec<u8>>, VaultError> {
        serde_json::to_vec(self)
            .map(Zeroizing::new)
            .map_err(|e| VaultError::Data(e.to_string()))
    }

    pub(crate) fn from_json(bytes: &[u8]) -> Result<VaultData, VaultError> {
        let peek: Peek =
            serde_json::from_slice(bytes).map_err(|e| VaultError::Data(e.to_string()))?;
        match peek.schema_version {
            SCHEMA_VERSION => {
                serde_json::from_slice(bytes).map_err(|e| VaultError::Data(e.to_string()))
            }
            v if v > SCHEMA_VERSION => Err(VaultError::UnsupportedSchema(v)),
            v => {
                let value: serde_json::Value =
                    serde_json::from_slice(bytes).map_err(|e| VaultError::Data(e.to_string()))?;
                let value = migrate(v, value)?;
                serde_json::from_value(value).map_err(|e| VaultError::Data(e.to_string()))
            }
        }
    }
}

/// Upgrades an older payload one version at a time by recursion. No migrations exist yet
/// (schema 1 is the first); add arms like `1 => migrate(2, v1_to_v2(value))`.
fn migrate(from: u32, value: serde_json::Value) -> Result<serde_json::Value, VaultError> {
    match from {
        SCHEMA_VERSION => Ok(value),
        v => Err(VaultError::Data(format!("no migration from schema {v}"))),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::KeyStatus;

    #[test]
    fn round_trip() {
        let mut d = VaultData::default();
        let mut k = GameKey::new("Undertale", "UNDR7-TALE0-00001");
        k.status = KeyStatus::Traded;
        d.keys.push(k);
        d.trades.push(Trade::new());
        let json = d.to_json().unwrap();
        assert_eq!(VaultData::from_json(&json).unwrap(), d);
    }

    #[test]
    fn newer_schema_is_refused() {
        let json = br#"{"schema_version": 999}"#;
        assert!(matches!(
            VaultData::from_json(json),
            Err(VaultError::UnsupportedSchema(999))
        ));
    }

    #[test]
    fn older_schema_without_migration_is_an_error() {
        let json = br#"{"schema_version": 0}"#;
        assert!(matches!(
            VaultData::from_json(json),
            Err(VaultError::Data(_))
        ));
    }

    #[test]
    fn empty_current_schema_gets_defaults() {
        let d = VaultData::from_json(br#"{"schema_version": 1}"#).unwrap();
        assert_eq!(d, VaultData::default());
    }
}
