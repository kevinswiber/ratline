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

/// One path's fingerprint as the observer carries it — including across the
/// worker/loop boundary. A newtype over the module-private alias so a
/// trigger's baseline can never be passed where an observer stamp belongs:
/// the observer keeps its OWN baselines, and advancing a trigger's would
/// swallow the fire and silently stop a pane refreshing.
#[derive(Copy, Clone, PartialEq, Eq, Debug)]
pub struct PathStamp(Fingerprint);

/// Stamp a path set. Taken by the loop when a bracket opens and by the
/// worker when it closes, both over the whole watched union.
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
pub struct BracketId(pub u64);

/// One spawn-to-exit interval of one source, with the watched union stamped
/// on both sides. `WindowLog` owns the lifecycle; `PathLedger` only reads a
/// completed one.
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
pub struct PathLedger {
    /// Sorted and deduplicated, so the stat order is deterministic.
    paths: Vec<std::path::PathBuf>,
    seen: std::collections::HashMap<std::path::PathBuf, PathStamp>,
    changes: std::collections::HashMap<std::path::PathBuf, Vec<Change>>,
}

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

    /// Test-only: place a change directly, so the suspicion tests can build
    /// a window without touching a filesystem.
    #[cfg(test)]
    pub fn inject(
        &mut self,
        path: &std::path::Path,
        at: std::time::Instant,
        containing: Vec<(SourceId, std::time::Duration)>,
    ) {
        self.record(path.to_path_buf(), at, containing);
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

/// Which trigger an observation came from. A `file:` source is keyed by its
/// path; a reader route by its spec's canonical string (`fifo:/tmp/x`,
/// `fd:3`) — the same text `TriggerSpec`'s `Display` produces and the notice
/// prints. Keying per trigger rather than per source is what lets two fifos
/// on ONE pane be credited separately instead of merged.
#[derive(Clone, PartialEq, Eq, Hash, Debug)]
pub struct TriggerKey(pub String);

/// One reader arrival. Its covering brackets are identified when the bytes
/// arrive; their WIDTHS are resolved later.
///
/// Storing widths here would be wrong: an arrival is commonly recorded while
/// its covering child is still running, when the final width does not exist
/// and elapsed-so-far would make a long-running child look artificially
/// tight — the mis-credit the median-width rule exists to prevent.
pub struct Arrival {
    /// Staged: recorded here, read when the reader route resolves arrivals.
    #[allow(dead_code)]
    pub source: SourceId,
    pub trigger: TriggerKey,
    pub at: std::time::Instant,
    /// The brackets covering `at`, by id. EMPTY means nothing was in flight,
    /// which is knowable immediately and is all the veto needs.
    pub containing: Vec<BracketId>,
}

/// Every windowed quantity the suspicion test reads, in one place, all of it
/// evicting. Nothing here is cumulative: a count that cannot fall could never
/// let a repaired dashboard stop being suspected.
pub struct WindowLog {
    window: std::time::Duration,
    next_id: u64,
    /// Keyed by id, never by position — see `BracketId`.
    brackets: Vec<Bracket>,
    respawns: Vec<(SourceId, std::time::Instant)>,
    arrivals: Vec<Arrival>,
    overflows: Vec<std::time::Instant>,
}

impl WindowLog {
    pub fn new(window: std::time::Duration) -> WindowLog {
        WindowLog {
            window,
            next_id: 0,
            brackets: Vec::new(),
            respawns: Vec::new(),
            arrivals: Vec::new(),
            overflows: Vec::new(),
        }
    }

    pub fn open_bracket(
        &mut self,
        source: SourceId,
        at: std::time::Instant,
        open_stamps: Vec<(std::path::PathBuf, PathStamp)>,
    ) -> BracketId {
        let id = BracketId(self.next_id);
        self.next_id += 1;
        self.brackets.push(Bracket {
            id,
            source,
            opened: at,
            closed: None,
            open_stamps,
            close_stamps: Vec::new(),
        });
        id
    }

    /// Close a bracket and hand back the completed record, which the loop
    /// feeds to `PathLedger::observe_bracket`. `None` when the id has already
    /// been evicted — a long child whose bracket aged out is ordinary, not an
    /// error.
    pub fn close_bracket(
        &mut self,
        id: BracketId,
        at: std::time::Instant,
        close_stamps: Vec<(std::path::PathBuf, PathStamp)>,
    ) -> Option<&Bracket> {
        let bracket = self.brackets.iter_mut().find(|b| b.id == id)?;
        bracket.closed = Some(at);
        bracket.close_stamps = close_stamps;
        Some(bracket)
    }

    /// A trigger-driven respawn EVENT. Counted per window on demand, so the
    /// count falls again as evidence expires.
    pub fn record_respawn(&mut self, source: SourceId, at: std::time::Instant) {
        self.respawns.push((source, at));
    }

    /// A fifo/fd arrival, resolved against the brackets already owned here.
    /// Staged: the reader route is its only caller.
    #[allow(dead_code)]
    pub fn observe_arrival(
        &mut self,
        source: SourceId,
        trigger: TriggerKey,
        at: std::time::Instant,
    ) {
        let containing = self
            .brackets
            .iter()
            .filter(|b| b.spans(at))
            .map(|b| b.id)
            .collect();
        self.arrivals.push(Arrival {
            source,
            trigger,
            at,
            containing,
        });
    }

    /// A reader's queue overflowed, so arrivals were LOST. That cannot be
    /// treated as "no arrivals": the dropped one may have been the window's
    /// only exogenous observation, and losing it would turn a zero test into
    /// an accusation. Any window this touches abstains instead.
    /// Staged: a reader is the only thing that can overflow.
    #[allow(dead_code)]
    pub fn record_overflow(&mut self, at: std::time::Instant) {
        self.overflows.push(at);
    }
    pub fn respawns_in_window(&self, source: SourceId, now: std::time::Instant) -> usize {
        let cutoff = self.cutoff(now);
        self.respawns
            .iter()
            .filter(|(id, at)| *id == source && cutoff.is_none_or(|c| *at >= c))
            .count()
    }

    /// Fraction of the window during which ANY child was in flight, with
    /// overlapping brackets UNIONED rather than summed. Summing would report
    /// roughly double for the measured repro, whose panes overlap almost
    /// entirely, and could push a cheap loop over the abstention ceiling —
    /// turning a detectable loop into a silence.
    pub fn busy_fraction(&self, now: std::time::Instant) -> f64 {
        let Some(start) = self.cutoff(now) else {
            return 0.0;
        };
        let mut spans: Vec<(std::time::Instant, std::time::Instant)> = self
            .brackets
            .iter()
            .map(|b| (b.opened.max(start), b.end_or(now).min(now)))
            .filter(|(from, to)| to > from)
            .collect();
        spans.sort_by_key(|(from, _)| *from);
        let mut busy = std::time::Duration::ZERO;
        let mut merged: Option<(std::time::Instant, std::time::Instant)> = None;
        for (from, to) in spans {
            match merged {
                Some((m_from, m_to)) if from <= m_to => merged = Some((m_from, m_to.max(to))),
                Some((m_from, m_to)) => {
                    busy += m_to.duration_since(m_from);
                    merged = Some((from, to));
                }
                None => merged = Some((from, to)),
            }
        }
        if let Some((m_from, m_to)) = merged {
            busy += m_to.duration_since(m_from);
        }
        busy.as_secs_f64() / self.window.as_secs_f64()
    }

    /// CLOSED brackets containing `at`, with their final widths.
    ///
    /// Open brackets are deliberately absent: their width is not final, and
    /// reporting elapsed-so-far is the mis-credit this design exists to
    /// avoid. Ask `any_open` first — a caller that wants to classify a change
    /// as exogenous must not do so while a child is still running, or an
    /// unattributed change would wrongly clear the veto.
    /// Staged: the reader route resolves an arrival's brackets through it.
    /// The slice-cadence observer does NOT — it runs only while nothing is in
    /// flight, so what it hands the ledger is always empty by construction.
    #[allow(dead_code)]
    pub fn covering(&self, at: std::time::Instant) -> Vec<(SourceId, std::time::Duration)> {
        self.brackets
            .iter()
            .filter(|b| b.closed.is_some() && b.spans(at))
            .map(|b| (b.source, b.width().unwrap_or_default()))
            .collect()
    }

    /// Is any bracket still open over `at`?
    pub fn any_open(&self, at: std::time::Instant) -> bool {
        self.brackets
            .iter()
            .any(|b| b.closed.is_none() && b.opened <= at)
    }

    /// Arrivals for one trigger still inside the window.
    pub fn arrivals(&self, trigger: &TriggerKey) -> Vec<&Arrival> {
        self.arrivals
            .iter()
            .filter(|a| a.trigger == *trigger)
            .collect()
    }

    /// Resolve an arrival's covering brackets to final widths. `None` while
    /// any of them is still open: such an arrival is deferred from the credit
    /// stage — it resolves once the bracket closes — while still counting for
    /// the veto, which needs only whether `containing` was empty.
    pub fn widths(&self, arrival: &Arrival) -> Option<Vec<(SourceId, std::time::Duration)>> {
        arrival
            .containing
            .iter()
            .map(|id| {
                let bracket = self.brackets.iter().find(|b| b.id == *id)?;
                Some((bracket.source, bracket.width()?))
            })
            .collect()
    }

    /// True when a reader lost arrivals inside the current window.
    pub fn evidence_lost(&self, now: std::time::Instant) -> bool {
        let cutoff = self.cutoff(now);
        self.overflows
            .iter()
            .any(|at| cutoff.is_none_or(|c| *at >= c))
    }

    /// Drop what the window no longer covers. A bracket a live arrival still
    /// references is RETAINED even when it would otherwise age out, or that
    /// arrival's widths would become unresolvable and the credit rule would
    /// silently lose its input. Keying by id rather than position is what
    /// makes retaining a subset safe.
    pub fn evict(&mut self, now: std::time::Instant) {
        let Some(cutoff) = self.cutoff(now) else {
            return;
        };
        self.respawns.retain(|(_, at)| *at >= cutoff);
        self.overflows.retain(|at| *at >= cutoff);
        self.arrivals.retain(|a| a.at >= cutoff);
        let referenced: std::collections::HashSet<BracketId> = self
            .arrivals
            .iter()
            .flat_map(|a| a.containing.iter().copied())
            .collect();
        self.brackets
            .retain(|b| b.end_or(now) >= cutoff || referenced.contains(&b.id));
    }

    fn cutoff(&self, now: std::time::Instant) -> Option<std::time::Instant> {
        now.checked_sub(self.window)
    }
}

impl Bracket {
    /// Final width, or `None` while the child is still running.
    pub fn width(&self) -> Option<std::time::Duration> {
        Some(self.closed?.saturating_duration_since(self.opened))
    }

    /// When this bracket ends, treating a live child as ending now.
    fn end_or(&self, now: std::time::Instant) -> std::time::Instant {
        self.closed.unwrap_or(now)
    }

    /// Does this bracket cover `at`? An open bracket covers everything from
    /// its start onward.
    /// Staged with `covering`, its only caller.
    #[allow(dead_code)]
    fn spans(&self, at: std::time::Instant) -> bool {
        self.opened <= at && self.closed.is_none_or(|closed| closed >= at)
    }
}

// ── The suspicion test ─────────────────────────────────────────────────
//
// The loop feeds the ledger and the log, evaluates the test once per
// iteration, and paints the verdict. What remains staged below is the READER
// route only: a fifo/fd arrival has no caller until the reader tasks land,
// and the notice reads the ordering. Each allow names what reaches it; one
// surviving that means something the plan said would be called is not.

/// The suspicion test's thresholds. The defaults are research starting
/// points, not results, and none of them is user-facing: under report-only a
/// false positive is cosmetic, so a switch can be added later without
/// breaking anyone, while removing one could not.
pub struct LoopSuspicion {
    pub window: std::time::Duration,
    pub min_respawns: usize,
    pub abstain_at_or_above: f64,
}

impl Default for LoopSuspicion {
    fn default() -> LoopSuspicion {
        LoopSuspicion {
            window: std::time::Duration::from_secs(30),
            min_respawns: 50,
            abstain_at_or_above: 0.5,
        }
    }
}

/// One pane's windowed facts, assembled by the loop. Every count here is
/// already restricted to the window by `WindowLog`.
pub struct PaneWindow<'a> {
    pub source: SourceId,
    pub trigger_respawns: usize,
    /// Its `file:` paths, read through the ledger.
    pub watched: &'a [std::path::PathBuf],
    /// Its fifo/fd triggers, read through the log's arrivals. Separate
    /// because a reader route has no path to stat.
    pub readers: &'a [TriggerKey],
}
pub struct Verdict {
    /// The implicated panes; empty when nothing is suspected. A SET, because
    /// concurrent children make direction unavailable even as coincidence.
    pub panes: Vec<SourceId>,
    /// Present only where attribution was precise enough to order them.
    /// Staged: the badge is per-pane, so only the notice can name an order.
    #[allow(dead_code)]
    pub ordered: Option<Vec<SourceId>>,
    /// The test declined to answer. Distinct from an empty `panes`:
    /// abstaining is not the same as finding nothing. Staged: an abstention
    /// implicates nobody, so the badge already follows it through `panes`;
    /// the notice is where the difference could be said out loud.
    #[allow(dead_code)]
    pub abstained: bool,
}
impl LoopSuspicion {
    /// Which panes are implicated over the window ending at `now`.
    ///
    /// All four conditions must hold. The first three are not sufficient on
    /// their own: a legitimate one-way producer→consumer pair satisfies every
    /// one of them, which is why the graph test exists.
    pub fn evaluate(
        &self,
        now: std::time::Instant,
        ledger: &PathLedger,
        log: &WindowLog,
        panes: &[PaneWindow<'_>],
    ) -> Verdict {
        // Condition 3 first: it is the cheapest, and it short-circuits. Lost
        // reader evidence lands here too — missing evidence is not absent
        // evidence, and the veto below is a zero test that cannot survive a
        // silently dropped observation.
        if log.evidence_lost(now) || log.busy_fraction(now) >= self.abstain_at_or_above {
            return Verdict {
                panes: Vec::new(),
                ordered: None,
                abstained: true,
            };
        }

        let candidates: Vec<&PaneWindow<'_>> = panes
            .iter()
            .filter(|pane| pane.trigger_respawns >= self.min_respawns)
            .filter(|pane| self.closed_everywhere(ledger, log, pane))
            .collect();

        // Condition 4: an edge runs from whoever is credited with writing a
        // path to whoever watches it. A pane on a cycle in that graph is
        // implicated; a pane on a path through it is not.
        let mut edges: Vec<(SourceId, SourceId)> = Vec::new();
        let mut merged: Vec<Vec<SourceId>> = Vec::new();
        for pane in &candidates {
            for path in pane.watched {
                let credited = credit(ledger.changes(path));
                Self::add(&mut edges, &mut merged, &credited, pane.source);
            }
            for key in pane.readers {
                let changes = Self::arrival_changes(log, key);
                let credited = credit(&changes);
                Self::add(&mut edges, &mut merged, &credited, pane.source);
            }
        }

        let implicated = on_a_cycle(&candidates, &edges, &merged);
        let precise = merged.iter().all(|group| group.len() <= 1);
        Verdict {
            ordered: (precise && !implicated.is_empty()).then(|| implicated.clone()),
            panes: implicated,
            abstained: false,
        }
    }

    /// Condition 2: every trigger this pane watches recorded ZERO exogenous
    /// observations. One is enough to say the path has an outside writer.
    fn closed_everywhere(
        &self,
        ledger: &PathLedger,
        log: &WindowLog,
        pane: &PaneWindow<'_>,
    ) -> bool {
        let files_closed = pane.watched.iter().all(|p| ledger.exogenous(p) == 0);
        let readers_closed = pane.readers.iter().all(|key| {
            log.arrivals(key)
                .iter()
                .all(|arrival| !arrival.containing.is_empty())
        });
        // A pane watching nothing cannot be closed: there is no evidence
        // either way, and silence is not a positive.
        let watches_something = !pane.watched.is_empty() || !pane.readers.is_empty();
        watches_something && files_closed && readers_closed
    }

    /// A reader route's arrivals, in the shape the credit rule takes. An
    /// arrival whose covering bracket is still open is DEFERRED — it resolves
    /// once that bracket closes — rather than credited with a provisional
    /// width.
    fn arrival_changes(log: &WindowLog, key: &TriggerKey) -> Vec<Change> {
        log.arrivals(key)
            .iter()
            .filter_map(|arrival| {
                Some(Change {
                    at: arrival.at,
                    containing: log.widths(arrival)?,
                })
            })
            .collect()
    }

    fn add(
        edges: &mut Vec<(SourceId, SourceId)>,
        merged: &mut Vec<Vec<SourceId>>,
        credited: &[SourceId],
        watcher: SourceId,
    ) {
        for writer in credited {
            edges.push((*writer, watcher));
        }
        if credited.len() > 1 {
            merged.push(credited.to_vec());
        }
    }
}

/// Who wrote this path? Two stages, and both are load-bearing.
///
/// **Eligibility** keeps only the panes that were in flight for MORE than
/// half of the path's changes. Without it, a pane whose bracket merely
/// happens to overlap gets credited, and a legitimate chain looks like a
/// cycle.
///
/// **Tightness** then keeps only those whose median containing bracket is
/// within 2x of the tightest. That separates by an order of magnitude in
/// practice — a cycle's panes run children of near-identical cost because
/// they are the same work phase-locked by the same debounce, while a chain's
/// producer is arbitrarily more expensive than its consumer. The band is 2x
/// because the measured gap it sits in is far wider than that, not because it
/// was tuned.
fn credit(changes: &[Change]) -> Vec<SourceId> {
    if changes.is_empty() {
        return Vec::new();
    }
    let mut sources: Vec<SourceId> = changes
        .iter()
        .flat_map(|change| change.containing.iter().map(|(id, _)| *id))
        .collect();
    sources.sort();
    sources.dedup();

    // Stage one.
    let eligible: Vec<SourceId> = sources
        .into_iter()
        .filter(|id| {
            let covered = changes
                .iter()
                .filter(|c| c.containing.iter().any(|(s, _)| s == id))
                .count();
            covered * 2 > changes.len()
        })
        .collect();
    if eligible.is_empty() {
        return Vec::new();
    }

    // Stage two.
    let medians: Vec<(SourceId, std::time::Duration)> = eligible
        .into_iter()
        .map(|id| (id, median_width(changes, id)))
        .collect();
    let tightest = medians
        .iter()
        .map(|(_, width)| *width)
        .min()
        .unwrap_or_default();
    medians
        .into_iter()
        .filter(|(_, width)| *width <= tightest * 2)
        .map(|(id, _)| id)
        .collect()
}
fn median_width(changes: &[Change], id: SourceId) -> std::time::Duration {
    let mut widths: Vec<std::time::Duration> = changes
        .iter()
        .filter_map(|c| c.containing.iter().find(|(s, _)| *s == id).map(|(_, w)| *w))
        .collect();
    widths.sort();
    widths.get(widths.len() / 2).copied().unwrap_or_default()
}

/// Panes lying on a cycle of the observed graph. A merged group is one node,
/// so its self-edge is a cycle: when two panes' children overlap so much that
/// either could have written either path, collapsing them loses nothing —
/// the collapse IS the loop.
fn on_a_cycle(
    candidates: &[&PaneWindow<'_>],
    edges: &[(SourceId, SourceId)],
    merged: &[Vec<SourceId>],
) -> Vec<SourceId> {
    let node = |id: SourceId| -> SourceId {
        merged
            .iter()
            .find(|group| group.contains(&id))
            .and_then(|group| group.iter().min().copied())
            .unwrap_or(id)
    };
    let mut implicated: Vec<SourceId> = Vec::new();
    for pane in candidates {
        let start = node(pane.source);
        // Can this node reach itself?
        let mut seen: Vec<SourceId> = Vec::new();
        let mut stack: Vec<SourceId> = edges
            .iter()
            .filter(|(from, _)| node(*from) == start)
            .map(|(_, to)| node(*to))
            .collect();
        let mut cyclic = false;
        while let Some(current) = stack.pop() {
            if current == start {
                cyclic = true;
                break;
            }
            if seen.contains(&current) {
                continue;
            }
            seen.push(current);
            stack.extend(
                edges
                    .iter()
                    .filter(|(from, _)| node(*from) == current)
                    .map(|(_, to)| node(*to)),
            );
        }
        if cyclic {
            implicated.push(pane.source);
        }
    }
    implicated.sort();
    implicated
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

    // ── WindowLog (plan task 1.3) ───────────────────────────────────────
    //
    // The one windowed store. Everything in it expires, because a count that
    // cannot fall could never let a repaired dashboard stop being suspected.

    const W: Duration = Duration::from_secs(30);

    fn secs(n: u64) -> Duration {
        Duration::from_secs(n)
    }

    #[test]
    fn a_windowed_respawn_count_falls_as_evidence_expires() {
        // The property the whole design turns on. A cumulative counter would
        // pass the first assertion and fail the second, and self-clearing
        // would be impossible.
        let mut log = WindowLog::new(W);
        let t0 = Instant::now();
        for i in 0..50 {
            log.record_respawn(SourceId(0), t0 + Duration::from_millis(i * 10));
        }
        assert_eq!(log.respawns_in_window(SourceId(0), t0 + secs(1)), 50);
        assert_eq!(log.respawns_in_window(SourceId(0), t0 + secs(40)), 0);
    }

    #[test]
    fn respawns_are_counted_per_source() {
        let mut log = WindowLog::new(W);
        let t0 = Instant::now();
        log.record_respawn(SourceId(0), t0);
        log.record_respawn(SourceId(1), t0);
        log.record_respawn(SourceId(1), t0);
        assert_eq!(log.respawns_in_window(SourceId(0), t0 + secs(1)), 1);
        assert_eq!(log.respawns_in_window(SourceId(1), t0 + secs(1)), 2);
    }

    #[test]
    fn busy_fraction_unions_overlapping_brackets_rather_than_summing() {
        // The measured repro's panes overlap almost entirely. Summing would
        // report double and could push a cheap loop over the abstention
        // ceiling, turning a detectable loop into a silence.
        let mut log = WindowLog::new(secs(10));
        let t0 = Instant::now();
        let a = log.open_bracket(SourceId(0), t0, Vec::new());
        let b = log.open_bracket(SourceId(1), t0, Vec::new());
        log.close_bracket(a, t0 + secs(1), Vec::new());
        log.close_bracket(b, t0 + secs(1), Vec::new());

        let f = log.busy_fraction(t0 + secs(10));
        assert!(
            (f - 0.1).abs() < 0.01,
            "two overlapping 1s brackets in 10s is 10%, not 20% — got {f}"
        );
    }

    #[test]
    fn busy_fraction_sums_disjoint_brackets() {
        let mut log = WindowLog::new(secs(10));
        let t0 = Instant::now();
        let a = log.open_bracket(SourceId(0), t0, Vec::new());
        log.close_bracket(a, t0 + secs(1), Vec::new());
        let b = log.open_bracket(SourceId(1), t0 + secs(5), Vec::new());
        log.close_bracket(b, t0 + secs(6), Vec::new());

        let f = log.busy_fraction(t0 + secs(10));
        assert!(
            (f - 0.2).abs() < 0.01,
            "two disjoint 1s brackets in 10s is 20% — got {f}"
        );
    }

    #[test]
    fn an_open_bracket_counts_as_busy_up_to_now() {
        // Treating a live child as zero-width would make a long-running pane
        // look idle, which is exactly backwards.
        let mut log = WindowLog::new(secs(10));
        let t0 = Instant::now();
        log.open_bracket(SourceId(0), t0 + secs(9), Vec::new());
        let f = log.busy_fraction(t0 + secs(10));
        assert!(
            (f - 0.1).abs() < 0.01,
            "a still-open 1s bracket is 10% — got {f}"
        );
    }

    #[test]
    fn close_bracket_returns_the_completed_record() {
        // The loop hands it straight to PathLedger::observe_bracket, so a
        // unit return would leave that seam unreachable through this API.
        let mut log = WindowLog::new(W);
        let t0 = Instant::now();
        let id = log.open_bracket(SourceId(2), t0, Vec::new());
        let closed = log.close_bracket(id, t0 + Duration::from_millis(7), Vec::new());
        let closed = closed.expect("a live id must close");
        assert_eq!(closed.source, SourceId(2));
        assert_eq!(closed.width(), Some(Duration::from_millis(7)));
    }

    #[test]
    fn an_evicted_bracket_never_shifts_a_live_bracket_id() {
        // The correctness test behind BracketId. With positional indices,
        // evicting the older bracket would shift the index the second source
        // still holds, and this close would land on the wrong record.
        let mut log = WindowLog::new(secs(10));
        let t0 = Instant::now();
        let old = log.open_bracket(SourceId(0), t0, Vec::new());
        log.close_bracket(old, t0 + secs(1), Vec::new());
        let live = log.open_bracket(SourceId(7), t0 + secs(9), Vec::new());

        log.evict(t0 + secs(20)); // the old bracket is now outside the window

        let closed = log
            .close_bracket(live, t0 + secs(20), Vec::new())
            .expect("the live bracket must still be closeable");
        assert_eq!(closed.source, SourceId(7), "closed the wrong record");
    }

    #[test]
    fn closing_an_evicted_id_is_a_no_op_returning_none() {
        let mut log = WindowLog::new(secs(10));
        let t0 = Instant::now();
        let id = log.open_bracket(SourceId(0), t0, Vec::new());
        log.close_bracket(id, t0 + secs(1), Vec::new());
        log.evict(t0 + secs(30));
        assert!(log.close_bracket(id, t0 + secs(30), Vec::new()).is_none());
    }

    #[test]
    fn covering_reports_closed_brackets_with_final_widths() {
        let mut log = WindowLog::new(W);
        let t0 = Instant::now();
        let id = log.open_bracket(SourceId(4), t0, Vec::new());
        log.close_bracket(id, t0 + Duration::from_millis(20), Vec::new());

        let over = log.covering(t0 + Duration::from_millis(10));
        assert_eq!(over, vec![(SourceId(4), Duration::from_millis(20))]);
        assert!(log.covering(t0 + secs(5)).is_empty(), "outside the bracket");
    }

    #[test]
    fn covering_withholds_an_open_bracket_and_any_open_reports_it() {
        // Reporting elapsed-so-far as a width is the mis-credit this design
        // exists to avoid, so `covering` withholds an open bracket entirely
        // and a caller asks `any_open` before treating a change as exogenous.
        let mut log = WindowLog::new(W);
        let t0 = Instant::now();
        log.open_bracket(SourceId(0), t0, Vec::new());
        assert!(log.covering(t0 + Duration::from_millis(5)).is_empty());
        assert!(log.any_open(t0 + Duration::from_millis(5)));
    }

    #[test]
    fn an_arrival_with_nothing_in_flight_is_exogenous_immediately() {
        // Emptiness needs no width, so the veto never waits on one.
        let mut log = WindowLog::new(W);
        let t0 = Instant::now();
        log.observe_arrival(SourceId(0), TriggerKey("fifo:/tmp/a".into()), t0);
        let key = TriggerKey("fifo:/tmp/a".into());
        let arrivals = log.arrivals(&key);
        assert_eq!(arrivals.len(), 1);
        assert!(arrivals[0].containing.is_empty());
    }

    #[test]
    fn an_arrival_captures_bracket_identity_at_record_time_and_widths_later() {
        // Identity must be captured immediately, or eviction could remove the
        // brackets that gave the arrival meaning. Width must NOT be: an
        // arrival recorded during an open child would freeze elapsed-so-far
        // and credit a long child as artificially tight.
        let mut log = WindowLog::new(W);
        let t0 = Instant::now();
        let id = log.open_bracket(SourceId(1), t0, Vec::new());
        log.observe_arrival(
            SourceId(1),
            TriggerKey("fifo:/tmp/a".into()),
            t0 + Duration::from_millis(3),
        );

        let key = TriggerKey("fifo:/tmp/a".into());
        {
            let arrivals = log.arrivals(&key);
            assert_eq!(arrivals[0].containing, vec![id], "identity is captured now");
            assert!(log.widths(arrivals[0]).is_none(), "width is not final yet");
        }

        log.close_bracket(id, t0 + Duration::from_millis(40), Vec::new());
        let arrivals = log.arrivals(&key);
        assert_eq!(
            log.widths(arrivals[0]),
            Some(vec![(SourceId(1), Duration::from_millis(40))]),
            "the FINAL width, not elapsed-so-far"
        );
    }

    #[test]
    fn two_triggers_on_one_pane_are_kept_separate_not_merged() {
        // Without per-trigger identity the credit rule has nothing to apply
        // itself to on the reader route.
        let mut log = WindowLog::new(W);
        let t0 = Instant::now();
        log.observe_arrival(SourceId(0), TriggerKey("fifo:/tmp/a".into()), t0);
        log.observe_arrival(SourceId(0), TriggerKey("fifo:/tmp/b".into()), t0);
        log.observe_arrival(SourceId(0), TriggerKey("fifo:/tmp/b".into()), t0);

        assert_eq!(log.arrivals(&TriggerKey("fifo:/tmp/a".into())).len(), 1);
        assert_eq!(log.arrivals(&TriggerKey("fifo:/tmp/b".into())).len(), 2);
    }

    #[test]
    fn an_overflow_forces_abstention_for_any_window_it_touches() {
        // Missing evidence is not absent evidence: the dropped arrival may
        // have been the window's only exogenous observation.
        let mut log = WindowLog::new(W);
        let t0 = Instant::now();
        log.record_overflow(t0);
        assert!(log.evidence_lost(t0 + secs(1)));
        assert!(
            !log.evidence_lost(t0 + secs(40)),
            "and it expires with the window"
        );
    }

    #[test]
    fn eviction_drops_brackets_respawns_arrivals_and_overflows_alike() {
        let mut log = WindowLog::new(secs(10));
        let t0 = Instant::now();
        let id = log.open_bracket(SourceId(0), t0, Vec::new());
        log.close_bracket(id, t0 + Duration::from_millis(5), Vec::new());
        log.record_respawn(SourceId(0), t0);
        log.observe_arrival(SourceId(0), TriggerKey("fifo:/tmp/a".into()), t0);
        log.record_overflow(t0);

        log.evict(t0 + secs(30));

        assert_eq!(log.respawns_in_window(SourceId(0), t0 + secs(30)), 0);
        assert!(log.arrivals(&TriggerKey("fifo:/tmp/a".into())).is_empty());
        assert!(!log.evidence_lost(t0 + secs(30)));
        assert!((log.busy_fraction(t0 + secs(30)) - 0.0).abs() < 1e-9);
    }

    #[test]
    fn eviction_keeps_a_bracket_that_still_overlaps_the_window() {
        // A bracket that opened before the cutoff but closed inside it still
        // contributes duty; dropping it by open time alone would silently
        // under-report a long child.
        let mut log = WindowLog::new(secs(10));
        let t0 = Instant::now();
        let id = log.open_bracket(SourceId(0), t0, Vec::new());
        log.close_bracket(id, t0 + secs(6), Vec::new());

        log.evict(t0 + secs(11)); // cutoff is t0 + 1s: opened before, closed after

        let f = log.busy_fraction(t0 + secs(11));
        assert!(
            f > 0.0,
            "a bracket overlapping the window must survive — got {f}"
        );
    }

    #[test]
    fn eviction_retains_a_bracket_a_live_arrival_still_references() {
        // Otherwise the arrival's widths become unresolvable and the credit
        // rule silently loses its input.
        let mut log = WindowLog::new(secs(10));
        let t0 = Instant::now();
        let id = log.open_bracket(SourceId(0), t0, Vec::new());
        log.close_bracket(id, t0 + Duration::from_millis(5), Vec::new());
        // The arrival is recent; its bracket is old.
        log.observe_arrival(SourceId(0), TriggerKey("fifo:/tmp/a".into()), t0);
        log.arrivals(&TriggerKey("fifo:/tmp/a".into()));

        log.evict(t0 + Duration::from_millis(500));

        let key = TriggerKey("fifo:/tmp/a".into());
        let arrivals = log.arrivals(&key);
        assert_eq!(arrivals.len(), 1);
        assert_eq!(
            log.widths(arrivals[0]),
            Some(vec![(SourceId(0), Duration::from_millis(5))]),
            "the referenced bracket must have been retained"
        );
    }

    // ── LoopSuspicion (plan task 1.2) ───────────────────────────────────
    //
    // All four conditions, and the credit rule tested directly on synthetic
    // changes so the two stages can be exercised without a filesystem.

    fn ms(n: u64) -> Duration {
        Duration::from_millis(n)
    }

    /// `n` changes, each covered by the given (source, width) pairs.
    fn changes_all(n: usize, containing: &[(SourceId, Duration)]) -> Vec<Change> {
        let t = Instant::now();
        (0..n)
            .map(|i| Change {
                at: t + ms(i as u64),
                containing: containing.to_vec(),
            })
            .collect()
    }

    #[test]
    fn credit_merges_two_panes_whose_children_cost_the_same() {
        // A cycle's panes run near-identical children, because they are the
        // same work phase-locked by the same debounce. Measured at 1.0-1.1x.
        let credited = credit(&changes_all(
            10,
            &[(SourceId(0), ms(22)), (SourceId(1), ms(24))],
        ));
        assert_eq!(credited, vec![SourceId(0), SourceId(1)]);
    }

    #[test]
    fn credit_rejects_a_producer_whose_bracket_merely_contains_the_consumers() {
        // The measured chain: producer 168ms, consumer 6.5ms — 25.9x, so the
        // producer is not within 2x of the tightest and is not credited.
        // Without stage two both would merge and an acyclic chain would look
        // like a loop.
        let credited = credit(&changes_all(
            10,
            &[(SourceId(0), ms(168)), (SourceId(1), ms(6))],
        ));
        assert_eq!(credited, vec![SourceId(1)], "only the tight one");
    }

    #[test]
    fn credit_requires_more_than_half_the_changes_not_merely_some() {
        // Stage one, and it is not optional: a pane whose bracket happens to
        // overlap a minority of a path's changes is not its writer, however
        // tight that bracket is.
        let mut changes = changes_all(8, &[(SourceId(1), ms(5))]);
        changes.extend(changes_all(
            2,
            &[(SourceId(1), ms(5)), (SourceId(9), ms(1))],
        ));
        let credited = credit(&changes);
        assert_eq!(
            credited,
            vec![SourceId(1)],
            "SourceId(9) covered 2 of 10 and must not be credited despite being tighter"
        );
    }

    #[test]
    fn credit_of_nothing_is_nothing() {
        assert!(credit(&[]).is_empty());
        assert!(credit(&changes_all(3, &[])).is_empty());
    }

    /// A pane that has made enough trigger-driven respawns to be a candidate.
    fn pane<'a>(
        id: usize,
        watched: &'a [std::path::PathBuf],
        readers: &'a [TriggerKey],
    ) -> PaneWindow<'a> {
        PaneWindow {
            source: SourceId(id),
            trigger_respawns: 50,
            watched,
            readers,
        }
    }

