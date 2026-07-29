//! The trigger surface's pure half: spec parsing for `--trigger`.
//!
//! Threads and file descriptors live in `term::tap`; this module owns
//! what can be tested without a terminal — the scheme grammar and its
//! teaching errors.

// The watch CLI surface consumes this module in the commits that follow;
// the allow comes off with that wiring.
#![allow(dead_code)]

use anyhow::{anyhow, bail};

/// One parsed `--trigger` source.
#[derive(Clone, PartialEq, Eq, Debug)]
pub enum TriggerSpec {
    /// A named pipe, opened read + dummy-write so it never sees EOF.
    #[cfg(unix)]
    Fifo(std::path::PathBuf),
    /// A path (file or directory) stat-polled by mtime on the loop's
    /// own slice — the portable source.
    File(std::path::PathBuf),
    /// An inherited descriptor folded into a reader's select set.
    #[cfg(unix)]
    Fd(i32),
}

/// `select(2)`'s hard ceiling: `FD_SET` on a descriptor at or past it
/// writes out of bounds, so the guard lives at parse time.
#[cfg(unix)]
const FD_SETSIZE: i32 = 1024;

/// Parse one `--trigger` spec. Bare paths are rejected — the scheme is
/// the contract, and the error teaches it.
pub fn parse_trigger(s: &str) -> anyhow::Result<TriggerSpec> {
    let teach = || anyhow!("invalid trigger {s:?}: expected fifo:PATH, file:PATH, or fd:N");
    let Some((scheme, rest)) = s.split_once(':') else {
        return Err(teach());
    };
    if rest.is_empty() {
        return Err(teach());
    }
    match scheme {
        "file" => Ok(TriggerSpec::File(std::path::PathBuf::from(rest))),
        #[cfg(unix)]
        "fifo" => Ok(TriggerSpec::Fifo(std::path::PathBuf::from(rest))),
        #[cfg(unix)]
        "fd" => {
            let n: i32 = rest
                .parse()
                .map_err(|_| anyhow!("invalid trigger {s:?}: fd:N takes a number"))?;
            if n < 0 {
                bail!("invalid trigger {s:?}: fd:N takes a non-negative number");
            }
            if n >= FD_SETSIZE {
                bail!(
                    "fd:{n} is out of range for select(2); descriptors must be below {FD_SETSIZE}"
                );
            }
            Ok(TriggerSpec::Fd(n))
        }
        #[cfg(windows)]
        "fifo" | "fd" => {
            bail!("{scheme}: triggers are unix-only; use file:PATH")
        }
        _ => Err(teach()),
    }
}

/// Collapses trigger fires into one spawn request per window. The
/// window is ANCHORED at the first unserved fire — later fires inside
/// it are already covered by the spawn it owes. (A sliding window would
/// starve under sustained sub-window writes: a busy log written faster
/// than the window would never repaint.)
pub struct DebounceGate {
    window: std::time::Duration,
    deadline: Option<std::time::Instant>,
}

impl DebounceGate {
    pub fn new(window: std::time::Duration) -> DebounceGate {
        DebounceGate {
            window,
            deadline: None,
        }
    }

    /// Record a fire. Opens a window only if none is open.
    pub fn fire(&mut self, now: std::time::Instant) {
        if self.deadline.is_none() {
            self.deadline = Some(now + self.window);
        }
    }

    /// True exactly once per window, when it closes; clears the window.
    pub fn due(&mut self, now: std::time::Instant) -> bool {
        if self.deadline.is_some_and(|deadline| now >= deadline) {
            self.deadline = None;
            true
        } else {
            false
        }
    }
}

#[cfg(test)]
mod tests {
    use std::path::PathBuf;
    use std::time::{Duration, Instant};

    use super::*;

    const D: Duration = Duration::from_millis(250);

