//! Non-sensitive preferences in `config.toml`: vault location and theme.
//! Everything sensitive lives in the encrypted vault instead.

use std::path::{Path, PathBuf};

use directories::ProjectDirs;
use serde::{Deserialize, Serialize};

use crate::theme::ThemeId;

#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Config {
    pub vault_path: Option<PathBuf>,
    pub theme: Option<String>,
}

fn dirs() -> Option<ProjectDirs> {
    ProjectDirs::from("", "", "KeyKeeper")
}

impl Config {
    /// `%APPDATA%\KeyKeeper\config\config.toml` on Windows.
    pub fn default_path() -> Option<PathBuf> {
        dirs().map(|d| d.config_dir().join("config.toml"))
    }

    /// `%APPDATA%\KeyKeeper\data\vault.kkvault` on Windows.
    pub fn default_vault_path() -> PathBuf {
        dirs()
            .map(|d| d.data_dir().join("vault.kkvault"))
            .unwrap_or_else(|| PathBuf::from("vault.kkvault"))
    }

    /// A missing or unreadable file gives the defaults; config is never worth refusing to start over.
    pub fn load(path: &Path) -> Config {
        std::fs::read_to_string(path)
            .ok()
            .and_then(|s| toml::from_str(&s).ok())
            .unwrap_or_default()
    }

    pub fn save(&self, path: &Path) -> std::io::Result<()> {
        if let Some(dir) = path.parent() {
            std::fs::create_dir_all(dir)?;
        }
        let text = toml::to_string_pretty(self).map_err(std::io::Error::other)?;
        std::fs::write(path, text)
    }

    pub fn theme(&self) -> ThemeId {
        self.theme
            .as_deref()
            .and_then(ThemeId::from_slug)
            .unwrap_or_default()
    }

    pub fn vault_path(&self) -> PathBuf {
        self.vault_path
            .clone()
            .unwrap_or_else(Config::default_vault_path)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn round_trip_and_defaults() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("nested").join("config.toml");
        assert_eq!(Config::load(&path), Config::default());
        assert_eq!(Config::default().theme(), ThemeId::Midnight);

        let c = Config {
            vault_path: Some(dir.path().join("v.kkvault")),
            theme: Some("forest".into()),
        };
        c.save(&path).unwrap();
        let back = Config::load(&path);
        assert_eq!(back, c);
        assert_eq!(back.theme(), ThemeId::Forest);
    }

    #[test]
    fn garbage_config_falls_back_to_defaults() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("config.toml");
        std::fs::write(&path, "this is = = not toml").unwrap();
        assert_eq!(Config::load(&path), Config::default());
        std::fs::write(&path, "theme = \"no-such-theme\"").unwrap();
        assert_eq!(Config::load(&path).theme(), ThemeId::Midnight);
    }
}