    /// A ledger holding exactly the changes given, with no filesystem.
    fn ledger_with(entries: &[(&std::path::Path, Vec<Change>)]) -> PathLedger {
        let mut ledger = PathLedger::new(Vec::new());
        for (path, changes) in entries {
            for change in changes {
                ledger.inject(path, change.at, change.containing.clone());
            }
        }
        ledger
    }

    #[test]
    fn a_two_pane_cycle_trips_and_names_both() {
        // Each pane writes the path the other watches, children overlapping,
        // nothing exogenous.
        let a = std::path::PathBuf::from("/sa");
        let b = std::path::PathBuf::from("/sb");
        let both = [(SourceId(0), ms(22)), (SourceId(1), ms(24))];
        let ledger = ledger_with(&[(&a, changes_all(10, &both)), (&b, changes_all(10, &both))]);
        let log = WindowLog::new(secs(30));
        let (wa, wb) = (vec![a.clone()], vec![b.clone()]);
        let none: Vec<TriggerKey> = Vec::new();
        let panes = [pane(0, &wa, &none), pane(1, &wb, &none)];

        let v = LoopSuspicion::default().evaluate(Instant::now(), &ledger, &log, &panes);
        assert_eq!(v.panes, vec![SourceId(0), SourceId(1)]);
        assert!(!v.abstained);
    }

