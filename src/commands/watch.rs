//! One loop, N sources: the shared frame engine behind `rat watch`
//! (one source, no box) and — through the same `run_registry` — any
//! registry of sources. The iteration order is law: signals →
//! spawn-if-due → drain-then-compose-once → triggers → nap → age
//! refresh → theme verify → events. The drain terminates because each
//! source has at most one tick in flight; nothing ever paints inside
//! it, and nothing outside this file writes to the terminal. Every
//! gesture is whole-dashboard: there is no focus concept, `action_for`
//! takes no pane, and a future per-pane gesture is a deliberate
//! re-opening of that decision, not a small edit.

use std::io::Write;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::{Duration, Instant};

use anyhow::{Context, anyhow};
use crossterm::tty::IsTty;

use crate::cli::WatchArgs;
use crate::color::{ColorProfile, SystemEnv};
use crate::core::child::{
    ChildSlot, ShutdownGuard, TickEvent, not_started, run_tick, spawn_live_tick, spawn_tick,
};
use crate::core::duration::parse_interval;
use crate::core::layout::{
    PaneBlock, PaneChrome, PaneRect, compose_panes, pane_order, pane_rects, render_pane,
    render_pane_collapsed, scroll_badge,
};
use crate::core::live::Emissions;
use crate::core::measure::{seal_rows, shift_chop};
use crate::core::pager::{PagerCommand, resolve_pagers};
use crate::core::registry::{
    Composition, LayoutNode, Overflow, PaneGeometry, Registry, Shebang, ShellMode, SourceId,
    SourceProgram, SourceSpec, TitleSource, shebang,
};
use crate::core::retain::{Keep, Retention, compact_count};
use crate::core::schedule::{Due, TickSchedule};
use crate::core::snapshot::{snapshot_body, snapshot_stamp, write_snapshot};
use crate::core::trigger::{
    BracketId, DebounceGate, MtimeWatchSet, PathLedger, TriggerSpec, Verdict, WindowLog,
    parse_trigger, stamps,
};
use crate::exit::{AppError, AppResult};
use crate::style_spec::StyleSpec;
use crate::term::history::History;
use crate::term::inline::{InlineRenderer, truncate_to_rows};
use crate::term::marks::{GUTTER_COLS, LineMark, changed_marks, mark_cells_with, prefix_rows};
use crate::term::mouse::MouseGuard;
use crate::term::scroll::{
    HSHIFT_STEP, LiveScroll, ScrollState, ScrollStep, paused_notice, scrolled_notice,
};
use crate::term::tap::{MouseEvent, MouseKind, TapEvent};
#[cfg(unix)]
use crate::term::tap::{TapChunk, TapScanner, TriggerReader, TtyTap};
#[cfg(unix)]
use crate::term::theme_notify::{OscColorKind, ThemeNotifyGuard, classify_colors, may_subscribe};
use crate::term::tty::{AltScreenGuard, ConsoleUtf8Guard, RawModeGuard};
use crate::theme::{Appearance, AppearanceSource, Palette};
use crate::ui::key::{Key, from_crossterm};

/// The longest the loop sleeps before it re-checks signals, the
/// channel, and the schedule — one wait slice.
const SLICE: Duration = Duration::from_millis(50);

/// How long a `--once` dashboard may sit with no output and no exit
/// before it says on stderr which pane it is waiting on. A false
/// positive costs one stderr line and nothing else.
const ONCE_QUIET: Duration = Duration::from_secs(5);

/// The resize arm's settle window: reflow is immediate, but the
/// respawn-all waits for the drag to go quiet. ANCHORED like every
/// ratto debounce, so a sustained drag respawns once per window rather
/// than starving. The reflow is independent of this — the window only
/// paces the respawn.
const RESIZE_DEBOUNCE: Duration = Duration::from_millis(250);

/// How long a superseded live child gets between SIGTERM and the
/// force-kill. Two seconds: measured TERM->exit latency for the
/// representative children (a follower, rat's own __follow, a shell
/// trap loop) is under 50 ms, so 2 s clears a compliant exit by more
/// than an order of magnitude while bounding a TERM-ignoring child's
/// staleness to less than one typical cadence.
const SUPERSEDE_GRACE: Duration = Duration::from_secs(2);

/// The newest composed frame and everything derived from it. Absent
/// until the first child completes: the loop is live during that first
/// run (q quits, keys dispatch), but there is nothing to paint yet.
struct Live {
    lines: Vec<String>,
    hash: u64,
    /// When the content last CHANGED, not when it was last produced.
    changed_at: jiff::Timestamp,
    /// `changed_at` as local HH:MM:SS, formatted once per change.
    since: String,
    /// The pane surface, when this frame was composed from declared
    /// boxes: the marks already in composed coordinates and each
    /// chrome-bearing pane's last-change time. `None` under a plain
    /// watch, forever.
    panes: Option<PaneLive>,
    /// What this frame's tick discarded, for the status row. Only the
    /// plain path sets it: a pane surface says so on the pane itself,
    /// where the reader can tell WHICH pane overflowed.
    dropped: Option<String>,
}

/// A composed frame's retained pane surface.
struct PaneLive {
    marks: Vec<LineMark>,
    ages: Vec<jiff::Timestamp>,
}

/// Everything the loop needs that is not per-source: paint knobs, mode
/// flags, and the pre-built chrome strings each constructor owns.
pub(crate) struct SessionArgs {
    pub once: bool,
    /// The tab title's fallback identity — the declaration file's
    /// stem. `Some` makes an interactive session own the terminal tab
    /// title (dashboard-only: watch always passes `None`); the text
    /// composes with the title role at emit time.
    pub tab_title: Option<String>,
    /// `--once` gives up after this long: exit 124, stdout EMPTY, the
    /// waiting panes named on stderr. `None` (the default, and the
    /// only value plain watch can carry — its once tick runs inline
    /// on the loop thread, where a bound is unenforceable) waits
    /// forever.
    pub once_timeout: Option<Duration>,
    pub clear: bool,
    /// Take the alternate screen for the run; gated on `interactive`
    /// like every other framing concern, and subsuming `clear` (the
    /// wipe-and-home happens inside the first frame either way).
    pub fullscreen: bool,
    pub no_hide_cursor: bool,
    pub no_sync: bool,
    /// Capture the mouse: the wheel scrolls the frame instead of the
    /// terminal's scrollback. Opt-in — capture costs plain-drag text
    /// selection for the whole window; `m` hands it back mid-session.
    pub mouse: bool,
    pub wrap: bool,
    pub max_height: Option<u16>,
    pub snapshot_dir: Option<std::path::PathBuf>,
    pub snapshot_ansi: bool,
    /// The run-constant footer suffix, pre-built by the constructor.
    pub live_tail: String,
    /// The `?` reference's first row — the caller names its surface.
    pub help_heading: &'static str,
    /// The caller's own section of the `?` reference, appended after
    /// the shared key families.
    pub help_extra: Vec<String>,
    /// Reflow boxes and respawn every source on a resize; off means the
    /// spawn step owns the geometry re-measure (one writer per mode).
    pub resize_respawn: bool,
    /// Append distinct frames to the scrollback instead of repainting
    /// in place (watch-only; a TTY concern — piped output already
    /// appends, so the loop gates this on ttyness).
    pub append: bool,
}

/// Parse the watch flags, build the one-source registry, run it. The
/// surface is byte-frozen: the flags, footer, compose, and child
/// environment all pass through unchanged.
pub fn run(args: WatchArgs, profile: ColorProfile, palette: Palette) -> AppResult {
    let triggers = args
        .trigger
        .iter()
        .map(|spec| parse_trigger(spec))
        .collect::<anyhow::Result<Vec<TriggerSpec>>>()?;
    let interval = resolve_interval(args.interval.as_deref(), !triggers.is_empty())?;
    let debounce = parse_interval(&args.trigger_debounce)?;
    // The footer label carries the user's own token; a defaulted interval
    // reads as its literal default. Trigger-only mode has no token at all.
    let interval_label = args
        .interval
        .as_deref()
        .or(triggers.is_empty().then_some("2s"));
    let live_tail = live_suffix(args.once, interval_label, !triggers.is_empty());
    let help_extra = trigger_help(&triggers);
    let shell = shell_mode(args.shell.as_ref())?;
    let registry = Registry::single(
        SourceSpec {
            id: String::new(),
            program: SourceProgram::Argv(if shell.runs_a_shell() {
                // A shell source keeps the raw script as ONE element,
                // or split-then-join would destroy its quoting.
                vec![args.command.join(" ")]
            } else {
                args.command.clone()
            }),
            shell,
            interval,
            triggers,
            debounce,
            // `rat watch` has no declaration to say otherwise. A live
            // source is a dashboard pane's opt-in; the plain watch path
            // stays exactly the batch loop it has always been.
            live: false,
        },
        args.title.clone(),
    );
    let session = SessionArgs {
        once: args.once,
        // The tab title is the dashboard's: a watch session is one
        // command, already named by the shell that launched it.
        tab_title: None,
        // Plain watch --once runs its tick inline on the loop thread,
        // so a bound is unenforceable here; the flag lives on
        // `rat dashboard` only.
        once_timeout: None,
        clear: args.clear,
        fullscreen: args.fullscreen,
        mouse: args.mouse,
        no_hide_cursor: args.no_hide_cursor,
        no_sync: args.no_sync,
        wrap: !args.no_wrap,
        max_height: args.max_height,
        snapshot_dir: args.snapshot_dir.clone(),
        snapshot_ansi: args.snapshot_ansi,
        live_tail,
        help_heading: "rat watch — keys",
        help_extra,
        resize_respawn: false,
        append: args.append,
    };
    run_registry(registry, session, profile, palette)
}

// The palette follows the terminal only where the reader can see its
// reports; elsewhere it stays the startup verdict for the whole run.
#[cfg_attr(windows, allow(unused_mut))]
pub(crate) fn run_registry(
    registry: Registry,
    session: SessionArgs,
    profile: ColorProfile,
    mut palette: Palette,
) -> AppResult {
    let (interrupted, terminated) = register_signals()?;
    let plain = matches!(registry.composition(), Composition::Plain { .. });

    let stdout = std::io::stdout();
    let is_tty = stdout.is_tty();
    // The looping tty mode reads keys (q quits, v pages the full frame), so
    // it owns the terminal input; children must not compete for it.
    let interactive = is_tty && !session.once;
    // Gate on interactive, not merely is_tty: a --once run must not
    // paint into a buffer nobody sees.
    let fullscreen = interactive && session.fullscreen;
    // is_tty, not interactive: a --once --append run on a terminal must
    // still append. Piped stdout ignores the flag entirely — the piped
    // path already IS the appended stream, which is what keeps its
    // bytes frozen by construction.
    let append = is_tty && session.append;
    // Raw mode clears OPOST/ONLCR, so LF alone drops a row without
    // returning the column (src/term/inline.rs's rule). A --once run
    // never enables raw mode and keeps the terminal's own ONLCR.
    let eol = if interactive { "\r\n" } else { "\n" };
    // Framing only makes sense on a terminal; piped output gets the plain
    // content so `rat watch | tee log` stays readable. Fullscreen
    // subsumes --clear: the same wipe-and-home inside the first frame
    // is what homes the origin on the fresh alternate screen.
    let mut renderer = InlineRenderer::new(stdout.lock())
        .with_cursor_hidden(is_tty && !session.no_hide_cursor && !append)
        .with_sync_output(is_tty && !session.no_sync)
        .with_clear_screen(is_tty && (session.clear || fullscreen));
    // The event-wait routes need a terminal to wake; only the stat-poll
    // works piped, so the other schemes refuse early, before any
    // terminal state changes. Per source: the pane that declared the
    // spec is the one the error names.
    if !interactive {
        for id in registry.ids() {
            let spec = registry.spec(id);
            if spec
                .triggers
                .iter()
                .any(|trigger| !matches!(trigger, TriggerSpec::File(_)))
            {
                return Err(anyhow!(
                    "{}fifo:/fd: triggers need an interactive terminal; use file:PATH",
                    pane_label(&registry, id)
                )
                .into());
            }
        }
    }
    // Script bodies are materialized ONCE per run, before the terminal
    // changes: a body that cannot be written is a load failure, not a
    // pane that fails forever. The binding is NAMED — `let _ =` would
    // remove the directory here and now — and it is declared BEFORE the
    // shutdown guards so it drops after them: children die first, then
    // the files they were running.
    let scripts = ScriptFiles::materialize(&registry)?;
    let _raw_guard = if interactive {
        Some(RawModeGuard::enable().context("enabling raw mode")?)
    } else {
        None
    };
    // AFTER the raw guard: reverse drop order leaves the alternate
    // screen first, then disables raw mode — the screen comes back
    // exactly as it was on every unwinding exit path.
    let _alt_guard = if fullscreen {
        Some(AltScreenGuard::enter().context("entering the alternate screen")?)
    } else {
        None
    };
    // After the raw guard, so its disable is written while echo is
    // still suppressed. Opt-in: capture takes the terminal's own wheel
    // (and plain-drag selection) for the whole window.
    let mut mouse_guard = if interactive && session.mouse {
        Some(MouseGuard::enable(std::io::stdout()).context("enabling mouse reporting")?)
    } else {
        None
    };
    // Watch owns the terminal's input while it loops. On unix it reads the
    // device itself: the terminal can send escape sequences unprompted, and
    // those have to be parsed by whoever owns the input stream. Exactly one
    // reader is attached at a time — see the pager arm below.
    #[cfg(unix)]
    let tap = if interactive {
        // When the device cannot be opened, this run keeps the event
        // library's pump instead.
        TtyTap::spawn().ok()
    } else {
        None
    };
    // Declared outside the tick loop so a report split across a tick
    // boundary still reassembles.
    #[cfg(unix)]
    let mut scanner = TapScanner::new();
    // Declared after the raw-mode guard so it drops first: the unsubscribe
    // is written while echo is still suppressed. The terminal only pushes
    // theme changes while this is live, and only the reader above ever
    // sees them.
    #[cfg(unix)]
    let mut theme_sub = may_subscribe(
        palette.source,
        profile,
        // No live theme re-verification in append mode: an adoption
        // respawn would re-announce a frame that only changed color.
        // Children still get the startup verdict via RAT_APPEARANCE.
        interactive && tap.is_some() && !append,
    )
    .then(|| ThemeNotifyGuard::subscribe(std::io::stdout()))
    .transpose()
    .context("subscribing to theme notifications")?;
    #[cfg(unix)]
    let mut verify = VerifyState::default();

    let title_line = match registry.composition() {
        Composition::Plain { title } => title.as_ref().map(|title| {
            StyleSpec {
                bold: true,
                ..StyleSpec::default()
            }
            .render(title, profile)
        }),
        Composition::Panes { .. } => None,
    };
    let faint = StyleSpec {
        faint: true,
        ..StyleSpec::default()
    };

    let (tx, rx) = std::sync::mpsc::channel::<TickEvent>();
    // `--once` arms no triggers: the run ends at the first composition,
    // so a watcher could only fire into a loop that has already left.
    let armed = !session.once;
    // The GLOBAL union of every watched path, computed once. Global and not
    // per-source on purpose: in a feedback loop one pane's command touches the
    // path another pane watches, so a source-local set could never connect the
    // two. Empty when nothing is armed, which is every `--once` run, and an
    // empty union makes every observer call below a no-op.
    let watched_union: Vec<std::path::PathBuf> = if armed {
        let mut paths: Vec<std::path::PathBuf> = registry
            .ids()
            .flat_map(|id| file_paths(&registry.spec(id).triggers))
            .collect();
        paths.sort();
        paths.dedup();
        paths
    } else {
        Vec::new()
    };
    // Per-source watched paths, kept so the evaluation can borrow them rather
    // than rebuild a Vec per iteration at 20 Hz.
    let per_source_watched: Vec<Vec<std::path::PathBuf>> = registry
        .ids()
        .map(|id| {
            if armed {
                file_paths(&registry.spec(id).triggers)
            } else {
                Vec::new()
            }
        })
        .collect();
    // The same idea for the reader route: its evidence is keyed by trigger,
    // not by path, because a fifo has nothing to stat. Empty on Windows,
    // which keeps the crossterm pump and opens no readers at all.
    let per_source_readers: Vec<Vec<crate::core::trigger::TriggerKey>> = registry
        .ids()
        .map(|id| {
            if cfg!(unix) && armed {
                registry
                    .spec(id)
                    .triggers
                    .iter()
                    .filter(|spec| !matches!(spec, TriggerSpec::File(_)))
                    .map(reader_key)
                    .collect()
            } else {
                Vec::new()
            }
        })
        .collect();
    // Whether this dashboard has ANY trigger evidence to collect. It must
    // span both routes: `watched_union` is a `file:`-only quantity, so gating
    // the observation on it alone left a fifo-only dashboard opening no
    // brackets and never evaluating — every arrival then read as exogenous,
    // because nothing had been recorded as covering it, and the veto silently
    // held forever. A cycle spinning at ~9 Hz went unreported for that reason
    // and no test noticed, because the unit tests supply a window directly.
    let observing =
        !watched_union.is_empty() || per_source_readers.iter().any(|keys| !keys.is_empty());
    // The observer's own baselines, deliberately separate from every
    // MtimeWatchSet over the same paths: sharing them would let an observer
    // poll consume a trigger's fire, and the pane would stop refreshing.
    let mut ledger = PathLedger::new(watched_union.clone());
    // DIAGNOSTIC ONLY: `RAT_TRIGGER_TRACE=<path>` records how the suspicion
    // test answered, every time it answers. A dashboard that never reports a
    // loop looks exactly like one that has none, and this is the only surface
    // that tells the two apart from outside the process. Unset — the shipped
    // case — costs one `var_os` at startup and nothing per iteration.
    let mut trace = TriggerTrace::open();
    let suspicion = crate::core::trigger::LoopSuspicion {
        explain: trace.is_some(),
        ..Default::default()
    };
    let mut log = WindowLog::new(suspicion.window);
    // The panes the notice has already announced — its latch, so one
    // loop announces itself once and a repaired one can announce again.
    let mut suspected: Vec<SourceId> = Vec::new();
    let mut runtime: Vec<SourceRuntime> = registry
        .ids()
        .map(|id| {
            let spec = registry.spec(id);
            let mut files = MtimeWatchSet::new(if armed {
                file_paths(&spec.triggers)
            } else {
                Vec::new()
            });
            // The baseline exists BEFORE the first spawn, per source: a
            // change landing between this source's first child and the
            // loop's first check must be detected, never absorbed into
            // the baseline.
            files.fired();
            SourceRuntime {
                schedule: TickSchedule::new(spec.interval),
                slot: ChildSlot::default(),
                tx: tx.clone(),
                // Built here, once, and outliving every child: the caps
                // must survive across reads, which is the whole
                // difference from the batch path's per-tick accumulator.
                // Both streams get the same policy the batch path would
                // have used, so a follower is bounded exactly as a
                // flooding command is.
                emissions: spec.live.then(|| {
                    let retention = retention_for(&registry, id);
                    Emissions::new(retention, retention)
                }),
                output: None,
                hash: 0,
                changed_at: jiff::Timestamp::UNIX_EPOCH,
                previous: None,
                marks: Vec::new(),
                failure: None,
                truncated: None,
                posted: false,
                gate: DebounceGate::new(spec.debounce),
                files,
                looping: false,
                bracket: None,
                #[cfg(unix)]
                readers: Vec::new(),
            }
        })
        .collect();

    // The terminal tab is the session's while it runs: stack pushed,
    // the marker title set, stack popped on every exit path via the
    // guard's Drop. Interactive dashboards only — a piped or --once
    // run never emits a byte of it, and watch passes no stem at all.
    // Declared after the raw-mode guard so the pop still writes while
    // echo is suppressed.
    let mut tab_title = match (&session.tab_title, interactive) {
        (Some(stem), true) => {
            let role = registry
                .title_source()
                .and_then(|source| title_role_text(source, &runtime));
            Some(
                crate::term::tab_title::TabTitleGuard::set_over_stack(
                    std::io::stdout(),
                    &crate::term::tab_title::tab_title_text(role.as_deref(), stem),
                )
                .context("setting the tab title")?,
            )
        }
        _ => None,
    };

    // Every exit from run_registry — return, `?`, panic — kills every
    // in-flight child through these guards' Drop. A NAMED binding:
    // `let _ =` would drop them here and now. One guard per slot, and
    // no registry-level lock: a shared mutex would serialize spawns
    // and could block a shutdown behind one of them.
    let _shutdown: Vec<ShutdownGuard> = runtime.iter().map(|r| r.slot.guard()).collect();
    // The fifo/fd reader threads: long-lived inside their source's
    // runtime — dropping a runtime entry joins its readers. Each slot
    // carries its own end-of-life state for the one-shot notice.
    #[cfg(unix)]
    if armed {
        let wake = tap.as_ref().map(TtyTap::sender);
        for id in registry.ids() {
            for spec in &registry.spec(id).triggers {
                if matches!(spec, TriggerSpec::File(_)) {
                    continue; // polled in the loop, no thread
                }
                if let TriggerSpec::Fd(0) = spec {
                    return Err(anyhow!(
                        "{}fd:0 is the terminal's own input while watch reads keys; \
                         use another descriptor",
                        pane_label(&registry, id)
                    )
                    .into());
                }
                runtime[id.0].readers.push(ReaderSlot {
                    reader: TriggerReader::open(spec, wake.clone())?,
                    spec: spec.clone(),
                    ended_seen: false,
                });
            }
        }
    }
    let live_tail = session.live_tail.clone();
    let mut view = ViewState {
        wrap: session.wrap,
        hshift: 0,
        gutter: false,
        highlight: false,
        alt_time: false,
    };
    // Per-pane view state, beside the whole-frame `view`: the gestures
    // move it, the composer reads it, and the repaint gate sees it
    // through the digest.
    let mut panes = PaneView::new(registry.len());
    // Loop-persistent geometry state: the one size/geometry pair the
    // spawn step, the composer, and the (future) resize arm consume. It
    // outlives the spawn branch — a completion can arrive in an
    // iteration with no new spawn, and the composer still needs an
    // in-scope geometry vector.
    let mut size = measure_size(is_tty, (80, 24));
    let mut geom = derive_geometry(&registry, size, session.max_height, view.gutter, &panes);
    // Each pane's window starts where its declaration puts it; the first
    // collect step's reanchor gives a pinned window its tail.
    for id in registry.ids() {
        let overflow = registry.pane(id).map(|p| p.overflow).unwrap_or_default();
        panes.scroll[id.0] = initial_pane_scroll(overflow, 0, geom[id.0].inner_rows as usize);
    }
    let mut resize_gate = DebounceGate::new(RESIZE_DEBOUNCE);
    // Zoom's honesty channel. A child is told its width only at spawn, so a
    // zoomed (or restored) pane needs one run to become honest — debounced
    // PER PANE (D3): pane A's outstanding obligation survives pane B's
    // gesture, and a burst of toggles on one pane still costs one run.
    let mut zoom_gates: Vec<DebounceGate> = registry
        .ids()
        .map(|_| DebounceGate::new(RESIZE_DEBOUNCE))
        .collect();
    // The once-notice clock starts when the loop does — the panes'
    // first spawns are due immediately, so loop age IS wait age.
    let once_started = Instant::now();
    let mut once_notice_sent = false;
    let mut previous_key: Option<PaintKey> = None;
    // Append mode's gate: CONTENT ONLY. A PaintKey carries the terminal
    // size and the palette, and on a TTY both move — a resize or theme
    // flip must never re-announce an unchanged frame. `previous_key`
    // stays None in append mode, which also keeps the 1 Hz age-refresh
    // arm structurally inert.
    let mut previous_append: Option<u64> = None;
    // The last SPOKEN exit badge — append mode's second, independent
    // gate (the exit stays out of the frame hash).
    let mut prev_exit: Option<String> = None;
    let mut live: Option<Live> = None;
    let mut pause: Option<PauseState> = None;
    let mut live_scroll: Option<LiveScroll> = None;
    let mut history = History::new();
    // `append && interactive` ⟺ append && !once (append implies
    // is_tty), and live_suffix returns "" under --once — so this also
    // never prints a useless stub on a one-shot run.
    if append && interactive {
        append_rows(
            &mut std::io::stdout().lock(),
            vec![append_banner(&live_tail)],
            eol,
        )?;
    }
    loop {
        // 1. Signals — checked every slice, child running or not.
        if interrupted.load(Ordering::Relaxed) {
            renderer.finish().context("restoring terminal")?;
            return Err(AppError::Aborted);
        }
        if terminated.load(Ordering::Relaxed) {
            renderer.finish().context("restoring terminal")?;
            return Ok(());
        }
        // 2. Start every source that is due. Each source has at most
        // one tick in flight; a child's environment is measured HERE,
        // before its spawn, on this thread. Each mode has exactly ONE
        // geometry re-measure site: without a resize arm the spawn
        // step re-measures when something is about to start (the
        // shipped cadence); with one, that arm is the pair's only
        // writer — a coincident spawn would otherwise consume the new
        // size first and blind the arm's change detection.
        let now = Instant::now();
        let due: Vec<SourceId> = registry
            .ids()
            .filter(|id| runtime[id.0].schedule.poll(now) == Due::Spawn)
            .collect();
        if !due.is_empty() {
            refresh_geometry_for_spawn(
                session.resize_respawn,
                measure_size(is_tty, size),
                gutter_reserve(view.gutter),
                &mut size,
                &mut geom,
                &registry,
            );
            for id in due {
                // Once mode has no loop to keep responsive: it runs the
                // tick on this thread and posts it to the channel a
                // worker would have used, so both modes share one
                // completion handler. The same detour catches an OS
                // that refuses a thread — that costs a stall, never a
                // tick. It stays a single-source detour: N sources run
                // inline would cost the sum of their runtimes, not the
                // max.
                // The bracket opens BEFORE the child does. Stamping after
                // the spawn is a race the child usually loses but need not:
                // a write landing before the opening stamp is taken is
                // invisible to that bracket — its two snapshots agree — and
                // the next idle poll then reports the change as EXOGENOUS,
                // which vetoes the very pane that made it. Opening early
                // can only widen the window, never lose a write.
                if observing {
                    let opened = log.open_bracket(id, Instant::now(), stamps(&watched_union));
                    runtime[id.0].bracket = Some(opened);
                    // Fence AFTER the bracket opens and BEFORE the child can
                    // write, so a promptly-served fence puts the interval's
                    // lower bound INSIDE this bracket — which is the only
                    // way a write by this child can ever be attributable to
                    // it. Fencing before the open would guarantee the
                    // interval starts too early and could never be covered.
                    //
                    // Per spawn, not once before the loop: brackets open one
                    // per iteration here, so a single pass before them would
                    // fence against brackets that do not exist yet.
                    //
                    // The pass is a separate statement from the mutation
                    // above so the `&runtime` borrow does not overlap the
                    // `runtime[id.0]` write.
                    #[cfg(unix)]
                    for r in runtime.iter() {
                        fence_all(&r.readers);
                    }
                }
                // Resolved once per spawn and shared by both paths: two
                // resolutions are two chances to disagree, and the
                // threaded path and the inline fallback disagreeing is
                // how a test ends up proving something the binary does
                // not do.
                let retention = retention_for(&registry, id);
                let mut inline = session.once && plain;
                if !inline {
                    let command = source_command(
                        &registry,
                        &scripts,
                        id,
                        interactive,
                        palette.appearance,
                        geom[id.0],
                    );
                    inline = match runtime[id.0].emissions.clone() {
                        // A live source is spawned once and never expected
                        // to exit, so its body reaches the loop through the
                        // outbox rather than at EOF — and it has NO inline
                        // fallback: running a child that never exits on this
                        // thread would wedge the loop for good. A worker the
                        // OS refuses is reported as the failure it is.
                        Some(emissions) => {
                            if let Err(err) = spawn_live_tick(
                                command,
                                id,
                                runtime[id.0].slot.clone(),
                                emissions,
                                runtime[id.0].tx.clone(),
                                watched_union.clone(),
                            ) {
                                let _ = runtime[id.0]
                                    .tx
                                    .send(TickEvent::Completed(not_started(id, err)));
                            }
                            false
                        }
                        None => spawn_tick(
                            command,
                            id,
                            runtime[id.0].slot.clone(),
                            runtime[id.0].tx.clone(),
                            watched_union.clone(),
                            retention,
                        )
                        .is_err(),
                    };
                }
                if inline {
                    let command = source_command(
                        &registry,
                        &scripts,
                        id,
                        interactive,
                        palette.appearance,
                        geom[id.0],
                    );
                    let _ = runtime[id.0].tx.send(TickEvent::Completed(run_tick(
                        command,
                        id,
                        watched_union.clone(),
                        retention,
                    )));
                }
            }
        }
        // 3. Drain EVERY queued event, then compose ONCE. The drain
        // terminates in at most N iterations: one source can have at
        // most one tick in flight, so it can post at most one outcome
        // per iteration. Only per-source state is touched in here;
        // painting inside this loop would put an intermediate
        // composition on screen — the one thing the iteration order
        // exists to prevent.
        //
        // A long-lived source's progress wakes do not weaken that
        // argument, and it is the OUTBOX that saves it, not the
        // channel: a wake is sent only when a body fills an empty
        // outbox, so a child flooding a slot nobody has read yet
        // publishes silently and still contributes at most one event.
        // TWO LISTS, and keeping them apart is the single most important
        // line here. `moved` is whatever changed the frame and gates the
        // compose; `drained` is COMPLETIONS ONLY and is what the
        // scheduler reads. A long-lived child that emits has not
        // finished — telling the scheduler otherwise clears `in_flight`
        // and spawns a second child beside the first, whose handle the
        // slot then overwrites, orphaning the original.
        let mut moved: Vec<SourceId> = Vec::new();
        let mut drained: Vec<SourceId> = Vec::new();
        let mut changed: Option<jiff::Timestamp> = None;
        let mut newest = jiff::Timestamp::UNIX_EPOCH;
        let mut piped_stderr: Vec<u8> = Vec::new();
        // A piped plain run has no status row to carry the marker, and
        // its stdout is somebody's data — so this rides stderr, beside
        // where a spawn error already goes.
        let mut piped_dropped: Option<String> = None;
        // Append mode's exit fact for this iteration's Plain outcome —
        // Some only when the child actually RAN: a spawn error has no
        // status, carries its reason as frame content, and must neither
        // speak nor update prev_exit (exit_badge(None) would read as a
        // clean exit and fake a recovery).
        let mut plain_exit: Option<Option<String>> = None;
        while let Ok(event) = rx.try_recv() {
            let outcome = match event {
                TickEvent::Completed(outcome) => outcome,
                TickEvent::Progress { source } => {
                    // At most one body per source per iteration by
                    // construction: the outbox is one slot and the wake
                    // is transition-gated. An empty slot means the loop
                    // already took this body, so the wake is spent.
                    let Some(emission) = runtime[source.0]
                        .emissions
                        .as_ref()
                        .and_then(Emissions::take)
                    else {
                        continue;
                    };
                    // A long-lived source is a PANE source by
                    // construction — `live` is a pane key and a plain
                    // watch has no pane declaration — so there is no
                    // plain arm to mirror here.
                    let spec = registry.spec(source);
                    let program = spawn_program(spec);
                    let lines = pane_body(
                        &emission.stdout.concat(),
                        &emission.stderr.concat(),
                        // An emission is output. A spawn error is a
                        // completion's business and cannot arrive here.
                        None,
                        &spec.id,
                        &program,
                    );
                    let changed_now = record_pane_body(
                        &mut runtime[source.0],
                        lines,
                        // A fresh live body SUPERSEDES the previous
                        // child's verdict. Nothing else would clear it:
                        // the replacement never completes, so it never
                        // posts a status, and its content would render
                        // under a dead child's `exit N` forever.
                        None,
                        emission.dropped,
                        emission.at,
                    );
                    changed = fold_changed_at(changed, changed_now, emission.at);
                    newest = newest.max(emission.at);
                    runtime[source.0].posted = true;
                    // `moved` and NOT `drained`: this source has content,
                    // and it has not finished. No bracket closes either —
                    // no child ran to completion.
                    moved.push(source);
                    continue;
                }
            };
            let id = outcome.source;
            let changed_now = match registry.composition() {
                Composition::Plain { .. } => {
                    let (stdout, stderr) = match outcome.spawn_error {
                        // Once mode fails loudly, before any paint; loop
                        // mode renders the failure as content so a
                        // transient error does not tear down the
                        // dashboard. Plain-only: under panes a spawn
                        // error is pane content, never an exit — a
                        // dashboard with one broken pane must still
                        // paint the others.
                        Some(err) if session.once => {
                            return Err(anyhow!(
                                "running {:?}: {err}",
                                spawn_program(registry.spec(id))
                            )
                            .into());
                        }
                        Some(err) => (
                            watch_spawn_error_text(&spawn_program(registry.spec(id)), &err)
                                .into_bytes(),
                            Vec::new(),
                        ),
                        // The retained lines concatenated: the composer
                        // decodes the WHOLE stream, so it must be handed
                        // one. Decoding line by line would render an
                        // empty stream as no rows instead of one, and
                        // keep a run of trailing blank lines the
                        // composer collapses.
                        None => (outcome.stdout.concat(), outcome.stderr.concat()),
                    };
                    let truncated = dropped_badge(outcome.dropped);
                    let mut combined = stdout.clone();
                    combined.extend_from_slice(&stderr);
                    // The marker joins the frame's change key for the
                    // same reason a pane's badge joins its pane's: a
                    // command can print an identical tail and start
                    // dropping, and a marker nothing repaints is a
                    // marker nobody sees.
                    if let Some(truncated) = &truncated {
                        combined.push(b'\n');
                        combined.extend_from_slice(truncated.as_bytes());
                    }
                    let hash = signature(&combined);
                    let r = &mut runtime[id.0];
                    let changed_now = hash != r.hash || !r.posted;
                    // The single source's rendered lines ARE the frame —
                    // the shipped composer, shipped arguments, shipped
                    // bytes.
                    r.output = Some(compose_frame(title_line.as_ref(), &stdout, &stderr, is_tty));
                    r.hash = hash;
                    r.changed_at = outcome.at;
                    // Derived per outcome, exactly as a pane's badge is:
                    // a command that stops flooding stops saying so.
                    r.truncated = truncated.clone();
                    piped_dropped = truncated;
                    piped_stderr = stderr;
                    if outcome.status.is_some() {
                        plain_exit = Some(exit_badge(outcome.status));
                    }
                    changed_now
                }
                Composition::Panes { .. } => {
                    // A pane's failure fails inside its own box: the
                    // spawn-error text as the body, the exit badge on
                    // the chrome row — and the badge joins the pane's
                    // hash, because a pane can print identical bytes
                    // and START failing, and that is a displayed
                    // change. The comparand, marks, and last-change
                    // stamp move only on a distinct output.
                    let spec = registry.spec(id);
                    let program = spawn_program(spec);
                    // Concatenated for the same reason as the plain path
                    // above: the body is decoded whole, never per line.
                    let lines = pane_body(
                        &outcome.stdout.concat(),
                        &outcome.stderr.concat(),
                        outcome.spawn_error.as_ref(),
                        &spec.id,
                        &program,
                    );
                    record_pane_body(
                        &mut runtime[id.0],
                        lines,
                        exit_badge(outcome.status),
                        outcome.dropped,
                        outcome.at,
                    )
                }
            };
            let r = &mut runtime[id.0];
            // One clock per tick, stamped at COMPLETION on the worker:
            // the absolute stamp, the counting form, and the history
            // entry a scrub later shows all name the instant the
            // content became current — even when the completion waited
            // (behind a pager, say) to be collected. Only a source
            // whose OWN content changed may re-date the frame.
            // Each branch settled its own hash and stamp; the fold only
            // needs to know whether THIS source's content moved.
            changed = fold_changed_at(changed, changed_now, outcome.at);
            newest = newest.max(outcome.at);
            r.posted = true;
            moved.push(id);
            drained.push(id);
            // The bracket closes with the completion. Whatever moved between
            // its two snapshots moved while THIS child was running — and
            // while every child that overlapped it was running too, since
            // the snapshots place the change inside the window rather than
            // at an instant.
            if let Some(open) = r.bracket.take()
                && let Some(closed) = log
                    .close_bracket(open, outcome.closed_at, outcome.close_stamps)
                    .cloned()
            {
                let others = log.overlapping(&closed);
                ledger.observe_bracket(&closed, &others);
            }
        }
        // MOVEMENT opens the gate, not completion: a long-lived source
        // changes the frame without ever finishing, and gating on
        // completion is exactly why a pane fed by one has been painting
        // nothing at all. Every completion also moved, so this is strictly
        // wider than what it replaces and the batch path is unaffected.
        if !moved.is_empty() {
            // The close fence is what keeps a genuine outside writer
            // PROVABLE. Without it the lower bound stays at the last open
            // fence, every later interval overlaps that bracket, and no
            // observation could ever be disjoint from everything — so the
            // veto would silently stop firing. The two fences have opposite
            // jobs and both are required; dropping either disables one half
            // of the signal without disabling the other.
            //
            // COMPLETIONS ONLY. An emission closes no bracket, because no
            // child ran to completion, so fencing on one would narrow the
            // attribution window with nothing to attribute.
            #[cfg(unix)]
            if observing && !drained.is_empty() {
                for r in runtime.iter() {
                    fence_all(&r.readers);
                }
            }
            let content = combined_hash(&runtime);
            // The live status row names the ABSOLUTE local time of the
            // last content change: a counting age would change every
            // tick and defeat the repaint gate. A drain in which
            // nothing actually changed carries the previous stamp
            // forward; with no previous frame the newest completion
            // dates it, which is the shipped first-frame behavior.
            let (changed_at, since) = match (changed, live.take()) {
                (Some(at), _) => (at, local_hms(at)),
                (None, Some(prev)) => (prev.changed_at, prev.since),
                (None, None) => (newest, local_hms(newest)),
            };
            // The terminal size joins the paint key: a resize must
            // repaint even when the content is unchanged. So does the
            // appearance: a palette swap must repaint even when the
            // child prints the same bytes. Without a resize arm this
            // is the collect-time measure shipped watch takes — the
            // same single-writer mode the spawn step uses; with one,
            // the arm keeps the pair fresh every iteration.
            refresh_geometry_for_spawn(
                session.resize_respawn,
                measure_size(is_tty, size),
                gutter_reserve(view.gutter),
                &mut size,
                &mut geom,
                &registry,
            );
            // The bodies just changed and the geometry is fresh: clamp
            // every pane's window into the new shape before it composes.
            reanchor_pane_scrolls(&mut panes, &runtime, &geom);
            // Composed once, above the repaint gate: the newest frame
            // is tracked on every completion, so paging always acts on
            // the newest content. Combining the single source's output
            // is the identity — the bytes cannot drift.
            let (lines, pane_live) = match registry.composition() {
                Composition::Plain { .. } => (runtime[0].output.clone().unwrap_or_default(), None),
                Composition::Panes { .. } => {
                    let block = compose_sources(
                        &registry,
                        &runtime,
                        &geom,
                        &panes,
                        view.alt_time,
                        &palette,
                        profile,
                    );
                    (
                        block.lines,
                        Some(PaneLive {
                            marks: block.marks,
                            ages: chrome_ages(&registry, &runtime),
                        }),
                    )
                }
            };
            let current = Live {
                lines,
                hash: content,
                changed_at,
                since,
                panes: pane_live,
                // Plain only: under panes the marker is on the pane that
                // overflowed, which is the question a reader would ask.
                dropped: match registry.composition() {
                    Composition::Plain { .. } => runtime[0].truncated.clone(),
                    Composition::Panes { .. } => None,
                },
            };
            if is_tty && !append {
                // Every distinct frame is retained (byte-capped,
                // deduped) so the scrub keys can walk back through it.
                // Scrub, freeze, and the change marks are the ring's
                // only readers, and append answers none of those keys —
                // the terminal's scrollback is the review surface.
                history.record(current.hash, &current.lines, newest);
            }
            // A resize while frozen must not leave the window past the frame.
            if let Some(p) = pause.as_mut() {
                let window = usize::from(window_rows(session.max_height, size.1));
                p.scroll = p.scroll.clamp(p.frozen.len(), window);
            }
            // A live window rides the tail whatever shape the frame takes:
            // a pinned window tracks the end, an unpinned one holds its
            // offset clamped into the new shape, and reaching the top
            // collapses to the live view. Freezing is never implicit — the
            // history ring holds any moment that slides away.
            if let Some(ls) = live_scroll {
                let window = usize::from(window_rows(session.max_height, size.1));
                let re = ls.reanchor(current.lines.len(), window);
                live_scroll = (!re.at_top()).then_some(re);
            }
            // While frozen the key holds the freeze-time content/appearance:
            // new child output and adopted palettes do not repaint, but
            // scroll, resize, the aging paused row, and the one-shot notice
            // still do.
            let key = paint_key(
                pause.as_ref(),
                live_scroll,
                current.hash,
                palette.appearance,
                size,
                view,
                panes.key(),
                displayed_age_key(
                    pause.as_ref(),
                    live_scroll,
                    view.alt_time,
                    current.changed_at,
                    current.panes.as_ref().map_or(&[][..], |p| &p.ages),
                ),
            );
            // Once mode emits exactly ONE complete frame: a partial
            // wave (some panes still running) composes and records but
            // must not reach the terminal or the pipe.
            let once_ready = !session.once || runtime.iter().all(|r| r.posted);
            if append {
                // The linear stream: one write per DISTINCT frame, and
                // never a rewrite. The whole frame appends (measured
                // shape), sealed by append_frame, terminated for the
                // live line discipline.
                if once_ready && previous_append != Some(current.hash) {
                    previous_append = Some(current.hash);
                    append_rows(
                        &mut std::io::stdout().lock(),
                        append_frame(&current.lines, current.dropped.as_deref()),
                        eol,
                    )?;
                }
                // The exit status has its OWN gate: a command can print
                // byte-identical output and START failing.
                if once_ready && let Some(now) = plain_exit.take() {
                    if let Some(row) = append_exit_line(prev_exit.as_deref(), now.as_deref()) {
                        append_rows(&mut std::io::stdout().lock(), vec![row], eol)?;
                    }
                    prev_exit = now;
                }
            } else if once_ready && previous_key != Some(key) {
                previous_key = Some(key);
                if is_tty {
                    repaint(
                        &mut renderer,
                        pause.as_ref(),
                        live_scroll,
                        &current,
                        &live_tail,
                        &palette,
                        view,
                        panes.key(),
                        focus_segment(&registry, &runtime, &geom, &panes).as_deref(),
                        None,
                        size,
                        session.max_height,
                        fullscreen,
                        &faint,
                        profile,
                        &history,
                    )?;
                } else {
                    let mut out = std::io::stdout().lock();
                    for line in &current.lines {
                        writeln!(out, "{line}").context("writing")?;
                    }
                    out.flush().context("flushing")?;
                    // Piped mode keeps the streams separate for log
                    // readability — under Plain only: a pane's stderr is
                    // already inside its own box (D-12 as sharpened:
                    // gate on the composition, not the source count).
                    if plain && !piped_stderr.is_empty() {
                        let mut err = std::io::stderr().lock();
                        err.write_all(&piped_stderr).context("writing stderr")?;
                        err.flush().context("flushing stderr")?;
                    }
                    // A drop is never silent. There is no status row
                    // here to carry it and stdout belongs to whoever is
                    // reading the data, so it goes where a diagnostic
                    // goes. A plain watch always keeps the newest, so
                    // naming which end survived costs one clause and
                    // saves a guess.
                    if plain && let Some(text) = &piped_dropped {
                        let mut err = std::io::stderr().lock();
                        writeln!(err, "rat watch: {text}; kept the last {MAX_RETAINED_LINES}")
                            .context("writing stderr")?;
                        err.flush().context("flushing stderr")?;
                    }
                }
            }
            live = Some(current);
            // The deadline is set when a tick COMPOSES, not when it is
            // drained: the fixed delay counts from the frame the reader
            // actually saw.
            //
            // `drained`, never `moved` — the one line I-96 exists to
            // protect. A source that emitted is still running, and
            // telling the scheduler it completed spawns a second child
            // beside it.
            for id in &drained {
                runtime[id.0].schedule.completed(Instant::now());
            }
            if session.once && runtime.iter().all(|r| r.posted) {
                break;
            }
        }
        // 3b. External triggers: collapse fires into one respawn
        // request per debounce window. Sits BEFORE the non-interactive
        // branch below so a piped watch refreshes on file changes too;
        // the spawn this requests happens on the next iteration's step
        // 2, at most one slice away.
        {
            let now = Instant::now();
            // Classify at slice cadence, but ONLY while nothing is in flight.
            // That is the idle case, and it is the only one that can produce
            // an EXOGENOUS observation — proof that something outside the
            // dashboard writes this path. While a child runs, attribution
            // comes from its bracket instead; classifying here would credit
            // the change to nobody and wrongly clear the veto.
            if !watched_union.is_empty() && !log.any_open(now) {
                ledger.observe(now, &[]);
            }
            // Whether a badge moved this iteration, so the one paint
            // below knows it has something to show even with no notice.
            let mut badge_moved = false;
            // The iteration's one-shot rows, batched into ONE notice.
            // PORTABLE, unlike the reader end-of-life lines it collects:
            // only fifo/fd readers can end, but a `file:` trigger loops
            // on every platform, so a `cfg(unix)` batch would lose the
            // loop report on Windows entirely.
            let mut notices: Vec<String> = Vec::new();
            for id in registry.ids() {
                let r = &mut runtime[id.0];
                // Before the gate consumes `fired`: the drain is a
                // separate output of the same reader, and one pass over
                // the slots touches each reader once.
                #[cfg(unix)]
                drain_reader_arrivals(&r.readers, &mut log, now);
                #[cfg(unix)]
                for slot in &mut r.readers {
                    if slot.reader.fired().swap(false, Ordering::SeqCst) {
                        r.gate.fire(now);
                    }
                    if !slot.ended_seen && slot.reader.ended().load(Ordering::SeqCst) {
                        slot.ended_seen = true; // rising edge, per reader
                        notices.push(ended_text(&registry, id, &slot.spec));
                    }
                }
                if r.files.fired() {
                    r.gate.fire(now);
                }
                if r.gate.due(now) {
                    // A fired trigger observed a change any in-flight
                    // child of THIS source predates: a respawn, never a
                    // plain request.
                    r.schedule.request_respawn();
                    // A live child never completes on its own, so the
                    // request above would wait out single-in-flight
                    // forever. The revocable kill is what discharges
                    // it: the killed child's completion hands the slot
                    // back, and the surviving request spawns the
                    // replacement. The kill is TERM-first with a
                    // bounded force, so a compliant child flushes
                    // before the replacement spawns. A batch child is
                    // NOT killed — its own completion discharges the
                    // request, as it always has.
                    if registry.spec(id).live {
                        r.slot.kill_current(SUPERSEDE_GRACE);
                    }
                    // The one site where a respawn is trigger-driven. Recorded
                    // as an EVENT, never a running count: a count that cannot
                    // fall could never let a repaired dashboard stop being
                    // suspected. An `interval` pane reaches its deadline
                    // without passing through this gate, which is what excludes
                    // a legitimately fast pane for free.
                    log.record_respawn(id, now);
                }
            }
            // One eviction pass per iteration bounds every windowed quantity.
            ledger.evict(now, suspicion.window);
            log.evict(now);
            // Then one evaluation, for the whole dashboard: the graph it tests
            // is a whole-dashboard object, not a per-source one.
            if observing {
                let pane_windows: Vec<crate::core::trigger::PaneWindow<'_>> = registry
                    .ids()
                    .map(|id| crate::core::trigger::PaneWindow {
                        source: id,
                        // Read from the window, never from a stored counter: a
                        // count that cannot fall could never let a repaired
                        // dashboard stop being suspected.
                        trigger_respawns: log.respawns_in_window(id, now),
                        watched: &per_source_watched[id.0],
                        readers: &per_source_readers[id.0],
                    })
                    .collect();
                let verdict = suspicion.evaluate(now, &ledger, &log, &pane_windows);
                if let Some(t) = trace.as_mut() {
                    t.record(now, &verdict);
                }
                // A badge appearing or clearing is a displayed change,
                // and it is decided HERE — with no outcome to ride. So
                // the transition recomposes from the retained outputs,
                // and the paint below puts it on screen.
                badge_moved =
                    apply_verdict(&mut runtime, &verdict.panes, !plain, jiff::Timestamp::now());
                if badge_moved {
                    recompose_live(
                        &mut live,
                        &registry,
                        &runtime,
                        &geom,
                        &panes,
                        view.alt_time,
                        &palette,
                        profile,
                    );
                    if let Some(l) = live.as_mut() {
                        restamp_live(l, &runtime);
                    }
                }
                // Badge = state, notice = event. The row is one-shot —
                // the next in-place repaint drops it — which is the
                // wrong home for a state and the right one for an event,
                // so only the RISING edge speaks.
                if rising_edge(&mut suspected, &verdict) {
                    notices.push(looping_text(&registry, &verdict.panes));
                }
            }
            // One paint for the iteration's out-of-band changes: the
            // batched one-shot rows, a badge that moved, or both. With
            // no frame yet there is nothing to paint over and the batch
            // drops. Piped output is left alone entirely — a notice is
            // chrome, and the piped contract is frame lines. (The badge
            // still reaches a piped dashboard: it is IN those lines.)
            if append {
                // Chrome events become their own rows: this is the only
                // surface here, and a one-shot row nobody paints is a
                // row nobody hears. ONE LINE EACH, never joined with
                // " · " — a listener parses lines. `badge_moved` is not
                // consulted: apply_verdict takes `boxed = !plain` and
                // returns false whenever it is false, so a plain
                // watch's badge can never move.
                if !notices.is_empty() {
                    let rows: Vec<String> = notices.iter().map(|n| append_notice(n)).collect();
                    append_rows(&mut std::io::stdout().lock(), rows, eol)?;
                }
            } else if is_tty
                && let (true, Some(l)) = (!notices.is_empty() || badge_moved, live.as_ref())
            {
                previous_key = Some(repaint(
                    &mut renderer,
                    pause.as_ref(),
                    live_scroll,
                    l,
                    &live_tail,
                    &palette,
                    view,
                    panes.key(),
                    focus_segment(&registry, &runtime, &geom, &panes).as_deref(),
                    (!notices.is_empty()).then(|| notices.join(" · ")),
                    crossterm::terminal::size().unwrap_or((80, 24)),
                    session.max_height,
                    fullscreen,
                    &faint,
                    profile,
                    &history,
                )?);
            }
        }
        // 3c. Resize: reflow NOW from retained outputs, respawn once
        // the size settles. This arm is the geometry pair's ONLY
        // writer in this mode (the spawn step's re-measure is gated
        // off), so a spawn coinciding with a resize can never consume
        // the new size first and blind this detection. Placement:
        // before the non-interactive branch, like the triggers.
        if session.resize_respawn {
            let measured = measure_size(is_tty, size);
            let step = detect_resize(
                measured,
                session.max_height,
                view.gutter,
                &panes,
                &mut size,
                &mut geom,
                &registry,
            );
            if step.geom_moved {
                resize_gate.fire(Instant::now());
                // Every pane's window moved with its inner_rows.
                reanchor_pane_scrolls(&mut panes, &runtime, &geom);
            }
            if step.size_moved
                && is_tty
                && let Some(l) = live.as_mut()
            {
                if step.geom_moved {
                    // Re-composed, never re-dated: the change key
                    // is output-derived and history records only
                    // collect-step compositions, so nothing here
                    // records and the stamps are untouched.
                    let block = compose_sources(
                        &registry,
                        &runtime,
                        &geom,
                        &panes,
                        view.alt_time,
                        &palette,
                        profile,
                    );
                    l.lines = block.lines;
                    l.panes = Some(PaneLive {
                        marks: block.marks,
                        ages: chrome_ages(&registry, &runtime),
                    });
                }
                // A frozen frame re-clamps only — it is a literal
                // copy and is never re-laid-out; a live window
                // reanchors.
                let window = usize::from(window_rows(session.max_height, size.1));
                if let Some(p) = pause.as_mut() {
                    p.scroll = p.scroll.clamp(p.frozen.len(), window);
                }
                if let Some(ls) = live_scroll {
                    let re = ls.reanchor(l.lines.len(), window);
                    live_scroll = (!re.at_top()).then_some(re);
                }
                // The notice-row pattern: paint in place and take
                // the key the paint returned, so the gate cannot be
                // left behind. The gate itself cannot serve here —
                // the content key is output-derived, so a pure
                // reflow leaves it equal.
                previous_key = Some(repaint(
                    &mut renderer,
                    pause.as_ref(),
                    live_scroll,
                    l,
                    &live_tail,
                    &palette,
                    view,
                    panes.key(),
                    focus_segment(&registry, &runtime, &geom, &panes).as_deref(),
                    None,
                    size,
                    session.max_height,
                    fullscreen,
                    &faint,
                    profile,
                    &history,
                )?);
            }
            if resize_gate.due(Instant::now()) {
                // Every in-flight child was started under the
                // superseded geometry and cannot satisfy this — the
                // theme arm's argument. A respawn, not a plain request.
                request_respawn_all(&mut runtime);
            }
        }
        // 3c'. Zoom's debounced respawns: per pane (D3), and only panes
        // that will run again. A live child is not killed by a view
        // gesture — the gutter toggle and the resize arm both already
        // declined that, and a zoom is a more emphatic view toggle, not
        // less (INV-5). `due` is called BEFORE the live test so a live
        // pane's fired window still clears instead of going stale.
        {
            let now = Instant::now();
            for id in registry.ids() {
                if zoom_gates[id.0].due(now) && !registry.spec(id).live {
                    runtime[id.0].schedule.request_respawn();
                }
            }
        }
        // 3d. A quiet `--once` says what it is waiting on: one batched
        // stderr line, once, and nothing else moves — no exit change,
        // no stdout byte, no timing (the nap below is computed the
        // same either way). Gated on panes: plain `rat watch --once`
        // runs its tick inline and never reaches this loop, and the
        // `rat dashboard: ` prefix would lie there anyway.
        if session.once && !plain && !once_notice_sent && once_started.elapsed() >= ONCE_QUIET {
            let waiting: Vec<SourceId> =
                registry.ids().filter(|id| !runtime[id.0].posted).collect();
            if !waiting.is_empty() {
                eprintln!("{}", once_waiting_text(&registry, &waiting, ONCE_QUIET));
            }
            once_notice_sent = true;
        }
        // The opt-in bound: exit 124 with the wait named, stdout EMPTY
        // — a partial frame is a lie in the mode whose stdout is the
        // frame, and the once paint gate has written nothing yet.
        // Children die via the shutdown guards on the way out.
        if let Some(bound) = session.once_timeout
            && session.once
            && !plain
            && once_started.elapsed() >= bound
        {
            let waiting: Vec<SourceId> =
                registry.ids().filter(|id| !runtime[id.0].posted).collect();
            if !waiting.is_empty() {
                renderer.finish().context("restoring terminal")?;
                return Err(AppError::Timeout(Some(anyhow!(
                    "{}",
                    once_timeout_text(&registry, &waiting, bound)
                ))));
            }
        }
        // 3e. A pane-sourced tab title follows the role: recomputed
        // per pass, re-emitted only when the text changes (the guard
        // is idempotent per text), so a calm dashboard writes nothing.
        if let Some(guard) = tab_title.as_mut()
            && let Some(stem) = session.tab_title.as_deref()
            && let Some(source) = registry.title_source()
        {
            let text = crate::term::tab_title::tab_title_text(
                title_role_text(source, &runtime).as_deref(),
                stem,
            );
            let _ = guard.set(&text);
        }
        // 4. How long we may sleep: never past the SOONEST deadline,
        // never past one slice, so a signal, a key, and a completing
        // child are all noticed promptly — and no source waits on
        // another's cadence.
        let nap = runtime
            .iter()
            .map(|r| r.schedule.nap(Instant::now(), SLICE))
            .min()
            .unwrap_or(SLICE);
        if !interactive {
            std::thread::sleep(nap);
            continue;
        }
        // 5. When the displayed row counts — either row, under the
        // flipped time style — the age advances once per second,
        // riding this nap cycle: a long-interval dashboard must not
        // wait a whole tick to admit how stale it is. The only
        // visible delta is the status row, so the repaint is bounded
        // to status-row bytes (and to none at all while the text
        // holds). Under the default stamps nothing here ever fires.
        if let Some(prev) = previous_key
            && let Some(l) = live.as_ref()
        {
            let want_age = displayed_age_key(
                pause.as_ref(),
                live_scroll,
                view.alt_time,
                l.changed_at,
                l.panes.as_ref().map_or(&[][..], |p| &p.ages),
            );
            if prev.age_secs != want_age {
                // A pane's counting stamp lives inside the composed
                // lines, so the refresh recomposes before it repaints;
                // under the default absolute style the key is 0 and
                // none of this ever runs.
                recompose_live(
                    &mut live,
                    &registry,
                    &runtime,
                    &geom,
                    &panes,
                    view.alt_time,
                    &palette,
                    profile,
                );
                let l = live.as_ref().expect("checked above");
                previous_key = Some(repaint(
                    &mut renderer,
                    pause.as_ref(),
                    live_scroll,
                    l,
                    &live_tail,
                    &palette,
                    view,
                    panes.key(),
                    focus_segment(&registry, &runtime, &geom, &panes).as_deref(),
                    None,
                    crossterm::terminal::size().unwrap_or((80, 24)),
                    session.max_height,
                    fullscreen,
                    &faint,
                    profile,
                    &history,
                )?);
            }
        }
        #[cfg(unix)]
        if let Some(sub) = theme_sub.as_mut() {
            if verify.pending && verify.in_flight_until.is_none() {
                verify.pending = false;
                // Ask once, and only while we own the input: the
                // replies land in our own reader and nowhere else.
                if sub.request_colors().is_ok() {
                    verify.fg = None;
                    verify.in_flight_until = Some(Instant::now() + crate::theme::PROBE_TIMEOUT);
                }
            }
            if verify
                .in_flight_until
                .is_some_and(|until| Instant::now() >= until)
            {
                // The terminal did not answer. A later report can arm
                // another exchange.
                verify.in_flight_until = None;
                verify.fg = None;
            }
        }
        #[cfg(unix)]
        let events = match tap.as_ref() {
            Some(tap) => {
                let waited = Instant::now();
                match tap.recv_timeout(nap) {
                    Some(TapChunk::Tty(chunk)) => scanner.feed(&chunk),
                    // An early wake is NOT a timed-out slice: account
                    // the real elapsed silence so a pending bare ESC
                    // still resolves honestly. The wake itself carries
                    // nothing — the trigger block reads the fired flag.
                    Some(TapChunk::Trigger) => scanner.idle(waited.elapsed()),
                    None => scanner.idle(nap),
                }
            }
            None => crossterm_slice(nap)?,
        };
        #[cfg(windows)]
        let events = crossterm_slice(nap)?;
        // A fast wheel spin delivers several notches in one read; fold
        // each same-direction run into one stepped action so a burst
        // repaints once, not once per notch.
        let events = fold_wheel(events);
        for event in events {
            if append {
                // Append mode's own dispatch, BEFORE the shared match,
                // so the painted table stays byte-frozen. The `continue`
                // also drops Mouse and theme events — none can arrive
                // (--mouse conflicts; the subscription is gated off) —
                // and keeps every painted key arm unreachable.
                if let TapEvent::Key(key) = event {
                    match append_action_for(key) {
                        AppendAction::Abort => {
                            // A near no-op — nothing was hidden — but
                            // every exit path restores through the same
                            // call; an exit that reads differently is
                            // one that drifts.
                            renderer.finish().context("restoring terminal")?;
                            return Err(AppError::Aborted);
                        }
                        AppendAction::Quit => {
                            renderer.finish().context("restoring terminal")?;
                            return Ok(());
                        }
                        AppendAction::Snapshot => {
                            let Some(l) = live.as_ref() else { continue };
                            let text = snapshot_frame(
                                &l.lines,
                                session.snapshot_dir.as_deref(),
                                session.snapshot_ansi,
                            );
                            append_rows(
                                &mut std::io::stdout().lock(),
                                vec![append_notice(&text)],
                                eol,
                            )?;
                        }
                        AppendAction::Help => {
                            append_rows(
                                &mut std::io::stdout().lock(),
                                append_help_lines(&session.help_extra),
                                eol,
                            )?;
                        }
                        AppendAction::Ignore => {}
                    }
                }
                continue;
            }
            match event {
                TapEvent::Key(_) | TapEvent::Mouse(_) => {
                    let action = match event {
                        TapEvent::Key(key) => action_for(key, mode_of(pause.as_ref(), live_scroll)),
                        // Gated on LIVE capture, not just the flag: a
                        // terminal that keeps reporting after a release
                        // (or that was never asked) must change nothing.
                        TapEvent::Mouse(ev) if mouse_guard.as_ref().is_some_and(|g| g.active()) => {
                            action_for_mouse(ev)
                        }
                        TapEvent::Mouse(_) => WatchAction::Ignore,
                        _ => unreachable!("matched above"),
                    };
                    match action {
                        WatchAction::Abort => {
                            renderer.finish().context("restoring terminal")?;
                            return Err(AppError::Aborted);
                        }
                        WatchAction::Quit => {
                            renderer.finish().context("restoring terminal")?;
                            return Ok(());
                        }
                        action @ (WatchAction::Page | WatchAction::Help) => {
                            // ? pages the key reference through the same
                            // ritual v pages the frame — one handoff path,
                            // and search over the bindings comes free. The
                            // reference needs no frame: it is static text,
                            // and the window before the first frame is
                            // exactly when a reader reaches for it.
                            let help;
                            let content: &[String] = if action == WatchAction::Help {
                                help = help_lines(session.help_heading, &session.help_extra);
                                &help
                            } else if let Some(id) =
                                pager_target(mode_of(pause.as_ref(), live_scroll), &panes)
                            {
                                // The pane's WHOLE retained body, not its
                                // window: the honest answer to a KeepTop
                                // pane whose retention gate shut, to a
                                // body too long to step through, and to a
                                // collapsed pane.
                                runtime[id.0].output.as_deref().unwrap_or(&[])
                            } else {
                                let Some(live) = live.as_ref() else { continue };
                                pause.as_ref().map_or(&live.lines, |p| &p.frozen)
                            };
                            // The pager reads the same terminal. Stop the
                            // pushes first, then park our reader, so a report
                            // can never land in a foreign reader's input.
                            #[cfg(unix)]
                            if let Some(sub) = theme_sub.as_mut() {
                                let _ = sub.suspend();
                            }
                            // The pager doesn't speak SGR mouse reports;
                            // leaving tracking on sprays bytes into its
                            // command line. Remember whether capture was
                            // ours to restore — an m-released mouse must
                            // stay released across the round trip.
                            let mouse_was_active =
                                mouse_guard.as_ref().is_some_and(MouseGuard::active);
                            if let Some(guard) = mouse_guard.as_mut() {
                                let _ = guard.suspend();
                            }
                            // Park our reader and require its confirmation
                            // before handing the input stream over.
                            // Unconfirmed means a reader may still be attached
                            // — never spawn a second one against it.
                            #[cfg(unix)]
                            let handed_off = tap.as_ref().is_none_or(|tap| tap.pause());
                            #[cfg(windows)]
                            let handed_off = true;
                            let pager_notice = if handed_off {
                                page_frame(content, &mut renderer, fullscreen)
                            } else {
                                Some(
                                    "pager unavailable: the input reader did not yield; try again"
                                        .to_string(),
                                )
                            };
                            // Reader first, then pushes: a report always has
                            // someone to read it.
                            #[cfg(unix)]
                            {
                                if let Some(tap) = tap.as_ref() {
                                    tap.resume();
                                }
                                if let Some(sub) = theme_sub.as_mut() {
                                    let _ = sub.resume();
                                }
                                // Whatever was in flight belongs to a terminal
                                // state we stopped listening to.
                                verify = VerifyState::default();
                            }
                            if mouse_was_active && let Some(guard) = mouse_guard.as_mut() {
                                let _ = guard.resume();
                            }
                            // Repaint immediately from the frame on hand:
                            // the content is already current, and a forced
                            // tick would stall the return by a whole child
                            // runtime on a slow dashboard. The pager left the
                            // diff invalidated, so this paints the full frame
                            // over the restored copy. With no frame yet there
                            // is nothing to restore: page_frame already called
                            // renderer.finish(), so the loop simply paints its
                            // first frame when the first child lands.
                            if let Some(live) = live.as_ref() {
                                let size = crossterm::terminal::size().unwrap_or((80, 24));
                                previous_key = Some(repaint(
                                    &mut renderer,
                                    pause.as_ref(),
                                    live_scroll,
                                    live,
                                    &live_tail,
                                    &palette,
                                    view,
                                    panes.key(),
                                    focus_segment(&registry, &runtime, &geom, &panes).as_deref(),
                                    pager_notice,
                                    size,
                                    session.max_height,
                                    fullscreen,
                                    &faint,
                                    profile,
                                    &history,
                                )?);
                            }
                        }
                        action @ (WatchAction::Scroll(_) | WatchAction::ScrollN(..)) => {
                            let (step, n) = match action {
                                WatchAction::Scroll(step) => (step, 1),
                                WatchAction::ScrollN(step, n) => (step, n),
                                _ => unreachable!("matched above"),
                            };
                            // With a pane focused the step addresses THAT
                            // pane's window over its own retained body;
                            // the wheel arrives as the same actions and
                            // follows. Recompose and repaint in place —
                            // the gutter toggle's shape — because pane
                            // identity is gone after `compose_panes`.
                            if let Some(id) =
                                scroll_target(mode_of(pause.as_ref(), live_scroll), &panes)
                            {
                                // A focused pane owns the keys even while
                                // collapsed; its window is not on screen
                                // to move, so the step declines HERE —
                                // never by falling back to the whole
                                // frame (INV-7).
                                if panes.collapsed[id.0] {
                                    continue;
                                }
                                let total = runtime[id.0].output.as_ref().map_or(0, Vec::len);
                                let window = geom[id.0].inner_rows as usize;
                                for _ in 0..n {
                                    panes.scroll[id.0] =
                                        panes.scroll[id.0].step(step, total, window);
                                }
                                recompose_live(
                                    &mut live,
                                    &registry,
                                    &runtime,
                                    &geom,
                                    &panes,
                                    view.alt_time,
                                    &palette,
                                    profile,
                                );
                                let Some(live) = live.as_ref() else { continue };
                                let size = crossterm::terminal::size().unwrap_or((80, 24));
                                previous_key = Some(repaint(
                                    &mut renderer,
                                    pause.as_ref(),
                                    live_scroll,
                                    live,
                                    &live_tail,
                                    &palette,
                                    view,
                                    panes.key(),
                                    focus_segment(&registry, &runtime, &geom, &panes).as_deref(),
                                    None,
                                    size,
                                    session.max_height,
                                    fullscreen,
                                    &faint,
                                    profile,
                                    &history,
                                )?);
                                continue;
                            }
                            let Some(live) = live.as_ref() else { continue };
                            // The repaint happens here, in place —
                            // re-entering the tick loop would re-run the
                            // child per keypress; a stepped action (one
                            // wheel notch is three lines) repaints once.
                            // A frozen window scrolls its copy; otherwise
                            // scrolling is always a live viewport —
                            // freezing is explicit (p or <), never a side
                            // effect of navigation.
                            let size = crossterm::terminal::size().unwrap_or((80, 24));
                            let window = usize::from(window_rows(session.max_height, size.1));
                            let was_live = pause.is_none() && live_scroll.is_none();
                            for _ in 0..n {
                                if let Some(p) = pause.as_mut() {
                                    p.scroll = p.scroll.step(step, p.frozen.len(), window);
                                } else if let Some(ls) = live_scroll {
                                    let stepped = ls.step(step, live.lines.len(), window);
                                    live_scroll = (!stepped.at_top()).then_some(stepped);
                                } else {
                                    let ls = LiveScroll::start(step, live.lines.len(), window);
                                    if ls.at_top() {
                                        // A top-reaching entry never
                                        // enters the mode — stay Live.
                                        break;
                                    }
                                    live_scroll = Some(ls);
                                }
                            }
                            if was_live && pause.is_none() && live_scroll.is_none() {
                                // Never left the live view: nothing to
                                // paint — including any scroll over a
                                // frame that fits the window.
                                continue;
                            }
                            previous_key = Some(repaint(
                                &mut renderer,
                                pause.as_ref(),
                                live_scroll,
                                live,
                                &live_tail,
                                &palette,
                                view,
                                panes.key(),
                                focus_segment(&registry, &runtime, &geom, &panes).as_deref(),
                                None,
                                size,
                                session.max_height,
                                fullscreen,
                                &faint,
                                profile,
                                &history,
                            )?);
                        }
                        WatchAction::Resume => {
                            let Some(live) = live.as_ref() else { continue };
                            // A pause shows genuinely stale content: ask for
                            // a fresh tick — satisfied by an in-flight child
                            // if one is running. EITHER way the collapse
                            // paints NOW, from the frame on hand: the key
                            // must visibly answer even while a slow child is
                            // still running.
                            if pause.take().is_some() {
                                request_now_all(&mut runtime);
                            }
                            live_scroll = None;
                            let size = crossterm::terminal::size().unwrap_or((80, 24));
                            previous_key = Some(repaint(
                                &mut renderer,
                                pause.as_ref(),
                                live_scroll,
                                live,
                                &live_tail,
                                &palette,
                                view,
                                panes.key(),
                                focus_segment(&registry, &runtime, &geom, &panes).as_deref(),
                                None,
                                size,
                                session.max_height,
                                fullscreen,
                                &faint,
                                profile,
                                &history,
                            )?);
                        }
                        WatchAction::Freeze => {
                            let Some(live) = live.as_ref() else { continue };
                            // A deliberate park: read a changing value in
                            // place. From a live window it freezes at the
                            // current offset; from the live view at zero.
                            let size = crossterm::terminal::size().unwrap_or((80, 24));
                            let window = usize::from(window_rows(session.max_height, size.1));
                            let offset = live_scroll.map_or(0, LiveScroll::offset);
                            pause.get_or_insert_with(|| PauseState {
                                frozen: live.lines.clone(),
                                scroll: ScrollState::at(offset).clamp(live.lines.len(), window),
                                content: live.hash,
                                appearance: palette.appearance,
                                viewed_at: jiff::Timestamp::now(),
                                history_seq: history.newest_seq(),
                            });
                            live_scroll = None;
                            previous_key = Some(repaint(
                                &mut renderer,
                                pause.as_ref(),
                                live_scroll,
                                live,
                                &live_tail,
                                &palette,
                                view,
                                panes.key(),
                                focus_segment(&registry, &runtime, &geom, &panes).as_deref(),
                                None,
                                size,
                                session.max_height,
                                fullscreen,
                                &faint,
                                profile,
                                &history,
                            )?);
                        }
                        action @ (WatchAction::ScrubBack | WatchAction::ScrubForward) => {
                            let Some(live) = live.as_ref() else { continue };
                            // A scrub is a pause with a cursor: park on
                            // a neighboring DISTINCT frame. The anchor
                            // is what the eye is on — the pause's entry
                            // (re-resolved through `nearest` if
                            // evicted), else the newest — so `<` always
                            // means "older than what I am looking at".
                            let anchor = pause
                                .as_ref()
                                .and_then(|p| p.history_seq)
                                .and_then(|seq| history.nearest(seq).map(|e| e.seq))
                                .or_else(|| history.newest_seq());
                            let Some(anchor) = anchor else { continue };
                            let entry = if action == WatchAction::ScrubBack {
                                history.prev(anchor)
                            } else {
                                history.next(anchor)
                            };
                            let size = crossterm::terminal::size().unwrap_or((80, 24));
                            let window = usize::from(window_rows(session.max_height, size.1));
                            let Some(entry) = entry else {
                                // Behind the oldest there is a genuine
                                // wall. Past the newest the frame on
                                // screen IS the live frame, so > steps
                                // out of the freeze at the carried
                                // offset — the same walk, one key, all
                                // the way back; Esc and F stay the
                                // reset-to-top exits.
                                if action == WatchAction::ScrubBack {
                                    continue;
                                }
                                let Some(p) = pause.take() else { continue };
                                request_now_all(&mut runtime);
                                let ls =
                                    LiveScroll::at(p.scroll.offset(), live.lines.len(), window);
                                live_scroll = (!ls.at_top()).then_some(ls);
                                previous_key = Some(repaint(
                                    &mut renderer,
                                    pause.as_ref(),
                                    live_scroll,
                                    live,
                                    &live_tail,
                                    &palette,
                                    view,
                                    panes.key(),
                                    focus_segment(&registry, &runtime, &geom, &panes).as_deref(),
                                    None,
                                    size,
                                    session.max_height,
                                    fullscreen,
                                    &faint,
                                    profile,
                                    &history,
                                )?);
                                continue;
                            };
                            // The scroll position is held across steps
                            // — a watched line stays under the eye.
                            let scroll = pause
                                .as_ref()
                                .map(|p| p.scroll)
                                .or_else(|| live_scroll.map(|ls| ScrollState::at(ls.offset())))
                                .unwrap_or_default();
                            pause = Some(PauseState {
                                frozen: entry.frame.clone(),
                                scroll: scroll.clamp(entry.frame.len(), window),
                                content: entry.sig,
                                appearance: pause
                                    .as_ref()
                                    .map_or(palette.appearance, |p| p.appearance),
                                viewed_at: entry.at,
                                history_seq: Some(entry.seq),
                            });
                            live_scroll = None;
                            previous_key = Some(repaint(
                                &mut renderer,
                                pause.as_ref(),
                                live_scroll,
                                live,
                                &live_tail,
                                &palette,
                                view,
                                panes.key(),
                                focus_segment(&registry, &runtime, &geom, &panes).as_deref(),
                                None,
                                size,
                                session.max_height,
                                fullscreen,
                                &faint,
                                profile,
                                &history,
                            )?);
                        }
                        WatchAction::ToggleCollapse => {
                            // With no focus there is no pane to act on,
                            // and no repaint to spend saying so (INV-12).
                            let Some(id) = panes.focus else { continue };
                            if live.is_none() {
                                continue;
                            }
                            // Render-only (INV-8): `geom` is NOT
                            // re-derived, the pane's spawn env and
                            // schedule are untouched, and the child
                            // keeps its width and its cadence. That
                            // non-event is the point — the next
                            // iteration's detect_resize derives the same
                            // geometry from the same state, compares
                            // equal, and never arms the 250ms gate.
                            panes.collapsed[id.0] = !panes.collapsed[id.0];
                            // INV-7 lists collapse/expand as a reanchor
                            // site. The clamp is a no-op in value here BY
                            // CONSTRUCTION — the window it reads is
                            // `geom[i].inner_rows`, which collapse leaves
                            // alone — and that is exactly why an expanded
                            // pane returns to the viewport it left.
                            reanchor_pane_scrolls(&mut panes, &runtime, &geom);
                            recompose_live(
                                &mut live,
                                &registry,
                                &runtime,
                                &geom,
                                &panes,
                                view.alt_time,
                                &palette,
                                profile,
                            );
                            let Some(live) = live.as_ref() else { continue };
                            let size = crossterm::terminal::size().unwrap_or((80, 24));
                            previous_key = Some(repaint(
                                &mut renderer,
                                pause.as_ref(),
                                live_scroll,
                                live,
                                &live_tail,
                                &palette,
                                view,
                                panes.key(),
                                focus_segment(&registry, &runtime, &geom, &panes).as_deref(),
                                None,
                                size,
                                session.max_height,
                                fullscreen,
                                &faint,
                                profile,
                                &history,
                            )?);
                        }
                        WatchAction::ToggleZoom => {
                            if live.is_none() {
                                continue;
                            }
                            // A gesture with no target is a no-op, never
                            // a guess (INV-12).
                            let Some(id) = panes.focus else { continue };
                            // z while zoomed unzooms: one key both ways.
                            panes.zoomed = (panes.zoomed != Some(id)).then_some(id);
                            // geom only — `size` stays the resize arm's,
                            // and this derivation uses the loop's size,
                            // so detect_resize compares EQUAL on the next
                            // iteration and the debounce gate stays
                            // unarmed (INV-1).
                            geom = derive_geometry(
                                &registry,
                                size,
                                session.max_height,
                                view.gutter,
                                &panes,
                            );
                            // Both directions moved this pane's width;
                            // the due check below owes it one honest run.
                            zoom_gates[id.0].fire(Instant::now());
                            // The pane's window just changed shape:
                            // clamp every viewport into it (INV-7).
                            reanchor_pane_scrolls(&mut panes, &runtime, &geom);
                            recompose_live(
                                &mut live,
                                &registry,
                                &runtime,
                                &geom,
                                &panes,
                                view.alt_time,
                                &palette,
                                profile,
                            );
                            let Some(live) = live.as_ref() else { continue };
                            let size = crossterm::terminal::size().unwrap_or((80, 24));
                            previous_key = Some(repaint(
                                &mut renderer,
                                pause.as_ref(),
                                live_scroll,
                                live,
                                &live_tail,
                                &palette,
                                view,
                                panes.key(),
                                focus_segment(&registry, &runtime, &geom, &panes).as_deref(),
                                None,
                                size,
                                session.max_height,
                                fullscreen,
                                &faint,
                                profile,
                                &history,
                            )?);
                        }
                        action @ (WatchAction::FocusNext
                        | WatchAction::FocusPrev
                        | WatchAction::FocusMove(_)
                        | WatchAction::ClearFocus) => {
                            // A pane gesture needs a boxed registry and
                            // a composed frame: plain watch has no pane
                            // to address.
                            let Composition::Panes {
                                layout,
                                gap,
                                row_gap,
                                ..
                            } = registry.composition()
                            else {
                                continue;
                            };
                            if live.is_none() {
                                continue;
                            }
                            // The ladder (INV-12): zoomed → unzoom and
                            // KEEP the focus, so a second Esc clears it;
                            // only then does the focus rung run.
                            if action == WatchAction::ClearFocus
                                && let Some(id) = panes.zoomed.take()
                            {
                                geom = derive_geometry(
                                    &registry,
                                    size,
                                    session.max_height,
                                    view.gutter,
                                    &panes,
                                );
                                // The restore moved this pane's width too.
                                zoom_gates[id.0].fire(Instant::now());
                                // And its window: clamp every viewport
                                // back into the declared shape (INV-7).
                                reanchor_pane_scrolls(&mut panes, &runtime, &geom);
                                recompose_live(
                                    &mut live,
                                    &registry,
                                    &runtime,
                                    &geom,
                                    &panes,
                                    view.alt_time,
                                    &palette,
                                    profile,
                                );
                                let Some(live) = live.as_ref() else { continue };
                                let size = crossterm::terminal::size().unwrap_or((80, 24));
                                previous_key = Some(repaint(
                                    &mut renderer,
                                    pause.as_ref(),
                                    live_scroll,
                                    live,
                                    &live_tail,
                                    &palette,
                                    view,
                                    panes.key(),
                                    focus_segment(&registry, &runtime, &geom, &panes).as_deref(),
                                    None,
                                    size,
                                    session.max_height,
                                    fullscreen,
                                    &faint,
                                    profile,
                                    &history,
                                )?);
                                continue;
                            }
                            // While zoomed the other panes are not on
                            // screen; moving focus onto a hidden surface
                            // is the unzoom-guard problem v1 declines
                            // (INV-12).
                            if panes.zoomed.is_some() {
                                continue;
                            }
                            let order = pane_order(layout);
                            let next = match action {
                                WatchAction::FocusNext => focus_cycle(panes.focus, &order, true),
                                WatchAction::FocusPrev => focus_cycle(panes.focus, &order, false),
                                WatchAction::FocusMove(dir) => match panes.focus {
                                    None => order.first().copied(),
                                    Some(from) => {
                                        let sizes = pane_block_sizes(&geom, &panes);
                                        let mut rects = vec![PaneRect::default(); registry.len()];
                                        pane_rects(
                                            layout,
                                            &sizes,
                                            *gap,
                                            *row_gap,
                                            (0, 0),
                                            &mut rects,
                                        );
                                        // No wrap: an edge move holds.
                                        focus_neighbor(from, dir, &rects, &order).or(Some(from))
                                    }
                                },
                                _ => None,
                            };
                            if next == panes.focus {
                                // Nothing moved — an Esc with no focus,
                                // a move at the edge, a cycle over one
                                // pane. No repaint, so the gate is not
                                // disturbed either.
                                continue;
                            }
                            panes.focus = next;
                            recompose_live(
                                &mut live,
                                &registry,
                                &runtime,
                                &geom,
                                &panes,
                                view.alt_time,
                                &palette,
                                profile,
                            );
                            let Some(live) = live.as_ref() else { continue };
                            previous_key = Some(repaint(
                                &mut renderer,
                                pause.as_ref(),
                                live_scroll,
                                live,
                                &live_tail,
                                &palette,
                                view,
                                panes.key(),
                                focus_segment(&registry, &runtime, &geom, &panes).as_deref(),
                                None,
                                size,
                                session.max_height,
                                fullscreen,
                                &faint,
                                profile,
                                &history,
                            )?);
                        }
                        action @ (WatchAction::ToggleWrap
                        | WatchAction::ShiftLeft
                        | WatchAction::ShiftRight
                        | WatchAction::ToggleGutter
                        | WatchAction::ToggleHighlight
                        | WatchAction::ToggleTime) => {
                            if live.is_none() {
                                continue;
                            }
                            // View state, not scrollback state: applies to
                            // live and frozen frames alike, never freezes
                            // the tail, repaints in place. Right shift is
                            // unclamped, like less; left clamps at zero.
                            match action {
                                WatchAction::ToggleWrap => view.wrap = !view.wrap,
                                WatchAction::ToggleGutter => view.gutter = !view.gutter,
                                WatchAction::ToggleHighlight => {
                                    view.highlight = !view.highlight;
                                }
                                WatchAction::ToggleTime => {
                                    view.alt_time = !view.alt_time;
                                }
                                WatchAction::ShiftLeft => {
                                    view.hshift = view.hshift.saturating_sub(HSHIFT_STEP);
                                }
                                _ => view.hshift += HSHIFT_STEP,
                            }
                            if action == WatchAction::ToggleGutter {
                                // The gutter's columns come out of the
                                // allocation budget: recompute geom
                                // under the new reservation and reflow
                                // NOW from retained outputs (the
                                // ToggleTime pathway). geom only —
                                // `size` stays the resize arm's, so a
                                // coinciding real resize keeps its
                                // respawn; detect_resize applies the
                                // same reservation, so the next
                                // iteration compares equal and the
                                // gate stays unarmed. Stale until the
                                // next tick by design: interval and
                                // trigger panes re-read geom at their
                                // next spawn, and a live pane keeps
                                // its two-column ellipsis rather than
                                // being killed by a view toggle.
                                geom = derive_geometry(
                                    &registry,
                                    size,
                                    session.max_height,
                                    view.gutter,
                                    &panes,
                                );
                                recompose_live(
                                    &mut live,
                                    &registry,
                                    &runtime,
                                    &geom,
                                    &panes,
                                    view.alt_time,
                                    &palette,
                                    profile,
                                );
                            }
                            if action == WatchAction::ToggleTime {
                                // One style across the whole surface:
                                // the footer's row is built at paint
                                // time, but a pane's chrome row is
                                // composed INTO the frame, so the flip
                                // has to reach the composition.
                                recompose_live(
                                    &mut live,
                                    &registry,
                                    &runtime,
                                    &geom,
                                    &panes,
                                    view.alt_time,
                                    &palette,
                                    profile,
                                );
                            }
                            let Some(live) = live.as_ref() else { continue };
                            let size = crossterm::terminal::size().unwrap_or((80, 24));
                            previous_key = Some(repaint(
                                &mut renderer,
                                pause.as_ref(),
                                live_scroll,
                                live,
                                &live_tail,
                                &palette,
                                view,
                                panes.key(),
                                focus_segment(&registry, &runtime, &geom, &panes).as_deref(),
                                None,
                                size,
                                session.max_height,
                                fullscreen,
                                &faint,
                                profile,
                                &history,
                            )?);
                        }
                        WatchAction::ToggleMouse => {
                            // Hand the mouse back (plain-drag selection
                            // returns to the terminal) or take it again:
                            // a two-keystroke round trip instead of a
                            // condition lasting the whole session.
                            // Unbound without --mouse.
                            let Some(guard) = mouse_guard.as_mut() else {
                                continue;
                            };
                            let text = if guard.active() {
                                let _ = guard.suspend();
                                "mouse released — the wheel scrolls the terminal; m recaptures"
                            } else {
                                let _ = guard.resume();
                                "mouse captured — the wheel scrolls the frame"
                            };
                            let Some(live) = live.as_ref() else { continue };
                            let size = crossterm::terminal::size().unwrap_or((80, 24));
                            previous_key = Some(repaint(
                                &mut renderer,
                                pause.as_ref(),
                                live_scroll,
                                live,
                                &live_tail,
                                &palette,
                                view,
                                panes.key(),
                                focus_segment(&registry, &runtime, &geom, &panes).as_deref(),
                                Some(text.to_string()),
                                size,
                                session.max_height,
                                fullscreen,
                                &faint,
                                profile,
                                &history,
                            )?);
                        }
                        WatchAction::Snapshot => {
                            let Some(live) = live.as_ref() else { continue };
                            // The frozen frame when paused, the newest one
                            // when live; the path (or failure) surfaces
                            // through the notice row of an in-place paint.
                            let text = snapshot_frame(
                                pause.as_ref().map_or(&live.lines, |p| &p.frozen),
                                session.snapshot_dir.as_deref(),
                                session.snapshot_ansi,
                            );
                            let size = crossterm::terminal::size().unwrap_or((80, 24));
                            previous_key = Some(repaint(
                                &mut renderer,
                                pause.as_ref(),
                                live_scroll,
                                live,
                                &live_tail,
                                &palette,
                                view,
                                panes.key(),
                                focus_segment(&registry, &runtime, &geom, &panes).as_deref(),
                                Some(text),
                                size,
                                session.max_height,
                                fullscreen,
                                &faint,
                                profile,
                                &history,
                            )?);
                        }
                        WatchAction::Ignore => {}
                    }
                }
                // The reported value is ignored on purpose: it can
                // disagree with the colors actually on screen. Every
                // report re-arms, including one that arrives while an
                // exchange is already out — a terminal's first report
                // of a change can be measured too early.
                #[cfg(unix)]
                TapEvent::ThemeNotification(_) => {
                    verify.pending = true;
                }
                #[cfg(unix)]
                TapEvent::OscColor(kind, color) => {
                    if let Some(verdict) = verify.reply(kind, color)
                        && adopt(&mut palette, verdict)
                    {
                        let debug_notice = std::env::var_os("RAT_DEBUG_APPEARANCE")
                            .is_some()
                            .then(|| format!("appearance → {}", verdict.as_str()));
                        // Our own chrome flips at once; the fresh child
                        // this requests re-renders under the new
                        // environment. A RESPAWN, not a plain request: the
                        // in-flight child was started under the old
                        // RAT_APPEARANCE and cannot satisfy this.
                        request_respawn_all(&mut runtime);
                        if let Some(live) = live.as_ref() {
                            let size = crossterm::terminal::size().unwrap_or((80, 24));
                            previous_key = Some(repaint(
                                &mut renderer,
                                pause.as_ref(),
                                live_scroll,
                                live,
                                &live_tail,
                                &palette,
                                view,
                                panes.key(),
                                focus_segment(&registry, &runtime, &geom, &panes).as_deref(),
                                debug_notice,
                                size,
                                session.max_height,
                                fullscreen,
                                &faint,
                                profile,
                                &history,
                            )?);
                        }
                    }
                }
                // The theme events exist on Windows too; their unix
                // arms above are compiled out there, and the crossterm
                // pump never produces them.
                #[cfg(windows)]
                TapEvent::ThemeNotification(_) | TapEvent::OscColor(..) => {}
            }
        }
    }
    renderer.finish().context("restoring terminal")?;
    Ok(())
}

/// One fifo/fd trigger source as the loop tracks it: the reader (whose
/// Drop joins the thread), the spec for the end-of-life notice, and the
/// rising-edge latch that keeps that notice one-shot.
#[cfg(unix)]
struct ReaderSlot {
    reader: TriggerReader,
    spec: TriggerSpec,
    ended_seen: bool,
}

/// A reader trigger's key: its spec's canonical string.
///
/// **One definition, deliberately.** The drain writes arrivals under this key
/// and the evaluation looks them up under it, from two different places in the
/// loop. Building the string independently at each end would let them drift
/// into silent disagreement — the lookup would find nothing, the route would
/// contribute no evidence, and NOTHING would report an error, because "this
/// trigger saw no arrivals" is a legitimate state. That failure is invisible
/// by construction, so the possibility is removed rather than tested for.
///
/// Not `cfg(unix)`, even though only unix opens readers: the evaluation builds
/// its key list with a `cfg!(unix)` runtime test, so Windows still compiles
/// the reference and would fail to find a compile-time-gated definition. It is
/// permanently dead there instead, which is the same shape as the rest of the
/// unix-only surface.
#[cfg_attr(windows, allow(dead_code))]
fn reader_key(spec: &TriggerSpec) -> crate::core::trigger::TriggerKey {
    crate::core::trigger::TriggerKey(spec.to_string())
}

/// Hand this source's reader arrivals to the window, and report a lost one.
///
/// **Why there is no in-flight flag here.** The obvious design — a shared
/// `AtomicBool` the reader samples to say "something was running" — is simpler
/// and cannot work. It records *that* a child was in flight, never *which*
/// source's bracket covered the arrival, so the per-pane coverage and
/// median-width stages of the credit rule would have no input and this route
/// could not feed the graph test at all. It also cannot reconstruct aggregate
/// duty over the window when children overlap, because a boolean cannot be
/// unioned. Do not reinvent it.
///
/// So the reader reports **when**, and the window decides what it means:
/// `observe_arrival` resolves each observation against the brackets the loop
/// already owns. Nothing is pre-judged here — an arrival older than the window
/// is still handed over, and eviction drops it, because a drain that filtered
/// would be a second place deciding what counts.
///
/// What the reader reports is a **window**, not an instant: bytes appeared
/// somewhere between its last proof of emptiness and its read. That window's
/// WIDTH is the whole question — an unfenced one is tens of milliseconds
/// wide against child brackets around a millisecond, which is what the fence
/// exists to shrink. Until then the loop reads only the upper bound, exactly
/// as it read an instant before, and the lower bound rides along unused.
///
/// The identity is not optional. Two fifos on one pane are two separate bodies
/// of evidence, and merging them under a source id would make the per-path
/// stages inapplicable to this route.
/// Ask every reader for a fresh proof that its descriptor is empty.
///
/// **Every reader, not just the spawning source's.** A pane's fifo is
/// written by some OTHER pane's child — that is what a trigger loop IS — so
/// the reader that needs the tight bound is never the one being spawned.
/// Fencing only the spawning source's readers would leave exactly the
/// reader that matters on its 50 ms cadence, and nothing would report it.
///
/// Cost is one non-blocking one-byte write per reader per call, and no
/// waiting: `fence()` cannot block and is never required to be served.
#[cfg(unix)]
fn fence_all(slots: &[ReaderSlot]) {
    for slot in slots {
        slot.reader.fence();
    }
}

#[cfg(unix)]
fn drain_reader_arrivals(
    slots: &[ReaderSlot],
    log: &mut crate::core::trigger::WindowLog,
    now: Instant,
) {
    for slot in slots {
        let key = reader_key(&slot.spec);
        for observation in slot.reader.take_arrivals() {
            log.observe_arrival(key.clone(), observation);
        }
        // Read every iteration, and it reports-and-clears: one lost arrival
        // makes THIS window abstain, not every window after it.
        if slot.reader.overflowed() {
            log.record_overflow(now);
        }
    }
}

/// DIAGNOSTIC ONLY: an append-only record of the suspicion test's answers.
///
/// Two things make it usable from CI, where the failing run is the one nobody
/// can attach a debugger to. It is THROTTLED, so a 50 ms loop does not turn a
/// fifteen-second run into thirty thousand lines; and it is UNTHROTTLED
/// whenever the answer itself moves, so the one transition that matters can
/// never be the sample that got skipped.
struct TriggerTrace {
    file: std::fs::File,
    started: Instant,
    last_written: Option<Instant>,
    last_answer: Option<(Vec<crate::core::registry::SourceId>, bool)>,
}

impl TriggerTrace {
    /// `Some` only when `RAT_TRIGGER_TRACE` names a path that opens. A
    /// diagnostic that aborts the run it is diagnosing is worse than none, so
    /// an unopenable path is simply no trace.
    fn open() -> Option<TriggerTrace> {
        let path = std::env::var_os("RAT_TRIGGER_TRACE")?;
        Some(TriggerTrace {
            file: std::fs::File::create(path).ok()?,
            started: Instant::now(),
            last_written: None,
            last_answer: None,
        })
    }

    fn record(&mut self, now: Instant, verdict: &crate::core::trigger::Verdict) {
        let answer = (verdict.panes.clone(), verdict.abstained);
        let moved = self.last_answer.as_ref() != Some(&answer);
        let due = self
            .last_written
            .is_none_or(|at| now.duration_since(at) >= Duration::from_millis(200));
        if !moved && !due {
            return;
        }
        self.last_written = Some(now);
        self.last_answer = Some(answer);
        let Some(why) = verdict.why.as_deref() else {
            return;
        };
        use std::io::Write as _;
        let _ = writeln!(
            self.file,
            "t={:.3} {why} -> panes={:?} abstain={}",
            now.duration_since(self.started).as_secs_f64(),
            verdict.panes.iter().map(|s| s.0).collect::<Vec<_>>(),
            u8::from(verdict.abstained),
        );
    }
}

/// Which surface the loop is showing. `pause` and (later) a live window
/// are never both active; the freeze remains reachable from every mode.
#[derive(Copy, Clone, PartialEq, Eq, Debug)]
enum FrameMode {
    Live,
    LiveScrolled,
    Paused,
}

/// The current mode. The freeze wins; `pause` and `live_scroll` are never
/// both active.
fn mode_of(pause: Option<&PauseState>, live_scroll: Option<LiveScroll>) -> FrameMode {
    if pause.is_some() {
        FrameMode::Paused
    } else if live_scroll.is_some() {
        FrameMode::LiveScrolled
    } else {
        FrameMode::Live
    }
}

/// What one key means, resolved by `action_for`.
#[derive(Copy, Clone, PartialEq, Eq, Debug)]
enum WatchAction {
    Abort,
    Quit,
    Page,
    Help,
    Snapshot,
    Resume,
    Freeze,
    ScrubBack,
    ScrubForward,
    Scroll(ScrollStep),
    /// A stepped scroll: one wheel notch is three lines, and a folded
    /// burst is one repaint.
    ScrollN(ScrollStep, usize),
    ToggleWrap,
    ShiftLeft,
    ShiftRight,
    ToggleGutter,
    ToggleHighlight,
    ToggleTime,
    ToggleMouse,
    /// Per-pane gestures. Live only: a frozen or scrubbed frame is a
    /// composed string with no pane identity left in it.
    FocusNext,
    FocusPrev,
    FocusMove(FocusDir),
    ClearFocus,
    ToggleZoom,
    ToggleCollapse,
    Ignore,
}

/// Which way a directional focus move travels.
#[derive(Copy, Clone, PartialEq, Eq, Debug)]
enum FocusDir {
    Left,
    Right,
    Up,
    Down,
}

/// What append mode does with one key. A separate, CLOSED vocabulary —
/// not a FrameMode arm — because the set is four and must stay four:
/// everything rat's viewport keys drive (scroll, freeze, scrub, wrap,
/// shift, gutter, highlight, time style, mouse) is inert here BY
/// DESIGN. The terminal's own scrollback is the review surface, so
/// consuming those keys would take something away and give nothing
/// back. `t` in particular must stay inert — it is what arms the
/// once-per-second counting footer.
#[derive(Copy, Clone, PartialEq, Eq, Debug)]
enum AppendAction {
    Abort,
    Quit,
    Snapshot,
    Help,
    Ignore,
}

/// Append mode's key table — see [`AppendAction`] for why it is closed.
fn append_action_for(key: Key) -> AppendAction {
    match key {
        Key::CtrlC => AppendAction::Abort,
        Key::Char('q') => AppendAction::Quit,
        Key::Char('S') => AppendAction::Snapshot,
        Key::Char('?') => AppendAction::Help,
        _ => AppendAction::Ignore,
    }
}

/// A chrome event as a spoken row. The `rat watch: ` prefix is the
/// piped drop marker's own — one prefix, one meaning: this row is rat
/// talking, not the command. In a linear stream mixed with child
/// output that distinction has nowhere else to live.
fn append_notice(text: &str) -> String {
    format!("rat watch: {text}")
}

/// The child's exit status as a row, or None when nothing changed.
///
/// Kept OUT of the frame hash ON PURPOSE: folding it in would change
/// WHEN THE PIPED PATH RE-EMITS a frame — a command printing identical
/// bytes while flipping 0 to 1 would start re-emitting — and those
/// bytes are frozen. Two independent gates instead.
///
/// Silence means success: a first tick that exits 0 says nothing; a
/// recovery speaks, because "it passes now" is the moment worth
/// hearing. (Plain watch shows the exit code nowhere else — this row
/// is new information, not moved chrome.)
fn append_exit_line(previous: Option<&str>, current: Option<&str>) -> Option<String> {
    if previous == current {
        return None;
    }
    Some(match current {
        Some(badge) => format!("rat watch: {badge}"),
        None => "rat watch: exit 0".to_string(),
    })
}

/// The one startup row. Removing the footer removes `· ? help` — the
/// only discoverability breadcrumb rat ships — and the cadence; this
/// line replaces both, once, before the first frame, so it costs no
/// re-announcement. `tail` is `live_suffix`'s output: the one home of
/// the interval's meaning.
fn append_banner(tail: &str) -> String {
    format!("rat watch: appending{tail}")
}

/// The key reference, APPENDED rather than paged — everything this
/// mode says it says by appending, and the pager is an in-place
/// alternate-screen surface, the exact thing the mode avoids. The
/// shared `help_lines` is not reused: twenty of its keys are inert
/// here, and a reference that names inert keys teaches a mode that
/// does not exist.
fn append_help_lines(extra: &[String]) -> Vec<String> {
    let mut lines: Vec<String> = [
        "rat watch --append — keys",
        "",
        "  q                  quit",
        "  Ctrl-C             abort",
        "  S                  snapshot the newest frame to a file",
        "  ?                  this key reference",
        "",
        "  Every frame rat has printed is still in the terminal's own",
        "  scrollback — scroll there to review. rat's viewport keys are",
        "  inert in this mode on purpose.",
    ]
    .into_iter()
    .map(str::to_string)
    .collect();
    lines.extend(extra.iter().cloned());
    lines
}

/// The whole binding table, for both input paths and every mode — unix and
/// Windows read their keys differently but mean the same things by them.
/// What a Scroll action does while live is the loop's business, not the
/// table's. View keys never freeze. A pane gesture names a direction,
/// never a pane — which is why this table still has no pane parameter:
/// the loop resolves which pane a direction reaches.
fn action_for(key: Key, mode: FrameMode) -> WatchAction {
    match key {
        Key::CtrlC => WatchAction::Abort,
        Key::Char('q') => WatchAction::Quit,
        Key::Char('v') | Key::Enter => WatchAction::Page,
        Key::Char('?') => WatchAction::Help,
        Key::Char('S') => WatchAction::Snapshot,
        Key::Char('j') | Key::Down => WatchAction::Scroll(ScrollStep::LineDown),
        Key::Char('k') | Key::Up => WatchAction::Scroll(ScrollStep::LineUp),
        Key::Char('d') => WatchAction::Scroll(ScrollStep::HalfDown),
        Key::Char('u') => WatchAction::Scroll(ScrollStep::HalfUp),
        Key::Char('f') | Key::PageDown => WatchAction::Scroll(ScrollStep::PageDown),
        Key::Char('b') | Key::PageUp => WatchAction::Scroll(ScrollStep::PageUp),
        Key::Char('g') | Key::Home => WatchAction::Scroll(ScrollStep::Top),
        Key::Char('G') | Key::End => WatchAction::Scroll(ScrollStep::Bottom),
        Key::Char('w') => WatchAction::ToggleWrap,
        Key::Char('h') | Key::Left => WatchAction::ShiftLeft,
        Key::Char('l') | Key::Right => WatchAction::ShiftRight,
        Key::Char('D') => WatchAction::ToggleGutter,
        Key::Char('c') => WatchAction::ToggleHighlight,
        Key::Char('t') => WatchAction::ToggleTime,
        Key::Char('m') => WatchAction::ToggleMouse,
        Key::Tab if mode == FrameMode::Live => WatchAction::FocusNext,
        Key::BackTab if mode == FrameMode::Live => WatchAction::FocusPrev,
        Key::Alt('h') if mode == FrameMode::Live => WatchAction::FocusMove(FocusDir::Left),
        Key::Alt('j') if mode == FrameMode::Live => WatchAction::FocusMove(FocusDir::Down),
        Key::Alt('k') if mode == FrameMode::Live => WatchAction::FocusMove(FocusDir::Up),
        Key::Alt('l') if mode == FrameMode::Live => WatchAction::FocusMove(FocusDir::Right),
        Key::Char('z') if mode == FrameMode::Live => WatchAction::ToggleZoom,
        Key::Space if mode == FrameMode::Live => WatchAction::ToggleCollapse,
        Key::Esc if mode == FrameMode::Live => WatchAction::ClearFocus,
        Key::Esc | Key::Char('F') if mode != FrameMode::Live => WatchAction::Resume,
        Key::Char('p') if mode != FrameMode::Paused => WatchAction::Freeze,
        Key::Char('<') | Key::Char(',') => WatchAction::ScrubBack,
        Key::Char('>') | Key::Char('.') if mode == FrameMode::Paused => WatchAction::ScrubForward,
        _ => WatchAction::Ignore,
    }
}

/// The next or previous pane in reading order, wrapping. With no focus
/// any focus gesture lands on the first pane, so reaching for a pane
/// always gets one.
fn focus_cycle(from: Option<SourceId>, order: &[SourceId], forward: bool) -> Option<SourceId> {
    let first = order.first().copied();
    let Some(at) = from.and_then(|id| order.iter().position(|o| *o == id)) else {
        return first;
    };
    let n = order.len();
    let next = if forward { at + 1 } else { at + n - 1 };
    order.get(next % n).copied()
}

/// The pane a directional move lands on, or None at the edge of the
/// composition. A candidate lies STRICTLY beyond the focused pane's
/// edge in the direction of travel and overlaps it by at least one cell
/// on the cross axis; the nearest edge wins, and a tie goes to
/// whichever candidate reads first.
///
/// Ratto separates panes by `gap`/`row_gap` cells, so a touching-edges
/// test would find no neighbour at all; the distance minimum plus the
/// overlap filter is the gap-proof equivalent.
fn focus_neighbor(
    from: SourceId,
    dir: FocusDir,
    rects: &[PaneRect],
    order: &[SourceId],
) -> Option<SourceId> {
    let here = *rects.get(from.0)?;
    order
        .iter()
        .copied()
        .filter(|id| *id != from)
        .filter_map(|id| Some((id, edge_distance(here, *rects.get(id.0)?, dir)?)))
        .min_by_key(|(_, distance)| *distance)
        .map(|(id, _)| id)
}

/// How far `there` lies beyond `here`'s edge in the direction of
/// travel, or None when it is not a candidate at all.
fn edge_distance(here: PaneRect, there: PaneRect, dir: FocusDir) -> Option<usize> {
    // Half-open spans: they share a cell when each starts before the
    // other ends.
    let overlaps = |a: usize, a_len: usize, b: usize, b_len: usize| a < b + b_len && b < a + a_len;
    match dir {
        FocusDir::Left => (there.col + there.cols <= here.col
            && overlaps(here.row, here.rows, there.row, there.rows))
        .then(|| here.col - (there.col + there.cols)),
        FocusDir::Right => (here.col + here.cols <= there.col
            && overlaps(here.row, here.rows, there.row, there.rows))
        .then(|| there.col - (here.col + here.cols)),
        FocusDir::Up => (there.row + there.rows <= here.row
            && overlaps(here.col, here.cols, there.col, there.cols))
        .then(|| here.row - (there.row + there.rows)),
        FocusDir::Down => (here.row + here.rows <= there.row
            && overlaps(here.col, here.cols, there.col, there.cols))
        .then(|| there.row - (here.row + here.rows)),
    }
}

/// Each pane's rendered block size for the rect walk: its declared box,
/// or one row when it is collapsed.
fn pane_block_sizes(geom: &[PaneGeometry], panes: &PaneView) -> Vec<(usize, usize)> {
    geom.iter()
        .zip(&panes.collapsed)
        .map(|(g, collapsed)| {
            (
                if *collapsed { 1 } else { g.rows as usize },
                g.cells as usize,
            )
        })
        .collect()
}

/// The mouse's half of the binding table: the wheel drives the scroll
/// actions the keys already drive (a notch is three lines; shift, a
/// half window; a horizontal wheel, the h/l shift), and every other
/// report maps to nothing. The event loop gates on live capture
/// before consulting this table — a report rat did not ask for (or
/// one arriving after `m` released the mouse) changes nothing.
fn action_for_mouse(ev: MouseEvent) -> WatchAction {
    let n = usize::from(ev.notches.max(1));
    match ev.kind {
        MouseKind::WheelDown if ev.shift => WatchAction::ScrollN(ScrollStep::HalfDown, n),
        MouseKind::WheelUp if ev.shift => WatchAction::ScrollN(ScrollStep::HalfUp, n),
        MouseKind::WheelDown => WatchAction::ScrollN(ScrollStep::LineDown, 3 * n),
        MouseKind::WheelUp => WatchAction::ScrollN(ScrollStep::LineUp, 3 * n),
        MouseKind::WheelLeft => WatchAction::ShiftLeft,
        MouseKind::WheelRight => WatchAction::ShiftRight,
        MouseKind::Other => WatchAction::Ignore,
    }
}

/// Fold consecutive same-direction wheel notches into one event with a
/// summed notch count. Keys and other events pass through in order.
fn fold_wheel(events: Vec<TapEvent>) -> Vec<TapEvent> {
    let mut out: Vec<TapEvent> = Vec::with_capacity(events.len());
    for event in events {
        if let TapEvent::Mouse(m) = &event
            && matches!(m.kind, MouseKind::WheelUp | MouseKind::WheelDown)
            && let Some(TapEvent::Mouse(prev)) = out.last_mut()
            && prev.kind == m.kind
            && prev.shift == m.shift
        {
            prev.notches = prev.notches.saturating_add(m.notches);
            continue;
        }
        out.push(event);
    }
    out
}

/// A frozen frame and the window's place in it. The copy never changes
/// while paused; children keep ticking into `full_lines` behind it, so
/// resume repaints the newest content immediately. `content`/`appearance`
/// hold the freeze-time values the repaint gate compares.
struct PauseState {
    frozen: Vec<String>,
    scroll: ScrollState,
    content: u64,
    appearance: Appearance,
    /// When the viewed frame was current: freeze time for a plain pause,
    /// the entry's capture time for a scrub. The counting age on the
    /// paused row measures from here.
    viewed_at: jiff::Timestamp,
    /// The history entry this pause is anchored on. The tail keeps
    /// recording behind a freeze, so without this anchor a scrub-back
    /// would jump FORWARD to unseen frames.
    history_seq: Option<u64>,
}

/// How long lines are shown: wrapped (today's path) or chopped, shifted
/// `hshift` columns right, with or without the change gutter. View
/// state, not scrollback state: it survives freeze/resume and pager
/// round-trips.
#[derive(Copy, Clone, PartialEq, Eq, Debug)]
struct ViewState {
    wrap: bool,
    hshift: usize,
    /// The margin column marking lines that changed against the
    /// previous distinct frame. Implies chopped rendering: mark i
    /// aligning with line i needs 1:1 line-to-row.
    gutter: bool,
    /// Reverse-video highlights on the changed characters themselves,
    /// patched over the child's own styling. Wrap-agnostic: the
    /// splices are zero display cells.
    highlight: bool,
    /// Flip both time rows from wall-clock stamps to counting ages.
    /// Presentation only — each row keeps its meaning, and both rows
    /// always share one style.
    alt_time: bool,
}

/// Per-pane view state: what the pane gestures move and the composer
/// reads. Loop-local beside `view`, never inside `SourceRuntime` —
/// that vector is capture and scheduling state, iterated as runtime by
/// the change hash and the respawn requests.
struct PaneView {
    focus: Option<SourceId>,
    zoomed: Option<SourceId>,
    /// Indexed by `SourceId`; `len() == registry.len()`.
    collapsed: Vec<bool>,
    scroll: Vec<LiveScroll>,
}

impl PaneView {
    fn new(len: usize) -> PaneView {
        PaneView {
            focus: None,
            zoomed: None,
            collapsed: vec![false; len],
            scroll: vec![LiveScroll::at(0, 0, 0); len],
        }
    }

    /// The digest the repaint gate compares. The per-pane vectors fold
    /// into one hash — the `combined_hash` shape — because a bitset
    /// would cap the pane count, and the gate's key must stay `Copy`.
    fn key(&self) -> PaneViewKey {
        use std::hash::{Hash, Hasher};
        let mut hasher = std::hash::DefaultHasher::new();
        for (collapsed, scroll) in self.collapsed.iter().zip(&self.scroll) {
            collapsed.hash(&mut hasher);
            scroll.offset().hash(&mut hasher);
            scroll.pinned().hash(&mut hasher);
        }
        PaneViewKey {
            focus: self.focus,
            zoomed: self.zoomed,
            panes: hasher.finish(),
        }
    }
}

/// Everything per-pane the repaint gate must see, in a shape a
/// `PaintKey` can hold: `Copy`, comparable, and no `Vec`.
#[derive(Copy, Clone, PartialEq, Eq, Debug, Default)]
struct PaneViewKey {
    focus: Option<SourceId>,
    zoomed: Option<SourceId>,
    panes: u64,
}

/// Everything a painted frame depends on; the repaint gate compares two of
/// these. While paused, `content`/`appearance` are the freeze-time values,
/// so new child output never repaints a frozen frame — but scroll, resize,
/// and view toggles do.
#[derive(Copy, Clone, PartialEq, Debug)]
struct PaintKey {
    content: u64,
    cols: u16,
    rows: u16,
    appearance: Appearance,
    offset: usize,
    paused: bool,
    wrap: bool,
    hshift: usize,
    gutter: bool,
    highlight: bool,
    alt_time: bool,
    /// The per-pane view, digested. Without it the gate is pane-blind:
    /// content and size are unchanged by a focus or collapse gesture.
    view: PaneViewKey,
    /// Whole seconds since the viewed frame was current; 0 while live.
    /// Advancing once per second is what lets the paused age repaint.
    age_secs: u64,
}

/// The one construction of the repaint gate's key: while paused it holds
/// the freeze-time content/appearance and the window's offset; live it
/// holds this tick's values.
#[allow(clippy::too_many_arguments)]
fn paint_key(
    pause: Option<&PauseState>,
    live_scroll: Option<LiveScroll>,
    live_content: u64,
    live_appearance: Appearance,
    size: (u16, u16),
    view: ViewState,
    view_key: PaneViewKey,
    age_secs: u64,
) -> PaintKey {
    // A live-scrolled key carries the LIVE hash: the tail keeps
    // repainting under the offset — that is the point of the mode. The
    // displayed age arrives computed (which surface counts is the
    // caller's business, see `displayed_age`) and rides verbatim.
    let (content, appearance, offset, paused) = match pause {
        Some(p) => (p.content, p.appearance, p.scroll.offset(), true),
        None => (
            live_content,
            live_appearance,
            live_scroll.map_or(0, LiveScroll::offset),
            false,
        ),
    };
    PaintKey {
        content,
        cols: size.0,
        rows: size.1,
        appearance,
        offset,
        paused,
        wrap: view.wrap,
        hshift: view.hshift,
        gutter: view.gutter,
        highlight: view.highlight,
        alt_time: view.alt_time,
        view: view_key,
        age_secs,
    }
}

/// Whole seconds since `t`, clamped at zero.
fn age_seconds(t: jiff::Timestamp) -> u64 {
    (jiff::Timestamp::now().as_second() - t.as_second()).max(0) as u64
}

/// The paused row's counting age, pre-formatted: a short grace reads
/// "just now" (which also keeps early repaints byte-identical), then the
/// exact age counts second by second.
fn age_text(age_secs: u64) -> String {
    if age_secs < 10 {
        "just now".to_string()
    } else {
        format!(
            "{} ago",
            crate::core::duration::format_long(age_secs as i64)
        )
    }
}

/// Whole seconds the displayed status row is counting; 0 when it shows
/// an absolute stamp or no time at all. ONE style at a time: every row
/// stamps by default and counts only when flipped — a frozen frame must
/// never read differently from the live one beside it. The scrolled row
/// carries the live row's time segment, so it counts with it: a gate
/// that reads it as time-less paints the counting text once and stalls.
fn displayed_age(
    pause: Option<&PauseState>,
    _live_scroll: Option<LiveScroll>,
    alt_time: bool,
    changed_at: jiff::Timestamp,
) -> u64 {
    match (alt_time, pause) {
        (true, Some(p)) => age_seconds(p.viewed_at),
        (true, None) => age_seconds(changed_at),
        _ => 0,
    }
}

/// The live row's time segment: the last-change stamp, or its counting
/// form when the presentation is flipped.
fn live_time_segment(alt_time: bool, since: &str, live_age_secs: u64) -> String {
    if alt_time {
        format!("changed {}", age_text(live_age_secs))
    } else {
        format!("since {since}")
    }
}

/// The paused row's segment: the viewed frame's wall clock, or its
/// counting age when the presentation is flipped — the same
/// stamp-then-counter order the live row follows.
fn paused_time_segment(alt_time: bool, viewed_at: jiff::Timestamp, age_secs: u64) -> String {
    if alt_time {
        age_text(age_secs)
    } else {
        format!("at {}", local_hms(viewed_at))
    }
}

/// The one home of the interval's meaning beside triggers: the user's
/// token always wins; no token means today's 2s default — unless a
/// trigger exists, which makes polling opt-in (`None` = trigger-only).
fn resolve_interval(user: Option<&str>, triggered: bool) -> anyhow::Result<Option<Duration>> {
    match (user, triggered) {
        (Some(token), _) => Ok(Some(parse_interval(token)?)),
        (None, false) => Ok(Some(Duration::from_secs(2))),
        (None, true) => Ok(None),
    }
}

/// `t` as local wall-clock HH:MM:SS — the `since` stamp's format.
fn local_hms(t: jiff::Timestamp) -> String {
    t.to_zoned(jiff::tz::TimeZone::system())
        .strftime("%H:%M:%S")
        .to_string()
}

/// The key reference `?` pages: plain text, grouped the way the keys
/// are learned, under the caller's heading and followed by whatever
/// section belongs to the caller's surface — the shared body lives
/// here, the surface-specific tail with the caller. Content only —
/// the pager owns presentation.
fn help_lines(heading: &str, extra: &[String]) -> Vec<String> {
    let mut lines: Vec<String> = std::iter::once(heading.to_string())
        .chain(
            [
                "",
                "  q                  quit",
                "  v, Enter           view the full frame in the pager",
                "  ?                  this key reference",
                "  S                  snapshot the viewed frame to a file",
                "",
                "  j/k, Up/Down       scroll one line (opens a live window)",
                "  d/u                scroll half a window",
                "  f/b, PgDn/PgUp     scroll a full window",
                "  g, Home            top — and back to the live view",
                "  G, End             bottom — stick to the tail",
                "",
                "  p                  freeze the frame in place (the command keeps running)",
                "  Esc, F             resume the live tail",
                "  <, ,               step back through distinct frames",
                "  >, .               step forward — past the newest, back to live",
                "",
                "  w                  wrap or chop long lines",
                "  h/l, Left/Right    shift the view horizontally",
                "  D                  toggle the change gutter",
                "  c                  toggle the change highlights",
                "  t                  time style: wall-clock stamps or counting ages",
                "  m                  capture or release the mouse (with --mouse)",
            ]
            .into_iter()
            .map(str::to_string),
        )
        .collect();
    lines.extend(RETENTION_HELP.iter().map(|l| (*l).to_string()));
    lines.extend(extra.iter().cloned());
    lines
}

/// What `1.2k lines dropped` means.
///
/// Both commands bound what they retain, so this rides the shared
/// reference rather than a per-command section. Two facts here are not
/// guessable from the marker: the command was never stopped or slowed —
/// rat reads to the end and stops KEEPING, which is what stops a child
/// blocking on a pipe nobody drains — and WHICH end survives is the
/// pane's own declaration, so a reader looking at a head where they
/// wanted a tail is looking at a declaration, not a fault.
///
/// Wrapped by hand: `?` pages plain grouped text through the pager, so
/// there is no wrapping engine to lean on, and nothing here may exceed
/// the width the key table already sets.
const RETENTION_HELP: &[&str] = &[
    "",
    "  dropped lines:",
    "    A command keeps at most 1000 lines per run. Past that, the",
    "    marker `1.2k lines dropped` says how many did not survive —",
    "    on the pane that overflowed, or on the status row of a plain",
    "    watch.",
    "",
    "    Nothing is stopped or slowed to make that happen: rat reads",
    "    the command's output to the end and stops KEEPING, so a child",
    "    never blocks writing into a pipe nobody is draining. Which",
    "    end survives is the pane's `overflow` — the head by default,",
    "    the tail where declared, and the tail always for a plain",
    "    watch.",
];

/// Watch's slice of the key reference: the trigger sources it was
/// started with. Empty when none are configured, so the reference
/// stays clean.
fn trigger_help(triggers: &[TriggerSpec]) -> Vec<String> {
    if triggers.is_empty() {
        return Vec::new();
    }
    let mut lines = vec![String::new(), "  refresh triggers:".to_string()];
    lines.extend(triggers.iter().map(|spec| format!("    {spec}")));
    lines
}

/// The run-constant tail of every live row: the refresh mode (so the
/// next repaint can be anticipated) and the one discoverability
/// breadcrumb — everything else the footer used to advertise lives in
/// the `?` reference now, including the trigger sources themselves.
/// Empty in once mode: no cadence, no keys. Run-constant BY DESIGN: a
/// countdown or fire counter would defeat the repaint gate.
fn live_suffix(once: bool, interval: Option<&str>, triggered: bool) -> String {
    if once {
        return String::new();
    }
    match (interval, triggered) {
        (Some(interval), false) => format!(" · every {interval} · ? help"),
        (Some(interval), true) => format!(" · every {interval} or on trigger · ? help"),
        (None, true) => " · on trigger · ? help".to_string(),
        (None, false) => {
            // Unrepresentable by the resolve_interval rule.
            debug_assert!(false, "no interval and no trigger");
            String::new()
        }
    }
}

/// The live status row: the truncation notice when rows are hidden,
/// carrying the pre-formatted time segment, and the retention marker
/// when the last tick outran what is kept.
///
/// The two say different things and both can be true at once. `… N more
/// lines` is about this VIEWPORT — the rows exist and scrolling reaches
/// them. `N lines dropped` is about the CONTENT — those lines are gone
/// and no key will bring them back.
fn live_notice(
    hidden: usize,
    time_seg: &str,
    dropped: Option<&str>,
    focus_seg: Option<&str>,
) -> String {
    let mut row = if hidden > 0 {
        format!("… {hidden} more lines · {time_seg}")
    } else {
        time_seg.to_string()
    };
    if let Some(dropped) = dropped {
        row.push_str(" · ");
        row.push_str(dropped);
    }
    if let Some(focus) = focus_seg {
        row.push_str(" · ");
        row.push_str(focus);
    }
    row
}

/// The consolidated in-place paint: build the key for the current
/// (pause, view) state, draw the matching source, and return the key that
/// was painted — so a dispatch arm cannot paint and forget the gate.
#[allow(clippy::too_many_arguments)]
fn repaint(
    renderer: &mut InlineRenderer<std::io::StdoutLock<'static>>,
    pause: Option<&PauseState>,
    live_scroll: Option<LiveScroll>,
    live: &Live,
    live_tail: &str,
    palette: &Palette,
    view: ViewState,
    view_key: PaneViewKey,
    focus_seg: Option<&str>,
    notice: Option<String>,
    size: (u16, u16),
    max_height: Option<u16>,
    fullscreen: bool,
    faint: &StyleSpec,
    profile: ColorProfile,
    history: &History,
) -> anyhow::Result<PaintKey> {
    let age_secs = displayed_age_key(
        pause,
        live_scroll,
        view.alt_time,
        live.changed_at,
        live.panes.as_ref().map_or(&[][..], |p| &p.ages),
    );
    let key = paint_key(
        pause,
        live_scroll,
        live.hash,
        palette.appearance,
        size,
        view,
        view_key,
        age_secs,
    );
    let (source, offset, mode) = match (pause, live_scroll) {
        (Some(p), _) => (p.frozen.as_slice(), p.scroll.offset(), FrameMode::Paused),
        (None, Some(ls)) => (live.lines.as_slice(), ls.offset(), FrameMode::LiveScrolled),
        (None, None) => (live.lines.as_slice(), 0, FrameMode::Live),
    };
    // Marks: a live pane surface shows the per-pane marks already
    // composed into frame coordinates; a paused or scrubbed frame — and
    // every plain-watch frame — compares the viewed composition against
    // the previous DISTINCT frame: the pause's anchored entry
    // (re-resolved through `nearest` when evicted), else the newest. No
    // predecessor means no marks.
    let marks: Option<Vec<LineMark>> = (view.gutter || view.highlight).then(|| {
        let anchor = match pause {
            Some(p) => p
                .history_seq
                .and_then(|seq| history.nearest(seq).map(|e| e.seq)),
            None => history.newest_seq(),
        };
        let prev = anchor.and_then(|seq| history.prev(seq));
        paint_marks(
            live.panes.is_some(),
            mode,
            live.panes.as_ref().map_or(&[][..], |p| &p.marks),
            source,
            prev.map(|e| e.frame.as_slice()),
        )
    });
    // The accent rides the live theme system, so the cell is built at
    // paint time from the current palette, never cached.
    let mark_cell = format!(
        "{} ",
        StyleSpec {
            bold: true,
            foreground: Some(palette.accent),
            ..StyleSpec::default()
        }
        .render("▌", profile)
    );
    // The ink for a changed coverage glyph (block/braille): bold +
    // accent foreground, built at paint time like the gutter cell so
    // it rides the live theme. Empty under Ascii (highlights are off
    // there anyway).
    let solid_mark = {
        let prefix = StyleSpec {
            bold: true,
            foreground: Some(palette.accent),
            ..StyleSpec::default()
        }
        .sgr_prefix(profile);
        if prefix.is_empty() {
            String::new()
        } else {
            format!("\x1b[{prefix}m")
        }
    };
    let time_seg_live = live_time_segment(view.alt_time, &live.since, age_seconds(live.changed_at));
    let time_paused = pause.map_or_else(
        || age_text(0),
        |p| paused_time_segment(view.alt_time, p.viewed_at, age_seconds(p.viewed_at)),
    );
    paint_frame(
        renderer,
        source,
        offset,
        mode,
        view,
        notice,
        size,
        max_height,
        fullscreen,
        faint,
        profile,
        &time_seg_live,
        live_tail,
        &time_paused,
        marks.as_deref(),
        &mark_cell,
        &solid_mark,
        live.dropped.as_deref(),
        focus_seg,
    )?;
    Ok(key)
}

/// Columns the change gutter takes from the width budget while it is
/// on; zero otherwise. One spelling, used by every geometry site.
fn gutter_reserve(gutter: bool) -> u16 {
    if gutter { GUTTER_COLS as u16 } else { 0 }
}

/// A pane's window as its declaration asks for it: `KeepTop` is the head,
/// held; `KeepBottom` is the tail, pinned. These are the SAME two states
/// the shipped `render_pane` clip had — which is what lets the viewport
/// replace that clip with an offset and change no bytes.
fn initial_pane_scroll(overflow: Overflow, total: usize, window: usize) -> LiveScroll {
    match overflow {
        Overflow::KeepTop => LiveScroll::at(0, total, window),
        Overflow::KeepBottom => LiveScroll::start(ScrollStep::Bottom, total, window),
    }
}

/// Whether a pane's window is where its declaration puts it — the one
/// test for "show no position badge" and "the reader has taken over".
/// NEVER `LiveScroll::at_top()`: that is the whole-frame collapse rule
/// (offset 0 means the live view), and a KeepBottom pane at rest sits at
/// its tail with a nonzero offset.
fn pane_at_rest(scroll: LiveScroll, overflow: Overflow) -> bool {
    match overflow {
        Overflow::KeepTop => scroll.offset() == 0 && !scroll.pinned(),
        Overflow::KeepBottom => scroll.pinned(),
    }
}

/// A pane's position, or `None` when there is nothing to say: at rest
/// (its declaration's own window) or with no body at all. THE one
/// predicate both surfaces consult — the chrome badge and the footer's
/// D5 range — so a pane can never contradict itself across them.
fn pane_scroll_badge(
    scroll: LiveScroll,
    overflow: Overflow,
    total: usize,
    window: usize,
) -> Option<String> {
    (!pane_at_rest(scroll, overflow) && total > 0)
        .then(|| scroll_badge(scroll.offset(), window, total))
}

/// Which pane a scroll step addresses, or `None` for the whole frame.
/// Live-only (INV-3): `Paused`/`LiveScrolled` are composed strings with no
/// pane identity left in them. A focused pane owns the keys even while
/// collapsed — the arm declines the step rather than handing the keys
/// back to the whole frame.
fn scroll_target(mode: FrameMode, panes: &PaneView) -> Option<SourceId> {
    let id = panes.focus?;
    (mode == FrameMode::Live).then_some(id)
}

/// Which body `v`/Enter hands to the pager: the focused pane's while
/// Live, else the whole frame (live, frozen, or scrubbed). The same
/// predicate as the scroll retarget — the two gestures diverge at their
/// ARMS (a collapsed pane's scroll step declines; its page serves the
/// body a reader has no other way to see), never here.
fn pager_target(mode: FrameMode, panes: &PaneView) -> Option<SourceId> {
    scroll_target(mode, panes)
}

/// THE reanchor. Every site that changes a pane's body or its window
/// clamps every pane's window back into the new shape here — the collect
/// step, the resize reflow, and (as consumers) the zoom and collapse arms.
/// A pinned window rides its tail; an unpinned one HOLDS its offset,
/// clamped (D4). Nothing here resets on a hash change or a failure: the
/// clamp is the only thing allowed to move a reader's place.
fn reanchor_pane_scrolls(panes: &mut PaneView, runtime: &[SourceRuntime], geom: &[PaneGeometry]) {
    for (i, scroll) in panes.scroll.iter_mut().enumerate() {
        let total = runtime[i].output.as_ref().map_or(0, Vec::len);
        let window = geom[i].inner_rows as usize;
        *scroll = scroll.reanchor(total, window);
    }
}

/// The ONE geometry derivation. The gesture arms, the resize arm, and
/// `detect_resize` all call this with the same inputs, or a view
/// gesture reads as a resize and restarts every child 250ms later.
/// Explicit parameters, never a captured closure: the failure mode is
/// "two sites derived from different state", and a closure makes that
/// invisible again.
fn derive_geometry(
    registry: &Registry,
    size: (u16, u16),
    max_height: Option<u16>,
    gutter: bool,
    panes: &PaneView,
) -> Vec<PaneGeometry> {
    let mut geom = registry.geometry_reserving(size, gutter_reserve(gutter));
    // Everything below is the zoom override, applied AFTER the declared
    // derivation and never instead of it: the hidden panes' boxes are
    // exactly what an unzoomed frame would give them, which is what makes
    // unzoom a plain re-derivation with nothing saved (INV-5).
    let Some(id) = panes.zoomed else {
        return geom;
    };
    let Composition::Panes { title, .. } = registry.composition() else {
        return geom;
    };
    let Some(pane) = registry.pane(id) else {
        return geom;
    };
    // The same destructure compose_sources performs at its head — the
    // title row it prepends is a row the zoomed box cannot also have.
    let title_rows = u16::from(matches!(title, TitleSource::Static(_)));
    let cells = size.0.saturating_sub(gutter_reserve(gutter));
    let rows = window_rows(max_height, size.1).saturating_sub(title_rows);
    if let Some(slot) = geom.get_mut(id.0) {
        *slot = PaneGeometry {
            cells,
            rows,
            inner_cols: cells.saturating_sub(pane.frame_cols()),
            inner_rows: rows.saturating_sub(pane.frame_rows()),
        };
    }
    geom
}

/// The painted body: `max_height`, else terminal rows − 2 (one row for the
/// notice line, one for the cursor row below the frame).
fn window_rows(max_height: Option<u16>, rows: u16) -> u16 {
    max_height.unwrap_or_else(|| rows.saturating_sub(2))
}

/// One frame's lines: title first, child stdout, then child stderr when it
/// joins the frame (a raw write to the terminal would shift the cursor and
/// corrupt the relative repaint). Trailing newlines are trimmed, not the
/// interior ones.
fn compose_frame(
    title: Option<&String>,
    stdout: &[u8],
    stderr: &[u8],
    join_stderr: bool,
) -> Vec<String> {
    let mut lines: Vec<String> = Vec::new();
    if let Some(title) = title {
        lines.push(title.clone());
    }
    lines.extend(crate::core::decode::stream_lines(stdout));
    if join_stderr && !stderr.is_empty() {
        lines.extend(crate::core::decode::stream_lines(stderr));
    }
    lines
}

/// The linear stream's one write. Nothing here moves the cursor: an
/// appended row is the one thing a screen reader reliably announces,
/// and a row that can be rewritten is a row that can be destroyed
/// mid-word. Each row seals as ITS OWN group — a single-row seal
/// closes any open SGR and has nothing to replay onto — because rat's
/// standalone rows carry user-provided text (a trigger path, a
/// snapshot path) that can smuggle an open escape toward the shell
/// prompt. Rows that open nothing pass through byte-identical.
fn append_rows<W: Write>(out: &mut W, rows: Vec<String>, eol: &str) -> anyhow::Result<()> {
    for row in rows {
        for sealed in seal_rows(vec![row]) {
            out.write_all(sealed.as_bytes()).context("writing")?;
        }
        out.write_all(eol.as_bytes()).context("writing")?;
    }
    out.flush().context("flushing")
}

/// What one distinct frame contributes to the appended stream: the
/// composed rows sealed as ONE group (style continuity across the
/// child's own rows), then the retention marker OUTSIDE that group —
/// `seal_rows` close-and-replays within the vec it is given, so a
/// marker sealed alongside the body would wear the child's open color.
///
/// WHOLE-FRAME, NO SEPARATOR — a frame that leads with a changing row
/// self-delimits, and unchanged rows inside a changed frame re-read
/// per burst by design. The marker wording is the piped sentence
/// verbatim: one wording per fact.
fn append_frame(lines: &[String], dropped: Option<&str>) -> Vec<String> {
    let mut rows = seal_rows(lines.to_vec());
    if let Some(text) = dropped {
        rows.push(format!(
            "rat watch: {text}; kept the last {MAX_RETAINED_LINES}"
        ));
    }
    rows
}

/// The single place a tty frame body is painted: assemble the rows, draw.
#[allow(clippy::too_many_arguments)]
fn paint_frame(
    renderer: &mut InlineRenderer<std::io::StdoutLock<'static>>,
    lines: &[String],
    offset: usize,
    mode: FrameMode,
    view: ViewState,
    notice: Option<String>,
    size: (u16, u16),
    max_height: Option<u16>,
    fullscreen: bool,
    faint: &StyleSpec,
    profile: ColorProfile,
    time_seg_live: &str,
    live_tail: &str,
    time_paused: &str,
    marks: Option<&[LineMark]>,
    mark_cell: &str,
    solid_mark: &str,
    dropped: Option<&str>,
    focus_seg: Option<&str>,
) -> anyhow::Result<()> {
    let kept = frame_rows(
        lines,
        offset,
        mode,
        view,
        notice,
        size,
        max_height,
        fullscreen,
        faint,
        profile,
        time_seg_live,
        live_tail,
        time_paused,
        marks,
        mark_cell,
        solid_mark,
        dropped,
        focus_seg,
    );
    renderer.draw(&kept, size.0).context("writing frame")?;
    Ok(())
}

/// One painted frame's rows: truncate the window's slice, seal the body,
/// append the paused row or the truncation notice, then the one-shot
/// notice row.
#[allow(clippy::too_many_arguments)]
fn frame_rows(
    lines: &[String],
    offset: usize,
    mode: FrameMode,
    view: ViewState,
    notice: Option<String>,
    size: (u16, u16),
    max_height: Option<u16>,
    fullscreen: bool,
    faint: &StyleSpec,
    profile: ColorProfile,
    time_seg_live: &str,
    live_tail: &str,
    time_paused: &str,
    marks: Option<&[LineMark]>,
    mark_cell: &str,
    solid_mark: &str,
    dropped: Option<&str>,
    focus_seg: Option<&str>,
) -> Vec<String> {
    let (cols, rows) = size;
    // Fullscreen pins the status row to the bottom screen row and
    // must never paint the bottom-most row itself: a trailing newline
    // there scrolls the alternate screen and destroys the top body
    // row (there is no scrollback to absorb it). With the default
    // terminal-derived budget, a one-shot notice row therefore takes
    // its row from the body; a --max-height cap keeps the author's
    // number and floats as it always did.
    let fills = fullscreen && max_height.is_none();
    let max_rows =
        window_rows(max_height, rows).saturating_sub(u16::from(fills && notice.is_some()));
    let start = match mode {
        FrameMode::Live => 0,
        FrameMode::LiveScrolled | FrameMode::Paused => offset.min(lines.len()),
    };
    // The gutter is its own region: content renders into what is left.
    let content_cols = usize::from(cols).saturating_sub(if view.gutter { GUTTER_COLS } else { 0 });
    // The character highlights splice attribute-only SGR into the body
    // rows, so a profile that forbids SGR gets none from them either.
    let highlight = view.highlight && profile != ColorProfile::Ascii;
    let spliced = |i: usize, line: &String| -> String {
        match (highlight, marks) {
            (true, Some(ms)) => mark_cells_with(
                line,
                ms.get(start + i).map_or(&[][..], |m| m.cells.as_slice()),
                (!solid_mark.is_empty()).then_some(solid_mark),
            ),
            _ => line.clone(),
        }
    };
    // A nonzero shift implies chopped lines, less's own rule — and so do
    // live-scrolling and the gutter, where an offset (or a mark) counts
    // lines and only chopping makes a line one row. Chopped rendering is
    // 1:1 line-to-row; wrapped rendering is today's path. The splice
    // runs BEFORE the chop, whose state replay carries a mark across
    // the cut.
    let (mut kept, hidden) =
        if !view.wrap || view.hshift > 0 || mode == FrameMode::LiveScrolled || view.gutter {
            let end = (start + usize::from(max_rows)).min(lines.len());
            let kept: Vec<String> = lines[start..end]
                .iter()
                .enumerate()
                .map(|(i, line)| shift_chop(&spliced(i, line), view.hshift, content_cols))
                .collect();
            (kept, lines.len() - end)
        } else {
            truncate_to_rows(
                lines[start..]
                    .iter()
                    .enumerate()
                    .map(|(i, line)| spliced(i, line))
                    .collect(),
                max_rows,
                cols,
            )
        };
    // Body rows only: the status and notice rows below are chrome and
    // stay unprefixed at full width. Marks may be present for a
    // highlight-only paint; the margin column needs the gutter itself.
    if view.gutter
        && let Some(marks) = marks
    {
        kept = prefix_rows(kept, marks, start, mark_cell);
    }
    // Seal AFTER the splice and the gutter margin (their cell indices
    // count the pre-seal bytes): each body row ends closed and reopens
    // what the child left open, so the chrome below — and the terminal
    // at rest — never inherits a child color. The chop path above
    // already closes per row, so sealing it changes nothing.
    kept = seal_rows(kept);
    if fills {
        // Pad by RENDERED rows (a wrapped line occupies several) so
        // the status row sits on the bottom screen row instead of
        // floating under short content. A constant painted height is
        // also what keeps the row differ eligible across frames whose
        // line count moves.
        for _ in crate::term::inline::rendered_rows(&kept, cols)..max_rows {
            kept.push(String::new());
        }
    }
    let status = match mode {
        FrameMode::Paused => paused_notice(time_paused, offset, kept.len(), lines.len()),
        FrameMode::LiveScrolled => {
            scrolled_notice(time_seg_live, live_tail, offset, kept.len(), lines.len())
        }
        FrameMode::Live => live_notice(
            hidden,
            &format!("{time_seg_live}{live_tail}"),
            dropped,
            focus_seg,
        ),
    };
    kept.push(faint.render(&status, profile));
    if let Some(text) = notice {
        kept.push(faint.render(&text, profile));
    }
    kept
}

/// Write `lines` to a timestamped file and describe the outcome for the
/// notice row. The snapshot is the data, not the viewport: wrap, shift,
/// and scroll state never change what lands in the file.
fn snapshot_frame(lines: &[String], dir: Option<&std::path::Path>, ansi: bool) -> String {
    let dir = dir
        .map(std::path::Path::to_path_buf)
        .or_else(|| std::env::current_dir().ok())
        .unwrap_or_else(|| std::path::PathBuf::from("."));
    let stamp = snapshot_stamp(&jiff::Timestamp::now().to_zoned(jiff::tz::TimeZone::system()));
    let body = snapshot_body(lines, ansi);
    match write_snapshot(&dir, &stamp, &body) {
        Ok(path) => format!("snapshot → {}", path.display()),
        Err(err) => format!("snapshot failed ({err}) — set --snapshot-dir or RAT_SNAPSHOT_DIR"),
    }
}

/// Hand the full untruncated frame to the user's pager (RAT_PAGER, PAGER,
/// then less -R, then more.com on Windows), bat-style. The loop resumes
/// when the pager exits; a failure to launch becomes a status line, never
/// an error exit.
fn page_frame(
    lines: &[String],
    renderer: &mut InlineRenderer<std::io::StdoutLock<'static>>,
    fullscreen: bool,
) -> Option<String> {
    let pagers = resolve_pagers(&SystemEnv);
    let mut used = pagers.first().map(|p| p.bin.clone()).unwrap_or_default();
    let _ = crossterm::terminal::disable_raw_mode();
    let _ = renderer.finish();
    // `?1049` is not a nesting counter: the default pager takes the
    // alternate screen itself, and its leave would drop the terminal
    // to the normal buffer with rat's content gone. Leave first; the
    // pager runs on the normal screen like the inline mode always had.
    if fullscreen {
        use std::io::Write;
        let mut out = std::io::stdout();
        let _ = out.write_all(b"\x1b[?1049l");
        let _ = out.flush();
    }
    // The pager inherits the console; keep it decoding UTF-8 while the
    // pager owns the screen (more.com garbles the frame otherwise).
    let _console_utf8 = ConsoleUtf8Guard::enable();

    let result = (|| -> std::io::Result<()> {
        let (bin, mut child) = spawn_first(&pagers)?;
        used = bin;
        // Quitting the pager before it reads everything is normal; do not
        // let the default SIGPIPE disposition kill the watch for it.
        #[cfg(unix)]
        unsafe {
            libc::signal(libc::SIGPIPE, libc::SIG_IGN);
        }
        let write_result = (|| -> std::io::Result<()> {
            let mut stdin = child.stdin.take().expect("stdin piped");
            for line in lines {
                writeln!(stdin, "{line}")?;
            }
            Ok(())
        })();
        #[cfg(unix)]
        unsafe {
            libc::signal(libc::SIGPIPE, libc::SIG_DFL);
        }
        match write_result {
            Err(err) if err.kind() != std::io::ErrorKind::BrokenPipe => return Err(err),
            _ => {}
        }
        child.wait()?;
        Ok(())
    })();

    let _ = crossterm::terminal::enable_raw_mode();
    if fullscreen {
        // Re-entered fresh: the alternate screen we left was
        // discarded, so there is no frame of ours to resume over —
        // the next draw starts from a blank buffer.
        use std::io::Write;
        let mut out = std::io::stdout();
        let _ = out.write_all(b"\x1b[?1049h");
        let _ = out.flush();
        renderer.restart_on_blank_screen();
    } else {
        renderer.resume_over_own_frame();
    }
    match result {
        Ok(()) => None,
        Err(err) => Some(format!(
            "pager {used:?} failed ({err}) — set RAT_PAGER or install less"
        )),
    }
}

/// Spawn the first launchable pager candidate; on Windows the default chain
/// ends in the stock more.com, so this only fails when every candidate is
/// missing (or a configured pager is).
fn spawn_first(pagers: &[PagerCommand]) -> std::io::Result<(String, std::process::Child)> {
    let mut last_err =
        std::io::Error::new(std::io::ErrorKind::NotFound, "no pager candidates resolved");
    for pager in pagers {
        match std::process::Command::new(&pager.bin)
            .args(&pager.args)
            .stdin(std::process::Stdio::piped())
            .spawn()
        {
            Ok(child) => return Ok((pager.bin.clone(), child)),
            Err(err) => last_err = err,
        }
    }
    Err(last_err)
}

/// The child for one tick, fully configured on the loop thread: the
/// shell or direct form, a null stdin while we own the keyboard, and
/// the per-tick environment measured by the CALLER — whoever runs the
/// command never reads the terminal and never sees the palette. This
/// function is the seam a future non-process source would replace:
/// everything upstream of it deals only in a spec and a geometry.
fn build_source_command(
    spec: &SourceSpec,
    script: Option<&std::path::Path>,
    interactive: bool,
    appearance: Appearance,
    geom: PaneGeometry,
) -> std::process::Command {
    let mut command = match (&spec.program, script) {
        // A shebang body, materialized. On unix the KERNEL parses the
        // `#!`; on Windows we did.
        (SourceProgram::Script(body), Some(path)) => {
            let line = shebang(body).expect("a materialized script has a shebang");
            interpreter_command(SHEBANG_ARM, &line, path)
        }
        // A body with no `#!`: the same shell path a `command` under a
        // shell takes, which is what mirrors unix ENOEXEC. Only
        // no-shebang bodies reach here (materialization covers every
        // shebang body), and those are never Direct.
        (SourceProgram::Script(body), None) => {
            let (program, flags) = shell_invocation(&spec.shell)
                .expect("a shebang-less script body never resolves to Direct");
            shell_command(&program, flags, body)
        }
        // Shipped behavior, untouched bytes. A shell spec holds the
        // raw script as one element, so the join reproduces it byte
        // for byte.
        (SourceProgram::Argv(argv), _) => match shell_invocation(&spec.shell) {
            Some((program, flags)) => shell_command(&program, flags, &argv.join(" ")),
            None => {
                let mut cmd = std::process::Command::new(&argv[0]);
                cmd.args(&argv[1..]);
                cmd
            }
        },
    };
    if interactive {
        command.stdin(std::process::Stdio::null());
    }
    // Children lay out against their pane without a tty side channel:
    // the geometry is re-measured every tick, so scripts adapt to
    // resizes live. Without panes the inner size IS the terminal size,
    // so the plain-watch environment cannot move.
    command.env("RAT_WIDTH", geom.inner_cols.to_string());
    command.env("RAT_HEIGHT", geom.inner_rows.to_string());
    // Children inherit the controlling terminal, so a child that resolved its
    // own appearance would query a terminal this process is reading from.
    // Hand it the verdict instead.
    command.env("RAT_APPEARANCE", appearance.as_str());
    command
}

/// How many lines any one source retains.
///
/// Measured rather than picked: a thousand lines is about 2% of a loop
/// slice to diff and some 76 KB of memory — three orders of magnitude
/// above any pane's height, and three below the point where a single
/// emission eats a whole slice. It is not declarable per pane, and that
/// is deliberate: a bound nobody has hit does not need a knob, and
/// adding one later is additive where removing one is not.
const MAX_RETAINED_LINES: usize = 1000;

/// What a tick discarded, said out loud, or `None` when it kept
/// everything.
///
/// **A LINE count, never a byte figure**, because the bound is a line
/// count — a size here would describe a limit that does not exist.
/// Derived per outcome exactly as the exit badge is, so it clears by
/// itself: a pane that stops flooding stops accusing itself.
///
/// Large counts are the normal case for anything that trips this, so
/// the number is abbreviated to keep the marker inside a chrome row
/// that is already carrying two other badges.
fn dropped_badge(dropped: usize) -> Option<String> {
    (dropped > 0).then(|| format!("{} lines dropped", compact_count(dropped)))
}

/// The policy for one source: how much it retains, and which end.
///
/// **This is the one place the declaration's vocabulary and the
/// worker's meet.** The worker knows nothing about panes, so the
/// mapping cannot live there; the loop knows both, so it lives here.
///
/// A source with no pane box is `rat watch`, which has no declared
/// overflow at all. It keeps the tail — a watch is for what its command
/// is printing now, and its frame is already a tail with scrollback
/// behind it. Note that this is a visible change for a watch whose
/// command floods: it used to retain everything.
fn retention_for(registry: &Registry, id: SourceId) -> Retention {
    let keep = match registry.pane(id).map(|pane| pane.overflow) {
        Some(Overflow::KeepTop) => Keep::Top,
        Some(Overflow::KeepBottom) | None => Keep::Bottom,
    };
    Retention {
        max_lines: MAX_RETAINED_LINES,
        keep,
    }
}

/// `build_source_command` plus the pane identity, which only exists
/// under a declared layout: `Registry::pane` answers `None` without
/// one, so no RAT_PANE is ever exported to a plain watch child.
fn source_command(
    registry: &Registry,
    scripts: &ScriptFiles,
    id: SourceId,
    interactive: bool,
    appearance: Appearance,
    geom: PaneGeometry,
) -> std::process::Command {
    let mut command = build_source_command(
        registry.spec(id),
        scripts.path(id),
        interactive,
        appearance,
        geom,
    );
    if registry.pane(id).is_some() {
        command.env("RAT_PANE", &registry.spec(id).id);
    }
    command
}

/// Unix: restore the terminal on INT/TERM/HUP. Windows: the interactive
/// path reads Ctrl-C as a key event; piped mode has no terminal state to
/// restore, so default console handling suffices.
#[cfg(unix)]
fn register_signals() -> Result<(Arc<AtomicBool>, Arc<AtomicBool>), AppError> {
    let interrupted = Arc::new(AtomicBool::new(false));
    let terminated = Arc::new(AtomicBool::new(false));
    signal_hook::flag::register(signal_hook::consts::SIGINT, Arc::clone(&interrupted))
        .context("registering SIGINT")?;
    signal_hook::flag::register(signal_hook::consts::SIGTERM, Arc::clone(&terminated))
        .context("registering SIGTERM")?;
    signal_hook::flag::register(signal_hook::consts::SIGHUP, Arc::clone(&terminated))
        .context("registering SIGHUP")?;
    Ok((interrupted, terminated))
}

#[cfg(windows)]
fn register_signals() -> Result<(Arc<AtomicBool>, Arc<AtomicBool>), AppError> {
    Ok((
        Arc::new(AtomicBool::new(false)),
        Arc::new(AtomicBool::new(false)),
    ))
}

/// One wait slice through the event library's pump: the Windows input path,
/// and the unix fallback when the terminal device cannot be opened. Non-key
/// events are discarded exactly as they were before the split.
fn crossterm_slice(nap: Duration) -> anyhow::Result<Vec<TapEvent>> {
    if !crossterm::event::poll(nap).context("polling events")? {
        return Ok(Vec::new());
    }
    match crossterm::event::read().context("reading event")? {
        crossterm::event::Event::Key(key_event) => match from_crossterm(key_event) {
            Some(key) => Ok(vec![TapEvent::Key(key)]),
            None => Ok(Vec::new()),
        },
        crossterm::event::Event::Mouse(mouse_event) => {
            use crossterm::event::MouseEventKind as K;
            let kind = match mouse_event.kind {
                K::ScrollUp => MouseKind::WheelUp,
                K::ScrollDown => MouseKind::WheelDown,
                K::ScrollLeft => MouseKind::WheelLeft,
                K::ScrollRight => MouseKind::WheelRight,
                _ => return Ok(Vec::new()),
            };
            Ok(vec![TapEvent::Mouse(MouseEvent {
                kind,
                shift: mouse_event
                    .modifiers
                    .contains(crossterm::event::KeyModifiers::SHIFT),
                notches: 1,
            })])
        }
        _ => Ok(Vec::new()),
    }
}

/// A terminal's theme notification says *that* something changed, not what
/// the colors now are — it reports the application's appearance, which can
/// disagree with the palette actually on screen. So a notification only
/// arms a measurement: one foreground/background exchange, classified by
/// the same rule the startup probe uses.
#[cfg(unix)]
#[derive(Default)]
struct VerifyState {
    pending: bool,
    fg: Option<xterm_color::Color>,
    in_flight_until: Option<Instant>,
}

#[cfg(unix)]
impl VerifyState {
    /// Feed one color reply. Returns the verdict once the background reply
    /// completes an exchange this loop actually asked for.
    fn reply(&mut self, kind: OscColorKind, color: xterm_color::Color) -> Option<Appearance> {
        // A reply nobody asked for is never adopted.
        self.in_flight_until?;
        match kind {
            OscColorKind::Foreground => {
                self.fg = Some(color);
                None
            }
            OscColorKind::Background => {
                self.in_flight_until = None;
                let verdict = classify_colors(self.fg.as_ref(), &color);
                self.fg = None;
                Some(verdict)
            }
        }
    }
}

/// Adopt an appearance the terminal reported. Returns true when the verdict
/// actually changed; a repeat is a no-op, so a terminal that re-announces an
/// unchanged theme costs nothing.
#[cfg_attr(windows, allow(dead_code))] // Called from the unix input path.
fn adopt(palette: &mut Palette, reported: Appearance) -> bool {
    if palette.appearance == reported {
        return false;
    }
    *palette = Palette::builtin(reported, AppearanceSource::Notification);
    true
}

/// Where a frame's geometry comes from when the terminal cannot be
/// measured: the RAT_WIDTH/RAT_HEIGHT a parent handed down, then the
/// fallback. This is what lets a piped `rat dashboard --once` nested
/// inside another dashboard's pane size itself to that pane instead of
/// a hardcoded 80x24. Unparsable values fall through, not fail.
fn size_fallback(
    env_cols: Option<&str>,
    env_rows: Option<&str>,
    fallback: (u16, u16),
) -> (u16, u16) {
    let parse = |v: Option<&str>| v.and_then(|v| v.parse::<u16>().ok());
    (
        parse(env_cols).unwrap_or(fallback.0),
        parse(env_rows).unwrap_or(fallback.1),
    )
}

/// One measure of the frame's geometry. Interactive mode asks the
/// terminal. Piped mode asks the CONSUMER first: the frame is being
/// composed for whoever reads the pipe, and RAT_WIDTH/RAT_HEIGHT is
/// that consumer saying how wide it is — the controlling terminal
/// (still reachable through /dev/tty even when stdout is a pipe) is
/// the wrong authority there.
fn measure_size(is_tty: bool, fallback: (u16, u16)) -> (u16, u16) {
    let measured = crossterm::terminal::size().ok();
    if is_tty {
        return measured.unwrap_or(fallback);
    }
    let base = measured.unwrap_or(fallback);
    let cols = std::env::var("RAT_WIDTH").ok();
    let rows = std::env::var("RAT_HEIGHT").ok();
    size_fallback(cols.as_deref(), rows.as_deref(), base)
}

/// The plain-watch spawn-step re-measure. A NO-OP when a resize arm is
/// running: that arm is the geometry pair's only writer then — a
/// spawn-step write would consume a coincident resize first and blind
/// the arm's change detection (no reflow, no debounce fire, every
/// not-due pane stranded at the superseded geometry).
fn refresh_geometry_for_spawn(
    resize_respawn: bool,
    measured: (u16, u16),
    reserved: u16,
    size: &mut (u16, u16),
    geom: &mut Vec<PaneGeometry>,
    registry: &Registry,
) {
    if resize_respawn {
        return;
    }
    *size = measured;
    *geom = registry.geometry_reserving(measured, reserved);
}

/// What one resize detection observed.
struct ResizeStep {
    size_moved: bool,
    geom_moved: bool,
}

/// The resize arm's detection: advance the geometry pair from a fresh
/// measure and report what moved. The arm's reflow keys off
/// `size_moved`; the respawn is owed only when a pane's INNER geometry
/// moved — that is the child's environment. Declared heights are
/// pinned, so a terminal that only grew rows changes the window and no
/// child's world — EXCEPT under a zoom, whose row budget is
/// `window_rows` and genuinely follows the terminal.
fn detect_resize(
    measured: (u16, u16),
    max_height: Option<u16>,
    gutter: bool,
    panes: &PaneView,
    size: &mut (u16, u16),
    geom: &mut Vec<PaneGeometry>,
    registry: &Registry,
) -> ResizeStep {
    let size_moved = measured != *size;
    *size = measured;
    // The same derivation every other geometry site applies: a view
    // change that moved the stored geom must compare EQUAL here, or
    // the very next iteration arms the debounce gate and every child —
    // live ones included — restarts 250ms after a keypress.
    let next = derive_geometry(registry, measured, max_height, gutter, panes);
    let geom_moved = next != *geom;
    if geom_moved {
        *geom = next;
    }
    ResizeStep {
        size_moved,
        geom_moved,
    }
}

/// A pane's error prefix: empty for plain watch (whose messages are
/// byte-frozen), the pane's name under a declared layout — at N the
/// question "whose trigger/whose error?" has an answer.
fn pane_label(registry: &Registry, id: SourceId) -> String {
    match registry.pane(id) {
        Some(_) => format!("pane {}: ", registry.spec(id).id),
        None => String::new(),
    }
}

/// The one-shot end-of-life line for a dead reader. `rat watch` keeps
/// its shipped sentence byte for byte; a pane names itself first.
/// Only fifo/fd readers can end, and those are unix-only — on Windows
/// this has no caller by construction.
#[cfg_attr(windows, allow(dead_code))]
fn ended_text(registry: &Registry, id: SourceId, spec: &TriggerSpec) -> String {
    match registry.pane(id) {
        Some(_) => format!("{}: trigger ended: {spec}", registry.spec(id).id),
        None => format!("trigger ended: {spec}"),
    }
}

/// The one-shot line for a suspected trigger loop: the panes, what is
/// suspected, and the evidence.
///
/// Unlike `ended_text` this has callers on EVERY platform — only
/// fifo/fd readers can end, but a `file:` trigger loops anywhere — so it
/// carries no `cfg` of any kind, deliberately.
///
/// **`suspected` is the load-bearing word.** rat observes that watched
/// paths change while it is busy and never while it is idle, which is
/// what a loop looks like rather than proof of one. The badge is
/// confident because nine cells cannot hold a hedge; this row has the
/// room, so it does the hedging for both.
///
/// The paths are what make the claim falsifiable — a user who knows the
/// accusation is wrong can see what rat is reading and carry on, because
/// nothing has been stopped. They are named as a SET and deduplicated:
/// two panes watching one path name it once, and no ordering is implied
/// anywhere in the sentence, because concurrent children make direction
/// unavailable even as coincidence.
fn looping_text(registry: &Registry, panes: &[SourceId]) -> String {
    let names: Vec<&str> = panes
        .iter()
        .filter(|id| registry.pane(**id).is_some())
        .map(|id| registry.spec(*id).id.as_str())
        .collect();
    let mut watched: Vec<String> = Vec::new();
    for spec in panes.iter().map(|id| registry.spec(*id)) {
        for trigger in &spec.triggers {
            let text = trigger.to_string();
            if !watched.contains(&text) {
                watched.push(text);
            }
        }
    }
    // With no pane there is no name to lead with, so the sentence starts
    // at the suspicion — the rule `ended_text` already follows.
    let who = if names.is_empty() {
        String::new()
    } else {
        format!("{}: ", names.join(", "))
    };
    format!(
        "{who}trigger loop suspected: {} — ? help",
        watched.join(", ")
    )
}

/// Whether this verdict implicates a pane the last one did not — the
/// notice's edge. The latch is the previously announced set, carried by
/// the caller and updated here.
///
/// **The edge is over the SET, not over "anyone at all".** The panes of
/// one cycle cross the respawn threshold a hop apart, not together, so
/// an empty→non-empty latch would fire while the set held only whichever
/// pane got there first — and then stay silent as the rest arrived,
/// leaving a one-shot row naming half a loop and half its evidence. That
/// is not a hypothetical: the end-to-end test caught it announcing `a`
/// alone in a two-pane cycle. Announcing again as the set grows costs a
/// second row a hop later, which REPLACES the first (the row is
/// one-shot), so what survives on screen is the whole set.
///
/// The latch is the panes announced during this **episode**, and only a
/// verdict implicating NOBODY ends it. A set that merely shrinks says
/// nothing — membership wobbles while a loop spins, and forgetting a pane
/// the moment it drops out would re-announce it the moment it returned,
/// which is a repaint storm reporting nothing new.
///
/// An **abstention holds** the latch instead of resetting it. Declining
/// to answer is not the same as finding nothing (`Verdict::abstained`
/// exists to tell them apart), and a busy dashboard abstains — so a
/// reset would re-announce one unbroken loop every time the dashboard
/// got busy and then quiet again.
fn rising_edge(latch: &mut Vec<SourceId>, verdict: &Verdict) -> bool {
    if verdict.abstained {
        return false;
    }
    if verdict.panes.is_empty() {
        latch.clear();
        return false;
    }
    let mut grew = false;
    for id in &verdict.panes {
        if !latch.contains(id) {
            latch.push(*id);
            grew = true;
        }
    }
    grew
}

/// What the dashboard's title READS AS — the plain text a consumer
/// (the terminal tab title) gets, never a rendered line. Static text
/// is itself; a pane-sourced title is the referenced pane's latest
/// output — escapes stripped, first non-empty line, trimmed — with
/// the declared fallback while the pane has not spoken. Silence is
/// not a title: all-blank output keeps the fallback too.
fn title_role_text(
    title: &crate::core::registry::TitleSource,
    runtime: &[SourceRuntime],
) -> Option<String> {
    use crate::core::registry::TitleSource;
    match title {
        TitleSource::None => None,
        TitleSource::Static(text) => Some(text.clone()),
        TitleSource::Pane { source, fallback } => runtime[source.0]
            .output
            .as_deref()
            .and_then(|lines| {
                lines
                    .iter()
                    .map(|line| crate::core::measure::strip_escapes(line).trim().to_string())
                    .find(|line| !line.is_empty())
            })
            .or_else(|| fallback.clone()),
    }
}

/// `pane "logs"` / `panes "logs", "metrics"` — the names a diagnostic
/// points with, quoted the way the author wrote them.
fn pane_list(registry: &Registry, waiting: &[SourceId]) -> String {
    let names = waiting
        .iter()
        .map(|id| format!("{:?}", registry.spec(*id).id))
        .collect::<Vec<_>>()
        .join(", ");
    let noun = if waiting.len() == 1 { "pane" } else { "panes" };
    format!("{noun} {names}")
}

/// `5s` / `300ms` — a wait, briefly.
fn brief_duration(d: Duration) -> String {
    if d.subsec_millis() == 0 {
        format!("{}s", d.as_secs())
    } else {
        format!("{}ms", d.as_millis())
    }
}

/// The one-shot stderr notice for a `--once` run that has gone quiet:
/// which panes it is waiting on, for how long, and what to WRITE. A
/// pane that never exits by design must be declared `live=#true` — but
/// a pane that already declared it is never told to; when every waiting
/// pane is live, the wait is simply a child that has not spoken yet.
/// Pure and portable: no clock reads, no cfg, no mutation.
fn once_waiting_text(registry: &Registry, waiting: &[SourceId], after: Duration) -> String {
    let all_live = waiting.iter().all(|id| registry.spec(*id).live);
    let state = if all_live {
        if waiting.len() == 1 {
            "the live child has printed nothing yet. "
        } else {
            "the live children have printed nothing yet. "
        }
    } else {
        "no output, no exit. A command that follows instead of exiting \
         must be declared `live=#true`; "
    };
    format!(
        "rat dashboard: --once is still waiting on {} after {}: {state}`--once-timeout 30s` bounds the wait.",
        pane_list(registry, waiting),
        brief_duration(after),
    )
}

/// What `--once-timeout` says when it gives up: the same
/// declared/undeclared split as the notice, past tense. An undeclared
/// pane never FINISHED — finishing was its contract — and is taught
/// the declaration; a declared-live pane never produced its first
/// output, and there is nothing to teach.
fn once_timeout_text(registry: &Registry, waiting: &[SourceId], after: Duration) -> String {
    let all_live = waiting.iter().all(|id| registry.spec(*id).live);
    let tail = if all_live {
        if waiting.len() == 1 {
            "never produced its first output."
        } else {
            "never produced their first output."
        }
    } else {
        "never finished. A command that follows instead of exiting must \
         be declared `live=#true`."
    };
    format!(
        "--once gave up after {}: {} {tail}",
        brief_duration(after),
        pane_list(registry, waiting),
    )
}

/// A pane's spawn-error wording: the pane names itself (the default
/// border draws no title, so the body is the only surface that can),
/// the OS reason comes next, the path comes last. A pane truncates
/// from the RIGHT at a declared width no terminal can widen, so
/// whatever sits last is what no reader sees — and the path is the
/// part the author just typed, while the reason exists nowhere else.
fn pane_spawn_error_text(pane: &str, program: &str, err: &std::io::Error) -> String {
    format!("{pane}: {err}: {program:?}")
}

/// Plain watch's looping spawn-error line, byte for byte the shipped
/// wording. Full-width, so nothing truncates the reason off the end.
fn watch_spawn_error_text(program: &str, err: &std::io::Error) -> String {
    format!("watch: {program:?}: {err}")
}

/// One pane's body lines for one outcome: the spawn-error text when the
/// command never started, otherwise the child's stdout FOLLOWED BY its
/// stderr — that order is the rule, the generalization of shipped
/// watch, which shows both regardless of exit. Trailing newlines are
/// trimmed per stream, exactly as `compose_frame` does.
fn pane_body(
    stdout: &[u8],
    stderr: &[u8],
    spawn_error: Option<&std::io::Error>,
    pane: &str,
    program: &str,
) -> Vec<String> {
    if let Some(err) = spawn_error {
        return vec![pane_spawn_error_text(pane, program, err)];
    }
    output_lines(stdout, stderr)
}

/// The chrome row's failure badge: a nonzero exit, and nothing else. A
/// spawn error's text IS the body, so it carries no badge; a successful
/// tick returns `None`, which is how the badge clears.
fn exit_badge(status: Option<std::process::ExitStatus>) -> Option<String> {
    let status = status?;
    (!status.success()).then(|| match status.code() {
        Some(code) => format!("exit {code}"),
        // A signalled child has no code; name the signal's absence
        // rather than inventing one.
        None => "killed".to_string(),
    })
}

/// One pane's change signature: its body AND every badge it carries,
/// which are part of what the pane displays. A pane can print
/// byte-identical output and start failing, or start looping — without
/// the badges in the pane's own hash, the composition would carry a
/// badge nothing ever paints. The combining key stays outcome-derived
/// and geometry-free.
fn body_signature(
    lines: &[String],
    failure: Option<&str>,
    looping: bool,
    truncated: Option<&str>,
) -> u64 {
    let mut bytes = lines.join("\n").into_bytes();
    if let Some(failure) = failure {
        bytes.push(b'\n');
        bytes.extend_from_slice(failure.as_bytes());
    }
    if looping {
        bytes.push(b'\n');
        bytes.extend_from_slice(b"looping");
    }
    if let Some(truncated) = truncated {
        bytes.push(b'\n');
        bytes.extend_from_slice(truncated.as_bytes());
    }
    signature(&bytes)
}

/// Record one source's fresh body. The comparand and the marks move
/// ONLY on a distinct output — that is what makes a slow pane's mark
/// outlive a fast pane's ticks — and so does the last-CHANGE stamp the
/// chrome row shows. Marks are computed here, unconditionally, not
/// lazily at paint time: they must already exist when the gutter is
/// switched on, and per-pane diffs are strictly less work than the
/// composed diff they replace.
/// Fold one fresh body into a pane's runtime and report whether what the
/// pane DISPLAYS moved — which is the question the compose gate asks,
/// and not the same as whether the source produced anything.
///
/// The one path for the only two things that produce a body: a child's
/// completion and a long-lived child's emission. They differ in exactly
/// one argument — the exit badge, which an emission never carries — and
/// two copies of the rest would be two chances for a live pane and a
/// batch pane to disagree about what a body is.
///
/// The badges are set HERE, before recording, because `record_output`
/// folds them into the pane's hash: a badge that cannot repaint is a
/// badge nobody sees.
fn record_pane_body(
    r: &mut SourceRuntime,
    lines: Vec<String>,
    failure: Option<String>,
    dropped: usize,
    at: jiff::Timestamp,
) -> bool {
    let old_hash = r.hash;
    let was_posted = r.posted;
    r.failure = failure;
    r.truncated = dropped_badge(dropped);
    record_output(r, lines, at);
    r.hash != old_hash || !was_posted
}

fn record_output(r: &mut SourceRuntime, lines: Vec<String>, at: jiff::Timestamp) {
    // Every badge is part of what the pane displays, so they all join
    // this pane's hash: a badge that cannot repaint is invisible.
    let hash = body_signature(
        &lines,
        r.failure.as_deref(),
        r.looping,
        r.truncated.as_deref(),
    );
    if r.output.is_none() || r.hash != hash {
        r.previous = r.output.take();
        r.marks = changed_marks(r.previous.as_deref(), &lines);
        r.hash = hash;
        r.changed_at = at;
        r.output = Some(lines);
    }
}

/// Write the suspicion verdict into per-source state, and report
/// whether any pane's badge actually moved on screen.
///
/// The verdict is decided in the trigger arm, while `record_output` runs
/// while draining an outcome — so a badge that only rode the drain would
/// appear at the next completion at the earliest, and a CLEARED badge
/// would sit on an idle pane indefinitely. A transition therefore
/// re-enters `record_output` with the pane's RETAINED body: the same
/// path `· exit N` takes, so the two badges beside each other can never
/// behave differently, and so the pane is re-dated exactly as a badge
/// transition already re-dates it. Nothing re-runs, and no schedule is
/// touched.
///
/// `boxed` is whether the panes have a chrome row at all: a plain watch
/// has nowhere to show a badge, and a pane that has not yet completed
/// has no body to recompose, so both record the state and repaint
/// nothing. The badge folds into the hash at their next `record_output`.
fn apply_verdict(
    runtime: &mut [SourceRuntime],
    implicated: &[SourceId],
    boxed: bool,
    at: jiff::Timestamp,
) -> bool {
    let mut moved = false;
    for (i, r) in runtime.iter_mut().enumerate() {
        let looping = implicated.contains(&SourceId(i));
        if r.looping == looping {
            continue;
        }
        r.looping = looping;
        if let (true, Some(body)) = (boxed, r.output.clone()) {
            record_output(r, body, at);
            moved = true;
        }
    }
    moved
}

/// Carry the frame's own change key and stamp to the panes' after a
/// verdict moved a badge — what the drain step would have done had an
/// outcome carried the change. The footer names the last time anything
/// on screen changed, and that is now one of these panes. Only the
/// verdict path calls this: the resize reflow and the counting refresh
/// deliberately re-date nothing, because neither changed any content.
fn restamp_live(live: &mut Live, runtime: &[SourceRuntime]) {
    let at = runtime
        .iter()
        .map(|r| r.changed_at)
        .max()
        .unwrap_or(live.changed_at);
    live.hash = combined_hash(runtime);
    live.changed_at = at;
    live.since = local_hms(at);
}

/// Which marks a paint uses. Under panes, a LIVE or LIVE-SCROLLED
/// surface shows the per-pane marks already composed into frame
/// coordinates; a PAUSED or SCRUBBED frame — and every plain-watch
/// frame — diffs the viewed composition against its history
/// predecessor, the only comparand a past composition has.
fn paint_marks(
    panes: bool,
    mode: FrameMode,
    live_marks: &[LineMark],
    viewed: &[String],
    prev: Option<&[String]>,
) -> Vec<LineMark> {
    if panes && mode != FrameMode::Paused {
        return live_marks.to_vec();
    }
    changed_marks(prev, viewed)
}

/// Whole seconds every counting row is showing, folded into one gate
/// value: today's number for a plain watch, a hash of (footer age,
/// each pane's age) for a dashboard with `t` flipped — 0 under the
/// default absolute style either way, which is what keeps a parked
/// dashboard byte-silent.
fn displayed_age_key(
    pause: Option<&PauseState>,
    live_scroll: Option<LiveScroll>,
    alt_time: bool,
    changed_at: jiff::Timestamp,
    pane_changed_at: &[jiff::Timestamp],
) -> u64 {
    use std::hash::{Hash, Hasher};

    let footer = displayed_age(pause, live_scroll, alt_time, changed_at);
    // A frozen frame's pane chrome is frozen BYTES — a literal copy is
    // never re-laid-out — so only the paused row can count while
    // parked, and that is exactly today's number.
    if !alt_time || pause.is_some() || pane_changed_at.is_empty() {
        return footer;
    }
    let mut hasher = std::hash::DefaultHasher::new();
    footer.hash(&mut hasher);
    for at in pane_changed_at {
        age_seconds(*at).hash(&mut hasher);
    }
    hasher.finish()
}

/// Each chrome-bearing pane's last-change time, in registry order.
/// Empty under a plain watch and under a dashboard whose panes all
/// declare `chrome = false`: nothing on screen is counting.
fn chrome_ages(registry: &Registry, runtime: &[SourceRuntime]) -> Vec<jiff::Timestamp> {
    registry
        .ids()
        .filter(|id| registry.pane(*id).is_some_and(|p| p.chrome))
        .map(|id| runtime[id.0].changed_at)
        .collect()
}

/// Recompose the live frame in place from the retained outputs — the
/// resize reflow, the `t` flip, and the 1 Hz counting refresh all
/// re-enter the compose through here. Nothing is re-dated and nothing
/// is recorded: history takes only collect-step compositions.
#[allow(clippy::too_many_arguments)]
/// A pane's display name: its declared title where it has one, else
/// the source's id. ONE resolution, so the border label and the
/// footer's focus segment can never name the same pane differently.
fn pane_display_name(registry: &Registry, id: SourceId) -> &str {
    registry
        .pane(id)
        .and_then(|pane| pane.title.as_deref())
        .unwrap_or(&registry.spec(id).id)
}

/// The footer's focus segment: the focused pane's display name, or
/// None when nothing is focused.
///
/// A footer segment may change only when a `PaintKey` field changes —
/// the run-constant tail rule — and this one is admissible exactly
/// because `PaneViewKey.focus` is IN the key. A counter here would not
/// be, and would go silently stale.
fn focus_segment(
    registry: &Registry,
    runtime: &[SourceRuntime],
    geom: &[PaneGeometry],
    view: &PaneView,
) -> Option<String> {
    let id = view.focus?;
    let mut seg = format!("focus {}", pane_display_name(registry, id));
    // D5: a `chrome = #false` pane is scrollable and has nowhere on
    // itself to say where its window is, so the footer says it. A
    // chromed pane already carries the badge, and saying it twice would
    // move the footer for something the reader can already see.
    if let Some(pane) = registry.pane(id)
        && !pane.chrome
        && let Some(badge) = pane_scroll_badge(
            view.scroll[id.0],
            pane.overflow,
            runtime[id.0].output.as_ref().map_or(0, Vec::len),
            geom[id.0].inner_rows as usize,
        )
    {
        seg.push_str(" · ");
        seg.push_str(&badge);
    }
    Some(seg)
}

#[allow(clippy::too_many_arguments)]
fn recompose_live(
    live: &mut Option<Live>,
    registry: &Registry,
    runtime: &[SourceRuntime],
    geom: &[PaneGeometry],
    view: &PaneView,
    alt_time: bool,
    palette: &Palette,
    profile: ColorProfile,
) {
    let Some(l) = live.as_mut() else {
        return;
    };
    if matches!(registry.composition(), Composition::Plain { .. }) {
        return;
    }
    let block = compose_sources(registry, runtime, geom, view, alt_time, palette, profile);
    l.lines = block.lines;
    l.panes = Some(PaneLive {
        marks: block.marks,
        ages: chrome_ages(registry, runtime),
    });
}

/// The `Composition::Panes` compose step — ONE named function, because
/// it has exactly three re-entrants: the collect step, the resize
/// arm's reflow, and the counting-refresh path. Renders every source
/// into its declared box and joins them by the layout tree.
#[allow(clippy::too_many_arguments)]
fn compose_sources(
    registry: &Registry,
    runtime: &[SourceRuntime],
    geom: &[PaneGeometry],
    view: &PaneView,
    alt_time: bool,
    palette: &Palette,
    profile: ColorProfile,
) -> PaneBlock {
    let Composition::Panes {
        layout,
        gap,
        row_gap,
        title,
    } = registry.composition()
    else {
        return PaneBlock::default();
    };
    let blocks: Vec<PaneBlock> = registry
        .ids()
        .map(|id| {
            let source = &runtime[id.0];
            let spec = registry.spec(id);
            let pane = registry
                .pane(id)
                .expect("a Panes registry boxes every source");
            let cadence = cadence_label(spec);
            // The last-CHANGE stamp, never last-produced: a produced-at
            // stamp would repaint every tick and cost byte-silence. A
            // pane that has not run yet has no instant to name. `t`
            // flips this row with the footer — one style across the
            // whole surface.
            let stamp = if !source.posted {
                "…".to_string()
            } else if alt_time {
                age_text(age_seconds(source.changed_at))
            } else {
                local_hms(source.changed_at)
            };
            let body = source.output.as_deref().unwrap_or(&[]);
            let scrolled = pane_scroll_badge(
                view.scroll[id.0],
                pane.overflow,
                body.len(),
                geom[id.0].inner_rows as usize,
            );
            let chrome = PaneChrome {
                title: pane_display_name(registry, id),
                cadence: &cadence,
                stamp: &stamp,
                failure: source.failure.as_deref(),
                looping: source.looping,
                truncated: source.truncated.as_deref(),
                scrolled: scrolled.as_deref(),
                focused: view.focus == Some(id),
                zoomed: view.zoomed == Some(id),
            };
            if view.collapsed[id.0] && view.zoomed != Some(id) {
                // Collapse hides the body; zoom overrules it (INV-12),
                // so a zoomed pane composes its full content and the
                // bit survives to restore the row on unzoom.
                return render_pane_collapsed(pane, geom[id.0], &chrome, palette, profile);
            }
            render_pane(
                body,
                &source.marks,
                pane,
                geom[id.0],
                // The pane's own window. At rest this IS
                // `overflow_clip(pane.overflow, body.len(), inner_rows)`
                // — brief seam 5, pinned by the at-rest equality test.
                view.scroll[id.0].offset(),
                &chrome,
                palette,
                profile,
            )
        })
        .collect();
    // A zoomed pane composes ALONE: compose_panes takes its root as a
    // parameter, so the hidden panes' blocks are simply never joined.
    // Bound first — a `&LayoutNode::Pane(id)` built inside the call
    // would be a temporary that dies at the semicolon.
    let zoom_root = view.zoomed.map(LayoutNode::Pane);
    let mut block = compose_panes(
        zoom_root.as_ref().unwrap_or(layout),
        &blocks,
        *gap,
        *row_gap,
    );
    // The dashboard's own name: one bold line above the composed
    // panes, the same treatment `rat watch --title` gives a plain
    // frame — and prepended HERE so the collect step, the resize
    // reflow, and the counting refresh all carry it by construction.
    // Truncated to the composed width, never the terminal's: the
    // title belongs to the dashboard it names. The empty mark keeps
    // marks aligned to lines; a title never carries a change mark.
    if let crate::core::registry::TitleSource::Static(title) = title {
        let composed = block
            .lines
            .iter()
            .map(|line| crate::core::measure::display_width(line))
            .max()
            .unwrap_or(0);
        let text =
            crate::core::measure::truncate_display(title, composed, crate::core::measure::ELLIPSIS);
        block.lines.insert(
            0,
            StyleSpec {
                bold: true,
                ..StyleSpec::default()
            }
            .render(&text, profile),
        );
        block.marks.insert(0, LineMark::default());
    }
    block
}

/// The pane chrome's cadence phrase. Unlike watch's footer, which
/// echoes the user's own `-n` token, a pane's interval is a `Duration`
/// by the time it reaches the engine, so the phrase is formatted back
/// from it.
pub(crate) fn cadence_label(spec: &SourceSpec) -> String {
    // A live source was spawned ONCE and is not on a cadence, so an
    // interval here would be a plain false statement — measured on
    // main, the pane read `every 2s` while its child had been running
    // continuously since startup. Checked FIRST because a live spec
    // may still carry an interval (the respawn delay if the child
    // exits) and triggers; they simply do not describe how it runs.
    if spec.live {
        return "live".to_string();
    }
    match (spec.interval, spec.triggers.is_empty()) {
        (Some(interval), true) => format!("every {}", interval_words(interval)),
        (Some(interval), false) => {
            format!("every {} or on trigger", interval_words(interval))
        }
        (None, false) => "on trigger".to_string(),
        (None, true) => "once".to_string(),
    }
}

/// A duration as the shortest honest token: 90s stays `90s` (never
/// `1m30s` — one unit, no arithmetic surprises), whole minutes and
/// hours shorten, sub-second prints millis.
fn interval_words(interval: Duration) -> String {
    let millis = interval.as_millis();
    let secs = interval.as_secs();
    if millis == 0 || !millis.is_multiple_of(1000) {
        return format!("{millis}ms");
    }
    if secs.is_multiple_of(3600) {
        return format!("{}h", secs / 3600);
    }
    if secs.is_multiple_of(60) {
        return format!("{}m", secs / 60);
    }
    format!("{secs}s")
}

/// A child's raw bytes as frame lines: decoded by `core::decode`, the
/// trailing newline dropped, stderr lines after stdout's when present.
fn output_lines(stdout: &[u8], stderr: &[u8]) -> Vec<String> {
    let mut lines: Vec<String> = Vec::new();
    lines.extend(crate::core::decode::stream_lines(stdout));
    if !stderr.is_empty() {
        lines.extend(crate::core::decode::stream_lines(stderr));
    }
    lines
}

/// Runtime state per source: the schedule, the slot, and everything the
/// drain updates. The registry itself stays pure — these are the
/// resources it deliberately does not carry.
struct SourceRuntime {
    schedule: TickSchedule,
    slot: ChildSlot,
    tx: std::sync::mpsc::Sender<TickEvent>,
    /// This source's shared caps and one-slot outbox — `Some` only when
    /// the source declared itself live. `None` says batch, and the batch
    /// path never touches one, which is what keeps its bytes identical.
    emissions: Option<Emissions>,
    /// This source's rendered lines: without panes, the whole frame the
    /// shipped composer produced; with them, the child's own lines
    /// awaiting their box.
    output: Option<Vec<String>>,
    hash: u64,
    changed_at: jiff::Timestamp,
    /// The previous DISTINCT output — the mark comparand and nothing
    /// else, which is what makes a slow pane's mark outlive a fast
    /// pane's ticks.
    previous: Option<Vec<String>>,
    /// This source's marks against `previous`, in its own output
    /// coordinates; recomputed only when the output changes.
    marks: Vec<LineMark>,
    /// The chrome row's failure badge ("exit 3"), derived per outcome.
    failure: Option<String>,
    /// The chrome row's truncation marker ("1.2k lines dropped"), also
    /// derived per outcome, so a pane that stops flooding stops saying
    /// so. On the plain path this is what the status row and the piped
    /// run's stderr report instead.
    truncated: Option<String>,
    /// Whether this source has completed at least once — the once-mode
    /// exit condition at N sources.
    posted: bool,
    /// This source's own debounce window: its fires collapse into one
    /// respawn of THIS pane per window.
    gate: DebounceGate,
    /// The `file:` triggers it stat-polls.
    files: MtimeWatchSet,
    /// Whether the suspicion test currently implicates this pane. Separate
    /// from `failure` on purpose: that is outcome-derived and recomputed on
    /// every tick, while this is signal-derived and spans ticks — and a pane
    /// can legitimately be both failing and looping.
    looping: bool,
    /// Its open bracket, by stable id. Never a positional index: eviction
    /// removes older brackets, which would shift an index this still holds
    /// and close the wrong record.
    bracket: Option<BracketId>,
    /// Its fifo/fd reader threads; dropping the runtime joins them.
    #[cfg(unix)]
    readers: Vec<ReaderSlot>,
}

/// The change key: a combining hash over the per-source OUTPUT hashes
/// in registry order. Never over composed bytes — a resize would then
/// re-date the content and record a spurious distinct frame.
fn combined_hash(runtime: &[SourceRuntime]) -> u64 {
    use std::hash::{Hash, Hasher};
    let mut hasher = std::hash::DefaultHasher::new();
    for r in runtime {
        r.hash.hash(&mut hasher);
    }
    hasher.finish()
}

/// Fold one drained outcome into the frame's change stamp: only a
/// source whose OWN content changed may re-date the frame, and the
/// newest such stamp wins.
fn fold_changed_at(
    acc: Option<jiff::Timestamp>,
    changed: bool,
    at: jiff::Timestamp,
) -> Option<jiff::Timestamp> {
    match (acc, changed) {
        (acc, false) => acc,
        (Some(best), true) => Some(best.max(at)),
        (None, true) => Some(at),
    }
}

/// Every gesture is whole-dashboard: a resume means the whole surface
/// is stale, so every source is asked for a tick — the request an
/// in-flight child CAN discharge, because its environment is unchanged.
fn request_now_all(runtime: &mut [SourceRuntime]) {
    for r in runtime {
        r.schedule.request_now();
    }
}

/// A theme adoption supersedes the environment every in-flight child
/// was started under, so no completion discharges it: every source
/// spawns a fresh child.
fn request_respawn_all(runtime: &mut [SourceRuntime]) {
    for r in runtime {
        r.schedule.request_respawn();
    }
}

/// The `file:` paths among a set of triggers. On Windows `File` is the
/// only variant, so the match is exhaustively `Some` and clippy wants a
/// plain map — the unix arms are what make it a filter.
#[cfg_attr(windows, allow(clippy::unnecessary_filter_map))]
fn file_paths(triggers: &[TriggerSpec]) -> Vec<std::path::PathBuf> {
    triggers
        .iter()
        .filter_map(|trigger| match trigger {
            TriggerSpec::File(path) => Some(path.clone()),
            #[cfg(unix)]
            _ => None,
        })
        .collect()
}

fn signature(bytes: &[u8]) -> u64 {
    use std::hash::{Hash, Hasher};
    let mut hasher = std::hash::DefaultHasher::new();
    bytes.hash(&mut hasher);
    hasher.finish()
}

/// The program `ShellMode::Platform` runs — resolved at spawn time,
/// exactly as `%COMSPEC%` always was.
#[cfg(unix)]
fn platform_shell() -> String {
    "sh".to_string()
}

#[cfg(windows)]
fn platform_shell() -> String {
    std::env::var("COMSPEC").unwrap_or_else(|_| "cmd".to_string())
}

/// The platform shell's "run this string" flag. Fixed rather than
/// looked up in the dialect table: bare `--shell` (and `shell=#true`)
/// keeps its exact historical bytes even when `%COMSPEC%` names
/// something exotic — selecting different flags takes an explicit
/// `--shell=NAME`.
#[cfg(unix)]
fn platform_flags() -> &'static [&'static str] {
    &["-c"]
}

#[cfg(windows)]
fn platform_flags() -> &'static [&'static str] {
    &["/C"]
}

/// The shell's own flag(s) for "run this string": `cmd` takes `/C`,
/// PowerShell `-NoProfile -Command`, and everything else `-c` — sh,
/// bash, zsh, fish, nu, dash, ksh all agree on it. Matched on the file
/// NAME with a `.exe` stripped and lowercased, so a full path,
/// `pwsh.exe` and `PWSH.EXE` all land on the same row. An unknown name
/// is not an error: `-c` is what a program invoked with a script takes,
/// and a wrong guess surfaces as the child's own diagnostic.
///
/// `-NoProfile` because the child respawns every tick: the profile's
/// load cost would recur at the interval and anything it prints would
/// land in the frame. A script that wants the profile opts back in
/// (`--shell=pwsh -- '. $PROFILE; …'`); there would be no way to opt
/// out.
fn command_flags(program: &str) -> &'static [&'static str] {
    match interpreter_name(program).as_str() {
        "cmd" => &["/C"],
        "powershell" | "pwsh" => &["-NoProfile", "-Command"],
        _ => &["-c"],
    }
}

/// A program's dialect key: the file name, `.exe` stripped, lowercased.
/// `command_flags` and the interpreter tables must agree on what a
/// program is CALLED, or a full path would answer one table and not
/// the other. Windows names a program with its extension; unix never
/// does — and ONLY `.exe` is stripped, so a wrapper script `cmd.sh`
/// keeps its own row.
fn interpreter_name(program: &str) -> String {
    let name = std::path::Path::new(program)
        .file_name()
        .unwrap_or_default()
        .to_string_lossy()
        .to_ascii_lowercase();
    name.strip_suffix(".exe").unwrap_or(&name).to_string()
}

/// The program and flags a mode spawns, or `None` when there is no
/// shell. `Platform` NEVER consults the dialect table: bare `--shell`
/// is frozen bytes even under an exotic `COMSPEC`.
fn shell_invocation(mode: &ShellMode) -> Option<(String, &'static [&'static str])> {
    match mode {
        ShellMode::Direct => None,
        ShellMode::Platform => Some((platform_shell(), platform_flags())),
        ShellMode::Named(name) => Some((name.clone(), command_flags(name))),
    }
}

/// The `--shell` flag as a mode. Absent is no shell, bare is the
/// platform's, and a value names the program. An EMPTY value names
/// nothing — almost always a variable that expanded to nothing — so it
/// is refused rather than quietly taking the platform's shell.
fn shell_mode(flag: Option<&Option<String>>) -> anyhow::Result<ShellMode> {
    match flag {
        None => Ok(ShellMode::Direct),
        Some(None) => Ok(ShellMode::Platform),
        Some(Some(name)) if name.trim().is_empty() => Err(anyhow!(
            "--shell= names an empty shell — likely a variable that expanded \
             to nothing; write --shell=NAME (e.g. --shell=fish) or bare \
             --shell for the platform's shell"
        )),
        Some(Some(name)) => Ok(ShellMode::Named(name.clone())),
    }
}

/// The program a spec actually spawns — the SHELL under a shell mode,
/// where `command[0]` is the script rather than a program. A spawn
/// error must name what failed to start.
fn spawn_program(spec: &SourceSpec) -> String {
    match &spec.program {
        SourceProgram::Script(body) => match shebang(body) {
            Some(line) => shebang_program(SHEBANG_ARM, &line),
            // Unreachable for a well-formed spec (a shebang-less body
            // is never Direct), but a diagnostic path must not panic.
            None => shell_invocation(&spec.shell)
                .map(|(program, _)| program)
                .unwrap_or_default(),
        },
        SourceProgram::Argv(argv) => match shell_invocation(&spec.shell) {
            Some((program, _)) => program,
            None => argv[0].clone(),
        },
    }
}

/// One shell, one script. The platform lives in
/// `platform_shell`/`platform_flags` and the dialect in
/// `command_flags`; the one `#[cfg]` here is QUOTING, a concern only
/// Windows has — unix hands the script over as a real argv element
/// and never serializes it into a command line.
fn shell_command(program: &str, flags: &[&str], script: &str) -> std::process::Command {
    let mut cmd = std::process::Command::new(program);
    cmd.args(flags);
    // cmd.exe does not parse the `\"` escapes MSVCRT-style argument
    // quoting writes, so a body handed to `/C` must be the author's
    // bytes verbatim — exactly what typing it at a prompt would send.
    // `/C` marks the dialect: only cmd (and the Windows platform
    // shell, which is cmd) takes it. The other dialects keep the
    // quoted form their parsers understand.
    #[cfg(windows)]
    if matches!(flags, ["/C"]) {
        use std::os::windows::process::CommandExt;
        cmd.raw_arg(script);
        return cmd;
    }
    cmd.arg(script);
    cmd
}

/// Every source's materialized script, by `SourceId`, and the private
/// directory holding them. Bodies are static for a run: written ONCE
/// at load, the path re-executed on every respawn tick, the directory
/// removed when this drops (children are already down by drop order —
/// the shutdown guards are declared after this binding). A dashboard
/// with no `#!` body builds the `Default` — no directory, no syscall,
/// nothing on disk. Removal failure is silent litter in the OS tmpdir
/// (`TempDir::drop` ignores errors by design); never a retry, never a
/// message.
#[derive(Default)]
struct ScriptFiles {
    /// Held for Drop alone: OWNING the TempDir is what removes the
    /// directory when this drops. Nothing reads it — the paths below
    /// are the lookups — so the field is dead to the reachability
    /// analysis on purpose.
    #[allow(dead_code)]
    dir: Option<tempfile::TempDir>,
    paths: Vec<Option<std::path::PathBuf>>,
}

impl ScriptFiles {
    /// Write every shebang body once. A no-shebang `Script` body gets
    /// no file — it runs through the shell fallback route instead, so
    /// `path` answering `Some` is exactly "this body has a shebang".
    fn materialize(registry: &Registry) -> anyhow::Result<ScriptFiles> {
        let bodies: Vec<(SourceId, Shebang, &str)> = registry
            .ids()
            .filter_map(|id| match &registry.spec(id).program {
                SourceProgram::Script(body) => shebang(body).map(|line| (id, line, body.as_str())),
                SourceProgram::Argv(_) => None,
            })
            .collect();
        if bodies.is_empty() {
            return Ok(ScriptFiles::default());
        }
        let mut builder = tempfile::Builder::new();
        builder.prefix("rat-script.");
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            builder.permissions(std::fs::Permissions::from_mode(0o700));
        }
        let dir = builder.tempdir().context("creating the script directory")?;
        let mut paths = vec![None; registry.ids().count()];
        for (id, line, body) in bodies {
            // The INDEX is what makes the name unique (duplicate ids
            // are legal, first-win); the id part is a debugging
            // courtesy, BOUNDED so a legal but long id cannot push the
            // file name past the filesystem's NAME_MAX. Ids are ASCII
            // (RFC 3986 unreserved), so char truncation is byte
            // truncation.
            let id_part: String = registry.spec(id).id.chars().take(40).collect();
            let stem = format!("{}-{}", id.0, id_part);
            let (name, bytes) = script_file(SHEBANG_ARM, &line, &stem, body);
            let path = dir.path().join(name);
            // Mode set AT creation — no window where the file lacks it;
            // the handle drops before any spawn (sidesteps ETXTBSY).
            let mut options = std::fs::OpenOptions::new();
            options.write(true).create_new(true);
            #[cfg(unix)]
            {
                use std::os::unix::fs::OpenOptionsExt;
                options.mode(0o700);
            }
            use std::io::Write;
            options
                .open(&path)
                .and_then(|mut file| file.write_all(bytes.as_bytes()))
                .with_context(|| format!("writing the script for {:?}", registry.spec(id).id))?;
            paths[id.0] = Some(path);
        }
        Ok(ScriptFiles {
            dir: Some(dir),
            paths,
        })
    }

    fn path(&self, id: SourceId) -> Option<&std::path::Path> {
        self.paths.get(id.0)?.as_deref()
    }
}

/// Which half of the platform split executes a `#!` body.
#[derive(Copy, Clone, PartialEq, Eq, Debug)]
enum ShebangArm {
    /// The kernel re-executes a `#!` file itself, so the file may be
    /// called anything, every byte is the author's, and all it needs
    /// is the exec bit.
    Kernel,
    /// No kernel shebang: WE name the interpreter, and some
    /// interpreters insist on an extension before they will read the
    /// file.
    Interpreter,
}

/// The arm this build runs. A `const`, not a `#[cfg]` item: BOTH arms
/// stay compiled and unit-tested on every platform, and the branch
/// this build does not take is folded away.
const SHEBANG_ARM: ShebangArm = if cfg!(windows) {
    ShebangArm::Interpreter
} else {
    ShebangArm::Kernel
};

/// What the interpreter arm spawns, and the arguments that precede the
/// script path.
///
/// `#!/usr/bin/env X` has no `env` on Windows to delegate to, so `X` is
/// what runs — and the standard spawn path then does the search `env`
/// would have done (Rust's `Command` resolves a bare name on Windows
/// PATH itself, inferring only `.exe`, before `CreateProcessW`; PATHEXT
/// is a cmd.exe concept and does not apply), which is why nothing here
/// walks PATH by hand. A unix-absolute interpreter path means nothing
/// on Windows either, so it is reduced to its file name: `#!/bin/bash`
/// runs whatever `bash` the PATH answers with (Git Bash, if installed),
/// and a name nothing answers surfaces as the spawn error naming it.
/// No WSL routing, no `py.exe -3` alias, no validation list.
///
/// `env -S` is approximated by `shell_words::split`, whose quoting
/// differs from `env -S`'s own (`\_`, `${VAR}`, `#` comments). A
/// malformed `-S` remainder (an unbalanced quote) is a ROUTE, not an
/// error: the whole remainder becomes the program name verbatim and
/// the spawn error names it — on unix the kernel runs `env -S` natively
/// and rat never parses it, so only this arm can even see the
/// malformation.
fn interpreter_invocation(line: &Shebang) -> (String, Vec<String>) {
    let interpreter = match line.interpreter.rsplit_once('/') {
        Some((_, name)) if line.interpreter.starts_with('/') => name.to_string(),
        _ => line.interpreter.clone(),
    };
    if interpreter_name(&interpreter) != "env" {
        return (interpreter, line.arg.iter().cloned().collect());
    }
    match line.arg.as_deref() {
        None => (interpreter, Vec::new()),
        Some(arg) => match arg.strip_prefix("-S") {
            Some(rest) => {
                let rest = rest.trim_start_matches([' ', '\t']);
                match shell_words::split(rest) {
                    Ok(mut words) if !words.is_empty() => {
                        let program = words.remove(0);
                        (program, words)
                    }
                    _ => (rest.to_string(), Vec::new()),
                }
            }
            None => (arg.to_string(), Vec::new()),
        },
    }
}

/// The flags the interpreter arm puts before the script PATH — a
/// different question from `command_flags`, which answers "run this
/// STRING". `cmd` needs `/C` either way; PowerShell needs `-NoProfile`
/// but NOT `-Command`, which takes an expression rather than a file;
/// every other interpreter takes the path bare.
fn interpreter_flags(program: &str) -> &'static [&'static str] {
    match interpreter_name(program).as_str() {
        "cmd" => &["/C"],
        "powershell" | "pwsh" => &["-NoProfile"],
        _ => &[],
    }
}

/// The extension the interpreter arm must give the tempfile. Measured
/// on Windows: `cmd /C` refuses an extensionless file ("is not
/// recognized as an internal or external command") and runs the
/// identical bytes named `.cmd`; pwsh refuses anything that is not
/// `.ps1` ("does not have a '.ps1' extension"). Python, node and the
/// sh-likes do not care. The kernel arm needs none of it — a 0700 file
/// with a `#!` line runs whatever it is called, pwsh included
/// (measured on macOS).
fn script_extension(arm: ShebangArm, program: &str) -> &'static str {
    if arm == ShebangArm::Kernel {
        return "";
    }
    match interpreter_name(program).as_str() {
        "cmd" => ".cmd",
        "powershell" | "pwsh" => ".ps1",
        _ => "",
    }
}

/// The bytes the tempfile gets: the AUTHOR'S, verbatim — with exactly
/// one exception. On the interpreter arm a `cmd` body loses its `#!`
/// line (`#` is not batch syntax; cmd would try to run it) and gains a
/// guaranteed single trailing newline (cmd may skip a final line that
/// has none). Everywhere else no byte moves: the kernel READS the `#!`
/// line, and it is a harmless comment to sh, python and PowerShell.
fn script_bytes(arm: ShebangArm, program: &str, body: &str) -> String {
    if arm != ShebangArm::Interpreter || interpreter_name(program) != "cmd" {
        return body.to_string();
    }
    let rest = match body.split_once('\n') {
        Some((_, rest)) => rest,
        None => "",
    };
    format!("{}\n", rest.trim_end_matches('\n'))
}

/// The one seam the materializer calls: a body's file name and its
/// bytes, for one arm. Extension and rewrite both key on the RESOLVED
/// program (post-`env` substitution), so `#!/usr/bin/env pwsh` gets
/// `.ps1`.
fn script_file(arm: ShebangArm, line: &Shebang, stem: &str, body: &str) -> (String, String) {
    let (program, _) = interpreter_invocation(line);
    let name = format!("{stem}{}", script_extension(arm, &program));
    (name, script_bytes(arm, &program, body))
}

/// The Command a `#!` body spawns. The kernel arm execs the FILE — the
/// kernel parses the `#!` line itself. The interpreter arm invokes the
/// resolved interpreter: our flags, then the author's shebang
/// argument(s), then the path.
fn interpreter_command(
    arm: ShebangArm,
    line: &Shebang,
    path: &std::path::Path,
) -> std::process::Command {
    match arm {
        ShebangArm::Kernel => std::process::Command::new(path),
        ShebangArm::Interpreter => {
            let (program, args) = interpreter_invocation(line);
            let mut cmd = std::process::Command::new(&program);
            cmd.args(interpreter_flags(&program)).args(args).arg(path);
            cmd
        }
    }
}

/// The program THIS arm asks the OS to start, for the spawn error. On
/// the kernel arm `execve` re-executes the `#!` line's own path, so
/// ENOENT names that; on the interpreter arm it names the program we
/// resolved. Never the tempfile — the file is ours and we just wrote
/// it; what can fail to start is the interpreter.
fn shebang_program(arm: ShebangArm, line: &Shebang) -> String {
    match arm {
        ShebangArm::Kernel => line.interpreter.clone(),
        ShebangArm::Interpreter => interpreter_invocation(line).0,
    }
}

#[cfg(test)]
mod tests {
    use ratatui::style::Color;

    use super::*;
    use crate::core::layout::overflow_clip;
    use crate::term::scroll::max_offset;

    const ALL_MODES: [FrameMode; 3] = [FrameMode::Live, FrameMode::LiveScrolled, FrameMode::Paused];

    #[test]
    fn the_append_key_table_answers_only_the_four_keys() {
        // ENUMERATED over every key action_for binds — a checklist of
        // remembered keys is not proof that no other key is live.
        let bound = [
            Key::CtrlC,
            Key::Char('q'),
            Key::Char('v'),
            Key::Enter,
            Key::Char('?'),
            Key::Char('S'),
            Key::Char('j'),
            Key::Down,
            Key::Char('k'),
            Key::Up,
            Key::Char('d'),
            Key::Char('u'),
            Key::Char('f'),
            Key::PageDown,
            Key::Char('b'),
            Key::PageUp,
            Key::Char('g'),
            Key::Home,
            Key::Char('G'),
            Key::End,
            Key::Char('w'),
            Key::Char('h'),
            Key::Left,
            Key::Char('l'),
            Key::Right,
            Key::Char('D'),
            Key::Char('c'),
            Key::Char('t'),
            Key::Char('m'),
            Key::Esc,
            Key::Char('F'),
            Key::Char('p'),
            Key::Char('<'),
            Key::Char(','),
            Key::Char('>'),
            Key::Char('.'),
            Key::Tab,
            Key::BackTab,
            Key::Alt('h'),
            Key::Alt('j'),
            Key::Alt('k'),
            Key::Alt('l'),
            Key::Char('z'),
            Key::Space,
        ];
        for key in bound {
            let expect = match key {
                Key::CtrlC => AppendAction::Abort,
                Key::Char('q') => AppendAction::Quit,
                Key::Char('S') => AppendAction::Snapshot,
                Key::Char('?') => AppendAction::Help,
                _ => AppendAction::Ignore,
            };
            assert_eq!(append_action_for(key), expect, "{key:?}");
        }
    }

    #[test]
    fn the_append_reference_names_every_key_the_table_answers() {
        let lines = append_help_lines(&[]).join("\n");
        for needle in ["q  ", "Ctrl-C", "S  ", "?  "] {
            assert!(lines.contains(needle), "missing {needle:?} in {lines}");
        }
    }

    #[test]
    fn the_append_banner_carries_the_run_constant_tail() {
        // One home for the cadence's meaning: the banner reuses
        // live_suffix's output, never a second spelling.
        let tail = live_suffix(false, Some("2s"), false);
        assert_eq!(append_banner(&tail), format!("rat watch: appending{tail}"));
    }

    #[test]
    fn the_exit_row_speaks_only_on_a_transition() {
        assert_eq!(append_exit_line(None, None), None);
        assert_eq!(
            append_exit_line(None, Some("exit 3")),
            Some("rat watch: exit 3".to_string())
        );
        assert_eq!(append_exit_line(Some("exit 3"), Some("exit 3")), None);
        assert_eq!(
            append_exit_line(Some("exit 3"), None),
            Some("rat watch: exit 0".to_string())
        );
    }

    #[test]
    fn a_notice_row_wears_the_rat_prefix() {
        assert_eq!(
            append_notice("trigger ended: fifo:/tmp/x"),
            "rat watch: trigger ended: fifo:/tmp/x"
        );
    }

    #[test]
    fn an_append_frame_with_nothing_dropped_is_the_sealed_body() {
        // Plain rows pass seal_rows byte-identical.
        let body = vec!["a".to_string(), "b".to_string()];
        assert_eq!(append_frame(&body, None), body);
    }

    #[test]
    fn an_append_frame_seals_the_child_body() {
        let rows = append_frame(&["\x1b[31mred".to_string(), "next".to_string()], None);
        assert!(
            rows[0].contains("\x1b[0m"),
            "open SGR closed at the row end: {rows:?}"
        );
        // The last row must also end closed — nothing leaks to the prompt.
        let last = rows.last().unwrap();
        assert!(
            !last.contains('\x1b') || last.ends_with("\x1b[0m"),
            "the final row must end closed: {last:?}"
        );
    }

    #[test]
    fn an_open_child_color_never_styles_the_drop_marker_row() {
        // seal_rows close-and-replays WITHIN its vec, so the marker must
        // join AFTER sealing or the child's open color styles it.
        let rows = append_frame(&["\x1b[31mred".to_string()], Some("2.0k lines dropped"));
        let marker = rows.last().unwrap();
        assert!(
            marker.starts_with("rat watch: "),
            "no replayed SGR prefix: {marker:?}"
        );
        assert_eq!(
            *marker,
            format!("rat watch: 2.0k lines dropped; kept the last {MAX_RETAINED_LINES}")
        );
    }

    #[test]
    fn append_rows_writes_a_plain_row_verbatim_with_the_terminator() {
        let mut out: Vec<u8> = Vec::new();
        append_rows(&mut out, vec!["hi".to_string()], "\r\n").unwrap();
        assert_eq!(out, b"hi\r\n");
    }

    #[test]
    fn a_standalone_row_with_an_open_sgr_cannot_style_what_follows() {
        // User-provided text (a trigger path, a snapshot path) can carry
        // escapes into rat's own rows; each row seals as its own group at
        // the write boundary.
        let mut out: Vec<u8> = Vec::new();
        append_rows(
            &mut out,
            vec!["rat watch: trigger ended: fifo:/tmp/\x1b[31mevil".to_string()],
            "\n",
        )
        .unwrap();
        let text = String::from_utf8(out).unwrap();
        assert!(
            text.trim_end().ends_with("\x1b[0m"),
            "the open SGR must be closed before the terminator: {text:?}"
        );
    }

    #[test]
    fn the_interval_resolves_by_the_trigger_rule() {
        // (user token, has triggers) -> resolved. The user's token always
        // wins; no token means today's 2s default — unless a trigger
        // exists, which makes polling opt-in.
        let secs = |n| Some(Duration::from_secs(n));
        assert_eq!(resolve_interval(Some("5s"), false).unwrap(), secs(5));
        assert_eq!(resolve_interval(Some("5s"), true).unwrap(), secs(5));
        assert_eq!(resolve_interval(None, false).unwrap(), secs(2));
        assert_eq!(resolve_interval(None, true).unwrap(), None); // trigger-only
        assert!(resolve_interval(Some("bogus"), false).is_err());
    }

    #[test]
    fn todays_keys_mean_the_same_thing_in_every_mode() {
        for mode in ALL_MODES {
            assert_eq!(action_for(Key::CtrlC, mode), WatchAction::Abort);
            assert_eq!(action_for(Key::Char('q'), mode), WatchAction::Quit);
            assert_eq!(action_for(Key::Char('v'), mode), WatchAction::Page);
            assert_eq!(action_for(Key::Enter, mode), WatchAction::Page);
        }
    }

    #[test]
    fn navigation_keys_scroll() {
        use crate::term::scroll::ScrollStep;

        for mode in ALL_MODES {
            for (key, step) in [
                (Key::Char('j'), ScrollStep::LineDown),
                (Key::Down, ScrollStep::LineDown),
                (Key::Char('k'), ScrollStep::LineUp),
                (Key::Up, ScrollStep::LineUp),
                (Key::Char('d'), ScrollStep::HalfDown),
                (Key::Char('u'), ScrollStep::HalfUp),
                (Key::Char('f'), ScrollStep::PageDown),
                (Key::PageDown, ScrollStep::PageDown),
                (Key::Char('b'), ScrollStep::PageUp),
                (Key::PageUp, ScrollStep::PageUp),
                (Key::Char('g'), ScrollStep::Top),
                (Key::Home, ScrollStep::Top),
                (Key::Char('G'), ScrollStep::Bottom),
                (Key::End, ScrollStep::Bottom),
            ] {
                assert_eq!(
                    action_for(key, mode),
                    WatchAction::Scroll(step),
                    "{key:?} mode={mode:?}"
                );
            }
        }
    }

    #[test]
    fn esc_clears_focus_while_live_and_resumes_otherwise() {
        assert_eq!(
            action_for(Key::Esc, FrameMode::Live),
            WatchAction::ClearFocus
        );
        assert_eq!(
            action_for(Key::Esc, FrameMode::LiveScrolled),
            WatchAction::Resume
        );
        assert_eq!(action_for(Key::Esc, FrameMode::Paused), WatchAction::Resume);
    }

    #[test]
    fn the_focus_keys_bind_only_while_live() {
        // Per-pane gestures address a pane in the composed frame. A
        // frozen or scrubbed frame is a literal copy with no pane
        // identity in it, so the keys are inert there.
        assert_eq!(
            action_for(Key::Tab, FrameMode::Live),
            WatchAction::FocusNext
        );
        assert_eq!(
            action_for(Key::BackTab, FrameMode::Live),
            WatchAction::FocusPrev
        );
        for (c, dir) in [
            ('h', FocusDir::Left),
            ('j', FocusDir::Down),
            ('k', FocusDir::Up),
            ('l', FocusDir::Right),
        ] {
            assert_eq!(
                action_for(Key::Alt(c), FrameMode::Live),
                WatchAction::FocusMove(dir)
            );
        }
        for mode in [FrameMode::LiveScrolled, FrameMode::Paused] {
            assert_eq!(action_for(Key::Tab, mode), WatchAction::Ignore);
            assert_eq!(action_for(Key::BackTab, mode), WatchAction::Ignore);
            for c in ['h', 'j', 'k', 'l'] {
                assert_eq!(action_for(Key::Alt(c), mode), WatchAction::Ignore);
            }
        }
        for mode in ALL_MODES {
            // An unbound meta spelling stays inert, and the plain keys
            // keep the whole-frame meanings they have always had.
            assert_eq!(action_for(Key::Alt('x'), mode), WatchAction::Ignore);
            assert_eq!(action_for(Key::Char('h'), mode), WatchAction::ShiftLeft);
            assert_eq!(action_for(Key::Char('l'), mode), WatchAction::ShiftRight);
        }
    }

    #[test]
    fn the_cycle_walks_reading_order_and_wraps() {
        let order = [SourceId(2), SourceId(0), SourceId(1)];
        // With no focus, any focus gesture lands on the first pane —
        // forward and backward alike.
        assert_eq!(focus_cycle(None, &order, true), Some(SourceId(2)));
        assert_eq!(focus_cycle(None, &order, false), Some(SourceId(2)));
        assert_eq!(
            focus_cycle(Some(SourceId(2)), &order, true),
            Some(SourceId(0))
        );
        assert_eq!(
            focus_cycle(Some(SourceId(1)), &order, true),
            Some(SourceId(2)),
            "forward wraps past the last pane"
        );
        assert_eq!(
            focus_cycle(Some(SourceId(2)), &order, false),
            Some(SourceId(1)),
            "backward wraps past the first pane"
        );
        // A single pane cycles to itself, which the arm reads as a no-op.
        assert_eq!(
            focus_cycle(Some(SourceId(0)), &[SourceId(0)], true),
            Some(SourceId(0))
        );
        assert_eq!(focus_cycle(None, &[], true), None);
    }

    /// A 2x2 board: 4-row, 10-cell panes, gap 2 and row_gap 1.
    fn grid_rects() -> Vec<PaneRect> {
        vec![
            PaneRect {
                row: 0,
                col: 0,
                rows: 4,
                cols: 10,
            },
            PaneRect {
                row: 0,
                col: 12,
                rows: 4,
                cols: 10,
            },
            PaneRect {
                row: 5,
                col: 0,
                rows: 4,
                cols: 10,
            },
            PaneRect {
                row: 5,
                col: 12,
                rows: 4,
                cols: 10,
            },
        ]
    }

    #[test]
    fn a_directional_move_crosses_the_gap_to_the_nearest_neighbour() {
        // Zellij's exact-edge-adjacency test would find NOTHING here:
        // ratto separates panes by gap/row_gap cells, so candidacy is
        // "strictly beyond the edge, overlapping on the cross axis" and
        // the nearest edge wins.
        let rects = grid_rects();
        let order = [SourceId(0), SourceId(1), SourceId(2), SourceId(3)];
        let go = |from: usize, dir| focus_neighbor(SourceId(from), dir, &rects, &order);
        assert_eq!(go(0, FocusDir::Right), Some(SourceId(1)));
        assert_eq!(go(1, FocusDir::Left), Some(SourceId(0)));
        assert_eq!(go(0, FocusDir::Down), Some(SourceId(2)));
        assert_eq!(go(2, FocusDir::Up), Some(SourceId(0)));
        assert_eq!(go(3, FocusDir::Left), Some(SourceId(2)));
        assert_eq!(go(3, FocusDir::Up), Some(SourceId(1)));
    }

    #[test]
    fn a_directional_move_stops_at_the_edge() {
        let rects = grid_rects();
        let order = [SourceId(0), SourceId(1), SourceId(2), SourceId(3)];
        // No wrap: the cycle is the gesture that wraps.
        assert_eq!(
            focus_neighbor(SourceId(0), FocusDir::Left, &rects, &order),
            None
        );
        assert_eq!(
            focus_neighbor(SourceId(0), FocusDir::Up, &rects, &order),
            None
        );
        assert_eq!(
            focus_neighbor(SourceId(3), FocusDir::Right, &rects, &order),
            None
        );
        assert_eq!(
            focus_neighbor(SourceId(3), FocusDir::Down, &rects, &order),
            None
        );
    }

    #[test]
    fn a_pane_sharing_no_cross_axis_cells_is_not_a_neighbour() {
        // Left-of is not enough: the panes must share at least one row,
        // or a move left from the top-right lands on a pane the user
        // would not call "to the left of this one".
        let rects = vec![
            PaneRect {
                row: 0,
                col: 12,
                rows: 4,
                cols: 10,
            },
            PaneRect {
                row: 6,
                col: 0,
                rows: 4,
                cols: 10,
            },
        ];
        let order = [SourceId(0), SourceId(1)];
        assert_eq!(
            focus_neighbor(SourceId(0), FocusDir::Left, &rects, &order),
            None
        );
    }

    #[test]
    fn a_directional_tie_goes_to_the_first_pane_in_reading_order() {
        // One tall pane on the right, two stacked panes on its left:
        // both are the same distance away and both overlap it.
        let rects = vec![
            PaneRect {
                row: 0,
                col: 12,
                rows: 9,
                cols: 10,
            },
            PaneRect {
                row: 0,
                col: 0,
                rows: 4,
                cols: 10,
            },
            PaneRect {
                row: 5,
                col: 0,
                rows: 4,
                cols: 10,
            },
        ];
        let order = [SourceId(1), SourceId(2), SourceId(0)];
        assert_eq!(
            focus_neighbor(SourceId(0), FocusDir::Left, &rects, &order),
            Some(SourceId(1))
        );
        // Reading order decides, not id order: reverse it and the other
        // candidate wins.
        let flipped = [SourceId(2), SourceId(1), SourceId(0)];
        assert_eq!(
            focus_neighbor(SourceId(0), FocusDir::Left, &rects, &flipped),
            Some(SourceId(2))
        );
    }

    #[test]
    fn f_resumes_and_p_freezes() {
        assert_eq!(
            action_for(Key::Char('F'), FrameMode::Live),
            WatchAction::Ignore
        );
        assert_eq!(
            action_for(Key::Char('F'), FrameMode::LiveScrolled),
            WatchAction::Resume
        );
        assert_eq!(
            action_for(Key::Char('F'), FrameMode::Paused),
            WatchAction::Resume
        );
        assert_eq!(
            action_for(Key::Char('p'), FrameMode::Live),
            WatchAction::Freeze
        );
        assert_eq!(
            action_for(Key::Char('p'), FrameMode::LiveScrolled),
            WatchAction::Freeze
        );
        assert_eq!(
            action_for(Key::Char('p'), FrameMode::Paused),
            WatchAction::Ignore
        );
    }

    #[test]
    fn shift_d_toggles_the_gutter_in_every_mode() {
        for mode in ALL_MODES {
            assert_eq!(action_for(Key::Char('D'), mode), WatchAction::ToggleGutter);
        }
        // The half-page scroll is untouched by its shifted neighbour.
        for mode in ALL_MODES {
            assert_eq!(
                action_for(Key::Char('d'), mode),
                WatchAction::Scroll(ScrollStep::HalfDown)
            );
        }
    }

    #[test]
    fn c_toggles_the_highlight_in_every_mode() {
        for mode in ALL_MODES {
            assert_eq!(
                action_for(Key::Char('c'), mode),
                WatchAction::ToggleHighlight
            );
        }
    }

    #[test]
    fn t_toggles_the_time_display_in_every_mode() {
        for mode in ALL_MODES {
            assert_eq!(action_for(Key::Char('t'), mode), WatchAction::ToggleTime);
        }
    }

    #[test]
    fn view_keys_are_view_actions_in_every_mode() {
        for mode in ALL_MODES {
            assert_eq!(action_for(Key::Char('w'), mode), WatchAction::ToggleWrap);
            assert_eq!(action_for(Key::Char('h'), mode), WatchAction::ShiftLeft);
            assert_eq!(action_for(Key::Left, mode), WatchAction::ShiftLeft);
            assert_eq!(action_for(Key::Char('l'), mode), WatchAction::ShiftRight);
            assert_eq!(action_for(Key::Right, mode), WatchAction::ShiftRight);
        }
    }

    #[test]
    fn unbound_keys_are_ignored() {
        for mode in ALL_MODES {
            assert_eq!(action_for(Key::Char('x'), mode), WatchAction::Ignore);
            assert_eq!(action_for(Key::Alt('x'), mode), WatchAction::Ignore);
            assert_eq!(action_for(Key::Backspace, mode), WatchAction::Ignore);
        }
    }

    #[test]
    fn scrub_keys_walk_history() {
        for mode in ALL_MODES {
            assert_eq!(action_for(Key::Char('<'), mode), WatchAction::ScrubBack);
            assert_eq!(action_for(Key::Char(','), mode), WatchAction::ScrubBack);
        }
        // Forward only means something while parked on a past frame; from
        // a live surface there is nothing newer to step to. (Past the
        // newest entry the arm exits the freeze — same key table.)
        assert_eq!(
            action_for(Key::Char('>'), FrameMode::Paused),
            WatchAction::ScrubForward
        );
        assert_eq!(
            action_for(Key::Char('.'), FrameMode::Paused),
            WatchAction::ScrubForward
        );
        for mode in [FrameMode::Live, FrameMode::LiveScrolled] {
            assert_eq!(action_for(Key::Char('>'), mode), WatchAction::Ignore);
            assert_eq!(action_for(Key::Char('.'), mode), WatchAction::Ignore);
        }
    }

    #[test]
    fn s_is_the_snapshot_key() {
        for mode in ALL_MODES {
            assert_eq!(action_for(Key::Char('S'), mode), WatchAction::Snapshot);
            assert_eq!(action_for(Key::Char('s'), mode), WatchAction::Ignore);
        }
    }

    #[test]
    fn the_live_rows_carry_the_time_segment() {
        assert_eq!(
            live_notice(0, "since 18:47:53", None, None),
            "since 18:47:53"
        );
        assert_eq!(
            live_notice(8, "changed 14s ago", None, None),
            "… 8 more lines · changed 14s ago"
        );
        // The focus segment rides LAST — the pane's scroll range extends
        // it later, and a range before the drop marker would read as the
        // frame's.
        assert_eq!(
            live_notice(0, "since 18:47:53", None, Some("focus plan")),
            "since 18:47:53 · focus plan"
        );
        assert_eq!(
            live_notice(
                8,
                "changed 14s ago",
                Some("2.0k lines dropped"),
                Some("focus plan")
            ),
            "… 8 more lines · changed 14s ago · 2.0k lines dropped · focus plan"
        );
    }

    #[test]
    fn the_live_row_says_when_lines_were_dropped() {
        // A plain watch has no chrome row, so this is its surface. The
        // two counts mean different things and both can show at once:
        // the hidden rows are still there and scrolling reaches them,
        // the dropped ones are gone for good.
        assert_eq!(
            live_notice(0, "since 18:47:53", Some("2.0k lines dropped"), None),
            "since 18:47:53 · 2.0k lines dropped"
        );
        assert_eq!(
            live_notice(8, "since 18:47:53", Some("2.0k lines dropped"), None),
            "… 8 more lines · since 18:47:53 · 2.0k lines dropped"
        );
    }

    #[test]
    fn the_focus_segment_names_the_focused_pane() {
        let registry = two_weighted_panes();
        let runtime = vec![SourceRuntime::for_test(), SourceRuntime::for_test()];
        let geom = registry.geometry((80, 24));
        let mut panes = PaneView::new(registry.len());
        assert_eq!(focus_segment(&registry, &runtime, &geom, &panes), None);
        panes.focus = Some(SourceId(1));
        assert_eq!(
            focus_segment(&registry, &runtime, &geom, &panes),
            Some("focus right".to_string())
        );
    }

    #[test]
    fn one_resolution_names_a_pane_for_the_border_and_the_footer() {
        // The chrome row's title and the footer's focus segment must
        // never disagree about what a pane is called.
        let registry = two_weighted_panes();
        assert_eq!(pane_display_name(&registry, SourceId(0)), "left");
        assert_eq!(pane_display_name(&registry, SourceId(1)), "right");
    }

    #[test]
    fn the_live_suffix_names_the_interval_and_help() {
        // Today's bytes exactly, when no trigger exists.
        assert_eq!(
            live_suffix(false, Some("2s"), false),
            " · every 2s · ? help"
        );
        assert_eq!(
            live_suffix(false, Some("500ms"), false),
            " · every 500ms · ? help"
        );
        // Once mode has no cadence to anticipate and no keys to learn.
        assert_eq!(live_suffix(true, Some("2s"), false), "");
    }

    #[test]
    fn the_live_suffix_names_the_trigger_modes() {
        assert_eq!(
            live_suffix(false, Some("60s"), true),
            " · every 60s or on trigger · ? help"
        );
        assert_eq!(live_suffix(false, None, true), " · on trigger · ? help");
        assert_eq!(live_suffix(true, None, true), ""); // once still empties it
    }

    #[test]
    fn the_help_reference_names_the_trigger_sources() {
        let specs = vec![TriggerSpec::File("/tmp/state.json".into())];
        let lines = help_lines("rat watch — keys", &trigger_help(&specs));
        assert!(
            lines
                .iter()
                .any(|line| line.contains("file:/tmp/state.json")),
            "{lines:?}"
        );
        // And stays clean when none are configured.
        assert!(
            !help_lines("rat watch — keys", &trigger_help(&[]))
                .iter()
                .any(|line| line.contains("trigger")),
            "the untriggered reference must not mention triggers"
        );
    }

    #[test]
    fn the_row_before_the_status_row_starts_the_chrome_clean() {
        let view = ViewState {
            wrap: true,
            hshift: 0,
            gutter: false,
            highlight: false,
            alt_time: false,
        };
        let faint = StyleSpec {
            faint: true,
            ..StyleSpec::default()
        };
        // A child that leaves its color open: the body row must end
        // closed so the status row's CSI 2m starts from a clean state,
        // and the carry replays across body rows so a deliberate span
        // still looks the same.
        let lines = vec!["\x1b[31mred".to_string(), "still".to_string()];
        let rows = frame_rows(
            &lines,
            0,
            FrameMode::Live,
            view,
            None,
            (80, 24),
            None,
            false,
            &faint,
            ColorProfile::TrueColor,
            "since 12:00:00",
            " · ? help",
            "",
            None,
            "▌ ",
            "",
            None,
            None,
        );
        assert_eq!(rows[0], "\x1b[31mred\x1b[0m");
        assert_eq!(rows[1], "\x1b[31mstill\x1b[0m");
        assert!(
            rows[2].starts_with("\x1b[2m"),
            "the status row is chrome: {:?}",
            rows[2]
        );
    }

    #[test]
    fn plain_rows_reach_the_renderer_byte_identical() {
        let view = ViewState {
            wrap: true,
            hshift: 0,
            gutter: false,
            highlight: false,
            alt_time: false,
        };
        let faint = StyleSpec {
            faint: true,
            ..StyleSpec::default()
        };
        let lines = vec!["plain".to_string(), "\x1b[31mclosed\x1b[0m".to_string()];
        let rows = frame_rows(
            &lines,
            0,
            FrameMode::Live,
            view,
            None,
            (80, 24),
            None,
            false,
            &faint,
            ColorProfile::TrueColor,
            "since 12:00:00",
            " · ? help",
            "",
            None,
            "▌ ",
            "",
            None,
            None,
        );
        assert_eq!(rows[0], "plain");
        assert_eq!(rows[1], "\x1b[31mclosed\x1b[0m");
    }

    #[test]
    fn fullscreen_pads_the_body_and_reserves_the_notice_row() {
        let view = ViewState {
            wrap: true,
            hshift: 0,
            gutter: false,
            highlight: false,
            alt_time: false,
        };
        let faint = StyleSpec {
            faint: true,
            ..StyleSpec::default()
        };
        let lines = vec!["hi".to_string()];
        let build = |notice: Option<String>, max_height: Option<u16>| {
            frame_rows(
                &lines,
                0,
                FrameMode::Live,
                view,
                notice,
                (80, 24),
                max_height,
                true,
                &faint,
                ColorProfile::TrueColor,
                "since 12:00:00",
                " · ? help",
                "",
                None,
                "▌ ",
                "",
                None,
                None,
            )
        };
        // No notice: 22 padded body rows + the status row = 23 painted
        // rows on a 24-row screen — the bottom row stays the cursor's,
        // so nothing ever scrolls the alternate screen.
        let rows = build(None, None);
        assert_eq!(rows.len(), 23, "22 body rows + the status row");
        assert_eq!(rows[0], "hi");
        assert!(rows[1..22].iter().all(String::is_empty), "padded body");
        assert!(rows[22].contains("since 12:00:00"), "status pinned last");
        // A one-shot notice takes its row FROM the body: still 23
        // painted rows, never 24.
        let rows = build(Some("one notice".to_string()), None);
        assert_eq!(rows.len(), 23, "the notice row comes out of the body");
        assert!(rows[22].contains("one notice"), "the notice is last");
        // A --max-height cap keeps the author's number: no padding.
        let rows = build(None, Some(5));
        assert_eq!(rows.len(), 2, "a capped frame still floats");
    }

    #[test]
    fn the_wheel_drives_the_scroll_actions_the_keys_already_drive() {
        let wheel = |kind, shift, notches| MouseEvent {
            kind,
            shift,
            notches,
        };
        // A notch is three lines; a folded burst multiplies.
        assert_eq!(
            action_for_mouse(wheel(MouseKind::WheelDown, false, 1)),
            WatchAction::ScrollN(ScrollStep::LineDown, 3)
        );
        assert_eq!(
            action_for_mouse(wheel(MouseKind::WheelUp, false, 4)),
            WatchAction::ScrollN(ScrollStep::LineUp, 12)
        );
        // Shift flips to half windows.
        assert_eq!(
            action_for_mouse(wheel(MouseKind::WheelDown, true, 2)),
            WatchAction::ScrollN(ScrollStep::HalfDown, 2)
        );
        // A horizontal wheel is the h/l shift.
        assert_eq!(
            action_for_mouse(wheel(MouseKind::WheelLeft, false, 1)),
            WatchAction::ShiftLeft
        );
        assert_eq!(
            action_for_mouse(wheel(MouseKind::WheelRight, false, 1)),
            WatchAction::ShiftRight
        );
        // Presses, releases, motion: mapped to nothing.
        assert_eq!(
            action_for_mouse(wheel(MouseKind::Other, false, 1)),
            WatchAction::Ignore
        );
    }

    #[test]
    fn a_wheel_burst_folds_into_one_stepped_event() {
        let notch = |kind| {
            TapEvent::Mouse(MouseEvent {
                kind,
                shift: false,
                notches: 1,
            })
        };
        let folded = fold_wheel(vec![
            notch(MouseKind::WheelDown),
            notch(MouseKind::WheelDown),
            notch(MouseKind::WheelDown),
            TapEvent::Key(Key::Char('q')),
            notch(MouseKind::WheelDown),
        ]);
        assert_eq!(folded.len(), 3, "three down, a key, one more down");
        assert!(matches!(
            folded[0],
            TapEvent::Mouse(MouseEvent { notches: 3, .. })
        ));
        // Direction changes never merge.
        let folded = fold_wheel(vec![notch(MouseKind::WheelDown), notch(MouseKind::WheelUp)]);
        assert_eq!(folded.len(), 2);
    }

    #[test]
    fn paint_key_matches_the_live_and_paused_shapes() {
        let view = ViewState {
            wrap: true,
            hshift: 4,
            gutter: false,
            highlight: false,
            alt_time: false,
        };
        // Expected keys are always built via `key()`: an empty
        // DefaultHasher finishes to a nonzero value, so a PaneViewKey
        // literal (or Default's zero) would pin the wrong number.
        let panes = PaneView::new(0);
        // The key carries the caller's displayed age verbatim; which
        // surface counts is the caller's business.
        let live = paint_key(
            None,
            None,
            42,
            Appearance::Dark,
            (80, 24),
            view,
            panes.key(),
            14,
        );
        assert_eq!(
            live,
            PaintKey {
                content: 42,
                cols: 80,
                rows: 24,
                appearance: Appearance::Dark,
                offset: 0,
                paused: false,
                wrap: true,
                hshift: 4,
                gutter: false,
                highlight: false,
                alt_time: false,
                view: panes.key(),
                age_secs: 14,
            }
        );
        let scroll = ScrollState::default().step(ScrollStep::LineDown, 50, 10);
        let p = PauseState {
            frozen: vec!["x".to_string()],
            scroll,
            content: 7,
            appearance: Appearance::Light,
            viewed_at: jiff::Timestamp::now(),
            history_seq: None,
        };
        // A live-scrolled key carries the LIVE hash and the window offset:
        // the tail keeps repainting under the offset.
        let ls = LiveScroll::start(ScrollStep::LineDown, 50, 10);
        let scrolled = paint_key(
            None,
            Some(ls),
            42,
            Appearance::Dark,
            (80, 24),
            view,
            panes.key(),
            14,
        );
        assert_eq!(
            scrolled,
            PaintKey {
                content: 42,
                cols: 80,
                rows: 24,
                appearance: Appearance::Dark,
                offset: 1,
                paused: false,
                wrap: true,
                hshift: 4,
                gutter: false,
                highlight: false,
                alt_time: false,
                view: panes.key(),
                age_secs: 14,
            }
        );
        let paused = paint_key(
            Some(&p),
            None,
            42,
            Appearance::Dark,
            (80, 24),
            view,
            panes.key(),
            14,
        );
        assert_eq!(
            paused,
            PaintKey {
                content: 7,
                cols: 80,
                rows: 24,
                appearance: Appearance::Light,
                offset: scroll.offset(),
                paused: true,
                wrap: true,
                hshift: 4,
                gutter: false,
                highlight: false,
                alt_time: false,
                view: panes.key(),
                age_secs: 14,
            }
        );
    }

    #[test]
    fn the_age_reads_just_now_then_counts() {
        assert_eq!(age_text(0), "just now");
        assert_eq!(age_text(9), "just now");
        assert_eq!(age_text(10), "10s ago");
        assert_eq!(age_text(14), "14s ago");
        assert_eq!(age_text(75), "1m 15s ago");
    }

    #[test]
    fn the_displayed_age_counts_only_where_the_row_counts() {
        let old = jiff::Timestamp::from_second(jiff::Timestamp::now().as_second() - 100)
            .expect("timestamp");
        let p = PauseState {
            frozen: vec!["x".to_string()],
            scroll: ScrollState::default(),
            content: 7,
            appearance: Appearance::Dark,
            viewed_at: old,
            history_seq: None,
        };
        let ls = LiveScroll::start(ScrollStep::LineDown, 50, 10);
        // Counting arms (clock tolerance: at least the constructed
        // age) — BOTH rows count exactly when flipped, never before.
        assert!(
            displayed_age(Some(&p), None, true, old) >= 100,
            "paused flipped counts"
        );
        assert!(
            displayed_age(None, None, true, old) >= 100,
            "live flipped counts"
        );
        // Default arms are stamps: exactly zero.
        assert_eq!(
            displayed_age(Some(&p), None, false, old),
            0,
            "paused default is a stamp"
        );
        assert_eq!(
            displayed_age(None, None, false, old),
            0,
            "live default is a stamp"
        );
        assert!(
            displayed_age(None, Some(ls), true, old) >= 100,
            "the scrolled row carries the live segment: flipped, it counts"
        );
        assert_eq!(
            displayed_age(None, Some(ls), false, old),
            0,
            "the scrolled default is a stamp"
        );
    }

    #[test]
    fn the_live_segment_flips_between_stamp_and_counter() {
        assert_eq!(live_time_segment(false, "18:47:53", 999), "since 18:47:53");
        assert_eq!(live_time_segment(true, "18:47:53", 0), "changed just now");
        assert_eq!(live_time_segment(true, "18:47:53", 14), "changed 14s ago");
        assert_eq!(
            live_time_segment(true, "18:47:53", 75),
            "changed 1m 15s ago"
        );
    }

    #[test]
    fn the_paused_segment_stamps_by_default_and_counts_flipped() {
        let t = jiff::Timestamp::from_second(1_785_067_200).expect("timestamp");
        assert_eq!(
            paused_time_segment(false, t, 999),
            format!("at {}", local_hms(t))
        );
        assert_eq!(paused_time_segment(true, t, 3), "just now");
        assert_eq!(paused_time_segment(true, t, 14), "14s ago");
    }

    #[test]
    fn local_hms_is_a_wall_clock_stamp() {
        let s = local_hms(jiff::Timestamp::from_second(1_785_067_200).expect("timestamp"));
        let b = s.as_bytes();
        assert_eq!(b.len(), 8, "HH:MM:SS: {s}");
        assert!(b[2] == b':' && b[5] == b':', "{s}");
        assert!(
            [0, 1, 3, 4, 6, 7].iter().all(|&i| b[i].is_ascii_digit()),
            "{s}"
        );
    }

    #[test]
    fn the_window_is_the_max_height_or_two_short_of_the_screen() {
        assert_eq!(window_rows(None, 24), 22);
        assert_eq!(window_rows(Some(5), 24), 5);
        assert_eq!(window_rows(None, 1), 0);
    }

    #[test]
    fn composing_a_frame_puts_the_title_first_and_stderr_last() {
        let title = "T".to_string();
        assert_eq!(
            compose_frame(Some(&title), b"a\nb\n", b"boom\n", true),
            vec!["T", "a", "b", "boom"]
        );
        assert_eq!(
            compose_frame(Some(&title), b"a\nb\n", b"boom\n", false),
            vec!["T", "a", "b"]
        );
    }

    /// Two panes that disagree about which end they keep, so one
    /// registry exercises both mappings.
    fn panes_keeping_opposite_ends() -> Registry {
        use crate::core::box_model::{BorderPreset, Sides};
        use crate::core::registry::{LayoutNode, Overflow, PaneBox, PaneWidth};
        let spec = |id: &str| SourceSpec {
            id: id.to_string(),
            program: SourceProgram::Argv(vec!["true".to_string()]),
            shell: ShellMode::Direct,
            interval: Some(Duration::from_secs(3600)),
            triggers: Vec::new(),
            debounce: Duration::from_millis(250),
            live: false,
        };
        let pane = |overflow| PaneBox {
            height: 5,
            width: PaneWidth::Weight(1),
            overflow,
            border: BorderPreset::Rounded,
            padding: Sides::default(),
            title: None,
            chrome: true,
        };
        Registry::panes(
            vec![spec("tail"), spec("head")],
            vec![pane(Overflow::KeepBottom), pane(Overflow::KeepTop)],
            LayoutNode::Column(vec![
                LayoutNode::Pane(SourceId(0)),
                LayoutNode::Pane(SourceId(1)),
            ]),
            0,
            0,
        )
        .expect("a valid two-pane registry")
    }

    fn once_registry(panes: &[(&str, bool)]) -> Registry {
        use crate::core::box_model::{BorderPreset, Sides};
        use crate::core::registry::{LayoutNode, Overflow, PaneBox, PaneWidth};
        let sources = panes
            .iter()
            .map(|(id, live)| SourceSpec {
                id: (*id).to_string(),
                program: SourceProgram::Argv(vec!["true".to_string()]),
                shell: ShellMode::Direct,
                interval: Some(Duration::from_secs(3600)),
                triggers: Vec::new(),
                debounce: Duration::from_millis(250),
                live: *live,
            })
            .collect::<Vec<_>>();
        let boxes = panes
            .iter()
            .map(|_| PaneBox {
                height: 3,
                width: PaneWidth::Weight(1),
                overflow: Overflow::KeepTop,
                border: BorderPreset::None,
                padding: Sides::default(),
                title: None,
                chrome: true,
            })
            .collect::<Vec<_>>();
        let cells = (0..panes.len())
            .map(|i| LayoutNode::Pane(SourceId(i)))
            .collect();
        Registry::panes(sources, boxes, LayoutNode::Column(cells), 0, 0).expect("a valid registry")
    }

    #[test]
    fn the_once_notice_names_every_waiting_pane_and_the_declaration_to_write() {
        let registry = once_registry(&[("logs", false), ("metrics", false)]);
        assert_eq!(
            once_waiting_text(&registry, &[SourceId(0)], Duration::from_secs(5)),
            "rat dashboard: --once is still waiting on pane \"logs\" after 5s: no output, no exit. A command that follows instead of exiting must be declared `live=#true`; `--once-timeout 30s` bounds the wait."
        );
        assert_eq!(
            once_waiting_text(
                &registry,
                &[SourceId(0), SourceId(1)],
                Duration::from_secs(5)
            ),
            "rat dashboard: --once is still waiting on panes \"logs\", \"metrics\" after 5s: no output, no exit. A command that follows instead of exiting must be declared `live=#true`; `--once-timeout 30s` bounds the wait."
        );
    }

    #[test]
    fn the_once_notice_drops_the_live_advice_when_every_waiting_pane_declared_it() {
        let registry = once_registry(&[("logs", true), ("tail", true), ("build", false)]);
        assert_eq!(
            once_waiting_text(&registry, &[SourceId(0)], Duration::from_secs(5)),
            "rat dashboard: --once is still waiting on pane \"logs\" after 5s: the live child has printed nothing yet. `--once-timeout 30s` bounds the wait."
        );
        // Plural stays grammatical.
        assert_eq!(
            once_waiting_text(
                &registry,
                &[SourceId(0), SourceId(1)],
                Duration::from_secs(5)
            ),
            "rat dashboard: --once is still waiting on panes \"logs\", \"tail\" after 5s: the live children have printed nothing yet. `--once-timeout 30s` bounds the wait."
        );
        // ANY undeclared waiting pane keeps the teaching advice.
        assert_eq!(
            once_waiting_text(
                &registry,
                &[SourceId(0), SourceId(2)],
                Duration::from_secs(5)
            ),
            "rat dashboard: --once is still waiting on panes \"logs\", \"build\" after 5s: no output, no exit. A command that follows instead of exiting must be declared `live=#true`; `--once-timeout 30s` bounds the wait."
        );
    }

    #[test]
    fn the_bound_names_the_pane_it_gave_up_on() {
        let registry = once_registry(&[("logs", false), ("metrics", false)]);
        assert_eq!(
            once_timeout_text(&registry, &[SourceId(0)], Duration::from_secs(30)),
            "--once gave up after 30s: pane \"logs\" never finished. A command that follows instead of exiting must be declared `live=#true`."
        );
        assert_eq!(
            once_timeout_text(
                &registry,
                &[SourceId(0), SourceId(1)],
                Duration::from_secs(30)
            ),
            "--once gave up after 30s: panes \"logs\", \"metrics\" never finished. A command that follows instead of exiting must be declared `live=#true`."
        );
    }

    #[test]
    fn the_bound_drops_the_live_advice_when_every_waiting_pane_declared_it() {
        // A declared-live pane must not be told to declare live, and
        // the verb changes with the state: it never produced output,
        // rather than never finished. Same rule as the notice: the
        // advice is dropped only when EVERY waiting pane is live.
        let registry = once_registry(&[("logs", true), ("tail", true), ("build", false)]);
        assert_eq!(
            once_timeout_text(&registry, &[SourceId(0)], Duration::from_secs(30)),
            "--once gave up after 30s: pane \"logs\" never produced its first output."
        );
        assert_eq!(
            once_timeout_text(
                &registry,
                &[SourceId(0), SourceId(1)],
                Duration::from_secs(30)
            ),
            "--once gave up after 30s: panes \"logs\", \"tail\" never produced their first output."
        );
        assert_eq!(
            once_timeout_text(
                &registry,
                &[SourceId(0), SourceId(2)],
                Duration::from_secs(30)
            ),
            "--once gave up after 30s: panes \"logs\", \"build\" never finished. A command that follows instead of exiting must be declared `live=#true`."
        );
    }

    #[test]
    fn a_truncated_pane_carries_a_marker_and_an_untruncated_one_does_not() {
        // Both directions, because the marker must not be sticky: a pane
        // that stops flooding would otherwise keep accusing itself. It is
        // derived per outcome, exactly as the exit badge is, so a tick
        // that fits produces nothing.
        assert_eq!(dropped_badge(0), None);
        assert_eq!(dropped_badge(90).as_deref(), Some("90 lines dropped"));
        assert_eq!(
            dropped_badge(1_234).as_deref(),
            Some("1.2k lines dropped"),
            "a flood's count has to fit a chrome row"
        );
        assert_eq!(
            dropped_badge(2_500_000).as_deref(),
            Some("2.5M lines dropped")
        );
    }

    #[test]
    fn the_marker_joins_the_panes_change_signature() {
        // Or the composition carries a marker nothing ever repaints —
        // the bug the looping badge already hit once.
        let lines = vec!["a".to_string()];
        assert_ne!(
            body_signature(&lines, None, false, None),
            body_signature(&lines, None, false, Some("90 lines dropped")),
        );
        // And a pane can be failing, looping and truncated at once.
        assert_ne!(
            body_signature(&lines, Some("exit 3"), true, None),
            body_signature(&lines, Some("exit 3"), true, Some("90 lines dropped")),
        );
    }

    #[test]
    fn a_keep_bottom_pane_retains_its_tail() {
        let registry = panes_keeping_opposite_ends();
        assert_eq!(retention_for(&registry, SourceId(0)).keep, Keep::Bottom);
    }

    #[test]
    fn a_keep_top_pane_retains_its_head() {
        // The default overflow, so this is the common case and it must
        // not be an afterthought.
        let registry = panes_keeping_opposite_ends();
        assert_eq!(retention_for(&registry, SourceId(1)).keep, Keep::Top);
    }

    #[test]
    fn every_source_gets_a_bound_and_none_is_unbounded() {
        // The property that closes the issue: no configuration reaches
        // the worker without one.
        let registry = panes_keeping_opposite_ends();
        for id in registry.ids() {
            assert!(retention_for(&registry, id).max_lines > 0);
        }
    }

    #[test]
    fn a_watch_session_with_no_pane_still_gets_a_policy() {
        // `rat watch` builds a single-source registry with no pane box,
        // so the lookup finds nothing. It must neither panic nor fall
        // back to unbounded — and it keeps the tail, because a watch is
        // for what its command is printing now.
        let registry = Registry::single(
            SourceSpec {
                id: "watch".to_string(),
                program: SourceProgram::Argv(vec!["true".to_string()]),
                shell: ShellMode::Direct,
                interval: Some(Duration::from_secs(2)),
                triggers: Vec::new(),
                debounce: Duration::from_millis(250),
                live: false,
            },
            None,
        );
        assert!(registry.pane(SourceId(0)).is_none());
        let r = retention_for(&registry, SourceId(0));
        assert_eq!(r.keep, Keep::Bottom);
        assert!(r.max_lines > 0);
    }

    /// The cadence seam's whole input space: live or not, an interval
    /// or not, a trigger or not — the three spec fields `cadence_label`
    /// reads.
    fn cadence_spec(live: bool, interval: Option<Duration>, triggered: bool) -> SourceSpec {
        SourceSpec {
            id: "follower".to_string(),
            program: SourceProgram::Argv(vec!["true".to_string()]),
            shell: ShellMode::Direct,
            interval,
            triggers: if triggered {
                vec![TriggerSpec::File(std::path::PathBuf::from("./t"))]
            } else {
                Vec::new()
            },
            debounce: Duration::from_millis(250),
            live,
        }
    }

    #[test]
    fn a_live_source_has_no_cadence_label() {
        // Measured on main: a live pane's chrome read `every 2s · …`
        // while its child had run continuously since startup. The pane
        // does not look broken, it looks idle — and the cadence is a
        // false statement.
        let label = cadence_label(&cadence_spec(true, Some(Duration::from_secs(2)), false));
        assert!(
            !label.contains("every"),
            "a live pane has no cadence: {label}"
        );
        assert_eq!(label, "live");
    }

    #[test]
    fn a_live_source_with_triggers_still_reads_as_live() {
        // `live` outranks the trigger wording: a follower is not
        // waiting on a trigger to run, it is already running.
        assert_eq!(cadence_label(&cadence_spec(true, None, true)), "live");
        assert_eq!(
            cadence_label(&cadence_spec(true, Some(Duration::from_secs(2)), true)),
            "live"
        );
    }

    #[test]
    fn every_batch_cadence_label_is_unchanged() {
        // The byte-identity witness at the label level: all four
        // shipped arms verbatim, so a new arm cannot quietly reorder
        // or reword them.
        let second = Duration::from_secs(1);
        assert_eq!(
            cadence_label(&cadence_spec(false, Some(second), false)),
            "every 1s"
        );
        assert_eq!(
            cadence_label(&cadence_spec(false, Some(second), true)),
            "every 1s or on trigger"
        );
        assert_eq!(
            cadence_label(&cadence_spec(false, None, true)),
            "on trigger"
        );
        assert_eq!(cadence_label(&cadence_spec(false, None, false)), "once");
    }

    #[test]
    fn the_dashboard_title_rides_row_zero_bold_and_only_when_declared() {
        // Through the same compose path every re-entrant repaints
        // with, so the collect step, the resize reflow, and the
        // counting refresh stay in sync by construction.
        use crate::core::box_model::{BorderPreset, Sides};
        use crate::core::registry::{LayoutNode, Overflow, PaneBox, PaneWidth};
        let pane = || PaneBox {
            height: 4,
            width: PaneWidth::Weight(1),
            overflow: Overflow::KeepTop,
            border: BorderPreset::Rounded,
            padding: Sides::default(),
            title: None,
            chrome: false,
        };
        use crate::core::registry::TitleSource;
        let build = |title: Option<&str>| {
            Registry::panes(
                vec![cadence_spec(false, Some(Duration::from_secs(2)), false)],
                vec![pane()],
                LayoutNode::Pane(SourceId(0)),
                0,
                0,
            )
            .expect("a valid one-pane registry")
            .with_title(
                title
                    .map(|t| TitleSource::Static(t.to_string()))
                    .unwrap_or_default(),
            )
        };
        let palette = Palette::builtin(Appearance::Dark, AppearanceSource::Default);
        let mut runtime = vec![SourceRuntime::for_test()];
        runtime[0].output = Some(vec!["seed".to_string()]);
        runtime[0].posted = true;

        let registry = build(Some("Deploy status"));
        let geom = registry.geometry((40, 10));
        let block = compose_sources(
            &registry,
            &runtime,
            &geom,
            &PaneView::new(registry.len()),
            false,
            &palette,
            ColorProfile::TrueColor,
        );
        assert!(
            block.lines[0].contains("Deploy status"),
            "the title heads the frame: {:?}",
            block.lines[0]
        );
        assert!(
            block.lines[0].contains("\u{1b}[1m"),
            "the title is bold, exactly as watch --title: {:?}",
            block.lines[0]
        );
        assert_eq!(
            block.lines.len(),
            block.marks.len(),
            "marks stay aligned to lines"
        );
        assert!(
            !block.marks[0].changed && block.marks[0].cells.is_empty(),
            "the title row carries no change mark"
        );

        let bare = build(None);
        let bare_block = compose_sources(
            &bare,
            &runtime,
            &geom,
            &PaneView::new(registry.len()),
            false,
            &palette,
            ColorProfile::TrueColor,
        );
        assert!(
            !bare_block.lines[0].contains("Deploy status"),
            "undeclared means absent: {:?}",
            bare_block.lines[0]
        );
        assert_eq!(
            block.lines.len(),
            bare_block.lines.len() + 1,
            "the title costs exactly one row"
        );

        // A pane-sourced title renders NOTHING extra: the referenced
        // pane is the visible title, wherever the file placed it.
        let referred = build(None).with_title(TitleSource::Pane {
            source: SourceId(0),
            fallback: Some("Fallback".to_string()),
        });
        let referred_block = compose_sources(
            &referred,
            &runtime,
            &geom,
            &PaneView::new(registry.len()),
            false,
            &palette,
            ColorProfile::TrueColor,
        );
        assert_eq!(
            referred_block.lines, bare_block.lines,
            "role donation adds no row and no bytes"
        );
    }

    #[test]
    fn the_title_role_reads_static_pane_or_fallback_in_that_order() {
        use crate::core::registry::TitleSource;
        let mut runtime = vec![SourceRuntime::for_test()];
        assert_eq!(
            title_role_text(&TitleSource::None, &runtime),
            None,
            "no declaration, no role text"
        );
        assert_eq!(
            title_role_text(&TitleSource::Static("Deploy".to_string()), &runtime).as_deref(),
            Some("Deploy")
        );
        let pane = TitleSource::Pane {
            source: SourceId(0),
            fallback: Some("Fallback".to_string()),
        };
        assert_eq!(
            title_role_text(&pane, &runtime).as_deref(),
            Some("Fallback"),
            "the fallback speaks while the pane has not"
        );
        // The pane's first NON-EMPTY line, escapes stripped, trimmed —
        // plain text is the role's contract (a tab title takes no SGR).
        runtime[0].output = Some(vec![
            String::new(),
            "  \u{1b}[1mBig \u{1b}[31mnews\u{1b}[0m  ".to_string(),
            "second".to_string(),
        ]);
        assert_eq!(
            title_role_text(&pane, &runtime).as_deref(),
            Some("Big news")
        );
        // All-blank output keeps the fallback: silence is not a title.
        runtime[0].output = Some(vec!["   ".to_string(), String::new()]);
        assert_eq!(
            title_role_text(&pane, &runtime).as_deref(),
            Some("Fallback")
        );
    }

    #[test]
    fn a_long_dashboard_title_truncates_to_the_composed_width() {
        use crate::core::box_model::{BorderPreset, Sides};
        use crate::core::measure::display_width;
        use crate::core::registry::{LayoutNode, Overflow, PaneBox, PaneWidth};
        let registry = Registry::panes(
            vec![cadence_spec(false, Some(Duration::from_secs(2)), false)],
            vec![PaneBox {
                height: 4,
                width: PaneWidth::Cells(20),
                overflow: Overflow::KeepTop,
                border: BorderPreset::Rounded,
                padding: Sides::default(),
                title: None,
                chrome: false,
            }],
            LayoutNode::Pane(SourceId(0)),
            0,
            0,
        )
        .expect("a valid one-pane registry")
        .with_title(crate::core::registry::TitleSource::Static(
            "a title much longer than twenty cells".to_string(),
        ));
        let mut runtime = vec![SourceRuntime::for_test()];
        runtime[0].output = Some(vec!["seed".to_string()]);
        runtime[0].posted = true;
        let geom = registry.geometry((60, 10));
        let palette = Palette::builtin(Appearance::Dark, AppearanceSource::Default);
        let block = compose_sources(
            &registry,
            &runtime,
            &geom,
            &PaneView::new(registry.len()),
            false,
            &palette,
            ColorProfile::Ascii,
        );
        let composed = block.lines[1..]
            .iter()
            .map(|line| display_width(line))
            .max()
            .expect("composed rows");
        assert!(
            display_width(&block.lines[0]) <= composed,
            "the title never outgrows the composed frame: {:?}",
            block.lines[0]
        );
        assert!(
            block.lines[0].contains('…'),
            "the cut is marked: {:?}",
            block.lines[0]
        );
    }

    #[test]
    fn a_live_panes_chrome_row_still_carries_its_time_and_exit_badge() {
        // Through the same compose path the loop repaints with, not a
        // hand-built row: the risk is a `live` label displacing a
        // segment, and only the assembled row can show that.
        use crate::core::box_model::{BorderPreset, Sides};
        use crate::core::registry::{LayoutNode, Overflow, PaneBox, PaneWidth};
        let registry = Registry::panes(
            vec![cadence_spec(true, Some(Duration::from_secs(2)), false)],
            vec![PaneBox {
                height: 6,
                width: PaneWidth::Weight(1),
                overflow: Overflow::KeepBottom,
                border: BorderPreset::Rounded,
                padding: Sides::default(),
                title: None,
                chrome: true,
            }],
            LayoutNode::Pane(SourceId(0)),
            0,
            0,
        )
        .expect("a valid one-pane registry");
        let mut runtime = vec![SourceRuntime::for_test()];
        runtime[0].output = Some(vec!["seed".to_string()]);
        runtime[0].posted = true;
        runtime[0].failure = Some("exit 1".to_string());
        let at = ago(60);
        runtime[0].changed_at = at;
        let geom = registry.geometry((60, 8));
        let palette = Palette::builtin(Appearance::Dark, AppearanceSource::Default);
        let block = compose_sources(
            &registry,
            &runtime,
            &geom,
            &PaneView::new(registry.len()),
            false,
            &palette,
            ColorProfile::Ascii,
        );
        let row = block
            .lines
            .iter()
            .find(|line| line.contains(" · "))
            .expect("one row is the chrome row");
        assert!(row.contains("live"), "{row}");
        assert!(!row.contains("every"), "{row}");
        assert!(row.contains("exit 1"), "{row}");
        assert!(row.contains(&local_hms(at)), "{row}");
    }

    #[test]
    fn empty_output_still_renders_one_empty_line() {
        // Whole-stream semantics: an empty stream is ONE empty body line,
        // not zero. A renderer that decoded a retained set element by
        // element would yield nothing here and every empty pane would
        // lose a row.
        assert_eq!(
            output_lines(&Vec::<Vec<u8>>::new().concat(), b""),
            vec![String::new()]
        );
    }

    #[test]
    fn trailing_blank_lines_collapse_exactly_as_they_do_today() {
        // The trim eats the whole RUN of newlines, not one — which a
        // per-element decode would preserve as separate empty lines.
        let retained = [b"a\n".to_vec(), b"\n".to_vec(), b"\n".to_vec()];
        assert_eq!(output_lines(&retained.concat(), b""), vec!["a".to_string()]);
    }

    #[test]
    fn the_plain_path_renders_a_capped_body_exactly_as_an_uncapped_one() {
        // The other render path does its own whole-stream decode, so the
        // pane test above does not cover it. Identity is the assertion:
        // retained-then-concatenated must equal the raw bytes it came
        // from, for every caller of either renderer.
        let raw = b"a\n\n\n".to_vec();
        let retained = [b"a\n".to_vec(), b"\n".to_vec(), b"\n".to_vec()];
        assert_eq!(
            compose_frame(None, &retained.concat(), b"", false),
            compose_frame(None, &raw, b"", false),
        );
        // And the empty case, where the two representations diverge most.
        assert_eq!(
            compose_frame(None, &Vec::<Vec<u8>>::new().concat(), b"", false),
            compose_frame(None, b"", b"", false),
        );
    }

    #[test]
    fn adopting_a_different_appearance_reresolves_the_palette() {
        let mut palette = Palette::builtin(Appearance::Dark, AppearanceSource::Osc);
        assert!(adopt(&mut palette, Appearance::Light));
        assert_eq!(palette.appearance, Appearance::Light);
        assert_eq!(palette.source, AppearanceSource::Notification);
        assert_eq!(palette.accent, Color::Indexed(129));
    }

    #[test]
    fn adopting_the_current_appearance_changes_nothing() {
        let mut palette = Palette::builtin(Appearance::Dark, AppearanceSource::Osc);
        assert!(!adopt(&mut palette, Appearance::Dark));
        // Not even the provenance moves: a repeat report is not a new
        // verdict, and `doctor` must keep reporting how the palette was
        // actually reached.
        assert_eq!(palette.source, AppearanceSource::Osc);
        assert_eq!(palette.accent, Color::Indexed(212));
    }

    #[test]
    fn adopting_back_restores_the_original_tokens() {
        let mut palette = Palette::builtin(Appearance::Dark, AppearanceSource::Osc);
        assert!(adopt(&mut palette, Appearance::Light));
        assert!(adopt(&mut palette, Appearance::Dark));
        assert_eq!(palette.appearance, Appearance::Dark);
        assert_eq!(palette.accent, Color::Indexed(212));
        assert_eq!(palette.on_accent, Color::Indexed(16));
    }

    #[cfg(unix)]
    fn white() -> xterm_color::Color {
        xterm_color::Color::rgb(u16::MAX, u16::MAX, u16::MAX)
    }

    #[cfg(unix)]
    fn black() -> xterm_color::Color {
        xterm_color::Color::rgb(0, 0, 0)
    }

    #[cfg(unix)]
    #[test]
    fn a_reply_nobody_asked_for_is_ignored() {
        let mut verify = VerifyState::default();
        assert_eq!(verify.reply(OscColorKind::Background, black()), None);
    }

    #[cfg(unix)]
    #[test]
    fn a_background_reply_completes_the_exchange() {
        use crate::theme::PROBE_TIMEOUT;

        let mut verify = VerifyState {
            in_flight_until: Some(Instant::now() + PROBE_TIMEOUT),
            ..VerifyState::default()
        };
        assert_eq!(verify.reply(OscColorKind::Foreground, white()), None);
        assert_eq!(
            verify.reply(OscColorKind::Background, black()),
            Some(Appearance::Dark)
        );
        // The exchange is over: a straggler cannot move the verdict again.
        assert!(verify.in_flight_until.is_none());
        assert_eq!(verify.reply(OscColorKind::Background, white()), None);
    }

    #[cfg(unix)]
    #[test]
    fn a_background_reply_alone_still_classifies() {
        use crate::theme::PROBE_TIMEOUT;

        let mut verify = VerifyState {
            in_flight_until: Some(Instant::now() + PROBE_TIMEOUT),
            ..VerifyState::default()
        };
        assert_eq!(
            verify.reply(OscColorKind::Background, white()),
            Some(Appearance::Light)
        );
    }

    fn stamp(secs: i64) -> jiff::Timestamp {
        jiff::Timestamp::from_second(secs).expect("a representable second")
    }

    /// A runtime carrying nothing but the output hash the key folds.
    /// The channel end is real but never used: these tests exercise
    /// the pure pieces, not the loop.
    fn runtime_with(hash: u64) -> SourceRuntime {
        let (tx, _rx) = std::sync::mpsc::channel::<TickEvent>();
        SourceRuntime {
            schedule: TickSchedule::new(Some(Duration::from_secs(2))),
            slot: ChildSlot::default(),
            tx,
            emissions: None,
            output: None,
            hash,
            changed_at: stamp(0),
            previous: None,
            marks: Vec::new(),
            failure: None,
            truncated: None,
            posted: false,
            looping: false,
            bracket: None,
            gate: DebounceGate::new(Duration::ZERO),
            files: MtimeWatchSet::new(Vec::new()),
            #[cfg(unix)]
            readers: Vec::new(),
        }
    }

    #[test]
    fn the_combining_key_changes_when_any_source_changes() {
        // The key is folded over the per-source OUTPUT hashes in
        // registry order: one pane moving must re-key the frame, and
        // two panes trading content must not collide with the
        // original — order is part of the key, not just membership.
        let base = [runtime_with(11), runtime_with(22)];
        let moved = [runtime_with(11), runtime_with(23)];
        let traded = [runtime_with(22), runtime_with(11)];
        assert_ne!(combined_hash(&base), combined_hash(&moved));
        assert_ne!(combined_hash(&base), combined_hash(&traded));
    }

    #[test]
    fn the_combining_key_is_stable_when_no_source_changes() {
        // Byte-silence's precondition: an unchanged dashboard keys
        // identically every iteration, or the gate repaints forever.
        let now = [runtime_with(11), runtime_with(22)];
        let again = [runtime_with(11), runtime_with(22)];
        assert_eq!(combined_hash(&now), combined_hash(&again));
        // And the key is content-only: geometry reaches the gate
        // through PaintKey.cols/rows, never through here.
        assert_eq!(combined_hash(&[]), combined_hash(&[]));
    }

    #[test]
    fn changed_at_takes_the_newest_changed_source() {
        // Two panes both moved in one drain: the frame is as fresh as
        // the newest of them, whichever order the channel handed them
        // over.
        let early = stamp(10);
        let late = stamp(40);
        assert_eq!(
            fold_changed_at(fold_changed_at(None, true, early), true, late),
            Some(late)
        );
        assert_eq!(
            fold_changed_at(fold_changed_at(None, true, late), true, early),
            Some(late)
        );
    }

    #[test]
    fn changed_at_ignores_an_unchanged_source_that_completed_later() {
        // Only a source whose OWN content changed may re-date the
        // frame. A heartbeat pane printing the same bytes every second
        // must never make the dashboard read as fresher than it is.
        let changed = stamp(10);
        let quiet = stamp(40);
        assert_eq!(
            fold_changed_at(fold_changed_at(None, true, changed), false, quiet),
            Some(changed)
        );
        // Nothing changed at all: the caller carries (changed_at,
        // since) forward from the previous frame.
        assert_eq!(fold_changed_at(None, false, quiet), None);
    }

    #[test]
    fn esc_requests_a_tick_on_every_source() {
        // A resume means the WHOLE dashboard is stale: no source keeps
        // its old deadline just because it is not the one the eye was
        // on. A plain request, not a respawn — each in-flight child was
        // started under an environment this gesture does not supersede,
        // so its completion may discharge the request.
        let t = Instant::now();
        let mut runtime = vec![runtime_with(1), runtime_with(2)];
        for r in &mut runtime {
            r.schedule.poll(t);
            r.schedule.completed(t);
            assert_eq!(r.schedule.poll(t), Due::Wait);
        }
        request_now_all(&mut runtime);
        for r in &mut runtime {
            assert_eq!(r.schedule.poll(t), Due::Spawn);
        }
    }

    #[test]
    fn a_theme_flip_requests_a_respawn_on_every_source() {
        // Every in-flight child was started under the superseded
        // RAT_APPEARANCE, so no completion can discharge this: each
        // source spawns a FRESH child, and one adoption cannot leave
        // half the dashboard in the old palette.
        let t = Instant::now();
        let mut runtime = vec![runtime_with(1), runtime_with(2)];
        for r in &mut runtime {
            assert_eq!(r.schedule.poll(t), Due::Spawn); // in flight
        }
        request_respawn_all(&mut runtime);
        for r in &mut runtime {
            r.schedule.completed(t); // the stale-env child lands …
            assert_eq!(r.schedule.poll(t), Due::Spawn); // … and did not satisfy it
        }
    }

    #[test]
    fn help_lines_carry_the_heading_and_the_extra_block() {
        // A3: the reference is shared chrome. The caller names the
        // surface and appends whatever section belongs to it; the key
        // families in between are the same text for everyone.
        let extra = vec![
            String::new(),
            "  refresh triggers:".to_string(),
            "    file:/tmp/state.json".to_string(),
        ];
        let lines = help_lines("rat watch — keys", &extra);
        assert_eq!(lines.first().map(String::as_str), Some("rat watch — keys"));
        assert_eq!(
            lines.last().map(String::as_str),
            Some("    file:/tmp/state.json")
        );
        assert!(
            lines
                .iter()
                .any(|line| line.contains("freeze the frame in place"))
        );
        // An empty extra block adds nothing at all — no stray blank
        // row, no empty heading — and the shared body is a prefix of
        // the extended one, so a section can only ever append.
        let bare = help_lines("rat watch — keys", &[]);
        assert!(!bare.iter().any(|line| line.contains("trigger")));
        assert_eq!(lines[..bare.len()], bare[..]);
    }

    #[cfg(unix)]
    fn exit_status(code: i32) -> std::process::ExitStatus {
        std::os::unix::process::ExitStatusExt::from_raw(code << 8)
    }
    #[cfg(windows)]
    fn exit_status(code: i32) -> std::process::ExitStatus {
        std::os::windows::process::ExitStatusExt::from_raw(code as u32)
    }

    #[test]
    fn a_spawn_error_becomes_the_panes_body_and_carries_no_exit_badge() {
        // A spawn error renders AS the error text. There is no exit
        // code to badge — the child never started.
        let err = std::io::Error::from(std::io::ErrorKind::NotFound);
        assert_eq!(
            pane_body(&[], &[], Some(&err), "plan", "no-such-binary"),
            vec![format!("plan: {err}: {:?}", "no-such-binary")]
        );
        assert_eq!(exit_badge(None), None);
    }

    #[test]
    fn a_panes_spawn_error_leads_with_the_reason_and_ends_with_the_path() {
        // A pane is chopped from the RIGHT, and a declared width no
        // terminal can widen — whatever sits last is what no reader sees.
        // The path is the least informative part and the one the author
        // just typed; the OS reason exists nowhere else.
        let err = std::io::Error::from(std::io::ErrorKind::NotFound);
        let body = pane_body(&[], &[], Some(&err), "flood", "/long/path/ra");
        assert_eq!(body, vec![format!("flood: {err}: {:?}", "/long/path/ra")]);
    }

    #[test]
    fn the_shipped_watch_wording_keeps_its_bytes() {
        // `rat watch`'s looping spawn-error line is byte-frozen: the
        // pane wording reordered, this one must not move.
        let err = std::io::Error::from(std::io::ErrorKind::NotFound);
        assert_eq!(
            watch_spawn_error_text("no-such-binary", &err),
            format!("watch: {:?}: {err}", "no-such-binary")
        );
    }

    #[test]
    fn a_nonzero_exit_shows_stdout_then_stderr_and_badges_the_code() {
        // The direct generalization of shipped watch, which shows
        // output and stderr regardless of exit — plus the badge.
        assert_eq!(
            pane_body(b"out-line\n", b"err-line\n", None, "plan", "prog"),
            vec!["out-line".to_string(), "err-line".to_string()]
        );
        assert_eq!(exit_badge(Some(exit_status(3))).as_deref(), Some("exit 3"));
    }

    #[test]
    fn a_successful_tick_clears_the_previous_failure() {
        // The badge is derived per outcome, never accumulated: the tick
        // after a failure carries None and the chrome row loses its
        // badge.
        assert_eq!(exit_badge(Some(exit_status(0))), None);
    }

    #[test]
    fn a_badge_that_appears_without_a_body_change_still_moves_the_hash() {
        // Same bytes, now failing: without the badge in the pane's hash
        // the composition would carry a badge nothing ever paints.
        let body = vec!["steady".to_string()];
        assert_ne!(
            body_signature(&body, None, false, None),
            body_signature(&body, Some("exit 3"), false, None)
        );
    }

    #[test]
    fn every_badge_is_in_the_change_signature_independently() {
        // The badges are chrome, and all of them are part of what the
        // pane DISPLAYS — so each is in its hash on its own. Every
        // combination of the three must be distinct: two sharing a slot
        // would let one badge mask another's arrival, and enumerating
        // the whole space is the only way to see that, since a list of
        // the pairs someone thought of cannot find the pair they did
        // not.
        let body = vec!["steady".to_string()];
        let all: Vec<u64> = [None, Some("exit 3")]
            .into_iter()
            .flat_map(|failure| {
                [false, true].into_iter().flat_map(move |looping| {
                    [None, Some("90 lines dropped")]
                        .into_iter()
                        .map(move |truncated| (failure, looping, truncated))
                })
            })
            .map(|(failure, looping, truncated)| body_signature(&body, failure, looping, truncated))
            .collect();
        assert_eq!(all.len(), 8, "the whole space, not a sample");
        for (i, a) in all.iter().enumerate() {
            for b in &all[i + 1..] {
                assert_ne!(a, b, "combination {i} collides");
            }
        }
        // And a badge CLEARS back to the un-badged signature. A hash
        // that only ever moved forward would repaint the badge's
        // arrival and then leave it on screen for good.
        assert_eq!(body_signature(&body, None, false, None), all[0]);
    }

    #[test]
    fn only_the_fired_sources_schedule_collapses() {
        // Two trigger-only sources (no deadline after their first
        // completion). One gate fires; only its schedule may spawn.
        // This is the executable spec of the per-source walk: it goes
        // red the moment the gates collapse back into one shared gate.
        let t = Instant::now();
        let mut gates = [
            DebounceGate::new(Duration::ZERO),
            DebounceGate::new(Duration::ZERO),
        ];
        let mut schedules = [TickSchedule::new(None), TickSchedule::new(None)];
        for s in &mut schedules {
            assert_eq!(s.poll(t), Due::Spawn); // the first tick, interval or not
            s.completed(t);
        }
        gates[0].fire(t);
        for (gate, schedule) in gates.iter_mut().zip(schedules.iter_mut()) {
            if gate.due(t) {
                schedule.request_respawn();
            }
        }
        assert_eq!(
            schedules[0].poll(t),
            Due::Spawn,
            "the fired source respawns"
        );
        assert_eq!(
            schedules[1].poll(t),
            Due::Wait,
            "its neighbour stays parked"
        );
    }

    /// Two bordered, chrome-bearing panes side by side, gap 1.
    fn zoom_row_registry() -> Registry {
        use crate::core::box_model::{BorderPreset, Sides};
        use crate::core::registry::{LayoutNode, Overflow, PaneBox, PaneWidth};
        let spec = |id: &str| SourceSpec {
            id: id.to_string(),
            program: SourceProgram::Argv(vec!["true".to_string()]),
            shell: ShellMode::Direct,
            interval: Some(Duration::from_secs(3600)),
            triggers: Vec::new(),
            debounce: Duration::from_millis(250),
            live: false,
        };
        let pane = || PaneBox {
            height: 5,
            width: PaneWidth::Weight(1),
            overflow: Overflow::KeepTop,
            border: BorderPreset::Rounded,
            padding: Sides::default(),
            title: None,
            chrome: true,
        };
        Registry::panes(
            vec![spec("left"), spec("right")],
            vec![pane(), pane()],
            LayoutNode::Row(vec![
                LayoutNode::Pane(SourceId(0)),
                LayoutNode::Pane(SourceId(1)),
            ]),
            1,
            0,
        )
        .expect("a valid two-pane row registry")
    }

    /// A `PaneView` for the geometry tests. Only `zoomed` is read by
    /// `derive_geometry`; the other fields are INV-2's shape.
    fn view_zooming(registry: &Registry, zoomed: Option<SourceId>) -> PaneView {
        let n = registry.len();
        PaneView {
            focus: zoomed,
            zoomed,
            collapsed: vec![false; n],
            scroll: vec![initial_pane_scroll(Overflow::KeepTop, 0, 0); n],
        }
    }

    #[test]
    fn z_zooms_the_focused_pane_only_while_live() {
        assert_eq!(
            action_for(Key::Char('z'), FrameMode::Live),
            WatchAction::ToggleZoom
        );
        // Per-pane gestures are Live-only (INV-3): a frozen or scrolled
        // frame is a composed string with no pane identity in it.
        for mode in [FrameMode::LiveScrolled, FrameMode::Paused] {
            assert_eq!(action_for(Key::Char('z'), mode), WatchAction::Ignore);
        }
        // One spelling: `Z` is not a second key.
        for mode in ALL_MODES {
            assert_eq!(action_for(Key::Char('Z'), mode), WatchAction::Ignore);
        }
    }

    #[test]
    fn a_zoomed_compose_joins_only_the_zoomed_pane() {
        let registry = zoom_row_registry();
        let palette = Palette::builtin(Appearance::Dark, AppearanceSource::Default);
        let mut runtime = vec![SourceRuntime::for_test(), SourceRuntime::for_test()];
        for (i, text) in ["alpha", "beta"].iter().enumerate() {
            runtime[i].output = Some(vec![(*text).to_string()]);
            runtime[i].posted = true;
        }
        let flat = compose_sources(
            &registry,
            &runtime,
            &derive_geometry(
                &registry,
                (80, 24),
                None,
                false,
                &view_zooming(&registry, None),
            ),
            &view_zooming(&registry, None),
            false,
            &palette,
            ColorProfile::TrueColor,
        );
        assert!(flat.lines.iter().any(|l| l.contains("beta")));

        let panes = view_zooming(&registry, Some(SourceId(0)));
        let geom = derive_geometry(&registry, (80, 24), None, false, &panes);
        let zoomed = compose_sources(
            &registry,
            &runtime,
            &geom,
            &panes,
            false,
            &palette,
            ColorProfile::TrueColor,
        );
        assert!(zoomed.lines.iter().any(|l| l.contains("alpha")));
        assert!(
            !zoomed.lines.iter().any(|l| l.contains("beta")),
            "the hidden pane's block is never joined"
        );
        assert_eq!(
            zoomed.lines.len(),
            geom[0].rows as usize,
            "one pane, the frame's rows"
        );
        assert_eq!(
            zoomed.lines.len(),
            zoomed.marks.len(),
            "marks stay aligned to lines"
        );
    }

    #[test]
    fn a_zoomed_pane_takes_the_frame_and_its_neighbour_keeps_its_box() {
        let registry = zoom_row_registry();
        let declared = derive_geometry(
            &registry,
            (80, 24),
            None,
            false,
            &view_zooming(&registry, None),
        );
        let zoomed = derive_geometry(
            &registry,
            (80, 24),
            None,
            false,
            &view_zooming(&registry, Some(SourceId(0))),
        );
        // cells = size.0 - gutter_reserve(false); rows =
        // window_rows(None, 24) with no static title row to pay for.
        assert_eq!(zoomed[0].cells, 80);
        assert_eq!(zoomed[0].rows, 22);
        // inner_* are the box arithmetic, not a second rule: rounded
        // border + chrome row.
        assert_eq!(
            zoomed[0].inner_cols,
            80 - registry.pane(SourceId(0)).unwrap().frame_cols()
        );
        assert_eq!(
            zoomed[0].inner_rows,
            22 - registry.pane(SourceId(0)).unwrap().frame_rows()
        );
        assert!(
            declared[0].cells < zoomed[0].cells && declared[0].rows < zoomed[0].rows,
            "the override must actually move the box: {:?} vs {:?}",
            declared[0],
            zoomed[0]
        );
        // Hidden panes keep their DECLARED geometry — they are simply not
        // composed (INV-5; Zellij's invariant stated positively).
        assert_eq!(zoomed[1], declared[1]);
    }

    #[test]
    fn the_zoom_override_pays_for_the_gutter_and_a_static_title() {
        let registry = zoom_row_registry();
        let panes = view_zooming(&registry, Some(SourceId(0)));
        let gutter = derive_geometry(&registry, (80, 24), None, true, &panes);
        assert_eq!(
            gutter[0].cells,
            80 - gutter_reserve(true),
            "the gutter is a region, not an overlay"
        );

        use crate::core::registry::TitleSource;
        let titled = zoom_row_registry().with_title(TitleSource::Static("board".to_string()));
        assert_eq!(
            derive_geometry(&titled, (80, 24), None, false, &panes)[0].rows,
            21,
            "a static title costs the zoomed pane exactly the row compose_sources prepends"
        );
        // A pane-sourced title renders no extra line, so it costs no row.
        let referred = zoom_row_registry().with_title(TitleSource::Pane {
            source: SourceId(1),
            fallback: None,
        });
        assert_eq!(
            derive_geometry(&referred, (80, 24), None, false, &panes)[0].rows,
            22
        );
        // max_height is the row authority when it is set.
        assert_eq!(
            derive_geometry(&registry, (80, 24), Some(10), false, &panes)[0].rows,
            10
        );
    }

    #[test]
    fn a_borderless_pane_zooms_to_the_whole_budget_and_a_tiny_terminal_saturates() {
        // once_registry's boxes are border=none, chrome=true, height 3.
        let bare = once_registry(&[("a", false), ("b", false)]);
        let panes = view_zooming(&bare, Some(SourceId(0)));
        let geom = derive_geometry(&bare, (80, 24), None, false, &panes);
        assert_eq!(
            geom[0].inner_cols, 80,
            "no border, no padding: inner IS the box"
        );
        assert_eq!(geom[0].inner_rows, 22 - 1, "only the chrome row is owed");

        // Nothing panics and nothing wraps at a terminal smaller than the
        // reservations: every subtraction saturates.
        let tiny = derive_geometry(&bare, (1, 1), None, true, &panes);
        assert_eq!(tiny[0].cells, 0);
        assert_eq!(tiny[0].rows, 0);
        assert_eq!(tiny[0].inner_cols, 0);
        assert_eq!(tiny[0].inner_rows, 0);
    }

    #[test]
    fn a_zoom_is_not_a_resize() {
        let registry = zoom_row_registry();
        let panes = view_zooming(&registry, Some(SourceId(0)));
        let mut size = (80u16, 24u16);
        let mut geom = derive_geometry(&registry, size, None, false, &panes);
        // Without this the test is vacuous: a derivation that ignored
        // `zoomed` would also compare equal below.
        assert_eq!(geom[0].cells, 80, "the stored geom is the ZOOMED geom");

        let step = detect_resize(size, None, false, &panes, &mut size, &mut geom, &registry);
        assert!(!step.size_moved);
        assert!(
            !step.geom_moved,
            "a view gesture that re-derived unequal arms the 250ms gate and \
             restarts every child — live ones included"
        );

        // The falsification arm: the equality above is a property of the
        // SHARED PaneView, not of the comparison. A detect_resize handed a
        // different view sees a real difference.
        let unzoomed = view_zooming(&registry, None);
        let moved = detect_resize(
            size, None, false, &unzoomed, &mut size, &mut geom, &registry,
        );
        assert!(
            moved.geom_moved,
            "the same geom under a different view MUST move"
        );
    }

    #[test]
    fn a_resize_while_zoomed_re_derives_every_pane_then_overrides() {
        let registry = zoom_row_registry();
        let panes = view_zooming(&registry, Some(SourceId(0)));
        let mut size = (80u16, 24u16);
        let mut geom = derive_geometry(&registry, size, None, false, &panes);

        let step = detect_resize(
            (120, 40),
            None,
            false,
            &panes,
            &mut size,
            &mut geom,
            &registry,
        );
        assert!(step.size_moved && step.geom_moved);
        // The zoomed pane tracks the new terminal, both axes.
        assert_eq!(geom[0].cells, 120);
        assert_eq!(geom[0].rows, 38);
        // The HIDDEN pane is exactly what an unzoomed derivation at the new
        // size gives it — never stale, never skipped (q5's Zellij bug,
        // stated positively).
        let declared = derive_geometry(
            &registry,
            (120, 40),
            None,
            false,
            &view_zooming(&registry, None),
        );
        assert_eq!(geom[1], declared[1]);
        // …and the zoom survived the resize.
        let after = detect_resize(
            (120, 40),
            None,
            false,
            &panes,
            &mut size,
            &mut geom,
            &registry,
        );
        assert!(
            !after.geom_moved,
            "the settled frame re-derives equal (INV-1)"
        );
    }

    #[test]
    fn a_rows_only_resize_moves_the_geometry_only_while_zoomed() {
        let registry = zoom_row_registry();
        let mut size = (80u16, 24u16);

        // Unzoomed, this is today's shipped behavior: declared heights are
        // pinned, so more rows change the window and no child's world.
        let flat = view_zooming(&registry, None);
        let mut geom = derive_geometry(&registry, size, None, false, &flat);
        let step = detect_resize(
            (80, 40),
            None,
            false,
            &flat,
            &mut size,
            &mut geom,
            &registry,
        );
        assert!(step.size_moved && !step.geom_moved);

        // Zoomed, the row budget IS the zoomed pane's height, so a
        // rows-only resize genuinely moves its inner geometry — and the
        // resize arm's debounced respawn-all is owed, as for any other
        // geometry change.
        let mut size = (80u16, 24u16);
        let panes = view_zooming(&registry, Some(SourceId(0)));
        let mut geom = derive_geometry(&registry, size, None, false, &panes);
        let step = detect_resize(
            (80, 40),
            None,
            false,
            &panes,
            &mut size,
            &mut geom,
            &registry,
        );
        assert!(step.geom_moved, "the zoomed pane's rows follow window_rows");
        assert_eq!(geom[0].rows, 38);
    }

    #[test]
    fn the_gutter_toggle_keeps_the_zoom_and_stays_off_the_gate() {
        let registry = zoom_row_registry();
        let panes = view_zooming(&registry, Some(SourceId(0)));
        let mut size = (80u16, 24u16);
        // What the gutter arm now computes (1.4 moved this line onto
        // derive_geometry; this test is the witness that it passes the view).
        let mut geom = derive_geometry(&registry, size, None, true, &panes);
        assert_eq!(
            geom[0].cells,
            80 - gutter_reserve(true),
            "still zoomed, two columns lighter"
        );
        let step = detect_resize(size, None, true, &panes, &mut size, &mut geom, &registry);
        assert!(
            !step.geom_moved,
            "a view toggle under a zoom must still compare equal, or every child restarts"
        );
    }

    #[test]
    fn space_toggles_collapse_only_while_live() {
        assert_eq!(
            action_for(Key::Space, FrameMode::Live),
            WatchAction::ToggleCollapse
        );
        // Per-pane gestures are Live-only (INV-3): a frozen or scrolled
        // frame is a composed string with no pane identity left in it.
        for mode in [FrameMode::LiveScrolled, FrameMode::Paused] {
            assert_eq!(
                action_for(Key::Space, mode),
                WatchAction::Ignore,
                "{mode:?}"
            );
        }
        // Not a second spelling: both input paths deliver 0x20 as
        // Key::Space (the crossterm map and the scanner), so the space
        // CHARACTER stays unbound and cannot drift into a second binding.
        for mode in ALL_MODES {
            assert_eq!(action_for(Key::Char(' '), mode), WatchAction::Ignore);
        }
    }

    #[test]
    fn a_collapsed_pane_composes_one_row_and_a_zoom_still_shows_its_body() {
        let registry = once_registry(&[("logs", false), ("metrics", false)]);
        let palette = Palette::builtin(Appearance::Dark, AppearanceSource::Default);
        let mut runtime: Vec<SourceRuntime> = (0..2).map(|_| SourceRuntime::for_test()).collect();
        runtime[0].output = Some(vec!["alpha-body".to_string()]);
        runtime[0].posted = true;
        runtime[1].output = Some(vec!["beta-body".to_string()]);
        runtime[1].posted = true;
        let geom = registry.geometry((40, 12));
        let mut panes = PaneView::new(registry.len());

        let full = compose_sources(
            &registry,
            &runtime,
            &geom,
            &panes,
            false,
            &palette,
            ColorProfile::Ascii,
        );
        panes.collapsed[0] = true;
        let collapsed = compose_sources(
            &registry,
            &runtime,
            &geom,
            &panes,
            false,
            &palette,
            ColorProfile::Ascii,
        );
        // Three declared rows, one rendered — the column shortens (INV-8).
        assert_eq!(full.lines.len() - collapsed.lines.len(), 2);
        assert_eq!(collapsed.marks.len(), collapsed.lines.len());
        assert!(
            !collapsed.lines.iter().any(|l| l.contains("alpha-body")),
            "a collapsed pane composes no body: {:?}",
            collapsed.lines
        );
        assert!(
            collapsed.lines[0].contains("logs"),
            "the row names its pane"
        );
        assert!(
            collapsed.lines.iter().any(|l| l.contains("beta-body")),
            "its sibling is untouched"
        );

        // INV-12: zoom composes the pane's FULL content even while its
        // collapsed bit is set, and the bit survives to restore it.
        panes.zoomed = Some(SourceId(0));
        let zoomed = compose_sources(
            &registry,
            &runtime,
            &geom,
            &panes,
            false,
            &palette,
            ColorProfile::Ascii,
        );
        assert!(
            zoomed.lines.iter().any(|l| l.contains("alpha-body")),
            "a zoomed pane shows its body regardless of collapse: {:?}",
            zoomed.lines
        );
        assert!(panes.collapsed[0], "zoom never clears the bit");
    }

    #[test]
    fn a_collapsed_pane_measures_one_row_for_the_directional_walk() {
        let registry = once_registry(&[("a", false), ("b", false)]);
        let geom = registry.geometry((40, 12));
        let mut panes = PaneView::new(registry.len());
        assert_eq!(pane_block_sizes(&geom, &panes), vec![(3, 40), (3, 40)]);
        panes.collapsed[0] = true;
        assert_eq!(
            pane_block_sizes(&geom, &panes),
            vec![(1, 40), (3, 40)],
            "Alt-j must aim at the row the screen has, not the one declared"
        );
    }

    #[test]
    fn zooming_reanchors_the_panes_viewport_to_the_new_window() {
        let registry = zoom_row_registry();
        let mut runtime = vec![SourceRuntime::for_test(), SourceRuntime::for_test()];
        let body: Vec<String> = (1..=60).map(|i| format!("r{i}")).collect();
        runtime[0].output = Some(body.clone());
        runtime[1].output = Some(body);
        let panes_at = |zoomed| {
            let panes = view_zooming(&registry, zoomed);
            let geom = derive_geometry(&registry, (80, 24), None, false, &panes);
            (panes, geom)
        };
        assert_eq!(
            (
                panes_at(None).1[0].inner_rows,
                panes_at(Some(SourceId(0))).1[0].inner_rows
            ),
            (2, 19)
        );

        // A pane at its KeepBottom rest — pinned to the tail — stays at the
        // tail when the window grows.
        let (mut panes, geom) = panes_at(Some(SourceId(0)));
        panes.scroll[0] = LiveScroll::start(ScrollStep::Bottom, 60, 2);
        reanchor_pane_scrolls(&mut panes, &runtime, &geom);
        assert!(panes.scroll[0].pinned());
        assert_eq!(panes.scroll[0].offset(), 60 - 19);

        // An unpinned offset HOLDS across the gesture (D4) …
        panes.scroll[0] = LiveScroll::at(5, 60, 2);
        reanchor_pane_scrolls(&mut panes, &runtime, &geom);
        assert_eq!(
            (panes.scroll[0].offset(), panes.scroll[0].pinned()),
            (5, false)
        );

        // … and is CLAMPED, never left past the end, when the bigger window
        // shortens the maximum offset from 58 to 41.
        panes.scroll[0] = LiveScroll::at(58, 60, 2);
        reanchor_pane_scrolls(&mut panes, &runtime, &geom);
        assert_eq!(panes.scroll[0].offset(), 41);

        // Unzoom is the same call with the declared geometry: the reader's
        // place is kept, never re-expanded toward the tail. The helper is
        // whole-view by design (it is also the collect step's), so the
        // neighbour is reanchored too — and a pane at rest reanchors to
        // itself, which is what keeps the gesture invisible to it.
        let (mut panes, geom) = panes_at(None);
        panes.scroll[0] = LiveScroll::at(41, 60, 19);
        let neighbour = panes.scroll[1];
        reanchor_pane_scrolls(&mut panes, &runtime, &geom);
        assert_eq!(panes.scroll[0].offset(), 41);
        assert_eq!(panes.scroll[1], neighbour, "a pane at rest is unmoved");
    }

    // The engine itself never names these two types (they live behind
    // the registry contract), so the test module imports them
    // explicitly — `use super::*` cannot supply them.
    fn two_weighted_panes() -> Registry {
        use crate::core::box_model::{BorderPreset, Sides};
        use crate::core::registry::{LayoutNode, Overflow, PaneBox, PaneWidth};
        let spec = |id: &str| SourceSpec {
            id: id.to_string(),
            program: SourceProgram::Argv(vec!["true".to_string()]),
            shell: ShellMode::Direct,
            interval: Some(Duration::from_secs(3600)),
            triggers: Vec::new(),
            debounce: Duration::from_millis(250),
            live: false,
        };
        let pane = || PaneBox {
            height: 5,
            width: PaneWidth::Weight(1),
            overflow: Overflow::KeepTop,
            border: BorderPreset::Rounded,
            padding: Sides::default(),
            title: None,
            chrome: true,
        };
        Registry::panes(
            vec![spec("left"), spec("right")],
            vec![pane(), pane()],
            LayoutNode::Column(vec![LayoutNode::Row(vec![
                LayoutNode::Pane(SourceId(0)),
                LayoutNode::Pane(SourceId(1)),
            ])]),
            1,
            0,
        )
        .expect("a valid two-pane registry")
    }

    #[test]
    fn resizes_inside_one_window_collapse_to_one_respawn() {
        // The gate is ANCHORED, not sliding: a burst of size changes
        // inside one window owes ONE respawn, and the window does not
        // move for the later fires.
        let t = Instant::now();
        let mut gate = DebounceGate::new(RESIZE_DEBOUNCE);
        gate.fire(t);
        gate.fire(t + Duration::from_millis(100));
        gate.fire(t + Duration::from_millis(200));
        assert!(
            !gate.due(t + Duration::from_millis(200)),
            "window still open"
        );
        assert!(gate.due(t + RESIZE_DEBOUNCE), "the window closes once");
        assert!(!gate.due(t + RESIZE_DEBOUNCE), "and only once");
    }

    #[test]
    fn a_respawn_request_reaches_every_source() {
        // A debounced resize respawns ALL sources, because every
        // in-flight child was started under the superseded geometry.
        let t = Instant::now();
        let mut schedules = [
            TickSchedule::new(Some(Duration::from_secs(3600))),
            TickSchedule::new(Some(Duration::from_secs(3600))),
        ];
        for s in &mut schedules {
            assert_eq!(s.poll(t), Due::Spawn);
            s.completed(t);
            assert_eq!(s.poll(t), Due::Wait, "parked for an hour");
        }
        for s in &mut schedules {
            s.request_respawn();
        }
        for s in &mut schedules {
            assert_eq!(s.poll(t), Due::Spawn);
        }
    }

    #[test]
    fn a_coincident_spawn_does_not_blind_the_resize_arm() {
        // The race, deterministically: ONE iteration in which a source
        // is due AND the terminal has already moved. With a resize arm
        // present the spawn step's writer is a no-op, so the arm still
        // observes the change and owes the respawn-all.
        let registry = two_weighted_panes();
        let mut size = (80, 24);
        let mut geom = registry.geometry(size);
        let before = geom.clone();
        refresh_geometry_for_spawn(true, (120, 24), 0, &mut size, &mut geom, &registry);
        assert_eq!(size, (80, 24), "the spawn step wrote nothing under panes");
        assert_eq!(geom, before);
        let panes = PaneView::new(registry.len());
        let step = detect_resize(
            (120, 24),
            None,
            false,
            &panes,
            &mut size,
            &mut geom,
            &registry,
        );
        assert!(step.size_moved, "detection survived the coincident spawn");
        assert!(
            step.geom_moved,
            "the inner widths moved, so a respawn is owed"
        );
        assert_ne!(geom, before, "the pair advanced exactly once, in the arm");
    }

    #[test]
    fn plain_mode_still_measures_at_spawn() {
        // The other half of the one-writer rule: without a resize arm
        // the spawn-step writer is live, the shipped watch cadence.
        let registry = two_weighted_panes();
        let mut size = (80, 24);
        let mut geom = registry.geometry(size);
        refresh_geometry_for_spawn(false, (120, 24), 0, &mut size, &mut geom, &registry);
        assert_eq!(size, (120, 24), "plain re-measures when something spawns");
        assert_eq!(geom, registry.geometry((120, 24)));
    }

    #[test]
    fn a_gutter_toggle_never_reads_as_a_resize() {
        // The respawn trap, as an assertion: the toggle narrows the
        // stored geom under the reservation, and detect_resize applies
        // the SAME reservation — so at an unchanged terminal size the
        // comparison is equal, the debounce gate stays unarmed, and no
        // child restarts 250ms after a keypress.
        let registry = two_weighted_panes();
        let panes = PaneView::new(registry.len());
        let mut size = (80, 24);
        let mut geom = derive_geometry(&registry, size, None, true, &panes);
        assert_ne!(
            geom,
            registry.geometry(size),
            "the reservation must actually narrow a panes layout"
        );
        let step = detect_resize(
            (80, 24),
            None,
            true,
            &panes,
            &mut size,
            &mut geom,
            &registry,
        );
        assert!(!step.size_moved, "the terminal did not move");
        assert!(
            !step.geom_moved,
            "a view toggle must never arm the respawn gate"
        );
        // Toggling back off under the zero reservation is symmetric.
        geom = derive_geometry(&registry, size, None, false, &panes);
        let step = detect_resize(
            (80, 24),
            None,
            false,
            &panes,
            &mut size,
            &mut geom,
            &registry,
        );
        assert!(!step.geom_moved);
    }

    #[test]
    fn the_view_key_moves_with_every_per_pane_bit() {
        let base = PaneView::new(3);
        // Equal state, equal key: the gate must never repaint on its own.
        assert_eq!(base.key(), PaneView::new(3).key());

        let mut focused = PaneView::new(3);
        focused.focus = Some(SourceId(2));
        assert_ne!(base.key(), focused.key());

        let mut zoomed = PaneView::new(3);
        zoomed.zoomed = Some(SourceId(2));
        assert_ne!(base.key(), zoomed.key());
        assert_ne!(focused.key(), zoomed.key());

        let mut collapsed = PaneView::new(3);
        collapsed.collapsed[1] = true;
        assert_ne!(base.key(), collapsed.key());
        // WHICH pane is what the hash must carry: the same bit on a
        // different pane is a different key.
        let mut other = PaneView::new(3);
        other.collapsed[2] = true;
        assert_ne!(collapsed.key(), other.key());

        let mut scrolled = PaneView::new(3);
        scrolled.scroll[0] = LiveScroll::at(2, 50, 10);
        assert_ne!(base.key(), scrolled.key());
    }

    #[test]
    fn a_pin_flip_alone_moves_the_view_key() {
        // A KeepBottom pane stepped off its tail and re-pinned at the same
        // offset differs ONLY in the pin bit; the gate must see it or the
        // badge/footer change it implies paints nothing.
        let mut a = PaneView::new(1);
        let mut b = PaneView::new(1);
        a.scroll[0] = LiveScroll::at(3, 10, 4);
        b.scroll[0] = LiveScroll::start(ScrollStep::Bottom, 7, 4);
        assert_eq!(
            a.scroll[0].offset(),
            b.scroll[0].offset(),
            "fixture premise"
        );
        assert_ne!(a.key(), b.key(), "the pin bit is a gate term (INV-2)");
    }

    #[test]
    fn a_keep_bottom_pane_is_at_rest_while_it_is_pinned() {
        // The declared clip: KeepBottom's rest state is the tail, and
        // `initial_pane_scroll` must produce exactly the offset the shipped
        // `render_pane` match produced for the same body.
        let rest = initial_pane_scroll(Overflow::KeepBottom, 46, 22);
        assert_eq!(rest.offset(), max_offset(46, 22));
        assert!(rest.pinned());
        assert!(pane_at_rest(rest, Overflow::KeepBottom));
        // And it keeps riding the tail across a tick, still at rest.
        let grown = rest.reanchor(60, 22);
        assert_eq!(grown.offset(), max_offset(60, 22));
        assert!(pane_at_rest(grown, Overflow::KeepBottom));
    }

    #[test]
    fn a_keep_top_pane_is_at_rest_at_offset_zero_unpinned() {
        let rest = initial_pane_scroll(Overflow::KeepTop, 46, 22);
        assert_eq!(rest.offset(), 0);
        assert!(!rest.pinned());
        assert!(pane_at_rest(rest, Overflow::KeepTop));
        // Stepped away, then back: `g` returns a KeepTop pane to rest.
        let moved = rest.step(ScrollStep::LineDown, 46, 22);
        assert!(!pane_at_rest(moved, Overflow::KeepTop));
        assert!(pane_at_rest(
            moved.step(ScrollStep::Top, 46, 22),
            Overflow::KeepTop
        ));
        // `G` on a KeepTop pane is NOT rest: it pinned the window to a tail
        // its declaration never asked for, and the badge must say so.
        let pinned = rest.step(ScrollStep::Bottom, 46, 22);
        assert!(!pane_at_rest(pinned, Overflow::KeepTop));
    }

    #[test]
    fn at_top_would_lie_in_both_directions_at_pane_scope() {
        // Why `pane_at_rest` exists at all (grounding §5). `at_top()` is the
        // whole-frame idiom — offset 0 collapses the mode — and at pane scope
        // it is wrong BOTH ways for a KeepBottom pane.
        let rest = initial_pane_scroll(Overflow::KeepBottom, 46, 22);
        assert!(!rest.at_top(), "at rest, and at_top() calls it scrolled");
        assert!(pane_at_rest(rest, Overflow::KeepBottom));

        let scrolled_to_head = rest.step(ScrollStep::Top, 46, 22);
        assert!(scrolled_to_head.at_top(), "at_top() calls this one at rest");
        assert!(
            !pane_at_rest(scrolled_to_head, Overflow::KeepBottom),
            "a KeepBottom pane parked at its head is scrolled, and the badge must show it"
        );
    }

    #[test]
    fn a_scroll_step_addresses_a_pane_only_when_one_is_focused_and_live() {
        let mut panes = PaneView::new(2);
        assert_eq!(
            scroll_target(FrameMode::Live, &panes),
            None,
            "no focus, no target"
        );

        panes.focus = Some(SourceId(1));
        assert_eq!(scroll_target(FrameMode::Live, &panes), Some(SourceId(1)));
        // Per-pane gestures are Live-only (INV-3): a frozen or
        // live-scrolled frame is a composed string with no pane identity,
        // so the whole-frame arm keeps those keys.
        assert_eq!(scroll_target(FrameMode::Paused, &panes), None);
        assert_eq!(scroll_target(FrameMode::LiveScrolled, &panes), None);
        // A focused pane OWNS the scroll keys even while collapsed: the
        // target stays Some, and the arm itself declines the step (the
        // collapse task's `continue`). Returning None here would route
        // the keys back to the whole-frame arm — the INV-7 violation,
        // not the no-op.
        panes.collapsed[1] = true;
        assert_eq!(scroll_target(FrameMode::Live, &panes), Some(SourceId(1)));
    }

    #[test]
    fn a_reanchor_holds_a_panes_offset_across_a_body_replacement() {
        // D4: a batch pane's body is REPLACED wholesale every run
        // (`record_output`), and the reader's place is positional. Holding
        // is the rule; the clamp is the only thing that may move it.
        let scroll =
            initial_pane_scroll(Overflow::KeepTop, 40, 5).step(ScrollStep::HalfDown, 40, 5);
        assert_eq!(scroll.offset(), 2);
        assert_eq!(scroll.reanchor(40, 5).offset(), 2, "same shape, same place");
        assert_eq!(
            scroll.reanchor(400, 5).offset(),
            2,
            "a longer body holds it"
        );
        assert_eq!(
            scroll.reanchor(3, 5).offset(),
            0,
            "a two-line failure body clamps it to the head — the only reset"
        );
        // A KeepBottom pane at rest keeps riding its tail across the same
        // replacement: that is today's behavior, unchanged.
        let tail = initial_pane_scroll(Overflow::KeepBottom, 40, 5);
        assert_eq!(tail.reanchor(400, 5).offset(), max_offset(400, 5));
    }

    #[test]
    fn a_pane_names_its_position_only_when_it_is_off_its_rest() {
        // Both surfaces consult this, so a pane can never carry a badge
        // on its chrome row and nothing on the footer (or "lines 1-0 of 0").
        let top = initial_pane_scroll(Overflow::KeepTop, 6, 4);
        assert_eq!(pane_scroll_badge(top, Overflow::KeepTop, 6, 4), None);
        assert_eq!(
            pane_scroll_badge(
                top.step(ScrollStep::LineDown, 6, 4),
                Overflow::KeepTop,
                6,
                4
            )
            .as_deref(),
            Some("lines 2-5 of 6")
        );
        // KeepBottom at rest is PINNED, not offset zero — the badge must
        // not appear for a log pane doing exactly what it was declared to.
        let tail = initial_pane_scroll(Overflow::KeepBottom, 6, 4);
        assert_eq!(tail.offset(), 2);
        assert_eq!(pane_scroll_badge(tail, Overflow::KeepBottom, 6, 4), None);
        assert_eq!(
            pane_scroll_badge(tail.step(ScrollStep::Top, 6, 4), Overflow::KeepBottom, 6, 4)
                .as_deref(),
            Some("lines 1-4 of 6")
        );
        // An empty body has no position to report, whatever its pin bit.
        let empty = initial_pane_scroll(Overflow::KeepBottom, 0, 4).step(ScrollStep::Top, 0, 4);
        assert_eq!(pane_scroll_badge(empty, Overflow::KeepBottom, 0, 4), None);
    }

    #[test]
    fn the_pager_follows_focus_only_while_live_and_serves_a_collapsed_pane() {
        let mut panes = PaneView::new(2);
        assert_eq!(
            pager_target(FrameMode::Live, &panes),
            None,
            "no focus, whole frame"
        );
        panes.focus = Some(SourceId(0));
        assert_eq!(pager_target(FrameMode::Live, &panes), Some(SourceId(0)));
        // Paused and scrubbed frames are composed strings with no pane
        // identity: they keep paging their frozen snapshot (INV-3).
        assert_eq!(pager_target(FrameMode::Paused, &panes), None);
        assert_eq!(pager_target(FrameMode::LiveScrolled, &panes), None);
        // A collapsed pane keeps BOTH targets: the gestures diverge at
        // their arms, never here — a collapsed pane's scroll step
        // declines in the arm (its window is not on screen to move),
        // while its page serves the body a reader has no other way to
        // see.
        panes.collapsed[0] = true;
        assert_eq!(pager_target(FrameMode::Live, &panes), Some(SourceId(0)));
        assert_eq!(scroll_target(FrameMode::Live, &panes), Some(SourceId(0)));
    }

    #[test]
    fn a_panes_window_at_rest_is_exactly_the_overflow_clip() {
        // Phase 2's byte-inertness pin. `render_pane`'s viewport becomes
        // `view.scroll[i].offset()` in 2.3; with no gesture pressed that
        // number must BE the clip the shipped match produced — at the
        // first frame and after every tick's reanchor, for both rules.
        for (total, window) in [(0, 5), (3, 5), (46, 22), (1000, 7)] {
            for overflow in [Overflow::KeepTop, Overflow::KeepBottom] {
                let rest = initial_pane_scroll(overflow, total, window);
                assert_eq!(
                    rest.offset(),
                    overflow_clip(overflow, total, window),
                    "{overflow:?} at rest over {total} lines in {window} rows"
                );
                // And it stays the clip as the body grows and shrinks —
                // a pinned window tracks, an unpinned one holds at 0.
                for grown in [total + 37, total.saturating_sub(2)] {
                    assert_eq!(
                        rest.reanchor(grown, window).offset(),
                        overflow_clip(overflow, grown, window),
                        "{overflow:?} after a tick to {grown} lines"
                    );
                }
            }
        }
    }

    #[test]
    fn the_derivation_is_the_reserved_allocation() {
        // One derivation, and in this phase it is exactly the reserved
        // allocation: nothing in the per-pane view moves a pane's cells.
        let registry = two_weighted_panes();
        let panes = PaneView::new(registry.len());
        for gutter in [false, true] {
            assert_eq!(
                derive_geometry(&registry, (80, 24), None, gutter, &panes),
                registry.geometry_reserving((80, 24), gutter_reserve(gutter))
            );
        }
    }

    #[test]
    fn a_per_pane_view_change_never_reads_as_a_resize() {
        // The plan's most important NON-event: a gesture that moves the
        // view must derive the same geometry, or the very next
        // iteration arms the 250ms gate and every child — live ones
        // included — restarts after a keypress.
        let registry = two_weighted_panes();
        let mut panes = PaneView::new(registry.len());
        let mut size = (80, 24);
        let mut geom = derive_geometry(&registry, size, None, false, &panes);
        panes.focus = Some(SourceId(1));
        panes.collapsed[0] = true;
        let step = detect_resize(
            (80, 24),
            None,
            false,
            &panes,
            &mut size,
            &mut geom,
            &registry,
        );
        assert!(!step.size_moved, "the terminal did not move");
        assert!(
            !step.geom_moved,
            "focus and collapse are not allocation terms"
        );
    }

    #[test]
    fn the_paint_key_carries_the_per_pane_view() {
        // The gate is output-derived at pane scope, so without this
        // field a gesture that changes only what is composed paints
        // nothing.
        let registry = two_weighted_panes();
        let mut panes = PaneView::new(registry.len());
        let view = ViewState {
            wrap: true,
            hshift: 0,
            gutter: false,
            highlight: false,
            alt_time: false,
        };
        let before = paint_key(
            None,
            None,
            42,
            Appearance::Dark,
            (80, 24),
            view,
            panes.key(),
            0,
        );
        panes.focus = Some(SourceId(1));
        let after = paint_key(
            None,
            None,
            42,
            Appearance::Dark,
            (80, 24),
            view,
            panes.key(),
            0,
        );
        assert_ne!(before, after, "same content, moved focus, new key");
        assert_eq!(after.view, panes.key());
    }

    /// A timestamp `secs` in the past, without timestamp arithmetic.
    fn ago(secs: i64) -> jiff::Timestamp {
        jiff::Timestamp::from_second(jiff::Timestamp::now().as_second() - secs)
            .expect("timestamp in range")
    }

    impl SourceRuntime {
        /// Enough runtime to drive the pure per-source state rules: no
        /// interval, an empty slot, and a channel whose receiver is
        /// dropped (nothing here ever sends).
        fn for_test() -> SourceRuntime {
            let (tx, _rx) = std::sync::mpsc::channel();
            SourceRuntime {
                schedule: TickSchedule::new(None),
                slot: ChildSlot::default(),
                tx,
                emissions: None,
                output: None,
                hash: 0,
                previous: None,
                marks: Vec::new(),
                changed_at: jiff::Timestamp::now(),
                failure: None,
                truncated: None,
                posted: false,
                looping: false,
                bracket: None,
                gate: DebounceGate::new(Duration::ZERO),
                files: MtimeWatchSet::new(Vec::new()),
                #[cfg(unix)]
                readers: Vec::new(),
            }
        }
    }

    #[test]
    // A one-range vec like `vec![0..1]` is exactly what a single marked
    // run looks like — not a mistyped `(0..1).collect()`.
    #[allow(clippy::single_range_in_vec_init)]
    fn a_paused_frame_marks_against_history_not_pane_outputs() {
        // A past composition's only comparand is its history
        // predecessor, so the per-pane vector must be ignored.
        let prev = vec!["a".to_string(), "b".to_string()];
        let viewed = vec!["a".to_string(), "B".to_string()];
        // Deliberately wrong per-pane marks: a paused paint must not
        // use them.
        let stale = vec![
            LineMark {
                changed: true,
                cells: vec![0..1],
            },
            LineMark::default(),
        ];
        let paused = paint_marks(true, FrameMode::Paused, &stale, &viewed, Some(&prev));
        assert_eq!(paused, changed_marks(Some(&prev), &viewed));
        assert!(!paused[0].changed, "row 0 did not change");
        assert!(paused[1].changed, "row 1 did");
        // Live and live-scrolled take the composed per-pane marks
        // verbatim.
        for mode in [FrameMode::Live, FrameMode::LiveScrolled] {
            assert_eq!(paint_marks(true, mode, &stale, &viewed, Some(&prev)), stale);
        }
        // Plain is unchanged in kind: always the composed-history diff.
        assert_eq!(
            paint_marks(false, FrameMode::Live, &stale, &viewed, Some(&prev)),
            changed_marks(Some(&prev), &viewed)
        );
    }

    #[test]
    fn a_panes_comparand_moves_only_on_its_own_change() {
        // The dwell rule as pure state: an identical tick moves nothing
        // — not the comparand, not the marks, not the last-change
        // stamp.
        let t0 = ago(60);
        let t1 = ago(30);
        let mut r = SourceRuntime::for_test();
        record_output(&mut r, vec!["one".to_string()], t0);
        record_output(&mut r, vec!["two".to_string()], t1);
        assert_eq!(r.previous.as_deref(), Some(&["one".to_string()][..]));
        assert!(r.marks[0].changed);
        assert_eq!(r.changed_at, t1);
        // An identical re-run: nothing moves (the chrome stamp is
        // last-CHANGE, never last-produced).
        record_output(&mut r, vec!["two".to_string()], ago(0));
        assert_eq!(r.changed_at, t1);
        assert!(r.marks[0].changed, "the mark dwells past an unchanged tick");
    }

    #[test]
    fn a_looping_transition_re_dates_the_pane_in_both_directions() {
        // The verdict is decided in the trigger arm, long after
        // `record_output` drained an outcome — so the transition has to
        // re-date the pane from its RETAINED body, with no child
        // completing. The clearing direction is the worse failure of the
        // two: a badge that only appears would sit on an idle pane for
        // as long as the dashboard runs.
        let t0 = ago(60);
        let mut rt = vec![SourceRuntime::for_test()];
        record_output(&mut rt[0], vec!["steady".to_string()], t0);
        rt[0].posted = true;
        let clean = rt[0].hash;

        let on = ago(30);
        assert!(apply_verdict(&mut rt, &[SourceId(0)], true, on));
        assert!(rt[0].looping);
        assert_ne!(rt[0].hash, clean, "the badge moved the pane's hash");
        assert_eq!(rt[0].changed_at, on, "a displayed change re-dates");
        assert_eq!(
            rt[0].output.as_deref(),
            Some(&["steady".to_string()][..]),
            "the retained body, verbatim — nothing re-ran"
        );

        let off = ago(10);
        assert!(apply_verdict(&mut rt, &[], true, off));
        assert!(!rt[0].looping);
        assert_eq!(rt[0].hash, clean, "clearing returns the un-badged hash");
        assert_eq!(rt[0].changed_at, off);

        // A verdict that moves nothing is not a displayed change, and
        // must not re-date anything: a repaint every slice would cost
        // the byte-silence the whole surface is built on.
        assert!(!apply_verdict(&mut rt, &[], true, ago(0)));
        assert_eq!(rt[0].changed_at, off);
    }

    #[test]
    fn a_cycle_marks_every_pane_in_the_set() {
        // The verdict names a SET, so a two-pane cycle marks BOTH of
        // them; a pane outside the set is left entirely alone.
        let t0 = ago(60);
        let mut rt: Vec<SourceRuntime> = (0..3).map(|_| SourceRuntime::for_test()).collect();
        for r in &mut rt {
            record_output(r, vec!["steady".to_string()], t0);
        }
        let at = ago(30);
        assert!(apply_verdict(
            &mut rt,
            &[SourceId(0), SourceId(1)],
            true,
            at
        ));
        assert!(rt[0].looping && rt[1].looping);
        assert!(!rt[2].looping, "a pane outside the set is untouched");
        assert_eq!(rt[2].changed_at, t0, "and is not re-dated");
    }

    #[test]
    fn a_looping_transition_takes_the_same_marks_path_as_a_failure_badge() {
        // The badge goes THROUGH `record_output` rather than around it,
        // so whatever an `· exit N` transition does to a pane's marks,
        // this does too — which is the point: the two badges sit beside
        // each other and must not behave differently. On an unchanged
        // body that is: no gutter mark at all, because marks are diffed
        // over the BODY and the chrome row is not in it.
        let body = vec!["steady".to_string()];
        let t0 = ago(60);
        let at = ago(30);

        let mut failing = SourceRuntime::for_test();
        record_output(&mut failing, body.clone(), t0);
        failing.failure = Some("exit 3".to_string());
        record_output(&mut failing, body.clone(), at);

        let mut rt = vec![SourceRuntime::for_test()];
        record_output(&mut rt[0], body.clone(), t0);
        apply_verdict(&mut rt, &[SourceId(0)], true, at);

        assert_eq!(failing.changed_at, at, "the failure badge re-dates");
        assert_eq!(rt[0].changed_at, at, "and so does the looping badge");
        assert_eq!(rt[0].marks, failing.marks, "one marks path, not two");
        assert!(
            failing.marks.iter().all(|m| !m.changed),
            "an unchanged body marks nothing — asserted, not assumed"
        );
        assert_eq!(rt[0].previous.as_deref(), Some(&body[..]));
    }

    /// Two panes, each triggering on a path — the notice's subject.
    fn two_triggered_panes(a: &str, b: &str) -> Registry {
        use crate::core::box_model::{BorderPreset, Sides};
        use crate::core::registry::{LayoutNode, Overflow, PaneBox, PaneWidth};
        let spec = |id: &str, path: &str| SourceSpec {
            id: id.to_string(),
            program: SourceProgram::Argv(vec!["true".to_string()]),
            shell: ShellMode::Direct,
            interval: None,
            triggers: vec![TriggerSpec::File(std::path::PathBuf::from(path))],
            debounce: Duration::from_millis(250),
            live: false,
        };
        let pane = || PaneBox {
            height: 5,
            width: PaneWidth::Weight(1),
            overflow: Overflow::KeepTop,
            border: BorderPreset::Rounded,
            padding: Sides::default(),
            title: None,
            chrome: true,
        };
        Registry::panes(
            vec![spec("a", a), spec("b", b)],
            vec![pane(), pane()],
            LayoutNode::Row(vec![
                LayoutNode::Pane(SourceId(0)),
                LayoutNode::Pane(SourceId(1)),
            ]),
            1,
            0,
        )
        .expect("a valid two-pane registry")
    }

    /// One evaluation's worth of latch input.
    fn edge(latch: &mut Vec<SourceId>, panes: &[SourceId], abstained: bool) -> bool {
        rising_edge(
            latch,
            &Verdict {
                panes: panes.to_vec(),
                ordered: None,
                abstained,
                why: None,
            },
        )
    }

    #[test]
    fn the_notice_names_every_pane_in_the_set_and_the_watched_paths() {
        // Panes first, then what is suspected, then the evidence — the
        // house shape: name the thing, name the place, show the fix. The
        // paths are what make the claim FALSIFIABLE: a user who knows
        // the accusation is wrong can see why rat thinks otherwise, and
        // carry on, because nothing has been stopped.
        let registry = two_triggered_panes("./sa", "./sb");
        assert_eq!(
            looping_text(&registry, &[SourceId(0), SourceId(1)]),
            "a, b: trigger loop suspected: file:./sa, file:./sb — ? help"
        );
    }

    #[test]
    fn the_notice_claims_no_direction_and_never_repeats_a_shared_path() {
        // No arrow, ever: with children overlapping, direction is not
        // available even as coincidence, so the text reads as a SET. The
        // evidence is a set too — two panes watching ONE path name it
        // once.
        let registry = two_triggered_panes("./same", "./same");
        let text = looping_text(&registry, &[SourceId(0), SourceId(1)]);
        assert_eq!(text, "a, b: trigger loop suspected: file:./same — ? help");
        assert!(!text.contains("->") && !text.contains('→'));
    }

    #[test]
    fn a_plain_watch_names_no_pane_but_still_shows_its_evidence() {
        // A single source triggering on a path its own command writes is
        // a one-pane self-cycle, and `rat watch` can be it. With no pane
        // there is no name to lead with, so the sentence starts at the
        // suspicion rather than at an empty label — the rule `ended_text`
        // already follows.
        let registry = Registry::single(
            SourceSpec {
                id: "watch".to_string(),
                program: SourceProgram::Argv(vec!["true".to_string()]),
                shell: ShellMode::Direct,
                interval: None,
                triggers: vec![TriggerSpec::File(std::path::PathBuf::from("./stamp"))],
                debounce: Duration::from_millis(250),
                live: false,
            },
            None,
        );
        assert_eq!(
            looping_text(&registry, &[SourceId(0)]),
            "trigger loop suspected: file:./stamp — ? help"
        );
    }

    #[test]
    fn the_notice_fires_once_per_rising_edge_and_re_arms_after_clearing() {
        // A latch, not a once-per-process flag. The condition holds for
        // as long as the loop spins, and a row repeated every 50 ms
        // would be useless and a repaint storm — but a dashboard that is
        // repaired and breaks again is news again.
        let mut latch = Vec::new();
        assert!(edge(&mut latch, &[SourceId(0)], false), "the loop began");
        assert!(!edge(&mut latch, &[SourceId(0)], false), "and it holds");
        assert!(
            edge(&mut latch, &[SourceId(0), SourceId(1)], false),
            "a pane the last row did not name IS news — the panes of one \
             cycle cross the threshold a hop apart, and a row naming half \
             a loop is the failure this replaces"
        );
        assert!(
            !edge(&mut latch, &[SourceId(0)], false),
            "but a set that SHRINKS says nothing; the badges show that"
        );
        assert!(
            !edge(&mut latch, &[SourceId(0), SourceId(1)], false),
            "and a pane that drops out and comes back is not news TWICE — \
             membership wobbles while a loop spins, and re-announcing it \
             would be a repaint storm carrying nothing new"
        );
        assert!(!edge(&mut latch, &[], false), "clearing is not news either");
        assert!(
            edge(&mut latch, &[SourceId(0)], false),
            "but a loop that returns after clearing IS a new episode"
        );
    }

    #[test]
    fn an_abstention_holds_the_latch_rather_than_re_arming_it() {
        // Declining to answer is not the same as finding nothing. A busy
        // dashboard abstains, and if that RESET the latch, one unbroken
        // loop would re-announce itself every time the dashboard got
        // busy and then quiet again.
        let mut latch = Vec::new();
        assert!(edge(&mut latch, &[SourceId(0)], false));
        assert!(!edge(&mut latch, &[], true), "abstaining says nothing");
        assert!(
            !edge(&mut latch, &[SourceId(0)], false),
            "still the same loop"
        );
        // And an abstention before anything was suspected cannot arm it.
        let mut fresh = Vec::new();
        assert!(!edge(&mut fresh, &[], true));
        assert!(edge(&mut fresh, &[SourceId(0)], false));
    }

    #[test]
    fn a_repainted_verdict_carries_the_frames_stamp_to_the_panes() {
        // The badge's arrival is the newest change on screen, so the
        // footer names it — exactly as it would have had an outcome
        // carried the change through the drain. A frame left on the old
        // stamp would say the surface has been still since before the
        // badge that just appeared on it.
        let t0 = ago(600);
        let mut rt: Vec<SourceRuntime> = (0..2).map(|_| SourceRuntime::for_test()).collect();
        for r in &mut rt {
            record_output(r, vec!["steady".to_string()], t0);
        }
        let mut live = Live {
            lines: vec!["steady".to_string()],
            hash: 0,
            changed_at: t0,
            since: local_hms(t0),
            panes: None,
            dropped: None,
        };
        let at = ago(5);
        apply_verdict(&mut rt, &[SourceId(1)], true, at);
        restamp_live(&mut live, &rt);
        assert_eq!(live.changed_at, at, "the newest pane change dates it");
        assert_eq!(live.since, local_hms(at));
        assert_eq!(live.hash, combined_hash(&rt), "and the key follows");
    }

    #[test]
    fn a_verdict_with_nothing_to_repaint_still_records_the_state() {
        // Two cases where the badge has no surface: a pane that has not
        // run yet has no retained body, and a plain watch has no chrome
        // row at all. Both record the state — it folds into the hash at
        // the pane's next `record_output` — and neither invents a body
        // nor re-dates a pane nobody can see.
        let mut fresh = vec![SourceRuntime::for_test()];
        assert!(!apply_verdict(&mut fresh, &[SourceId(0)], true, ago(30)));
        assert!(fresh[0].looping);
        assert!(fresh[0].output.is_none(), "no body was invented");

        let t0 = ago(60);
        let mut plain = vec![SourceRuntime::for_test()];
        record_output(&mut plain[0], vec!["steady".to_string()], t0);
        let hash = plain[0].hash;
        assert!(!apply_verdict(&mut plain, &[SourceId(0)], false, ago(30)));
        assert!(plain[0].looping);
        assert_eq!(plain[0].hash, hash, "a plain watch has no badge to paint");
        assert_eq!(plain[0].changed_at, t0);
    }

    #[test]
    fn absolute_stamps_keep_the_age_key_zero() {
        // The default style has no counting row anywhere, so the gate
        // value is 0 and a parked dashboard stays byte-silent —
        // whatever the panes' ages are.
        let footer = ago(600);
        let panes = [ago(600), ago(630)];
        assert_eq!(displayed_age_key(None, None, false, footer, &panes), 0);
        assert_eq!(displayed_age_key(None, None, false, footer, &[]), 0);
        // Flipped, each pane's age is folded in — otherwise the refresh
        // could never reach a pane's chrome row.
        let older = [ago(600), ago(720)];
        assert_ne!(
            displayed_age_key(None, None, true, footer, &panes),
            displayed_age_key(None, None, true, footer, &older)
        );
        // Plain keeps today's number exactly.
        assert_eq!(
            displayed_age_key(None, None, true, footer, &[]),
            displayed_age(None, None, true, footer)
        );
    }

    #[test]
    fn a_piped_frame_sizes_from_the_handed_down_geometry() {
        // A piped dashboard cannot measure a terminal; before falling
        // back to 80x24 it honors the RAT_WIDTH/RAT_HEIGHT its parent
        // handed down — which is what lets a nested `rat dashboard
        // --once` fill its pane instead of rendering at 80 columns and
        // getting chopped.
        assert_eq!(size_fallback(Some("60"), Some("20"), (80, 24)), (60, 20));
        assert_eq!(size_fallback(Some("60"), None, (80, 24)), (60, 24));
        assert_eq!(size_fallback(None, Some("20"), (80, 24)), (80, 20));
        assert_eq!(size_fallback(None, None, (80, 24)), (80, 24));
        // Garbage is ignored, not fatal: the fallback stands.
        assert_eq!(size_fallback(Some("wide"), Some("-3"), (80, 24)), (80, 24));
    }

    fn source_spec(command: &[&str], shell: ShellMode) -> SourceSpec {
        SourceSpec {
            id: String::new(),
            program: SourceProgram::Argv(command.iter().map(|s| s.to_string()).collect()),
            shell,
            interval: Some(Duration::from_secs(2)),
            triggers: Vec::new(),
            debounce: Duration::from_millis(250),
            live: false,
        }
    }

    /// The plain-watch geometry: the terminal size, verbatim.
    fn terminal_geom(cols: u16, rows: u16) -> PaneGeometry {
        PaneGeometry {
            cells: cols,
            rows,
            inner_cols: cols,
            inner_rows: rows,
        }
    }

    // Lossy-String views dodge the OsStr comparison-impl maze: these
    // are ASCII fixtures, so lossy is lossless here.
    fn program_of(cmd: &std::process::Command) -> String {
        cmd.get_program().to_string_lossy().into_owned()
    }

    fn argv_of(cmd: &std::process::Command) -> Vec<String> {
        cmd.get_args()
            .map(|a| a.to_string_lossy().into_owned())
            .collect()
    }

    #[test]
    fn every_child_is_told_the_frame_size_and_appearance() {
        let spec = source_spec(&["some-tool", "--flag"], ShellMode::Direct);
        let cmd =
            build_source_command(&spec, None, true, Appearance::Light, terminal_geom(100, 40));
        let envs: std::collections::HashMap<String, String> = cmd
            .get_envs()
            .filter_map(|(k, v)| {
                Some((
                    k.to_string_lossy().into_owned(),
                    v?.to_string_lossy().into_owned(),
                ))
            })
            .collect();
        assert_eq!(envs.get("RAT_WIDTH").map(String::as_str), Some("100"));
        assert_eq!(envs.get("RAT_HEIGHT").map(String::as_str), Some("40"));
        assert_eq!(
            envs.get("RAT_APPEARANCE").map(String::as_str),
            Some("light")
        );
    }

    #[test]
    fn direct_mode_runs_the_command_verbatim() {
        let spec = source_spec(&["some-tool", "--flag", "value"], ShellMode::Direct);
        let cmd = build_source_command(&spec, None, false, Appearance::Dark, terminal_geom(80, 24));
        assert_eq!(program_of(&cmd), "some-tool");
        assert_eq!(argv_of(&cmd), ["--flag", "value"]);
    }

    #[test]
    fn a_named_shell_runs_that_program() {
        let spec = source_spec(&["echo hi"], ShellMode::Named("fish".to_string()));
        let cmd = build_source_command(&spec, None, false, Appearance::Dark, terminal_geom(80, 24));
        assert_eq!(program_of(&cmd), "fish");
        assert_eq!(argv_of(&cmd), ["-c", "echo hi"]);
    }

    #[test]
    fn the_dialect_table_answers_by_file_name() {
        for sh in [
            "sh",
            "bash",
            "zsh",
            "fish",
            "nu",
            "dash",
            "ksh",
            "/opt/homebrew/bin/fish",
            "/bin/sh",
            // Only `.exe` is stripped: a wrapper script named after a
            // shell keeps `-c`.
            "cmd.sh",
            // Unknown names are not an error — `-c` is what a program
            // invoked with a script takes, and a wrong guess surfaces
            // as the child's own diagnostic.
            "no-such-shell-xyz",
        ] {
            assert_eq!(command_flags(sh), ["-c"], "{sh}");
        }
        for sh in ["cmd", "cmd.exe", "CMD.EXE", "/usr/local/bin/cmd"] {
            assert_eq!(command_flags(sh), ["/C"], "{sh}");
        }
        for sh in ["powershell", "pwsh", "pwsh.exe", "PowerShell.exe"] {
            assert_eq!(command_flags(sh), ["-NoProfile", "-Command"], "{sh}");
        }
    }

    fn shebang_line(interpreter: &str, arg: Option<&str>) -> Shebang {
        Shebang {
            interpreter: interpreter.to_string(),
            arg: arg.map(str::to_string),
        }
    }

    #[test]
    fn an_env_shebang_resolves_the_program_it_names() {
        // env X → X, no args.
        assert_eq!(
            interpreter_invocation(&shebang_line("/usr/bin/env", Some("fish"))),
            ("fish".to_string(), vec![])
        );
        // env -S splits the remainder shell-words-style.
        assert_eq!(
            interpreter_invocation(&shebang_line("/usr/bin/env", Some("-S deno run -A"))),
            (
                "deno".to_string(),
                vec!["run".to_string(), "-A".to_string()]
            )
        );
        // env's real contract: the WHOLE argument is the program name,
        // so `env python3 -u` fails naming "python3 -u" — the same
        // content as unix env's own error, different channel.
        assert_eq!(
            interpreter_invocation(&shebang_line("/usr/bin/env", Some("python3 -u"))),
            ("python3 -u".to_string(), vec![])
        );
        // Malformed -S quoting is a ROUTE, not a panic: the remainder
        // becomes the program verbatim and the spawn error names it.
        assert_eq!(
            interpreter_invocation(&shebang_line(
                "/usr/bin/env",
                Some(r#"-S deno run "unclosed"#)
            )),
            (r#"deno run "unclosed"#.to_string(), vec![])
        );
    }

    #[test]
    fn a_unix_absolute_interpreter_is_reduced_to_its_name() {
        // /bin/bash means nothing on Windows; the NAME is what PATH
        // answers. A Windows-native path survives verbatim.
        assert_eq!(
            interpreter_invocation(&shebang_line("/bin/bash", None)),
            ("bash".to_string(), vec![])
        );
        assert_eq!(
            interpreter_invocation(&shebang_line(r"C:\Py\python.exe", None)),
            (r"C:\Py\python.exe".to_string(), vec![])
        );
    }

    #[test]
    fn the_interpreter_arm_names_the_extension_each_interpreter_insists_on() {
        use ShebangArm::{Interpreter, Kernel};
        // Measured on the winvm: cmd refuses an extensionless file; pwsh
        // refuses anything that is not `.ps1`.
        assert_eq!(script_extension(Interpreter, "cmd"), ".cmd");
        assert_eq!(script_extension(Interpreter, "pwsh"), ".ps1");
        assert_eq!(script_extension(Interpreter, "powershell"), ".ps1");
        assert_eq!(script_extension(Interpreter, "pwsh.exe"), ".ps1");
        for p in ["python3", "node", "sh", "bash"] {
            assert_eq!(script_extension(Interpreter, p), "", "{p}");
        }
        // The kernel arm needs none of it — pwsh included (measured on
        // macOS: an extensionless 0700 `#!/usr/bin/env pwsh` file runs).
        for p in ["cmd", "pwsh", "powershell", "python3"] {
            assert_eq!(script_extension(Kernel, p), "", "{p}");
        }
    }

    #[test]
    fn a_cmd_body_loses_its_shebang_line_only_on_the_interpreter_arm() {
        use ShebangArm::{Interpreter, Kernel};
        let body = "#!cmd\nset /a 6*7";
        // `#` is not batch syntax; cmd would try to run the #! line —
        // and cmd may skip a final line with no newline, so the cmd
        // route also guarantees exactly one trailing newline.
        assert_eq!(script_bytes(Interpreter, "cmd", body), "set /a 6*7\n");
        // Everywhere else the bytes are the AUTHOR'S, verbatim: the
        // kernel READS the #! line, and it is a harmless comment to sh,
        // python and PowerShell. No newline is added or removed.
        assert_eq!(script_bytes(Kernel, "cmd", body), "#!cmd\nset /a 6*7");
        assert_eq!(
            script_bytes(Interpreter, "pwsh", "#!/usr/bin/env pwsh\n1+1"),
            "#!/usr/bin/env pwsh\n1+1"
        );
    }

    #[test]
    fn every_arm_but_interpreter_cmd_writes_the_authors_bytes_verbatim() {
        use ShebangArm::{Interpreter, Kernel};
        // One rewrite exists (Interpreter+cmd); every other cell of the
        // arm × interpreter space is identity — trailing-newline shape
        // included (none added, none removed).
        for (arm, program) in [
            (Kernel, "sh"),
            (Kernel, "cmd"),
            (Kernel, "pwsh"),
            (Interpreter, "sh"),
            (Interpreter, "pwsh"),
            (Interpreter, "python3"),
        ] {
            for body in ["#!/x\necho hi", "#!/x\necho hi\n", "#!/x\necho hi\n\n"] {
                assert_eq!(script_bytes(arm, program, body), body, "{arm:?} {program}");
            }
        }
        // The exception's newline guarantee: exactly one, idempotently.
        assert_eq!(script_bytes(Interpreter, "cmd", "#!cmd\nx\n\n"), "x\n");
        assert_eq!(script_bytes(Interpreter, "cmd", "#!cmd\nx\n"), "x\n");
    }

    #[test]
    fn interpreter_flags_are_not_the_command_flags() {
        // -Command takes an EXPRESSION; a file invocation must not get
        // it. The measured working form is `pwsh -NoProfile <file.ps1>`.
        assert_eq!(interpreter_flags("pwsh"), ["-NoProfile"]);
        assert_eq!(interpreter_flags("powershell"), ["-NoProfile"]);
        assert_eq!(interpreter_flags("cmd"), ["/C"]);
        assert_eq!(interpreter_flags("python3"), [] as [&str; 0]);
        // And the string-table stays what it was.
        assert_eq!(command_flags("pwsh"), ["-NoProfile", "-Command"]);
    }

    #[test]
    fn an_interpreter_command_puts_our_flags_then_the_authors_arg_then_the_path() {
        use ShebangArm::{Interpreter, Kernel};
        let line = shebang_line("/usr/bin/awk", Some("-f"));
        let path = std::path::Path::new("/tmp/rat-script/0-log");
        let cmd = interpreter_command(Interpreter, &line, path);
        assert_eq!(cmd.get_program(), "awk");
        let args: Vec<_> = cmd.get_args().collect();
        assert_eq!(args, ["-f", "/tmp/rat-script/0-log"]);
        // The kernel arm execs the FILE; the kernel does the rest.
        let cmd = interpreter_command(Kernel, &line, path);
        assert_eq!(cmd.get_program(), path.as_os_str());
        assert_eq!(cmd.get_args().count(), 0);
    }

    #[test]
    fn the_spawn_error_program_is_the_arms_own_answer() {
        use ShebangArm::{Interpreter, Kernel};
        // Kernel: execve re-executes the `#!` line's own path, so that
        // is what fails to start. Interpreter: the program we resolved.
        let line = shebang_line("/usr/bin/env", Some("python3"));
        assert_eq!(shebang_program(Kernel, &line), "/usr/bin/env");
        assert_eq!(shebang_program(Interpreter, &line), "python3");
    }

    #[test]
    fn a_script_file_names_its_stem_with_the_arms_extension() {
        use ShebangArm::{Interpreter, Kernel};
        let line = shebang_line("/usr/bin/env", Some("pwsh"));
        let (name, bytes) = script_file(Interpreter, &line, "0-log", "#!/usr/bin/env pwsh\n1+1");
        assert_eq!(name, "0-log.ps1");
        assert_eq!(bytes, "#!/usr/bin/env pwsh\n1+1");
        let (name, _) = script_file(Kernel, &line, "0-log", "#!/usr/bin/env pwsh\n1+1");
        assert_eq!(name, "0-log");
        // The cmd route: extension AND the strip, through the one seam.
        let line = shebang_line("cmd", None);
        let (name, bytes) = script_file(Interpreter, &line, "1-build", "#!cmd\nset /a 6*7");
        assert_eq!(name, "1-build.cmd");
        assert_eq!(bytes, "set /a 6*7\n");
    }

    fn script_spec(body: &str, shell: ShellMode) -> SourceSpec {
        SourceSpec {
            id: String::new(),
            program: SourceProgram::Script(body.to_string()),
            shell,
            interval: Some(Duration::from_secs(2)),
            triggers: Vec::new(),
            debounce: Duration::from_millis(250),
            live: false,
        }
    }

    #[test]
    fn a_shebang_body_runs_the_materialized_file() {
        let spec = script_spec("#!/usr/bin/env fish\necho hi", ShellMode::Platform);
        let path = std::path::Path::new("/tmp/rat-script/0-a");
        let cmd = build_source_command(
            &spec,
            Some(path),
            false,
            Appearance::Dark,
            terminal_geom(80, 24),
        );
        match SHEBANG_ARM {
            // The kernel parses the #! itself.
            ShebangArm::Kernel => assert_eq!(cmd.get_program(), path.as_os_str()),
            // We parsed it: fish from PATH, then the path.
            ShebangArm::Interpreter => assert_eq!(cmd.get_program(), "fish"),
        }
    }

    #[test]
    fn a_body_without_a_shebang_runs_through_the_panes_shell() {
        let spec = script_spec("echo hi", ShellMode::Named("sh".to_string()));
        let cmd = build_source_command(&spec, None, false, Appearance::Dark, terminal_geom(80, 24));
        assert_eq!(cmd.get_program(), "sh");
        assert_eq!(argv_of(&cmd), ["-c", "echo hi"]);
    }

    #[test]
    fn a_spawn_error_under_a_shebang_names_the_interpreter_not_the_tempfile() {
        // Direct descendant of "a spawn error under a shell names the
        // shell, not the script": the tempfile is ours and we just
        // wrote it; what can fail to start is the interpreter.
        let spec = script_spec("#!/usr/bin/env python3\nprint(1)", ShellMode::Platform);
        match SHEBANG_ARM {
            ShebangArm::Kernel => assert_eq!(spawn_program(&spec), "/usr/bin/env"),
            ShebangArm::Interpreter => assert_eq!(spawn_program(&spec), "python3"),
        }
        // The fallback body's failure names the shell.
        let spec = script_spec("echo hi", ShellMode::Named("fish".to_string()));
        assert_eq!(spawn_program(&spec), "fish");
    }

    #[test]
    fn a_dashboard_without_a_script_body_creates_no_temp_directory() {
        // No shebang body, no directory, no syscall — the Default.
        let registry = Registry::single(source_spec(&["true"], ShellMode::Direct), None);
        let scripts = ScriptFiles::materialize(&registry).unwrap();
        assert!(scripts.dir.is_none());
        assert!(scripts.path(SourceId(0)).is_none());
    }

    #[test]
    fn a_shebang_body_is_written_once_where_its_source_can_find_it() {
        let registry =
            Registry::single(script_spec("#!/bin/sh\necho hi", ShellMode::Platform), None);
        let scripts = ScriptFiles::materialize(&registry).unwrap();
        let path = scripts
            .path(SourceId(0))
            .expect("a shebang body has a path");
        // Index-prefixed stem: duplicate pane ids are legal (first-win),
        // so an id-only name would let two panes share one file.
        assert!(
            path.file_name()
                .unwrap()
                .to_str()
                .unwrap()
                .starts_with("0-")
        );
        // The author's bytes, verbatim (sh is not the cmd exception).
        let bytes = std::fs::read_to_string(path).unwrap();
        assert_eq!(bytes, "#!/bin/sh\necho hi");
        // A no-shebang Script body gets NO file — it takes the shell
        // fallback route instead.
        let registry = Registry::single(script_spec("echo hi", ShellMode::Platform), None);
        let scripts = ScriptFiles::materialize(&registry).unwrap();
        assert!(scripts.path(SourceId(0)).is_none());
        assert!(scripts.dir.is_none());
    }

    #[cfg(unix)]
    #[test]
    fn a_script_file_is_private_to_its_owner() {
        // Exists because tempfile's default directory mode is
        // umask-derived, NOT 0700 (verified in 3.27.0: dir mode is set
        // only when permissions are given) — omitting the explicit
        // permission would be silent and invisible in behavior.
        use std::os::unix::fs::PermissionsExt;
        let registry =
            Registry::single(script_spec("#!/bin/sh\necho hi", ShellMode::Platform), None);
        let scripts = ScriptFiles::materialize(&registry).unwrap();
        let path = scripts.path(SourceId(0)).unwrap().to_path_buf();
        let dir = path.parent().unwrap().to_path_buf();
        for p in [&dir, &path] {
            let mode = std::fs::metadata(p).unwrap().permissions().mode() & 0o777;
            assert_eq!(mode, 0o700, "{p:?}");
        }
    }

    #[test]
    fn a_very_long_pane_id_still_materializes() {
        // Ids have no declared length cap, and the file name embeds the
        // id. The INDEX carries uniqueness; the id part is a debugging
        // courtesy and must be bounded, or a legal id overflows the
        // filesystem's NAME_MAX while the same id works fine as a
        // non-materialized body.
        let mut spec = script_spec("#!/bin/sh\necho hi", ShellMode::Platform);
        spec.id = "p".repeat(254);
        let registry = Registry::single(spec, None);
        let scripts = ScriptFiles::materialize(&registry).expect("a long id still materializes");
        let name = scripts
            .path(SourceId(0))
            .expect("a shebang body has a path")
            .file_name()
            .unwrap()
            .to_string_lossy()
            .into_owned();
        assert!(name.len() < 255, "{} bytes: {name}", name.len());
        assert!(name.starts_with("0-"), "{name}");
    }

    #[test]
    fn the_script_directory_leaves_with_the_guard() {
        let registry =
            Registry::single(script_spec("#!/bin/sh\necho hi", ShellMode::Platform), None);
        let scripts = ScriptFiles::materialize(&registry).unwrap();
        let dir = scripts
            .path(SourceId(0))
            .unwrap()
            .parent()
            .unwrap()
            .to_path_buf();
        assert!(dir.exists());
        drop(scripts);
        assert!(!dir.exists());
    }

    #[cfg(windows)]
    #[test]
    fn a_backslash_path_reaches_the_same_table_row() {
        // Backslash is a path separator only on Windows, so this
        // spelling is pinned in the platform arm alone.
        assert_eq!(command_flags(r"C:\WINDOWS\system32\cmd.exe"), ["/C"]);
    }

    #[test]
    fn a_named_cmd_takes_slash_c_on_every_platform() {
        // The table is platform-free: pinning cmd→/C on unix too stops
        // a future #[cfg] creeping in.
        let spec = source_spec(&["set /a 6*7"], ShellMode::Named("cmd".to_string()));
        let cmd = build_source_command(&spec, None, false, Appearance::Dark, terminal_geom(80, 24));
        assert_eq!(argv_of(&cmd), ["/C", "set /a 6*7"]);
    }

    #[test]
    fn the_shell_flag_maps_to_a_mode() {
        assert_eq!(shell_mode(None).unwrap(), ShellMode::Direct);
        assert_eq!(shell_mode(Some(&None)).unwrap(), ShellMode::Platform);
        assert_eq!(
            shell_mode(Some(&Some("fish".to_string()))).unwrap(),
            ShellMode::Named("fish".to_string())
        );
        // Empty and whitespace-only both refuse, naming the problem
        // and the fix.
        for empty in ["", "   "] {
            let err = format!(
                "{:#}",
                shell_mode(Some(&Some(empty.to_string()))).unwrap_err()
            );
            assert!(err.contains("names an empty shell"), "{err}");
            assert!(err.contains("--shell=NAME"), "{err}");
        }
    }

    #[test]
    fn a_spawn_error_names_the_shell_not_the_script() {
        // Under a shell mode `command[0]` is the whole script, so an
        // error that blames it names the wrong thing entirely.
        let direct = source_spec(&["definitely-absent-xyz", "--flag"], ShellMode::Direct);
        assert_eq!(spawn_program(&direct), "definitely-absent-xyz");

        let named = source_spec(&["date; df -h"], ShellMode::Named("fish".to_string()));
        assert_eq!(spawn_program(&named), "fish");

        let platform = source_spec(&["date; df -h"], ShellMode::Platform);
        assert_eq!(spawn_program(&platform), platform_shell());
    }

    #[cfg(unix)]
    #[test]
    fn the_platform_flags_are_dash_c() {
        assert_eq!(platform_flags(), ["-c"]);
    }

    #[cfg(windows)]
    #[test]
    fn the_platform_flags_are_slash_c() {
        assert_eq!(platform_flags(), ["/C"]);
    }

    #[test]
    fn question_mark_pages_the_key_help() {
        for mode in ALL_MODES {
            assert_eq!(action_for(Key::Char('?'), mode), WatchAction::Help);
        }
    }

    #[test]
    fn the_help_names_the_key_families() {
        let text = help_lines("rat watch — keys", &[]).join("\n");
        for needle in [
            "quit",
            "pager",
            "snapshot",
            "freeze the frame in place",
            "resume the live tail",
            "step back",
            "wrap",
            "gutter",
            "highlights",
            "counting ages",
            "key reference",
        ] {
            assert!(text.contains(needle), "help must mention {needle:?}");
        }
    }

    #[test]
    fn shell_mode_goes_through_the_platform_shell() {
        let spec = source_spec(&["echo hi"], ShellMode::Platform);
        let cmd = build_source_command(&spec, None, false, Appearance::Dark, terminal_geom(80, 24));
        #[cfg(unix)]
        {
            assert_eq!(program_of(&cmd), "sh");
            assert_eq!(argv_of(&cmd), ["-c", "echo hi"]);
        }
        #[cfg(windows)]
        {
            let shell = std::env::var("COMSPEC").unwrap_or_else(|_| "cmd".to_string());
            assert_eq!(program_of(&cmd), shell);
            assert_eq!(argv_of(&cmd), ["/C", "echo hi"]);
        }
    }

    /// The drain seam, tested with real readers: the route is unix-only
    /// and a fifo is what it actually reads.
    #[cfg(unix)]
    mod reader_arrivals {
        use super::*;
        use crate::core::trigger::{TriggerKey, WindowLog};
        use crate::term::tap::TriggerReader;

        fn mkfifo(path: &std::path::Path) {
            let cpath =
                std::ffi::CString::new(path.as_os_str().as_encoded_bytes().to_vec()).unwrap();
            assert_eq!(unsafe { libc::mkfifo(cpath.as_ptr(), 0o600) }, 0, "mkfifo");
        }

        fn slot(dir: &std::path::Path, name: &str) -> (ReaderSlot, std::fs::File) {
            let path = dir.join(name);
            mkfifo(&path);
            let spec = TriggerSpec::Fifo(path.clone());
            let reader = TriggerReader::open(&spec, None).unwrap();
            let writer = std::fs::OpenOptions::new().write(true).open(&path).unwrap();
            (
                ReaderSlot {
                    reader,
                    spec,
                    ended_seen: false,
                },
                writer,
            )
        }

        /// Wait until the reader has actually PROVED its descriptor empty at
        /// least once. Sleeping a read slice and assuming it happened is a
        /// wall-clock premise a loaded CI runner does not honour; this waits
        /// for the fact.
        fn wait_for_empty_proof(reader: &TriggerReader) {
            let deadline = Instant::now() + Duration::from_secs(5);
            while Instant::now() < deadline {
                if reader.empty_since_for_test().is_some() {
                    return;
                }
                std::thread::sleep(Duration::from_millis(2));
            }
            panic!("the reader never proved its descriptor empty");
        }

        /// Drain until `n` observations have been recorded, or fail. The
        /// reader is a thread, so every assertion about what it recorded
        /// needs a settle. (`tap.rs` has its own copy for its own module;
        /// `TriggerReader`'s internals are not reachable from here.)
        fn wait_for_observations(
            reader: &TriggerReader,
            n: usize,
        ) -> Vec<crate::core::trigger::Observation> {
            let mut out = Vec::new();
            let deadline = Instant::now() + Duration::from_secs(3);
            while Instant::now() < deadline {
                out.extend(reader.take_arrivals());
                if out.len() >= n {
                    return out;
                }
                std::thread::sleep(Duration::from_micros(200));
            }
            panic!("wanted {n} observations, saw {}", out.len());
        }

        fn poke(slot: &ReaderSlot, writer: &mut std::fs::File) {
            use std::io::Write;
            slot.reader.fired().store(false, Ordering::SeqCst);
            writer.write_all(b"x").unwrap();
            let deadline = Instant::now() + Duration::from_secs(3);
            while Instant::now() < deadline {
                if slot.reader.fired().load(Ordering::SeqCst) {
                    return;
                }
                std::thread::sleep(Duration::from_millis(5));
            }
            panic!("the write never reached the reader");
        }

        #[test]
        fn an_arrival_is_drained_and_resolved_against_the_bracket_log() {
            // The whole seam. The reader supplied an interval; this is
            // where it gains meaning, and WindowLog decides what it
            // means — not the drain.
            let dir = tempfile::tempdir().unwrap();
            let (s, mut w) = slot(dir.path(), "a.fifo");
            let mut log = WindowLog::new(Duration::from_secs(30));
            let key =
                TriggerKey("fifo:".to_string() + &dir.path().join("a.fifo").display().to_string());

            // Wait for a proof of emptiness first. A reader's very first
            // arrival has no lower bound — it has never yet seen its
            // descriptor be not-readable — and an unbounded interval proves
            // nothing in either direction. Vetoing on it is precisely the
            // overclaim this plan removes, so the exogenous case now has a
            // precondition it did not have when an arrival was a bare
            // instant. Waited for, not slept through: the sleep was a
            // scheduling assumption and CI does not honour it.
            wait_for_empty_proof(&s.reader);
            poke(&s, &mut w);
            drain_reader_arrivals(std::slice::from_ref(&s), &mut log, Instant::now());
            let idle = log.arrivals(&key);
            assert_eq!(idle.len(), 1, "the arrival never reached the log");
            assert!(
                log.classify(&idle[0].observation).is_disjoint(),
                "nothing was in flight, so this is EXOGENOUS"
            );

            // Now with a child covering it.
            log.open_bracket(SourceId(0), Instant::now(), Vec::new());
            poke(&s, &mut w);
            drain_reader_arrivals(std::slice::from_ref(&s), &mut log, Instant::now());
            let all = log.arrivals(&key);
            assert_eq!(all.len(), 2);
            assert!(
                !log.classify(&all[1].observation).is_disjoint(),
                "a bracket was in flight, so this is not an outside writer"
            );
        }

        #[test]
        fn draining_arrivals_does_not_disturb_the_fired_flag() {
            // The gate swaps `fired` to decide a respawn. If the drain
            // consumed it, a fire would be lost and the pane would stop
            // refreshing with nothing on screen to say why.
            let dir = tempfile::tempdir().unwrap();
            let (s, mut w) = slot(dir.path(), "a.fifo");
            let mut log = WindowLog::new(Duration::from_secs(30));
            poke(&s, &mut w);
            drain_reader_arrivals(std::slice::from_ref(&s), &mut log, Instant::now());
            assert!(
                s.reader.fired().load(Ordering::SeqCst),
                "the drain cleared the gate's flag"
            );
        }

        #[test]
        fn each_arrival_is_passed_with_its_trigger_identity() {
            // Two fifos on ONE pane must land under different keys, or
            // the per-path stages of the credit rule cannot be applied
            // to this route at all.
            let dir = tempfile::tempdir().unwrap();
            let (a, mut wa) = slot(dir.path(), "a.fifo");
            let (b, mut wb) = slot(dir.path(), "b.fifo");
            let mut log = WindowLog::new(Duration::from_secs(30));
            poke(&a, &mut wa);
            poke(&b, &mut wb);
            poke(&b, &mut wb);
            drain_reader_arrivals(&[a, b], &mut log, Instant::now());
            let ka =
                TriggerKey("fifo:".to_string() + &dir.path().join("a.fifo").display().to_string());
            let kb =
                TriggerKey("fifo:".to_string() + &dir.path().join("b.fifo").display().to_string());
            assert_eq!(log.arrivals(&ka).len(), 1);
            assert_eq!(log.arrivals(&kb).len(), 2);
        }

        #[test]
        fn a_reader_overflow_makes_the_window_abstain() {
            // Missing evidence is not absent evidence — the whole veto
            // is a zero test, so a lost arrival must abstain, not accuse.
            let dir = tempfile::tempdir().unwrap();
            let (s, mut w) = slot(dir.path(), "a.fifo");
            let mut log = WindowLog::new(Duration::from_secs(30));
            let now = Instant::now();
            assert!(!log.evidence_lost(now), "nothing lost yet");
            for _ in 0..(crate::term::tap::ARRIVAL_CAP + 1) {
                poke(&s, &mut w);
            }
            drain_reader_arrivals(std::slice::from_ref(&s), &mut log, now);
            assert!(log.evidence_lost(now), "the overflow never reached the log");
        }

        #[test]
        fn arrivals_are_handed_over_as_timestamps_not_pre_judged() {
            // Eviction is WindowLog's job. This pins that the drain
            // passes the reader's instant through unchanged rather than
            // filtering or re-stamping it: an arrival recorded before
            // the window opened must still ARRIVE, and be dropped by
            // eviction, not silently skipped here.
            let dir = tempfile::tempdir().unwrap();
            let (s, mut w) = slot(dir.path(), "a.fifo");
            let mut log = WindowLog::new(Duration::from_millis(50));
            let key =
                TriggerKey("fifo:".to_string() + &dir.path().join("a.fifo").display().to_string());
            poke(&s, &mut w);
            std::thread::sleep(Duration::from_millis(120));
            let now = Instant::now();
            drain_reader_arrivals(std::slice::from_ref(&s), &mut log, now);
            assert_eq!(log.arrivals(&key).len(), 1, "the drain pre-judged it");
            log.evict(now);
            assert!(
                log.arrivals(&key).is_empty(),
                "eviction, not the drain, is what drops it"
            );
        }

        // ── Observations out (task 2.2) ─────────────────────────────────

        #[test]
        fn the_drain_carries_the_whole_observation_out_of_the_reader() {
            // The interval must survive `take_arrivals`. The LOG does not
            // store it yet — that is 4.1 — so this asserts at the reader
            // boundary, which is what this task owns.
            let dir = tempfile::tempdir().unwrap();
            let (s, mut w) = slot(dir.path(), "f.fifo");
            wait_for_empty_proof(&s.reader);
            poke(&s, &mut w);

            let observations = s.reader.take_arrivals();
            assert_eq!(observations.len(), 1);
            assert!(
                observations[0].empty_since.is_some(),
                "the lower bound must survive — without it every observation \
                 is Ambiguous once 4.1 wires it up, and the route contributes \
                 nothing"
            );
        }

        #[test]
        fn the_window_still_records_exactly_what_it_recorded_before() {
            // BEHAVIOUR-NEUTRALITY, asserted rather than assumed. This task
            // changes a type, not a decision: the log still stores an
            // instant, so the detector's answers must be identical. If this
            // fails, the flip has leaked out of 4.1 and some phase in
            // between will leave the cycle tests failing every run.
            let dir = tempfile::tempdir().unwrap();
            let (s, mut w) = slot(dir.path(), "g.fifo");
            wait_for_empty_proof(&s.reader);
            poke(&s, &mut w);

            let mut log = WindowLog::new(Duration::from_secs(30));
            let slots = vec![s];
            drain_reader_arrivals(&slots, &mut log, Instant::now());

            let key = reader_key(&slots[0].spec);
            assert_eq!(log.arrivals(&key).len(), 1);
        }

        // ── The fence sites (task 3.2) ──────────────────────────────────

        #[test]
        fn a_fence_between_the_bracket_and_the_spawn_makes_the_interval_covered() {
            // I-81's ORDER, tested by what it buys rather than by inspecting
            // call order. Open a bracket, fence, then write — exactly the
            // sequence the spawn step performs — and the observation must
            // classify as the running source's. Fence BEFORE the open and
            // this fails, because the lower bound lands outside the bracket.
            use crate::core::trigger::TemporalCoverage;
            let dir = tempfile::tempdir().unwrap();
            let (s, mut w) = slot(dir.path(), "fenced.fifo");
            wait_for_empty_proof(&s.reader); // an old proof exists

            let mut log = WindowLog::new(Duration::from_secs(30));
            let opened = log.open_bracket(SourceId(1), Instant::now(), Vec::new());
            s.reader.fence();
            // The fence is served; a real spawn costs far more than this.
            std::thread::sleep(Duration::from_millis(2));
            use std::io::Write as _;
            w.write_all(b"x").unwrap();

            let observation = wait_for_observations(&s.reader, 1)[0];
            log.close_bracket(opened, Instant::now(), Vec::new());
            // Match the variant and the contributor, not an exact width: the
            // width is real elapsed time, and asserting it would be
            // asserting the speed of the machine.
            match log.classify(&observation) {
                TemporalCoverage::Covered(contributors) => {
                    assert_eq!(contributors.len(), 1);
                    assert_eq!(contributors[0].0, SourceId(1));
                    assert!(
                        contributors[0].1.is_some(),
                        "the bracket closed, so a width exists"
                    );
                }
                other => panic!(
                    "with the fence inside the bracket the write is attributable, got {other:?}"
                ),
            }
        }

        #[test]
        fn fencing_reaches_every_reader_not_just_the_spawning_source() {
            // A pane's fifo is written by some OTHER pane's child, so the
            // reader needing the tight bound is never the one being spawned.
            // Fencing only the spawning source's readers would leave exactly
            // the reader that matters on its 50ms cadence, and nothing else
            // would notice.
            let dir = tempfile::tempdir().unwrap();
            let (a, _wa) = slot(dir.path(), "a-fence.fifo");
            let (b, _wb) = slot(dir.path(), "b-fence.fifo");
            let slots = vec![a, b];

            fence_all(&slots);

            assert_eq!(slots[0].reader.fences_for_test(), 1);
            assert_eq!(slots[1].reader.fences_for_test(), 1);
        }
    }
}