    #[test]
    fn a_fire_becomes_due_when_the_window_closes() {
        let t = Instant::now();
        let mut g = DebounceGate::new(D);
        assert!(!g.due(t)); // nothing fired yet
        g.fire(t);
        assert!(!g.due(t + Duration::from_millis(100))); // window open
        assert!(g.due(t + D)); // closes
        assert!(!g.due(t + D)); // exactly once
    }

    #[test]
    fn fires_inside_the_window_do_not_move_it() {
        // ANCHORED, not sliding: the spawn the window owes covers them.
        let t = Instant::now();
        let mut g = DebounceGate::new(D);
        g.fire(t);
        g.fire(t + Duration::from_millis(200));
        assert!(g.due(t + D)); // still the FIRST fire's deadline
    }

    #[test]
    fn a_fire_after_the_window_closed_opens_a_new_one() {
        let t = Instant::now();
        let mut g = DebounceGate::new(D);
        g.fire(t);
        assert!(g.due(t + D));
        g.fire(t + D * 2);
        assert!(!g.due(t + D * 2));
        assert!(g.due(t + D * 3));
    }

    #[test]
    fn a_zero_window_is_due_at_the_fire_instant() {
        let t = Instant::now();
        let mut g = DebounceGate::new(Duration::ZERO);
        g.fire(t);
        assert!(g.due(t));
        assert!(!g.due(t));
    }

    #[test]
    fn sustained_sub_window_fires_never_starve_the_spawn() {
        // The reason the window is anchored: a busy log written every
        // 50ms must still repaint once per window.
        let t = Instant::now();
        let mut g = DebounceGate::new(D);
        let mut spawns = 0;
        for i in 0..20 {
            let now = t + Duration::from_millis(50 * i);
            g.fire(now);
            if g.due(now) {
                spawns += 1;
            }
        }
        assert!(spawns >= 3, "starved: {spawns} spawns over 1s at D=250ms");
    }

    #[test]
    fn specs_parse_by_scheme() {
        assert_eq!(
            parse_trigger("file:/tmp/state.json").unwrap(),
            TriggerSpec::File(PathBuf::from("/tmp/state.json"))
        );
        #[cfg(unix)]
        {
            assert_eq!(
                parse_trigger("fifo:/tmp/rat.trigger").unwrap(),
                TriggerSpec::Fifo(PathBuf::from("/tmp/rat.trigger"))
            );
            assert_eq!(parse_trigger("fd:3").unwrap(), TriggerSpec::Fd(3));
        }
    }

    #[test]
    fn a_bare_path_teaches_the_schemes() {
        let err = parse_trigger("/tmp/state.json").unwrap_err().to_string();
        assert!(err.contains("fifo:"), "{err}");
        assert!(err.contains("file:"), "{err}");
        assert!(err.contains("fd:"), "{err}");
    }

    #[test]
    fn an_empty_path_after_a_scheme_is_rejected() {
        assert!(parse_trigger("file:").is_err());
        #[cfg(unix)]
        assert!(parse_trigger("fifo:").is_err());
    }

    #[test]
    fn a_non_numeric_fd_is_rejected() {
        #[cfg(unix)]
        assert!(parse_trigger("fd:three").is_err());
        #[cfg(unix)]
        assert!(parse_trigger("fd:-1").is_err());
    }

    #[cfg(unix)]
    #[test]
    fn an_fd_past_the_select_limit_is_rejected_at_parse() {
        // FD_SET on fd >= FD_SETSIZE (1024) writes out of bounds — the
        // guard lives at parse time so no reader is ever built on it.
        let err = parse_trigger("fd:1024").unwrap_err().to_string();
        assert!(err.contains("select"), "{err}");
        assert!(parse_trigger("fd:1023").is_ok());
    }

    #[cfg(windows)]
    #[test]
    fn unix_only_schemes_teach_file_on_windows() {
        for spec in ["fifo:/tmp/x", "fd:3"] {
            let err = parse_trigger(spec).unwrap_err().to_string();
            assert!(err.contains("file:"), "{err}");
        }
    }
}
