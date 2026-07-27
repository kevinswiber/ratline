/// How aggressively to emit color, from the global `--color` flag.
#[derive(Copy, Clone, PartialEq, Eq, Debug, Default, clap::ValueEnum)]
pub enum ColorMode {
    #[default]
    Auto,
    Always,
    Never,
}

/// What the terminal can render. Variant order is capability order so
/// `max` works for `ColorMode::Always`.
#[derive(Copy, Clone, PartialEq, Eq, PartialOrd, Ord, Debug)]
pub enum ColorProfile {
    Ascii,
    Ansi,
    Ansi256,
    TrueColor,
}

/// Environment access seam so detection stays pure and testable.
pub trait EnvSource {
    fn get(&self, key: &str) -> Option<String>;
}

pub struct SystemEnv;

impl EnvSource for SystemEnv {
    fn get(&self, key: &str) -> Option<String> {
        std::env::var(key).ok()
    }
}

#[cfg(test)]
pub struct MapEnv(pub std::collections::HashMap<String, String>);

#[cfg(test)]
impl EnvSource for MapEnv {
    fn get(&self, key: &str) -> Option<String> {
        self.0.get(key).cloned()
    }
}

/// Termenv-compatible profile detection (traced from termenv v0.16.0).
/// `stream_is_tty` is the ttyness of the UI stream — never stdout.
pub fn detect_profile(env: &dyn EnvSource, stream_is_tty: bool) -> ColorProfile {
    let force_on = env
        .get("CLICOLOR_FORCE")
        .is_some_and(|v| !v.is_empty() && v != "0");

    if env.get("NO_COLOR").is_some_and(|v| !v.is_empty()) {
        return ColorProfile::Ascii;
    }
    if env.get("CLICOLOR").is_some_and(|v| v == "0") && !force_on {
        return ColorProfile::Ascii;
    }

    let profile = base_profile(env, stream_is_tty);
    if force_on && profile == ColorProfile::Ascii {
        return ColorProfile::Ansi;
    }
    profile
}

fn base_profile(env: &dyn EnvSource, stream_is_tty: bool) -> ColorProfile {
    // CI systems report no tty semantics regardless of the stream.
    if env.get("CI").is_some_and(|v| !v.is_empty()) || !stream_is_tty {
        return ColorProfile::Ascii;
    }
    term_capability(env)
}

/// What TERM/COLORTERM alone say the terminal can render, with no tty or
/// kill-switch gating. This is what an explicit `--color always` trusts.
fn term_capability(env: &dyn EnvSource) -> ColorProfile {
    if env.get("GOOGLE_CLOUD_SHELL").as_deref() == Some("true") {
        return ColorProfile::TrueColor;
    }

    let term = env.get("TERM").unwrap_or_default();
    if let Some(colorterm) = env.get("COLORTERM") {
        match colorterm.to_lowercase().as_str() {
            "24bit" | "truecolor" => {
                // Old screen versions swallow truecolor; tmux is fine.
                if term.starts_with("screen") && env.get("TERM_PROGRAM").as_deref() != Some("tmux")
                {
                    return ColorProfile::Ansi256;
                }
                return ColorProfile::TrueColor;
            }
            "yes" | "true" => return ColorProfile::Ansi256,
            _ => {}
        }
    }

    match term.as_str() {
        "alacritty" | "contour" | "rio" | "wezterm" | "xterm-ghostty" | "xterm-kitty" => {
            return ColorProfile::TrueColor;
        }
        "linux" | "xterm" => return ColorProfile::Ansi,
        _ => {}
    }
    if term.contains("256color") {
        ColorProfile::Ansi256
    } else if term.contains("color") || term.contains("ansi") {
        ColorProfile::Ansi
    } else if term.is_empty() {
        bare_console_profile()
    } else {
        ColorProfile::Ascii
    }
}

/// What a console with no TERM at all can render. Native Windows sessions
/// never set TERM, and every OS this binary supports has a VT-capable,
/// 24-bit console; unix without TERM is an unknown terminal.
#[cfg(windows)]
fn bare_console_profile() -> ColorProfile {
    ColorProfile::TrueColor
}

#[cfg(not(windows))]
fn bare_console_profile() -> ColorProfile {
    ColorProfile::Ascii
}

pub fn resolve_profile(mode: ColorMode, env: &dyn EnvSource, is_tty: bool) -> ColorProfile {
    match mode {
        ColorMode::Never => ColorProfile::Ascii,
        // An explicit --color always outranks the ambient environment:
        // ttyness, NO_COLOR, CLICOLOR, and CI are all ignored, and
        // TERM/COLORTERM decide the depth (16-color at minimum).
        ColorMode::Always => term_capability(env).max(ColorProfile::Ansi),
        ColorMode::Auto => detect_profile(env, is_tty),
    }
}

#[cfg(test)]
mod tests {
    use std::collections::HashMap;

    use rstest::rstest;

    use super::*;

    fn env(pairs: &[(&str, &str)]) -> MapEnv {
        MapEnv(
            pairs
                .iter()
                .map(|(k, v)| (k.to_string(), v.to_string()))
                .collect::<HashMap<_, _>>(),
        )
    }

