//! The trigger surface's pure half: spec parsing for `--trigger`.
//!
//! Threads and file descriptors live in `term::tap`; this module owns
//! what can be tested without a terminal — the scheme grammar and its
//! teaching errors.

use anyhow::{anyhow, bail};

use crate::core::registry::SourceId;

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

impl std::fmt::Display for TriggerSpec {
    /// The scheme-prefixed form the user wrote — what `?` lists.
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            #[cfg(unix)]
            TriggerSpec::Fifo(path) => write!(f, "fifo:{}", path.display()),
            TriggerSpec::File(path) => write!(f, "file:{}", path.display()),
            #[cfg(unix)]
            TriggerSpec::Fd(n) => write!(f, "fd:{n}"),
        }
    }
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

/// One `file:` source: a path whose modification is detected by stat,
/// once per loop slice. A file's fingerprint is its mtime; a
/// directory's is the max of its own mtime and its immediate entries'
/// (non-recursive) — a directory's own mtime does not move when an
/// existing entry is edited in place. An absent path is a stable state:
/// no fire while absent, one fire on appearance. The first observation
/// only establishes the baseline.
pub struct MtimeWatch {
    path: std::path::PathBuf,
    last: Option<Fingerprint>,
}

/// `None` = the path is absent; `Some(t)` = its newest relevant mtime.
type Fingerprint = Option<std::time::SystemTime>;

impl MtimeWatch {
    pub fn new(path: std::path::PathBuf) -> MtimeWatch {
        MtimeWatch { path, last: None }
    }

    /// Stat the path and report whether it changed since the last call.
    pub fn fired(&mut self) -> bool {
        let current = fingerprint(&self.path);
        let changed = self.last.is_some_and(|last| last != current);
        self.last = Some(current);
        changed
    }
}

fn fingerprint(path: &std::path::Path) -> Fingerprint {
    let meta = std::fs::metadata(path).ok()?;
    let mut newest = meta.modified().ok()?;
    if meta.is_dir() {
        // Depth 1 only, unreadable entries skipped: the rule is cheap
        // and bounded by design — never point it at a source tree.
        for entry in std::fs::read_dir(path).ok()?.flatten() {
            if let Ok(modified) = entry.metadata().and_then(|m| m.modified()) {
                newest = newest.max(modified);
            }
        }
    }
    Some(newest)
}

/// Every `file:` spec of one watch run; fires when any member fires.
pub struct MtimeWatchSet(Vec<MtimeWatch>);

impl MtimeWatchSet {
    pub fn new(paths: Vec<std::path::PathBuf>) -> MtimeWatchSet {
        MtimeWatchSet(paths.into_iter().map(MtimeWatch::new).collect())
    }

    /// Poll every member — each baseline must advance even after one
    /// fires, so there is no short-circuit.
    pub fn fired(&mut self) -> bool {
        let mut any = false;
        for watch in &mut self.0 {
            any |= watch.fired();
        }
        any
    }
}

// ── The loop-observer surface, staged ───────────────────────────────────
//
// Everything from here to `PathLedger` is constructed by the merged loop, not
// by this module, and the loop does not call it YET. A bin crate's dead-code
// check counts a pub item nobody in the binary constructs, so each carries a
// staged allow until the wiring task lands and starts calling it. Removing
// these allows is that task's job and one of its acceptance criteria — if any
// survives the wiring, something the plan said would be called is not.

/// One path's fingerprint as the observer carries it — including across the
/// worker/loop boundary. A newtype over the module-private alias so a
/// trigger's baseline can never be passed where an observer stamp belongs:
/// the observer keeps its OWN baselines, and advancing a trigger's would
/// swallow the fire and silently stop a pane refreshing.
#[derive(Copy, Clone, PartialEq, Eq, Debug)]
#[allow(dead_code)]
pub struct PathStamp(Fingerprint);

