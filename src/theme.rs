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
    // A palette re-resolved from a terminal-pushed theme report;
    // constructed on the unix input path only.
    #[cfg_attr(windows, allow(dead_code))]
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
    // UI tier: the interactive surface. Relative order mirrors TOKEN_NAMES;
    // physically appended after the internal log fields.
    pub selection: Color,
    pub r#match: Color,
}

/// Every public token name, in declaration order. Pinned against `token()`
/// by a unit test.
#[allow(dead_code)] // Read by tests and documentation, not by commands.
pub const TOKEN_NAMES: [&str; 12] = [
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
    "selection",
    "match",
];

/// Reference tier: the raw values one appearance is built from — hue roles,
/// not uses. Not addressable by name from the CLI; `Palette::from_refs` is
/// the only reader.
///
/// Slots are typed `Color`, not `u8`, so a later value source can supply
/// `Rgb` or a low `Indexed` without changing this type or anything
/// downstream of `Palette::from_refs`.
///
/// base24 correspondence (documentation only; nothing reads it):
///
/// | slot           | base24                                  | dark | light | feeds                        |
/// |----------------|-----------------------------------------|------|-------|------------------------------|
/// | `red`          | `base08`                                | 196  | 160   | `error`                      |
/// | `orange`       | `base09`                                | 214  | 172   | `warn`                       |
/// | `yellow_green` | `base0A`                                | 192  | 100   | `log_warn`                   |
/// | `green`        | `base0B`                                | 42   | 28    | `ok`                         |
/// | `cyan`         | `base0C`                                | 86   | 30    | `info`                       |
/// | `blue`         | `base0D`                                | 63   | 25    | `debug`                      |
/// | `violet`       | `base0E`                                | 134  | 91    | `fatal`                      |
/// | `pink`         | `base17` (`base0E` under plain base16)  | 212  | 129   | `accent`                     |
/// | `pink_red`     | `base12` (`base08` under plain base16)  | 204  | 161   | `log_error`                  |
/// | `neutral_1`    | `base02`                                | 240  | 249   | `border`                     |
/// | `neutral_2`    | `base03`                                | 240  | 242   | `muted`                      |
///
/// Neutrals are ordered by distance from the background: `neutral_1` is
/// nearest (rules), `neutral_2` one step in (secondary text). Under dark
/// they coincide at 240 — deliberate; the light values are what separates
/// them.
#[derive(Copy, Clone, PartialEq, Eq, Debug)]
struct RefSlots {
    red: Color,
    orange: Color,
    yellow_green: Color,
    green: Color,
    cyan: Color,
    blue: Color,
    violet: Color,
    pink: Color,
    pink_red: Color,
    neutral_1: Color,
    neutral_2: Color,
}

const DARK_REFS: RefSlots = RefSlots {
    red: Color::Indexed(196),
    orange: Color::Indexed(214),
    yellow_green: Color::Indexed(192),
    green: Color::Indexed(42),
    cyan: Color::Indexed(86),
    blue: Color::Indexed(63),
    violet: Color::Indexed(134),
    pink: Color::Indexed(212),
    pink_red: Color::Indexed(204),
    neutral_1: Color::Indexed(240),
    neutral_2: Color::Indexed(240),
};

const LIGHT_REFS: RefSlots = RefSlots {
    red: Color::Indexed(160),
    orange: Color::Indexed(172),
    yellow_green: Color::Indexed(100),
    green: Color::Indexed(28),
    cyan: Color::Indexed(30),
    blue: Color::Indexed(25),
    violet: Color::Indexed(91),
    pink: Color::Indexed(129),
    pink_red: Color::Indexed(161),
    neutral_1: Color::Indexed(249),
    neutral_2: Color::Indexed(242),
};

/// Not ref-derived: a legibility pin against `accent`, not a hue of its
/// own. White on the light accent because that accent is darker than the
/// dark palette's, so white text carries better contrast on it than black
/// does.
const fn on_accent_for(appearance: Appearance) -> Color {
    match appearance {
        Appearance::Dark => Color::Black,
        Appearance::Light => Color::White,
    }
}

