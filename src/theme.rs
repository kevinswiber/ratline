//! Semantic color tokens resolved for the terminal's light or dark
//! appearance. Pure: no io, no environment, no filesystem.

use anyhow::anyhow;
use ratatui::style::Color;

use crate::color::{ColorProfile, EnvSource};
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

/// Where the verdict came from. `doctor` reports it; the theme-notification
/// subscription gate refuses to subscribe over an explicit choice.
#[derive(Copy, Clone, PartialEq, Eq, Debug)]
pub enum AppearanceSource {
    // The flag and the environment variable resolve to the same value, so
    // one variant covers both.
    Explicit,
    Osc,
    ColorFgBg,
    Default,
    // A palette re-resolved from a terminal-pushed theme report.
    Notification,
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
#[allow(dead_code)] // Read by tests and documentation, not by commands.
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

// Light-background partners for the values above. White-on-accent: the
// light accent is darker than the dark palette's, so white text carries
// better contrast on it than black does.
const LIGHT: Palette = Palette {
    appearance: Appearance::Light,
    source: AppearanceSource::Default,
    accent: Color::Indexed(129),
    on_accent: Color::White,
    muted: Color::Indexed(242),
    border: Color::Indexed(249),
    ok: Color::Indexed(28),
    warn: Color::Indexed(172),
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

/// What a run with no verdict looks like. Every built-in value is tuned for
/// a dark background, so this keeps a no-verdict run identical to the
/// terminal output shipped before appearance detection existed.
pub const DEFAULT_APPEARANCE: Appearance = Appearance::Dark;

/// How long to wait for the terminal to answer. Short enough not to be felt
/// on a command that runs once per shell line; a terminal that will not
/// answer at all is detected without waiting this long.
pub const PROBE_TIMEOUT: std::time::Duration = std::time::Duration::from_millis(300);

/// True when the process is allowed to ask the terminal. An explicit mode is
/// already an answer, and a colorless profile has nothing to ask about.
pub fn may_detect(mode: AppearanceMode, profile: ColorProfile) -> bool {
    mode == AppearanceMode::Auto && profile != ColorProfile::Ascii
}

/// Read the background palette index out of `COLORFGBG`. The value is either
/// `<fg>;<bg>` or the urxvt variant `<fg>;default;<bg>`, so the last
/// `;`-separated field is the background. A `default` or non-numeric field
/// yields no verdict, and at least one `;` is required so a stray
/// single-field value cannot be misread as a background index.
pub fn appearance_from_colorfgbg(env: &dyn EnvSource) -> Option<Appearance> {
    let value = env.get("COLORFGBG")?;
    value.find(';')?;
    let bg = value.rsplit(';').next()?.trim();
    if bg.is_empty() || bg.eq_ignore_ascii_case("default") {
        return None;
    }
    let bg = bg.parse::<u32>().ok()?;
    if bg < 8 {
        Some(Appearance::Dark)
    } else {
        Some(Appearance::Light)
    }
}

/// Collapse the request and whatever was detected into one verdict. Never
/// returns "maybe".
pub fn resolve_appearance(
    mode: AppearanceMode,
    detected: Option<(Appearance, AppearanceSource)>,
) -> (Appearance, AppearanceSource) {
    match mode {
        AppearanceMode::Light => (Appearance::Light, AppearanceSource::Explicit),
        AppearanceMode::Dark => (Appearance::Dark, AppearanceSource::Explicit),
        AppearanceMode::Auto => detected.unwrap_or((DEFAULT_APPEARANCE, AppearanceSource::Default)),
    }
}

#[cfg(test)]
mod tests {
    use std::collections::HashMap;

    use ratatui::style::Color;
    use rstest::rstest;

    use super::*;
    use crate::color::{ColorProfile, MapEnv};
    use crate::style_spec::parse_color;

    fn env(pairs: &[(&str, &str)]) -> MapEnv {
        MapEnv(
            pairs
                .iter()
                .map(|(k, v)| (k.to_string(), v.to_string()))
                .collect::<HashMap<_, _>>(),
        )
    }

    #[rstest]
    // Only Auto asks; an explicit mode is already an answer.
    #[case(AppearanceMode::Auto, ColorProfile::Ansi256, true)]
    #[case(AppearanceMode::Auto, ColorProfile::TrueColor, true)]
    #[case(AppearanceMode::Auto, ColorProfile::Ansi, true)]
    #[case(AppearanceMode::Light, ColorProfile::TrueColor, false)]
    #[case(AppearanceMode::Dark, ColorProfile::TrueColor, false)]
    // Ascii never asks, whatever the mode.
    #[case(AppearanceMode::Auto, ColorProfile::Ascii, false)]
    #[case(AppearanceMode::Light, ColorProfile::Ascii, false)]
    #[case(AppearanceMode::Dark, ColorProfile::Ascii, false)]
    fn may_detect_matrix(
        #[case] mode: AppearanceMode,
        #[case] profile: ColorProfile,
        #[case] expected: bool,
    ) {
        assert_eq!(may_detect(mode, profile), expected);
    }

    #[rstest]
    // An explicit mode wins and is reported as explicit, whatever was found.
    #[case(
        AppearanceMode::Light,
        None,
        Appearance::Light,
        AppearanceSource::Explicit
    )]
    #[case(
        AppearanceMode::Dark,
        None,
        Appearance::Dark,
        AppearanceSource::Explicit
    )]
    #[case(
        AppearanceMode::Light,
        Some((Appearance::Dark, AppearanceSource::Osc)),
        Appearance::Light,
        AppearanceSource::Explicit
    )]
    // Auto adopts the verdict and its provenance verbatim.
    #[case(
        AppearanceMode::Auto,
        Some((Appearance::Light, AppearanceSource::Osc)),
        Appearance::Light,
        AppearanceSource::Osc
    )]
    #[case(
        AppearanceMode::Auto,
        Some((Appearance::Light, AppearanceSource::ColorFgBg)),
        Appearance::Light,
        AppearanceSource::ColorFgBg
    )]
    // Auto with no verdict falls to the documented default.
    #[case(
        AppearanceMode::Auto,
        None,
        Appearance::Dark,
        AppearanceSource::Default
    )]
    fn resolve_appearance_matrix(
        #[case] mode: AppearanceMode,
        #[case] detected: Option<(Appearance, AppearanceSource)>,
        #[case] appearance: Appearance,
        #[case] source: AppearanceSource,
    ) {
        assert_eq!(resolve_appearance(mode, detected), (appearance, source));
    }

    #[test]
    fn colorfgbg_two_field_form() {
        let dark = env(&[("COLORFGBG", "15;0")]);
        assert_eq!(appearance_from_colorfgbg(&dark), Some(Appearance::Dark));
        let light = env(&[("COLORFGBG", "0;15")]);
        assert_eq!(appearance_from_colorfgbg(&light), Some(Appearance::Light));
        // The 8-index boundary: 7 is dark, 8 is light.
        assert_eq!(
            appearance_from_colorfgbg(&env(&[("COLORFGBG", "7;7")])),
            Some(Appearance::Dark)
        );
        assert_eq!(
            appearance_from_colorfgbg(&env(&[("COLORFGBG", "7;8")])),
            Some(Appearance::Light)
        );
    }

    #[test]
    fn colorfgbg_three_field_form() {
        // The urxvt variant `<fg>;default;<bg>`: the last field decides.
        assert_eq!(
            appearance_from_colorfgbg(&env(&[("COLORFGBG", "15;default;0")])),
            Some(Appearance::Dark)
        );
        assert_eq!(
            appearance_from_colorfgbg(&env(&[("COLORFGBG", "0;default;15")])),
            Some(Appearance::Light)
        );
    }

    #[test]
    fn colorfgbg_skips_default_or_malformed_backgrounds() {
        for value in [
            "7;default",          // background `default` is no verdict
            "15;default;default", // ditto in the three-field form
            "7;banana",           // non-numeric background
            "",                   // empty
            ";",                  // separator with no fields
            "7",                  // single field, even though it parses
            "default",
        ] {
            assert_eq!(
                appearance_from_colorfgbg(&env(&[("COLORFGBG", value)])),
                None,
                "{value:?} must yield no verdict"
            );
        }
        // Unset is also no verdict.
        assert_eq!(appearance_from_colorfgbg(&env(&[])), None);
    }

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