/// Stamp a path set. Taken by the loop when a bracket opens and by the
/// worker when it closes, both over the whole watched union.
#[allow(dead_code)]
pub fn stamps(paths: &[std::path::PathBuf]) -> Vec<(std::path::PathBuf, PathStamp)> {
    paths
        .iter()
        .map(|path| (path.clone(), PathStamp(fingerprint(path))))
        .collect()
}

/// A stable handle to one bracket. Monotonic and never reused, so evicting
/// an older bracket cannot shift what a live source is still holding — a
/// positional index would, and closing it would then hit the wrong record.
#[derive(Copy, Clone, PartialEq, Eq, PartialOrd, Ord, Hash, Debug)]
#[allow(dead_code)]
pub struct BracketId(pub u64);

/// One spawn-to-exit interval of one source, with the watched union stamped
/// on both sides. `WindowLog` owns the lifecycle; `PathLedger` only reads a
/// completed one.
#[allow(dead_code)]
pub struct Bracket {
    /// `WindowLog` keys its store by this and hands it back on close;
    /// `PathLedger` reads only the stamps.
    pub id: BracketId,
    pub source: SourceId,
    pub opened: std::time::Instant,
    /// `None` while the child is still running.
    pub closed: Option<std::time::Instant>,
    pub open_stamps: Vec<(std::path::PathBuf, PathStamp)>,
    pub close_stamps: Vec<(std::path::PathBuf, PathStamp)>,
}

/// One observed change to one watched path.
#[allow(dead_code)]
pub struct Change {
    pub at: std::time::Instant,
    /// The brackets this change fell inside, with each one's width. EMPTY
    /// means no child was in flight — which is what makes the change
    /// EXOGENOUS, and one exogenous observation is what clears suspicion.
    pub containing: Vec<(SourceId, std::time::Duration)>,
}

/// The observer's view of the watched union: its own baseline per path, and
/// the changes it has seen inside the window.
///
/// Deliberately separate from every `MtimeWatchSet`. The two stat the same
/// paths and must not share a baseline — see `PathStamp`.
#[allow(dead_code)]
pub struct PathLedger {
    /// Sorted and deduplicated, so the stat order is deterministic.
    paths: Vec<std::path::PathBuf>,
    seen: std::collections::HashMap<std::path::PathBuf, PathStamp>,
    changes: std::collections::HashMap<std::path::PathBuf, Vec<Change>>,
}

#[allow(dead_code)]
impl PathLedger {
    /// Take the baseline immediately: a path that already exists is not a
    /// change, the same rule `MtimeWatch` follows.
    pub fn new(paths: Vec<std::path::PathBuf>) -> PathLedger {
        let mut paths = paths;
        paths.sort();
        paths.dedup();
        let seen = stamps(&paths).into_iter().collect();
        PathLedger {
            paths,
            seen,
            changes: std::collections::HashMap::new(),
        }
    }

    /// Stat the union and record one change per path that moved. `brackets`
    /// is who was in flight over the interval since the last call; empty
    /// means the dashboard was idle.
    pub fn observe(
        &mut self,
        now: std::time::Instant,
        brackets: &[(SourceId, std::time::Duration)],
    ) {
        for (path, stamp) in stamps(&self.paths) {
            if self.moved(&path, stamp) {
                self.record(path, now, brackets.to_vec());
            }
        }
    }

    /// Read a completed bracket: any path differing between its two
    /// snapshots changed while that source's child was running.
    pub fn observe_bracket(&mut self, bracket: &Bracket) {
        let Some(closed) = bracket.closed else {
            return; // still running: there is no width yet, so nothing to say
        };
        let width = closed.saturating_duration_since(bracket.opened);
        let before: std::collections::HashMap<_, _> = bracket
            .open_stamps
            .iter()
            .map(|(path, stamp)| (path.clone(), *stamp))
            .collect();
        for (path, stamp) in &bracket.close_stamps {
            if before.get(path) == Some(stamp) {
                continue;
            }
            if self.moved(path, *stamp) {
                self.record(path.clone(), closed, vec![(bracket.source, width)]);
            }
        }
    }