    #[test]
    fn concurrent_children_are_never_ordered() {
        // Either pane could have written either path, so a direction would be
        // a claim the observation cannot support.
        let a = std::path::PathBuf::from("/sa");
        let both = [(SourceId(0), ms(22)), (SourceId(1), ms(24))];
        let ledger = ledger_with(&[(&a, changes_all(10, &both))]);
        let log = WindowLog::new(secs(30));
        let wa = vec![a.clone()];
        let none: Vec<TriggerKey> = Vec::new();
        let panes = [pane(0, &wa, &none), pane(1, &wa, &none)];

        let v = LoopSuspicion::default().evaluate(Instant::now(), &ledger, &log, &panes);
        assert!(v.ordered.is_none(), "a merged pair cannot be ordered");
    }

    #[test]
    fn a_one_way_producer_consumer_pair_does_not_trip() {
        // The false positive that forced condition 4 to exist: this satisfies
        // conditions 1, 2 and 3 exactly, and only the graph test excludes it.
        // Pane 0 writes /data; pane 1 watches it; nobody writes anything of
        // pane 0's.
        let data = std::path::PathBuf::from("/data");
        let ledger = ledger_with(&[(&data, changes_all(10, &[(SourceId(0), ms(6))]))]);
        let log = WindowLog::new(secs(30));
        let watched1 = vec![data.clone()];
        let watched0: Vec<std::path::PathBuf> = vec![std::path::PathBuf::from("/upstream")];
        let none: Vec<TriggerKey> = Vec::new();
        let panes = [pane(0, &watched0, &none), pane(1, &watched1, &none)];

        let v = LoopSuspicion::default().evaluate(Instant::now(), &ledger, &log, &panes);
        assert!(
            v.panes.is_empty(),
            "an acyclic one-way chain must never be implicated"
        );
    }

