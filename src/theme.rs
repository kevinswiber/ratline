//! Semantic color tokens resolved for the terminal's light or dark
//! appearance. Pure: no io, no environment, no filesystem.
#![allow(dead_code)] // Tokens land ahead of the commands that read them.

use anyhow::anyhow;
use ratatui::style::Color;

use crate::style_spec::parse_color;

/// What the user asked for. Mirrors `ColorMode` (`src/color.rs`).
#[derive(Copy, Clone, PartialEq, Eq, Debug, Default, clap::ValueEnum)]
pub enum AppearanceMode {
    #[default]
    Auto,
    Light,
    Dark,
}

/// A verdict. Mirrors `ColorProfile`: no `Auto`, because a verdict is never
/// "maybe".
#[derive(Copy, Clone, PartialEq, Eq, Debug)]
pub enum Appearance {
    Light,
    Dark,
}

impl Appearance {
    /// The value exported to child processes and accepted back from the
    /// environment.
    pub fn as_str(self) -> &'static str {
        match self {
            Appearance::Light => "light",
            Appearance::Dark => "dark",
        }
    }
}

/// Where the verdict came from. `doctor` reports it; nothing branches on it.
#[derive(Copy, Clone, PartialEq, Eq, Debug)]
pub enum AppearanceSource {
    // The flag and the environment variable resolve to the same value, so
    // one variant covers both.
    Explicit,
    Osc,
    ColorFgBg,
    Default,
}

/// The resolved token table. `Copy`, so it threads like `ColorProfile`.
#[derive(Copy, Clone, PartialEq, Eq, Debug)]
pub struct Palette {
    pub appearance: Appearance,
    pub source: AppearanceSource,
    pub accent: Color,
    pub on_accent: Color,
    pub muted: Color,
    pub border: Color,
    pub ok: Color,
    pub warn: Color,
    pub error: Color,
    pub debug: Color,
    pub info: Color,
    pub fatal: Color,
    // Log levels keep their own warn/error hues, distinct from the
    // threshold convention; not addressable by name.
    pub log_warn: Color,
    pub log_error: Color,
}

/// Every public token name, in declaration order. Pinned against `token()`
/// by a unit test.
pub const TOKEN_NAMES: [&str; 10] = [
    "accent",
    "on-accent",
    "muted",
    "border",
    "ok",
    "warn",
    "error",
    "debug",
    "info",
    "fatal",
];

const DARK: Palette = Palette {
    appearance: Appearance::Dark,
    source: AppearanceSource::Default,
    accent: Color::Indexed(212),
    on_accent: Color::Black,
    muted: Color::Indexed(240),
    border: Color::Indexed(240),
    ok: Color::Indexed(42),
    warn: Color::Indexed(214),
    error: Color::Indexed(196),
    debug: Color::Indexed(63),
    info: Color::Indexed(86),
    fatal: Color::Indexed(134),
    log_warn: Color::Indexed(192),
    log_error: Color::Indexed(204),
};

// Light-background partners for the values above.
const LIGHT: Palette = Palette {
    appearance: Appearance::Light,
    source: AppearanceSource::Default,
    accent: Color::Indexed(129),
    on_accent: Color::Black,
    muted: Color::Indexed(242),
    border: Color::Indexed(249),
    ok: Color::Indexed(28),
    warn: Color::Indexed(130),
    error: Color::Indexed(160),
    debug: Color::Indexed(25),
    info: Color::Indexed(30),
    fatal: Color::Indexed(91),
    log_warn: Color::Indexed(100),
    log_error: Color::Indexed(161),
};

impl Palette {
    pub fn token(&self, name: &str) -> Option<Color> {
        Some(match name {
            "accent" => self.accent,
            "on-accent" => self.on_accent,
            "muted" => self.muted,
            "border" => self.border,
            "ok" => self.ok,
            "warn" => self.warn,
            "error" => self.error,
            "debug" => self.debug,
            "info" => self.info,
            "fatal" => self.fatal,
            _ => return None,
        })
    }

