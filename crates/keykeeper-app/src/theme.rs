//! Theme presets. A theme is a `rune-kit` [`Kit`]: palette presets are seeded from three colours
//! (`bg`, `fg`, `accent`) via [`Kit::from_palette`]; the rest reuse RUNE's built-in kits.
//! The chosen [`ThemeId`] is a non-sensitive preference and is stored in `config.toml`
//!.

use std::sync::OnceLock;

use rune::color::hex;
use rune::{Color, ColorScheme};
use rune_kit::{Kit, PaletteSeed};

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Default)]
pub enum ThemeId {
    /// Deep navy with a soft blue accent. The default.
    #[default]
    Midnight,
    Amethyst,
    Forest,
    /// RUNE's opencode theme (warm, near-black).
    Opencode,
    /// Neutral greys.
    Graphite,
    Light,
    /// Midnight or Light, following the OS appearance.
    System,
}

impl ThemeId {
    pub const ALL: [ThemeId; 7] = [
        ThemeId::Midnight,
        ThemeId::Amethyst,
        ThemeId::Forest,
        ThemeId::Opencode,
        ThemeId::Graphite,
        ThemeId::Light,
        ThemeId::System,
    ];

    pub fn label(self) -> &'static str {
        match self {
            ThemeId::Midnight => "Midnight",
            ThemeId::Amethyst => "Amethyst",
            ThemeId::Forest => "Forest",
            ThemeId::Opencode => "Opencode",
            ThemeId::Graphite => "Graphite",
            ThemeId::Light => "Light",
            ThemeId::System => "System",
        }
    }

    /// Stable id for config files.
    pub fn slug(self) -> &'static str {
        match self {
            ThemeId::Midnight => "midnight",
            ThemeId::Amethyst => "amethyst",
            ThemeId::Forest => "forest",
            ThemeId::Opencode => "opencode",
            ThemeId::Graphite => "graphite",
            ThemeId::Light => "light",
            ThemeId::System => "system",
        }
    }

    #[allow(dead_code)] // used by config.toml loading (Phase 3)
    pub fn from_slug(s: &str) -> Option<ThemeId> {
        ThemeId::ALL
            .into_iter()
            .find(|t| t.slug().eq_ignore_ascii_case(s.trim()))
    }

    /// The concrete theme after resolving `System` against the OS appearance.
    pub fn resolve(self, system_dark: bool) -> ThemeId {
        match self {
            ThemeId::System if system_dark => ThemeId::Midnight,
            ThemeId::System => ThemeId::Light,
            t => t,
        }
    }

    #[allow(dead_code)] // used by config.toml loading (Phase 3)
    pub fn is_dark(self, system_dark: bool) -> bool {
        self.resolve(system_dark) != ThemeId::Light
    }

    /// What the native window chrome (title bar) should use.
    pub fn color_scheme(self) -> ColorScheme {
        match self {
            ThemeId::System => ColorScheme::System,
            ThemeId::Light => ColorScheme::Light,
            _ => ColorScheme::Dark,
        }
    }

    pub fn kit(self, system_dark: bool) -> &'static Kit {
        static MIDNIGHT: OnceLock<Kit> = OnceLock::new();
        static AMETHYST: OnceLock<Kit> = OnceLock::new();
        static FOREST: OnceLock<Kit> = OnceLock::new();
        static GRAPHITE: OnceLock<Kit> = OnceLock::new();
        match self.resolve(system_dark) {
            ThemeId::Midnight => MIDNIGHT.get_or_init(|| palette("#121829", "#e6eaf2", "#5b9fe3")),
            ThemeId::Amethyst => AMETHYST.get_or_init(|| palette("#1a1726", "#ece8f7", "#a78bfa")),
            ThemeId::Forest => FOREST.get_or_init(|| palette("#111a16", "#e4efe8", "#4fbf8a")),
            ThemeId::Graphite => GRAPHITE.get_or_init(|| Kit::neutral(true)),
            ThemeId::Light => rune_theme_opencode::kit(false),
            ThemeId::Opencode | ThemeId::System => rune_theme_opencode::kit(true),
        }
    }
}

fn color(s: &str) -> Color {
    hex(s).unwrap_or(Color::BLACK)
}

fn palette(bg: &str, fg: &str, accent: &str) -> Kit {
    Kit::from_palette(PaletteSeed {
        bg: color(bg),
        fg: color(fg),
        accent: color(accent),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn slugs_round_trip() {
        for t in ThemeId::ALL {
            assert_eq!(ThemeId::from_slug(t.slug()), Some(t));
        }
        assert_eq!(ThemeId::from_slug(" MIDNIGHT "), Some(ThemeId::Midnight));
        assert_eq!(ThemeId::from_slug("nope"), None);
    }

    #[test]
    fn default_is_dark_midnight() {
        assert_eq!(ThemeId::default(), ThemeId::Midnight);
        assert!(ThemeId::default().is_dark(false));
    }

    #[test]
    fn system_follows_os() {
        assert_eq!(ThemeId::System.resolve(true), ThemeId::Midnight);
        assert_eq!(ThemeId::System.resolve(false), ThemeId::Light);
    }

    #[test]
    fn every_theme_builds_a_kit_with_readable_text() {
        for t in ThemeId::ALL {
            for dark in [true, false] {
                let k = t.kit(dark);
                let (a, b) = (k.c.text_base.luminance(), k.c.page.luminance());
                let contrast = (a.max(b) + 0.05) / (a.min(b) + 0.05);
                assert!(
                    contrast >= 4.5,
                    "{t:?} (system_dark={dark}) text contrast {contrast:.2} < 4.5"
                );
            }
        }
    }
}