    #[test]
    fn an_expensive_producer_chain_does_not_trip_end_to_end() {
        // The measured S1b shape, driven through evaluate. A three-pane chain
        // A -> B -> C, where A's child is expensive enough to wholly contain
        // B's, so every write B makes to /d2 also falls inside A's bracket.
        //
        // Without the tightness stage, A and B are credited together for /d2
        // and MERGE into one node — and because B also watches /d1, which A
        // writes, that merged node gains an edge to itself and this acyclic
        // chain reads as a cycle. Its duty stays under the abstention ceiling
        // on purpose, so only the credit rule can save it.
        let d1 = std::path::PathBuf::from("/d1"); // A writes, B watches
        let d2 = std::path::PathBuf::from("/d2"); // B writes, C watches
        let ledger = ledger_with(&[
            (&d1, changes_all(10, &[(SourceId(0), ms(168))])),
            (
                &d2,
                changes_all(10, &[(SourceId(0), ms(168)), (SourceId(1), ms(6))]),
            ),
        ]);
        let log = WindowLog::new(secs(30));
        let (wa, wb, wc) = (
            vec![std::path::PathBuf::from("/upstream")],
            vec![d1.clone()],
            vec![d2.clone()],
        );
        let none: Vec<TriggerKey> = Vec::new();
        let panes = [
            pane(0, &wa, &none),
            pane(1, &wb, &none),
            pane(2, &wc, &none),
        ];

        let v = LoopSuspicion::default().evaluate(Instant::now(), &ledger, &log, &panes);
        assert!(
            v.panes.is_empty(),
            "an acyclic chain must not trip because one bracket contains another: {:?}",
            v.panes
        );
    }