    /// Drop changes older than the window, so a count means NOW.
    pub fn evict(&mut self, now: std::time::Instant, window: std::time::Duration) {
        let Some(cutoff) = now.checked_sub(window) else {
            return;
        };
        for changes in self.changes.values_mut() {
            changes.retain(|change| change.at >= cutoff);
        }
    }

    /// Changes to this path with no child in flight over them.
    pub fn exogenous(&self, path: &std::path::Path) -> usize {
        self.changes(path)
            .iter()
            .filter(|change| change.containing.is_empty())
            .count()
    }

    /// Every change to this path still inside the window — the credit rule's
    /// per-path input.
    pub fn changes(&self, path: &std::path::Path) -> &[Change] {
        self.changes.get(path).map_or(&[], Vec::as_slice)
    }

    /// Has this path moved since the observer last looked? Advances the
    /// observer's own baseline, never a trigger's.
    fn moved(&mut self, path: &std::path::Path, stamp: PathStamp) -> bool {
        match self.seen.get(path) {
            Some(last) if *last == stamp => false,
            _ => {
                self.seen.insert(path.to_path_buf(), stamp);
                true
            }
        }
    }

    fn record(
        &mut self,
        path: std::path::PathBuf,
        at: std::time::Instant,
        containing: Vec<(SourceId, std::time::Duration)>,
    ) {
        self.changes
            .entry(path)
            .or_default()
            .push(Change { at, containing });
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

    use std::path::Path;
    use std::time::SystemTime;

    /// Push a path's mtime forward deterministically — no sleeps, no
    /// filesystem-granularity dependence.
    fn touch_at(path: &Path, t: SystemTime) {
        std::fs::File::options()
            .append(true)
            .open(path)
            .unwrap()
            .set_modified(t)
            .unwrap();
    }

    #[test]
    fn the_first_observation_is_a_baseline_not_a_fire() {
        let dir = tempfile::tempdir().unwrap();
        let f = dir.path().join("state.json");
        std::fs::write(&f, b"x").unwrap();
        let mut w = MtimeWatch::new(f);
        assert!(!w.fired()); // baseline
        assert!(!w.fired()); // unchanged
    }

    #[test]
    fn an_mtime_change_fires_once() {
        let dir = tempfile::tempdir().unwrap();
        let f = dir.path().join("state.json");
        std::fs::write(&f, b"x").unwrap();
        let mut w = MtimeWatch::new(f.clone());
        w.fired();
        touch_at(&f, SystemTime::now() + Duration::from_secs(5));
        assert!(w.fired());
        assert!(!w.fired());
    }

    #[test]
    fn an_absent_path_is_stable_and_fires_on_appearance() {
        let dir = tempfile::tempdir().unwrap();
        let f = dir.path().join("not-yet");
        let mut w = MtimeWatch::new(f.clone());
        assert!(!w.fired()); // absent baseline
        assert!(!w.fired()); // still absent: stable
        std::fs::write(&f, b"x").unwrap();
        assert!(w.fired()); // appearance is a change
    }

    #[test]
    fn a_directory_fires_on_an_immediate_entrys_edit() {
        // The dir-mtime-only reading under-delivers: editing an existing
        // entry in place does not bump the directory's own mtime.
        let dir = tempfile::tempdir().unwrap();
        let f = dir.path().join("finding.md");
        std::fs::write(&f, b"x").unwrap();
        let mut w = MtimeWatch::new(dir.path().to_path_buf());
        w.fired();
        touch_at(&f, SystemTime::now() + Duration::from_secs(5));
        assert!(w.fired());
    }

    #[test]
    fn a_directory_is_not_recursive() {
        let dir = tempfile::tempdir().unwrap();
        let sub = dir.path().join("sub");
        std::fs::create_dir(&sub).unwrap();
        let deep = sub.join("deep.md");
        std::fs::write(&deep, b"x").unwrap();
        let mut w = MtimeWatch::new(dir.path().to_path_buf());
        w.fired();
        touch_at(&deep, SystemTime::now() + Duration::from_secs(5));
        assert!(!w.fired()); // depth-1 only: a nested edit is invisible
    }

    #[test]
    fn a_set_fires_when_any_member_fires() {
        let dir = tempfile::tempdir().unwrap();
        let a = dir.path().join("a");
        let b = dir.path().join("b");
        std::fs::write(&a, b"x").unwrap();
        std::fs::write(&b, b"x").unwrap();
        let mut set = MtimeWatchSet::new(vec![a, b.clone()]);
        set.fired();
        touch_at(&b, SystemTime::now() + Duration::from_secs(5));
        assert!(set.fired());
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

    // ── PathLedger (plan task 1.1) ──────────────────────────────────────
    //
    // The observer's own view of the watched set. It answers ONE question per
    // observed change: was any child in flight when it happened? A change
    // with no bracket over it is EXOGENOUS, and one exogenous observation is
    // what clears suspicion.

    /// The union as the loop builds it, with a baseline already taken.
    fn ledger_over(paths: &[&Path]) -> PathLedger {
        PathLedger::new(paths.iter().map(PathBuf::from).collect())
    }

    /// A fixed mtime base, so nothing depends on filesystem granularity.
    fn mtime_base() -> SystemTime {
        SystemTime::UNIX_EPOCH + Duration::from_secs(1_000_000)
    }

    #[test]
    fn a_change_with_no_bracket_over_it_is_exogenous() {
        let dir = tempfile::tempdir().unwrap();
        let f = dir.path().join("sa");
        std::fs::write(&f, b"0").unwrap();
        touch_at(&f, mtime_base());

        let mut ledger = ledger_over(&[&f]);
        let t = Instant::now();
        touch_at(&f, mtime_base() + Duration::from_secs(1));
        ledger.observe(t, &[]); // nothing in flight

        assert_eq!(ledger.exogenous(&f), 1);
    }

    #[test]
    fn a_change_inside_a_bracket_is_endogenous() {
        let dir = tempfile::tempdir().unwrap();
        let f = dir.path().join("sa");
        std::fs::write(&f, b"0").unwrap();
        touch_at(&f, mtime_base());

        let mut ledger = ledger_over(&[&f]);
        let t = Instant::now();
        touch_at(&f, mtime_base() + Duration::from_secs(1));
        ledger.observe(t, &[(SourceId(0), Duration::from_millis(7))]);

        assert_eq!(ledger.exogenous(&f), 0);
        assert_eq!(ledger.changes(&f).len(), 1);
    }

    #[test]
    fn the_first_stat_only_establishes_a_baseline() {
        // The rule MtimeWatch already follows: a path that exists at
        // construction is not a change.
        let dir = tempfile::tempdir().unwrap();
        let f = dir.path().join("sa");
        std::fs::write(&f, b"0").unwrap();

        let mut ledger = ledger_over(&[&f]);
        ledger.observe(Instant::now(), &[]);

        assert_eq!(ledger.exogenous(&f), 0);
    }

    #[test]
    fn an_absent_path_is_stable_and_its_appearance_is_one_change() {
        // Three of the four paths on a real dogfooding dashboard did not
        // exist, so this is the common case rather than an edge case.
        let dir = tempfile::tempdir().unwrap();
        let f = dir.path().join("not-yet");

        let mut ledger = ledger_over(&[&f]);
        let t = Instant::now();
        ledger.observe(t, &[]); // still absent: stable
        assert_eq!(ledger.exogenous(&f), 0);

        std::fs::write(&f, b"here").unwrap();
        ledger.observe(t + Duration::from_millis(50), &[]);
        assert_eq!(ledger.exogenous(&f), 1);
    }

    #[test]
    fn observe_bracket_credits_the_source_whose_bracket_it_was() {
        // A bracket carries its OWN two snapshots, the close one taken on the
        // worker, so a change during the child is endogenous even though the
        // loop only hears about it a slice later.
        let dir = tempfile::tempdir().unwrap();
        let f = dir.path().join("sa");
        std::fs::write(&f, b"0").unwrap();
        touch_at(&f, mtime_base());

        let mut ledger = ledger_over(&[&f]);
        let t = Instant::now();
        let open = stamps(std::slice::from_ref(&f));
        touch_at(&f, mtime_base() + Duration::from_secs(1));
        let close = stamps(std::slice::from_ref(&f));

        ledger.observe_bracket(&Bracket {
            id: BracketId(0),
            source: SourceId(3),
            opened: t,
            closed: Some(t + Duration::from_millis(9)),
            open_stamps: open,
            close_stamps: close,
        });

        assert_eq!(ledger.exogenous(&f), 0);
        let c = ledger.changes(&f);
        assert_eq!(c.len(), 1);
        assert_eq!(
            c[0].containing,
            vec![(SourceId(3), Duration::from_millis(9))]
        );
    }

    #[test]
    fn a_bracket_that_moved_nothing_records_no_change() {
        let dir = tempfile::tempdir().unwrap();
        let f = dir.path().join("sa");
        std::fs::write(&f, b"0").unwrap();

        let mut ledger = ledger_over(&[&f]);
        let t = Instant::now();
        let snap = stamps(std::slice::from_ref(&f));
        ledger.observe_bracket(&Bracket {
            id: BracketId(0),
            source: SourceId(0),
            opened: t,
            closed: Some(t + Duration::from_millis(5)),
            open_stamps: snap.clone(),
            close_stamps: snap,
        });
        assert!(ledger.changes(&f).is_empty());
    }

    #[test]
    fn a_bracket_advances_the_baseline_so_one_change_is_not_counted_twice() {
        let dir = tempfile::tempdir().unwrap();
        let f = dir.path().join("sa");
        std::fs::write(&f, b"0").unwrap();
        touch_at(&f, mtime_base());

        let mut ledger = ledger_over(&[&f]);
        let t = Instant::now();
        let open = stamps(std::slice::from_ref(&f));
        touch_at(&f, mtime_base() + Duration::from_secs(1));
        let close = stamps(std::slice::from_ref(&f));
        ledger.observe_bracket(&Bracket {
            id: BracketId(0),
            source: SourceId(0),
            opened: t,
            closed: Some(t + Duration::from_millis(5)),
            open_stamps: open,
            close_stamps: close,
        });
        assert_eq!(ledger.changes(&f).len(), 1);

        ledger.observe(t + Duration::from_millis(60), &[]);
        assert_eq!(
            ledger.changes(&f).len(),
            1,
            "the same change must not be counted twice"
        );
    }

    #[test]
    fn eviction_drops_changes_older_than_the_window() {
        let dir = tempfile::tempdir().unwrap();
        let f = dir.path().join("sa");
        std::fs::write(&f, b"0").unwrap();
        touch_at(&f, mtime_base());

        let mut ledger = ledger_over(&[&f]);
        let t = Instant::now();
        touch_at(&f, mtime_base() + Duration::from_secs(1));
        ledger.observe(t, &[]);
        assert_eq!(ledger.exogenous(&f), 1);

        ledger.evict(t + Duration::from_secs(31), Duration::from_secs(30));
        assert_eq!(ledger.exogenous(&f), 0, "the window must mean NOW");
    }

    #[test]
    fn the_ledger_never_swallows_a_trigger_set_fire() {
        // I-60, asserted structurally: an MtimeWatchSet over the same path
        // still fires after the ledger has stat'd it repeatedly. If the two
        // shared baseline state, the fire would be swallowed and the pane
        // would silently stop refreshing — which is the suppression design
        // this work rejects, arrived at by accident.
        let dir = tempfile::tempdir().unwrap();
        let f = dir.path().join("sa");
        std::fs::write(&f, b"0").unwrap();
        touch_at(&f, mtime_base());

        let mut set = MtimeWatchSet::new(vec![f.clone()]);
        assert!(!set.fired(), "baseline");
        let mut ledger = ledger_over(&[&f]);

        let t = Instant::now();
        touch_at(&f, mtime_base() + Duration::from_secs(1));
        ledger.observe(t, &[]);
        ledger.observe(t + Duration::from_millis(50), &[]);

        assert!(set.fired(), "the trigger must still see its own change");
    }
}