const DARK: Palette = Palette::from_refs(DARK_REFS, Appearance::Dark);
const LIGHT: Palette = Palette::from_refs(LIGHT_REFS, Appearance::Light);

impl Palette {
    /// The one seam where a set of reference values becomes a token table.
    /// `builtin` is its only caller today; a loader that reads a theme file
    /// or the terminal's own ANSI palette adds a second caller and changes
    /// nothing below this line.
    const fn from_refs(refs: RefSlots, appearance: Appearance) -> Palette {
        Palette {
            appearance,
            source: AppearanceSource::Default,
            // semantic tier: meaning and state.
            ok: refs.green,
            warn: refs.orange,
            error: refs.red,
            info: refs.cyan,
            debug: refs.blue,
            fatal: refs.violet,
            accent: refs.pink,
            log_warn: refs.yellow_green,
            log_error: refs.pink_red,
            // ui tier: chrome and interaction.
            muted: refs.neutral_2,
            border: refs.neutral_1,
            on_accent: on_accent_for(appearance),
            selection: refs.pink,
            r#match: refs.pink,
        }
    }

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
            "selection" => self.selection,
            "match" => self.r#match,
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
    fn semantic_tokens_take_their_values_from_ref_slots() {
        for (appearance, refs) in [
            (Appearance::Dark, DARK_REFS),
            (Appearance::Light, LIGHT_REFS),
        ] {
            let p = Palette::builtin(appearance, AppearanceSource::Default);
            let table: [(&str, Color, Color); 9] = [
                ("ok", p.ok, refs.green),
                ("warn", p.warn, refs.orange),
                ("error", p.error, refs.red),
                ("info", p.info, refs.cyan),
                ("debug", p.debug, refs.blue),
                ("fatal", p.fatal, refs.violet),
                ("accent", p.accent, refs.pink),
                ("log_warn", p.log_warn, refs.yellow_green),
                ("log_error", p.log_error, refs.pink_red),
            ];
            for (name, actual, expected) in table {
                assert_eq!(actual, expected, "{name} in {appearance:?}");
            }
        }
    }