    #[test]
    fn too_few_trigger_driven_respawns_never_trips() {
        // Condition 1, and note what it excludes for free: an interval pane
        // reaches its deadline without ever passing through the gate, so its
        // count stays zero however fast it runs.
        let a = std::path::PathBuf::from("/sa");
        let both = [(SourceId(0), ms(22)), (SourceId(1), ms(24))];
        let ledger = ledger_with(&[(&a, changes_all(10, &both))]);
        let log = WindowLog::new(secs(30));
        let wa = vec![a.clone()];
        let none: Vec<TriggerKey> = Vec::new();
        let mut slow = pane(0, &wa, &none);
        slow.trigger_respawns = 3;
        let panes = [slow];

        let v = LoopSuspicion::default().evaluate(Instant::now(), &ledger, &log, &panes);
        assert!(v.panes.is_empty());
    }

    #[test]
    fn one_exogenous_observation_clears_the_veto() {
        // Condition 2 is a zero test, not a rate.
        let a = std::path::PathBuf::from("/sa");
        let both = [(SourceId(0), ms(22)), (SourceId(1), ms(24))];
        let mut changes = changes_all(10, &both);
        changes.push(Change {
            at: Instant::now(),
            containing: Vec::new(), // nothing in flight: an outside writer
        });
        let ledger = ledger_with(&[(&a, changes)]);
        let log = WindowLog::new(secs(30));
        let wa = vec![a.clone()];
        let none: Vec<TriggerKey> = Vec::new();
        let panes = [pane(0, &wa, &none), pane(1, &wa, &none)];

        let v = LoopSuspicion::default().evaluate(Instant::now(), &ledger, &log, &panes);
        assert!(v.panes.is_empty(), "one exogenous change is enough");
    }