    /// The one entry point for a user-supplied color string. Tokens win;
    /// anything else is a literal, parsed by the untouched `parse_color`.
    pub fn resolve(&self, s: &str) -> anyhow::Result<Color> {
        match self.token(s) {
            Some(color) => Ok(color),
            None => parse_color(s).map_err(|_| anyhow!("invalid color or theme token: {s}")),
        }
    }

    pub fn builtin(appearance: Appearance, source: AppearanceSource) -> Palette {
        let mut palette = match appearance {
            Appearance::Dark => DARK,
            Appearance::Light => LIGHT,
        };
        palette.source = source;
        palette
    }
}

#[cfg(test)]
mod tests {
    use ratatui::style::Color;

    use super::*;
    use crate::style_spec::parse_color;

    #[test]
    fn every_token_name_resolves_in_both_palettes() {
        for appearance in [Appearance::Dark, Appearance::Light] {
            let palette = Palette::builtin(appearance, AppearanceSource::Default);
            for name in TOKEN_NAMES {
                assert!(
                    palette.token(name).is_some(),
                    "{name} is unresolved for {appearance:?}"
                );
            }
        }
    }

    #[test]
    fn token_accepts_nothing_beyond_the_name_list() {
        let palette = Palette::builtin(Appearance::Dark, AppearanceSource::Default);
        // Snake-case spellings, the internal log fields, and literals are
        // all non-tokens: `resolve` must hand them to the literal parser.
        for name in ["on_accent", "log-warn", "log-error", "accents", "", "212"] {
            assert!(palette.token(name).is_none(), "{name} must not be a token");
        }
    }

    #[test]
    fn no_token_shadows_a_literal_color_name() {
        for name in TOKEN_NAMES {
            assert!(
                parse_color(name).is_err(),
                "{name} collides with a color name"
            );
        }
    }

    #[test]
    fn resolve_falls_back_to_literal_syntax() {
        let palette = Palette::builtin(Appearance::Dark, AppearanceSource::Default);
        assert_eq!(palette.resolve("212").unwrap(), Color::Indexed(212));
        assert_eq!(palette.resolve("#ff00ff").unwrap(), Color::Rgb(255, 0, 255));
        assert_eq!(palette.resolve("red").unwrap(), Color::Red);
        assert_eq!(palette.resolve("accent").unwrap(), palette.accent);
        assert!(palette.resolve("definitely-not-a-color").is_err());
    }

    #[test]
    fn dark_reproduces_the_shipping_indices() {
        let p = Palette::builtin(Appearance::Dark, AppearanceSource::Default);
        assert_eq!(p.accent, Color::Indexed(212));
        assert_eq!(p.on_accent, Color::Black);
        assert_eq!(p.muted, Color::Indexed(240));
        assert_eq!(p.border, Color::Indexed(240));
        assert_eq!(p.ok, Color::Indexed(42));
        assert_eq!(p.warn, Color::Indexed(214));
        assert_eq!(p.error, Color::Indexed(196));
        assert_eq!(p.debug, Color::Indexed(63));
        assert_eq!(p.info, Color::Indexed(86));
        assert_eq!(p.fatal, Color::Indexed(134));
        assert_eq!(p.log_warn, Color::Indexed(192));
        assert_eq!(p.log_error, Color::Indexed(204));
    }

    #[test]
    fn light_differs_from_dark_on_every_token() {
        let dark = Palette::builtin(Appearance::Dark, AppearanceSource::Default);
        let light = Palette::builtin(Appearance::Light, AppearanceSource::Default);
        for name in TOKEN_NAMES {
            if name == "on-accent" {
                // Deliberately shared: black reads on both accents.
                assert_eq!(dark.token(name), light.token(name));
                continue;
            }
            assert_ne!(dark.token(name), light.token(name), "{name} is unpaired");
        }
    }

    #[test]
    fn builtin_stamps_appearance_and_provenance() {
        let p = Palette::builtin(Appearance::Light, AppearanceSource::Osc);
        assert_eq!(p.appearance, Appearance::Light);
        assert_eq!(p.source, AppearanceSource::Osc);
        assert_eq!(Appearance::Light.as_str(), "light");
        assert_eq!(Appearance::Dark.as_str(), "dark");
    }
}