    #[test]
    fn ui_tokens_take_their_values_from_ref_slots_or_a_pin() {
        for (appearance, refs) in [
            (Appearance::Dark, DARK_REFS),
            (Appearance::Light, LIGHT_REFS),
        ] {
            let p = Palette::builtin(appearance, AppearanceSource::Default);
            assert_eq!(p.muted, refs.neutral_2, "muted in {appearance:?}");
            assert_eq!(p.border, refs.neutral_1, "border in {appearance:?}");
            assert_eq!(p.selection, refs.pink, "selection in {appearance:?}");
            assert_eq!(p.r#match, refs.pink, "match in {appearance:?}");
            // Deliberately not ref-derived: a legibility pin against `accent`.
            assert_eq!(p.on_accent, on_accent_for(appearance));
        }
    }

    #[test]
    fn ref_slots_reproduce_the_shipping_indices() {
        assert_eq!(DARK_REFS.red, Color::Indexed(196));
        assert_eq!(DARK_REFS.orange, Color::Indexed(214));
        assert_eq!(DARK_REFS.yellow_green, Color::Indexed(192));
        assert_eq!(DARK_REFS.green, Color::Indexed(42));
        assert_eq!(DARK_REFS.cyan, Color::Indexed(86));
        assert_eq!(DARK_REFS.blue, Color::Indexed(63));
        assert_eq!(DARK_REFS.violet, Color::Indexed(134));
        assert_eq!(DARK_REFS.pink, Color::Indexed(212));
        assert_eq!(DARK_REFS.pink_red, Color::Indexed(204));
        assert_eq!(DARK_REFS.neutral_1, Color::Indexed(240));
        assert_eq!(DARK_REFS.neutral_2, Color::Indexed(240));
        assert_eq!(LIGHT_REFS.red, Color::Indexed(160));
        assert_eq!(LIGHT_REFS.orange, Color::Indexed(172));
        assert_eq!(LIGHT_REFS.yellow_green, Color::Indexed(100));
        assert_eq!(LIGHT_REFS.green, Color::Indexed(28));
        assert_eq!(LIGHT_REFS.cyan, Color::Indexed(30));
        assert_eq!(LIGHT_REFS.blue, Color::Indexed(25));
        assert_eq!(LIGHT_REFS.violet, Color::Indexed(91));
        assert_eq!(LIGHT_REFS.pink, Color::Indexed(129));
        assert_eq!(LIGHT_REFS.pink_red, Color::Indexed(161));
        assert_eq!(LIGHT_REFS.neutral_1, Color::Indexed(249));
        assert_eq!(LIGHT_REFS.neutral_2, Color::Indexed(242));
    }

    #[test]
    fn the_first_ten_token_names_are_the_v0_5_0_set() {
        assert_eq!(
            &TOKEN_NAMES[..10],
            &[
                "accent",
                "on-accent",
                "muted",
                "border",
                "ok",
                "warn",
                "error",
                "debug",
                "info",
                "fatal"
            ]
        );
    }

    #[test]
    fn selection_and_match_derive_from_the_accent_slot() {
        for (appearance, refs) in [
            (Appearance::Dark, DARK_REFS),
            (Appearance::Light, LIGHT_REFS),
        ] {
            let p = Palette::builtin(appearance, AppearanceSource::Default);
            assert_eq!(p.selection, refs.pink, "selection in {appearance:?}");
            assert_eq!(p.r#match, refs.pink, "match in {appearance:?}");
            assert_eq!(p.selection, p.accent);
            assert_eq!(p.resolve("selection").unwrap(), p.accent);
            assert_eq!(p.resolve("match").unwrap(), p.accent);
        }
    }

    #[test]
    fn every_palette_color_is_a_ref_slot_or_a_declared_pin() {
        for (appearance, refs) in [
            (Appearance::Dark, DARK_REFS),
            (Appearance::Light, LIGHT_REFS),
        ] {
            let p = Palette::builtin(appearance, AppearanceSource::Default);
            let ref_values = [
                refs.red,
                refs.orange,
                refs.yellow_green,
                refs.green,
                refs.cyan,
                refs.blue,
                refs.violet,
                refs.pink,
                refs.pink_red,
                refs.neutral_1,
                refs.neutral_2,
            ];
            let pinned: [(&str, Color); 1] = [("on-accent", on_accent_for(appearance))];
            // Over FIELDS, not TOKEN_NAMES: log_warn/log_error are included.
            let fields: [(&str, Color); 14] = [
                ("accent", p.accent),
                ("on-accent", p.on_accent),
                ("muted", p.muted),
                ("border", p.border),
                ("ok", p.ok),
                ("warn", p.warn),
                ("error", p.error),
                ("debug", p.debug),
                ("info", p.info),
                ("fatal", p.fatal),
                ("log_warn", p.log_warn),
                ("log_error", p.log_error),
                ("selection", p.selection),
                ("match", p.r#match),
            ];
            for (name, value) in fields {
                let is_ref = ref_values.contains(&value);
                let is_pin = pinned.iter().any(|(n, v)| *n == name && *v == value);
                assert!(
                    is_ref || is_pin,
                    "{name} in {appearance:?} has an invented value {value:?}"
                );
            }
        }
    }

    #[test]
    fn light_reproduces_the_shipping_indices() {
        let p = Palette::builtin(Appearance::Light, AppearanceSource::Default);
        assert_eq!(p.accent, Color::Indexed(129));
        assert_eq!(p.on_accent, Color::White);
        assert_eq!(p.muted, Color::Indexed(242));
        assert_eq!(p.border, Color::Indexed(249));
        assert_eq!(p.ok, Color::Indexed(28));
        assert_eq!(p.warn, Color::Indexed(172));
        assert_eq!(p.error, Color::Indexed(160));
        assert_eq!(p.debug, Color::Indexed(25));
        assert_eq!(p.info, Color::Indexed(30));
        assert_eq!(p.fatal, Color::Indexed(91));
        assert_eq!(p.log_warn, Color::Indexed(100));
        assert_eq!(p.log_error, Color::Indexed(161));
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