    #[test]
    fn a_pane_watching_nothing_is_never_implicated() {
        // Silence is not a positive: with no watched trigger there is no
        // evidence either way.
        let ledger = PathLedger::new(Vec::new());
        let log = WindowLog::new(secs(30));
        let nothing: Vec<std::path::PathBuf> = Vec::new();
        let none: Vec<TriggerKey> = Vec::new();
        let panes = [pane(0, &nothing, &none)];
        let v = LoopSuspicion::default().evaluate(Instant::now(), &ledger, &log, &panes);
        assert!(v.panes.is_empty());
    }

    #[test]
    fn a_busy_dashboard_abstains_rather_than_guessing() {
        // Condition 3. Duty lives in the LOG, as bracket intervals — the same
        // cycle observations, with brackets that nearly fill the window.
        let a = std::path::PathBuf::from("/sa");
        let both = [(SourceId(0), ms(22)), (SourceId(1), ms(24))];
        let ledger = ledger_with(&[(&a, changes_all(10, &both))]);
        let mut log = WindowLog::new(secs(10));
        let t0 = Instant::now();
        let id = log.open_bracket(SourceId(0), t0, Vec::new());
        log.close_bracket(id, t0 + secs(9), Vec::new());

        let wa = vec![a.clone()];
        let none: Vec<TriggerKey> = Vec::new();
        let panes = [pane(0, &wa, &none), pane(1, &wa, &none)];
        let v = LoopSuspicion::default().evaluate(t0 + secs(10), &ledger, &log, &panes);
        assert!(v.abstained, "90% duty must abstain");
        assert!(v.panes.is_empty(), "abstaining accuses nobody");
    }

