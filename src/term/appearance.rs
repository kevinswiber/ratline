//! Ask the terminal for its background and turn the answer into an
//! appearance. This is the only place in the binary that writes a query to
//! the terminal outside of `doctor`.
//!
//! The query is emitted only when stderr is a terminal and, on unix, only
//! from the foreground process group. Nothing here may be exercised by an
//! automated test: a live call writes to whatever terminal is running the
//! test, and a reply can be picked up by an unrelated interactive command.

use std::io::IsTerminal;
use std::time::Duration;

use terminal_colorsaurus::{QueryOptions, ThemeMode, theme_mode};

use crate::theme::Appearance;

fn appearance_of(mode: ThemeMode) -> Appearance {
    match mode {
        ThemeMode::Light => Appearance::Light,
        ThemeMode::Dark => Appearance::Dark,
    }
}

/// Both io preconditions, factored out so they can be checked without io.
fn gate_allows(stderr_is_tty: bool, in_foreground: bool) -> bool {
    stderr_is_tty && in_foreground
}

/// A background process group cannot change terminal attributes: the attempt
/// raises SIGTTOU and stops the process. Checking first costs microseconds
/// and writes nothing.
#[cfg(unix)]
fn in_foreground_pgrp() -> bool {
    use std::os::fd::AsRawFd;

    let fd = std::io::stderr().as_raw_fd();
    // SAFETY: both calls only read process/terminal state for `fd`.
    unsafe { libc::tcgetpgrp(fd) == libc::getpgrp() }
}

/// Windows has no process groups and no SIGTTOU, so the hazard the unix
/// check guards against does not exist.
#[cfg(not(unix))]
fn in_foreground_pgrp() -> bool {
    true
}

/// The terminal's background, or `None` when it cannot or will not answer.
/// Any failure is a non-verdict, never a guess.
pub fn probe(timeout: Duration) -> Option<Appearance> {
    if !gate_allows(std::io::stderr().is_terminal(), in_foreground_pgrp()) {
        return None;
    }
    // The struct is non-exhaustive upstream, so it is built from its default
    // and then adjusted.
    let mut options = QueryOptions::default();
    options.timeout = timeout;
    match theme_mode(options) {
        Ok(mode) => Some(appearance_of(mode)),
        Err(err) => {
            if std::env::var_os("RAT_DEBUG_APPEARANCE").is_some() {
                // Diagnostics go to stderr; stdout carries results only.
                eprintln!("rat: appearance query failed: {err}");
            }
            None
        }
    }
}

#[cfg(test)]
mod tests {
    use std::time::Duration;

    use super::*;

    #[test]
    fn the_gate_needs_a_terminal_on_stderr_and_the_foreground() {
        assert!(gate_allows(true, true));
        assert!(!gate_allows(false, true));
        assert!(!gate_allows(true, false));
        assert!(!gate_allows(false, false));
    }

    #[test]
    fn theme_modes_map_onto_appearances() {
        assert_eq!(
            appearance_of(terminal_colorsaurus::ThemeMode::Light),
            Appearance::Light
        );
        assert_eq!(
            appearance_of(terminal_colorsaurus::ThemeMode::Dark),
            Appearance::Dark
        );
    }

    #[test]
    fn probe_has_the_expected_shape() {
        // Coerced, never called: invoking it would write an escape sequence
        // to whatever terminal is running the suite.
        let f: fn(Duration) -> Option<Appearance> = probe;
        let _ = f;
    }
}
