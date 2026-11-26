//! Non-sensitive preferences in `config.toml`: vault location, theme, update checks, how the
//! library looks (view, density, columns) and the hover card. Everything sensitive lives in the
//! encrypted vault instead.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use directories::ProjectDirs;
use serde::{Deserialize, Serialize};

use crate::theme::ThemeId;

#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Config {
    pub vault_path: Option<PathBuf>,
    pub theme: Option<String>,
    /// "Check for updates automatically". `None` means on.
    pub check_updates: Option<bool>,
    /// A version the user chose to skip ("Skip this version").
    pub skipped_version: Option<String>,
    /// Show a game's details when the pointer rests on its name. `None` means on.
    pub hover_cards: Option<bool>,
    /// How long the pointer rests before the card opens, in milliseconds.
    pub hover_delay_ms: Option<u32>,
    /// Fetch store details and artwork from Steam. `None` means on.
    pub fetch_store_info: Option<bool>,
    /// Library layout.
    pub library: LibraryLayout,
    /// The window's size when it was last closed.
    pub window: Option<WindowSize>,
}

/// The window's size (in points) and whether it was maximized.
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
pub struct WindowSize {
    pub width: f32,
    pub height: f32,
    #[serde(default)]
    pub maximized: bool,
}

impl WindowSize {
    /// A sane size to open at: at least the minimum, at most a very large screen.
    pub fn clamped(self, min: (f32, f32)) -> (f32, f32) {
        let ok = |v: f32, lo: f32| {
            if v.is_finite() {
                v.clamp(lo, 8000.0)
            } else {
                lo
            }
        };
        (ok(self.width, min.0), ok(self.height, min.1))
    }
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum LibraryView {
    #[default]
    Table,
    Grid,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Density {
    #[default]
    Comfortable,
    Compact,
}

/// How the library table looks; remembered between sessions.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct LibraryLayout {
    pub view: LibraryView,
    pub density: Density,
    pub group_by_bundle: bool,
    pub hidden_columns: Vec<String>,
    pub column_widths: BTreeMap<String, f32>,
    pub column_order: Vec<String>,
    /// `"<column>:<asc|desc>"`.
    pub sort: Option<String>,
}

impl Default for LibraryLayout {
    fn default() -> Self {
        LibraryLayout {
            view: LibraryView::default(),
            density: Density::default(),
            group_by_bundle: false,
            // The date added is the default sort; the column itself is optional.
            hidden_columns: vec!["added".into()],
            column_widths: BTreeMap::new(),
            column_order: Vec::new(),
            sort: None,
        }
    }
}

pub const DEFAULT_HOVER_DELAY_MS: u32 = 450;

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

    pub fn check_updates(&self) -> bool {
        self.check_updates.unwrap_or(true)
    }

    pub fn hover_cards(&self) -> bool {
        self.hover_cards.unwrap_or(true)
    }

    pub fn hover_delay_secs(&self) -> f64 {
        f64::from(
            self.hover_delay_ms
                .unwrap_or(DEFAULT_HOVER_DELAY_MS)
                .clamp(0, 3000),
        ) / 1000.0
    }

    pub fn fetch_store_info(&self) -> bool {
        self.fetch_store_info.unwrap_or(true)
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
        assert!(
            Config::default().check_updates(),
            "update checks default to on"
        );

        let c = Config {
            vault_path: Some(dir.path().join("v.kkvault")),
            theme: Some("forest".into()),
            check_updates: Some(false),
            skipped_version: Some("0.2.0".into()),
            window: Some(WindowSize {
                width: 1400.0,
                height: 900.0,
                maximized: true,
            }),
            hover_cards: Some(false),
            hover_delay_ms: Some(100),
            fetch_store_info: Some(false),
            library: LibraryLayout {
                view: LibraryView::Grid,
                density: Density::Compact,
                group_by_bundle: true,
                hidden_columns: vec!["platform".into()],
                column_widths: [("key".to_string(), 180.0)].into_iter().collect(),
                column_order: vec!["name".into(), "status".into()],
                sort: Some("name:asc".into()),
            },
        };
        c.save(&path).unwrap();
        let back = Config::load(&path);
        assert_eq!(back, c);
        assert_eq!(back.theme(), ThemeId::Forest);
        assert!(!back.check_updates());
        assert!(!back.hover_cards() && !back.fetch_store_info());
        assert!((back.hover_delay_secs() - 0.1).abs() < 1e-9);
        let d = Config::default();
        assert!(d.hover_cards() && d.fetch_store_info());
        assert_eq!(d.library.view, LibraryView::Table);
        let w = WindowSize {
            width: 10.0,
            height: f32::NAN,
            maximized: false,
        };
        assert_eq!(w.clamped((960.0, 620.0)), (960.0, 620.0));
    }

    #[test]
    fn old_config_files_still_load() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("config.toml");
        std::fs::write(&path, "theme = \"forest\"\ncheck_updates = false\n").unwrap();
        let c = Config::load(&path);
        assert_eq!(c.theme(), ThemeId::Forest);
        assert_eq!(c.library, LibraryLayout::default());
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