    #[test]
    fn lost_reader_evidence_forces_abstention() {
        // A dropped arrival may have been the only exogenous observation, so
        // the veto cannot be trusted for this window.
        let a = std::path::PathBuf::from("/sa");
        let both = [(SourceId(0), ms(22)), (SourceId(1), ms(24))];
        let ledger = ledger_with(&[(&a, changes_all(10, &both))]);
        let mut log = WindowLog::new(secs(30));
        let t0 = Instant::now();
        log.record_overflow(t0);

        let wa = vec![a.clone()];
        let none: Vec<TriggerKey> = Vec::new();
        let panes = [pane(0, &wa, &none), pane(1, &wa, &none)];
        let v = LoopSuspicion::default().evaluate(t0 + secs(1), &ledger, &log, &panes);
        assert!(v.abstained);
    }

    #[test]
    fn an_arrival_with_an_unresolved_width_is_deferred_from_credit() {
        // It still counts for the veto — emptiness is known — but it must not
        // enter the median-width stage with an elapsed-so-far duration.
        let mut log = WindowLog::new(secs(30));
        let t0 = Instant::now();
        let key = TriggerKey("fifo:/tmp/a".into());
        log.open_bracket(SourceId(0), t0, Vec::new()); // never closed
        log.observe_arrival(SourceId(1), key.clone(), t0 + ms(3));

        let changes = LoopSuspicion::arrival_changes(&log, &key);
        assert!(
            changes.is_empty(),
            "an unresolved arrival contributes no credit input"
        );
    }
}