    #[rstest]
    // NO_COLOR wins over everything, including CLICOLOR_FORCE.
    #[case(&[("NO_COLOR", "1"), ("TERM", "xterm-256color")], true, ColorProfile::Ascii)]
    #[case(&[("NO_COLOR", "1"), ("CLICOLOR_FORCE", "1"), ("TERM", "xterm-256color")], true, ColorProfile::Ascii)]
    // CLICOLOR=0 disables unless force is on.
    #[case(&[("CLICOLOR", "0"), ("TERM", "xterm-256color")], true, ColorProfile::Ascii)]
    #[case(&[("CLICOLOR", "0"), ("CLICOLOR_FORCE", "1"), ("TERM", "xterm-256color")], true, ColorProfile::Ansi256)]
    // Force bumps Ascii (from not-a-tty) to Ansi; force=0 doesn't.
    #[case(&[("CLICOLOR_FORCE", "1"), ("TERM", "xterm-256color")], false, ColorProfile::Ansi)]
    #[case(&[("CLICOLOR_FORCE", "0"), ("TERM", "xterm-256color")], false, ColorProfile::Ascii)]
    // CI is treated as not-a-tty even when the stream is one.
    #[case(&[("CI", "1"), ("TERM", "xterm-256color")], true, ColorProfile::Ascii)]
    // COLORTERM truecolor, with the screen-without-tmux exception.
    #[case(&[("COLORTERM", "truecolor"), ("TERM", "screen")], true, ColorProfile::Ansi256)]
    #[case(&[("COLORTERM", "truecolor"), ("TERM", "screen"), ("TERM_PROGRAM", "tmux")], true, ColorProfile::TrueColor)]
    #[case(&[("COLORTERM", "24bit"), ("TERM", "xterm-256color")], true, ColorProfile::TrueColor)]
    #[case(&[("COLORTERM", "yes"), ("TERM", "xterm-256color")], true, ColorProfile::Ansi256)]
    // TERM exact tiers.
    #[case(&[("TERM", "xterm-kitty")], true, ColorProfile::TrueColor)]
    #[case(&[("TERM", "xterm-ghostty")], true, ColorProfile::TrueColor)]
    #[case(&[("TERM", "wezterm")], true, ColorProfile::TrueColor)]
    #[case(&[("TERM", "xterm")], true, ColorProfile::Ansi)]
    #[case(&[("TERM", "linux")], true, ColorProfile::Ansi)]
    // TERM substring tiers.
    #[case(&[("TERM", "xterm-256color")], true, ColorProfile::Ansi256)]
    #[case(&[("TERM", "vt100-color")], true, ColorProfile::Ansi)]
    #[case(&[("TERM", "ansi")], true, ColorProfile::Ansi)]
    #[case(&[("TERM", "dumb")], true, ColorProfile::Ascii)]
    // Cloud shell.
    #[case(&[("GOOGLE_CLOUD_SHELL", "true")], true, ColorProfile::TrueColor)]
    // Not a tty means no color, regardless of TERM.
    #[case(&[("TERM", "xterm-256color")], false, ColorProfile::Ascii)]
    fn detect_matrix(
        #[case] pairs: &[(&str, &str)],
        #[case] is_tty: bool,
        #[case] expected: ColorProfile,
    ) {
        assert_eq!(detect_profile(&env(pairs), is_tty), expected);
    }

    #[cfg(windows)]
    #[test]
    fn a_bare_console_defaults_to_truecolor() {
        // Native Windows sessions don't set TERM; modern consoles all
        // speak VT with 24-bit color.
        assert_eq!(detect_profile(&env(&[]), true), ColorProfile::TrueColor);
        // An explicit TERM still wins, dumb included.
        assert_eq!(
            detect_profile(&env(&[("TERM", "dumb")]), true),
            ColorProfile::Ascii
        );
        // The kill switches still kill.
        assert_eq!(
            detect_profile(&env(&[("NO_COLOR", "1")]), true),
            ColorProfile::Ascii
        );
        // Not-a-tty still means no color.
        assert_eq!(detect_profile(&env(&[]), false), ColorProfile::Ascii);
    }

    #[cfg(not(windows))]
    #[test]
    fn a_bare_environment_stays_colorless() {
        // No TERM on unix means an unknown terminal: no color.
        assert_eq!(detect_profile(&env(&[]), true), ColorProfile::Ascii);
    }

    #[test]
    fn always_ignores_kill_switches_at_full_depth() {
        // An explicit --color always outranks ambient NO_COLOR/CI, and TERM
        // decides the depth: 256-color, not a 16-color floor.
        let e = env(&[("NO_COLOR", "1"), ("TERM", "xterm-256color")]);
        assert_eq!(
            resolve_profile(ColorMode::Always, &e, false),
            ColorProfile::Ansi256
        );
        let ci = env(&[("CI", "1"), ("TERM", "xterm-ghostty")]);
        assert_eq!(
            resolve_profile(ColorMode::Always, &ci, false),
            ColorProfile::TrueColor
        );
    }

    #[test]
    fn never_overrides_force() {
        let e = env(&[("CLICOLOR_FORCE", "1"), ("TERM", "xterm-256color")]);
        assert_eq!(
            resolve_profile(ColorMode::Never, &e, true),
            ColorProfile::Ascii
        );
    }

    #[test]
    fn always_bumps_to_at_least_ansi() {
        let e = env(&[("TERM", "dumb")]);
        assert_eq!(
            resolve_profile(ColorMode::Always, &e, false),
            ColorProfile::Ansi
        );
        // Always ignores real ttyness: TERM decides, even into a pipe.
        let rich = env(&[("TERM", "xterm-256color")]);
        assert_eq!(
            resolve_profile(ColorMode::Always, &rich, false),
            ColorProfile::Ansi256
        );
    }

    #[test]
    fn auto_is_detect() {
        let e = env(&[("TERM", "xterm-256color")]);
        assert_eq!(
            resolve_profile(ColorMode::Auto, &e, true),
            ColorProfile::Ansi256
        );
    }
}
