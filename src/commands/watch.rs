//! One loop, N sources: the shared frame engine behind `rat watch`
//! (one source, no box) and — through the same `run_registry` — any
//! registry of sources. The iteration order is law: signals →
//! spawn-if-due → drain-then-compose-once → triggers → nap → age
//! refresh → theme verify → events. The drain terminates because each
//! source has at most one tick in flight; nothing ever paints inside
//! it, and nothing outside this file writes to the terminal.

use std::io::Write;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::{Duration, Instant};

use anyhow::{Context, anyhow};
use crossterm::tty::IsTty;

use crate::cli::WatchArgs;
use crate::color::{ColorProfile, SystemEnv};
use crate::core::child::{ChildSlot, ShutdownGuard, TickOutcome, run_tick, spawn_tick};
use crate::core::duration::parse_interval;
use crate::core::measure::shift_chop;
use crate::core::pager::{PagerCommand, resolve_pagers};
use crate::core::registry::{Composition, PaneGeometry, Registry, SourceId, SourceSpec};
use crate::core::schedule::{Due, TickSchedule};
use crate::core::snapshot::{snapshot_body, snapshot_stamp, write_snapshot};
use crate::core::trigger::{DebounceGate, MtimeWatchSet, TriggerSpec, parse_trigger};
use crate::exit::{AppError, AppResult};
use crate::style_spec::StyleSpec;
use crate::term::history::History;
use crate::term::inline::{InlineRenderer, truncate_to_rows};
use crate::term::marks::{GUTTER_COLS, LineMark, changed_marks, mark_cells, prefix_rows};
use crate::term::scroll::{
    HSHIFT_STEP, LiveScroll, ScrollState, ScrollStep, paused_notice, scrolled_notice,
};
use crate::term::tap::TapEvent;
#[cfg(unix)]
use crate::term::tap::{TapChunk, TapScanner, TriggerReader, TtyTap};
#[cfg(unix)]
use crate::term::theme_notify::{OscColorKind, ThemeNotifyGuard, classify_colors, may_subscribe};
use crate::term::tty::{ConsoleUtf8Guard, RawModeGuard};
use crate::theme::{Appearance, AppearanceSource, Palette};
use crate::ui::key::{Key, from_crossterm};

/// The longest the loop sleeps before it re-checks signals, the
/// channel, and the schedule — one wait slice.
const SLICE: Duration = Duration::from_millis(50);

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
}

/// Everything the loop needs that is not per-source: paint knobs, mode
/// flags, and the pre-built chrome strings each constructor owns.
pub(crate) struct SessionArgs {
    pub once: bool,
    pub clear: bool,
    pub no_hide_cursor: bool,
    pub no_sync: bool,
    pub wrap: bool,
    pub max_height: Option<u16>,
    pub snapshot_dir: Option<std::path::PathBuf>,
    pub snapshot_ansi: bool,
    /// The run-constant footer suffix, pre-built by the constructor.
    pub live_tail: String,
    /// Reflow boxes and respawn every source on a resize; off means the
    /// spawn step owns the geometry re-measure (one writer per mode).
    pub resize_respawn: bool,
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
    let registry = Registry::single(
        SourceSpec {
            name: String::new(),
            command: if args.shell {
                // A shell source keeps the raw script as ONE element,
                // or split-then-join would destroy its quoting.
                vec![args.command.join(" ")]
            } else {
                args.command.clone()
            },
            shell: args.shell,
            interval,
            triggers,
            debounce,
        },
        args.title.clone(),
    );
    let session = SessionArgs {
        once: args.once,
        clear: args.clear,
        no_hide_cursor: args.no_hide_cursor,
        no_sync: args.no_sync,
        wrap: !args.no_wrap,
        max_height: args.max_height,
        snapshot_dir: args.snapshot_dir.clone(),
        snapshot_ansi: args.snapshot_ansi,
        live_tail,
        resize_respawn: false,
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
    // Single-sourced trigger machinery, concatenated in registry order;
    // per-source gates and watch sets come with the pane semantics.
    let all_triggers: Vec<TriggerSpec> = registry
        .ids()
        .flat_map(|id| registry.spec(id).triggers.clone())
        .collect();

    let stdout = std::io::stdout();
    let is_tty = stdout.is_tty();
    // Framing only makes sense on a terminal; piped output gets the plain
    // content so `rat watch | tee log` stays readable.
    let mut renderer = InlineRenderer::new(stdout.lock())
        .with_cursor_hidden(is_tty && !session.no_hide_cursor)
        .with_sync_output(is_tty && !session.no_sync)
        .with_clear_screen(is_tty && session.clear);

    // The looping tty mode reads keys (q quits, v pages the full frame), so
    // it owns the terminal input; children must not compete for it.
    let interactive = is_tty && !session.once;
    // The event-wait routes need a terminal to wake; only the stat-poll
    // works piped, so the other schemes refuse early, before any
    // terminal state changes.
    if !interactive
        && all_triggers
            .iter()
            .any(|trigger| !matches!(trigger, TriggerSpec::File(_)))
    {
        return Err(
            anyhow!("fifo:/fd: triggers need an interactive terminal; use file:PATH").into(),
        );
    }
    let _raw_guard = if interactive {
        Some(RawModeGuard::enable().context("enabling raw mode")?)
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
    let mut theme_sub = may_subscribe(palette.source, profile, interactive && tap.is_some())
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

    let (tx, rx) = std::sync::mpsc::channel::<TickOutcome>();
    let mut runtime: Vec<SourceRuntime> = registry
        .ids()
        .map(|id| SourceRuntime {
            schedule: TickSchedule::new(registry.spec(id).interval),
            slot: ChildSlot::default(),
            tx: tx.clone(),
            output: None,
            hash: 0,
            changed_at: jiff::Timestamp::UNIX_EPOCH,
            posted: false,
        })
        .collect();
    // Every exit from run_registry — return, `?`, panic — kills every
    // in-flight child through these guards' Drop. A NAMED binding:
    // `let _ =` would drop them here and now. One guard per slot, and
    // no registry-level lock: a shared mutex would serialize spawns
    // and could block a shutdown behind one of them.
    let _shutdown: Vec<ShutdownGuard> = runtime.iter().map(|r| r.slot.guard()).collect();
    let mut file_watch = MtimeWatchSet::new(file_paths(&all_triggers));
    // The baseline exists BEFORE the first spawn: a change landing
    // between the first child's start and the loop's first check must
    // be detected, never absorbed into the baseline.
    file_watch.fired();
    let debounce = registry
        .ids()
        .next()
        .map(|id| registry.spec(id).debounce)
        .unwrap_or(SLICE);
    let mut gate = DebounceGate::new(debounce);
    // The fifo/fd reader threads: NAMED and long-lived — dropping this
    // vec joins every reader, so `let _ = …` would kill them here and
    // now. Each slot carries its own end-of-life state for the one-shot
    // notice.
    #[cfg(unix)]
    let mut trigger_readers: Vec<ReaderSlot> = {
        let wake = tap.as_ref().map(TtyTap::sender);
        let mut readers = Vec::new();
        for spec in &all_triggers {
            if matches!(spec, TriggerSpec::File(_)) {
                continue; // polled in the loop, no thread
            }
            if let TriggerSpec::Fd(0) = spec {
                return Err(anyhow!(
                    "fd:0 is the terminal's own input while watch reads keys; \
                     use another descriptor"
                )
                .into());
            }
            readers.push(ReaderSlot {
                reader: TriggerReader::open(spec, wake.clone())?,
                spec: spec.clone(),
                ended_seen: false,
            });
        }
        readers
    };
    let live_tail = session.live_tail.clone();
    // Loop-persistent geometry state: the one size/geometry pair the
    // spawn step, the composer, and the (future) resize arm consume. It
    // outlives the spawn branch — a completion can arrive in an
    // iteration with no new spawn, and the composer still needs an
    // in-scope geometry vector.
    let mut size = crossterm::terminal::size().unwrap_or((80, 24));
    let mut geom = registry.geometry(size);
    let mut previous_key: Option<PaintKey> = None;
    let mut live: Option<Live> = None;
    let mut pause: Option<PauseState> = None;
    let mut live_scroll: Option<LiveScroll> = None;
    let mut history = History::new();
    let mut view = ViewState {
        wrap: session.wrap,
        hshift: 0,
        gutter: false,
        highlight: false,
        alt_time: false,
    };
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
            if !session.resize_respawn {
                size = crossterm::terminal::size().unwrap_or(size);
                geom = registry.geometry(size);
            }
            for id in due {
                // Once mode has no loop to keep responsive: it runs the
                // tick on this thread and posts it to the channel a
                // worker would have used, so both modes share one
                // completion handler. The same detour catches an OS
                // that refuses a thread — that costs a stall, never a
                // tick. It stays a single-source detour: N sources run
                // inline would cost the sum of their runtimes, not the
                // max.
                let mut inline = session.once && plain;
                if !inline {
                    let command =
                        source_command(&registry, id, interactive, palette.appearance, geom[id.0]);
                    inline = spawn_tick(
                        command,
                        id,
                        runtime[id.0].slot.clone(),
                        runtime[id.0].tx.clone(),
                    )
                    .is_err();
                }
                if inline {
                    let command =
                        source_command(&registry, id, interactive, palette.appearance, geom[id.0]);
                    let _ = runtime[id.0].tx.send(run_tick(command, id));
                }
            }
        }
        // 3. Drain EVERY queued completion, then compose ONCE. The
        // drain terminates in at most N iterations: one source can
        // have at most one tick in flight, so it can post at most one
        // outcome per iteration. Only per-source state is touched in
        // here; painting inside this loop would put an intermediate
        // composition on screen — the one thing the iteration order
        // exists to prevent.
        let mut drained: Vec<SourceId> = Vec::new();
        let mut changed: Option<jiff::Timestamp> = None;
        let mut newest = jiff::Timestamp::UNIX_EPOCH;
        let mut piped_stderr: Vec<u8> = Vec::new();
        while let Ok(outcome) = rx.try_recv() {
            let id = outcome.source;
            let (stdout, stderr) = match outcome.spawn_error {
                // Once mode fails loudly, before any paint; loop mode
                // renders the failure as content so a transient error
                // does not tear down the dashboard.
                Some(err) if session.once => {
                    return Err(anyhow!("running {:?}: {err}", registry.spec(id).command[0]).into());
                }
                Some(err) => (
                    format!("watch: {:?}: {err}", registry.spec(id).command[0]).into_bytes(),
                    Vec::new(),
                ),
                None => (outcome.stdout, outcome.stderr),
            };
            let mut combined = stdout.clone();
            combined.extend_from_slice(&stderr);
            let hash = signature(&combined);
            let r = &mut runtime[id.0];
            // A source's own rendered lines. Without panes this IS the
            // whole frame — the shipped composer, shipped arguments,
            // shipped bytes.
            r.output = Some(match registry.composition() {
                Composition::Plain { .. } => {
                    compose_frame(title_line.as_ref(), &stdout, &stderr, is_tty)
                }
                Composition::Panes { .. } => {
                    unreachable!("the composed arm lands with the subcommand")
                }
            });
            // One clock per tick, stamped at COMPLETION on the worker:
            // the absolute stamp, the counting form, and the history
            // entry a scrub later shows all name the instant the
            // content became current — even when the completion waited
            // (behind a pager, say) to be collected. Only a source
            // whose OWN content changed may re-date the frame.
            changed = fold_changed_at(changed, hash != r.hash, outcome.at);
            newest = newest.max(outcome.at);
            r.hash = hash;
            r.changed_at = outcome.at;
            r.posted = true;
            piped_stderr = stderr;
            drained.push(id);
        }
        if !drained.is_empty() {
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
            if !session.resize_respawn {
                size = crossterm::terminal::size().unwrap_or(size);
                geom = registry.geometry(size);
            }
            // Composed once, above the repaint gate: the newest frame
            // is tracked on every completion, so paging always acts on
            // the newest content. Combining the single source's output
            // is the identity — the bytes cannot drift.
            let lines: Vec<String> = match registry.composition() {
                Composition::Plain { .. } => runtime[0].output.clone().unwrap_or_default(),
                Composition::Panes { .. } => {
                    unreachable!("the composed arm lands with the subcommand")
                }
            };
            let current = Live {
                lines,
                hash: content,
                changed_at,
                since,
            };
            if is_tty {
                // Every distinct frame is retained (byte-capped,
                // deduped) so the scrub keys can walk back through it.
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
                displayed_age(
                    pause.as_ref(),
                    live_scroll,
                    view.alt_time,
                    current.changed_at,
                ),
            );
            if previous_key != Some(key) {
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
                        None,
                        size,
                        session.max_height,
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
                    // Piped mode keeps the streams separate for log readability.
                    if !piped_stderr.is_empty() {
                        let mut err = std::io::stderr().lock();
                        err.write_all(&piped_stderr).context("writing stderr")?;
                        err.flush().context("flushing stderr")?;
                    }
                }
            }
            live = Some(current);
            // The deadline is set when a tick COMPOSES, not when it is
            // drained: the fixed delay counts from the frame the reader
            // actually saw.
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
        if !all_triggers.is_empty() {
            let now = Instant::now();
            #[cfg(unix)]
            let mut ended_notices: Vec<String> = Vec::new();
            #[cfg(unix)]
            for slot in &mut trigger_readers {
                if slot.reader.fired().swap(false, Ordering::SeqCst) {
                    gate.fire(now);
                }
                if !slot.ended_seen && slot.reader.ended().load(Ordering::SeqCst) {
                    slot.ended_seen = true; // rising edge, per reader
                    ended_notices.push(format!("trigger ended: {}", slot.spec));
                }
            }
            if file_watch.fired() {
                gate.fire(now);
            }
            if gate.due(now) {
                // A fired trigger observed a change any in-flight child
                // predates: a respawn, never a plain request.
                runtime[0].schedule.request_respawn();
            }
            // Every source that ended this iteration is named in ONE
            // one-shot notice row; with no frame yet the batch drops.
            #[cfg(unix)]
            if let (false, Some(l)) = (ended_notices.is_empty(), live.as_ref()) {
                let notice = ended_notices.join(" · ");
                let size = crossterm::terminal::size().unwrap_or((80, 24));
                previous_key = Some(repaint(
                    &mut renderer,
                    pause.as_ref(),
                    live_scroll,
                    l,
                    &live_tail,
                    &palette,
                    view,
                    Some(notice),
                    size,
                    session.max_height,
                    &faint,
                    profile,
                    &history,
                )?);
            }
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
        if let (Some(prev), Some(l)) = (previous_key, live.as_ref()) {
            let want_age = displayed_age(pause.as_ref(), live_scroll, view.alt_time, l.changed_at);
            if prev.age_secs != want_age {
                previous_key = Some(repaint(
                    &mut renderer,
                    pause.as_ref(),
                    live_scroll,
                    l,
                    &live_tail,
                    &palette,
                    view,
                    None,
                    crossterm::terminal::size().unwrap_or((80, 24)),
                    session.max_height,
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
        for event in events {
            match event {
                TapEvent::Key(key) => {
                    match action_for(key, mode_of(pause.as_ref(), live_scroll)) {
                        WatchAction::Abort => {
                            renderer.finish().context("restoring terminal")?;
                            return Err(AppError::Aborted);
                        }
                        WatchAction::Quit => {
                            renderer.finish().context("restoring terminal")?;
                            return Ok(());
                        }
                        action @ (WatchAction::Page | WatchAction::Help) => {
                            let Some(live) = live.as_ref() else { continue };
                            // ? pages the key reference through the same
                            // ritual v pages the frame — one handoff path,
                            // and search over the bindings comes free.
                            let help;
                            let content: &[String] = if action == WatchAction::Help {
                                help = help_lines(&all_triggers);
                                &help
                            } else {
                                pause.as_ref().map_or(&live.lines, |p| &p.frozen)
                            };
                            // The pager reads the same terminal. Stop the
                            // pushes first, then park our reader, so a report
                            // can never land in a foreign reader's input.
                            #[cfg(unix)]
                            if let Some(sub) = theme_sub.as_mut() {
                                let _ = sub.suspend();
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
                                page_frame(content, &mut renderer)
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
                            // Repaint immediately from the frame on hand:
                            // the content is already current, and a forced
                            // tick would stall the return by a whole child
                            // runtime on a slow dashboard. The pager left the
                            // diff invalidated, so this paints the full frame
                            // over the restored copy.
                            let size = crossterm::terminal::size().unwrap_or((80, 24));
                            previous_key = Some(repaint(
                                &mut renderer,
                                pause.as_ref(),
                                live_scroll,
                                live,
                                &live_tail,
                                &palette,
                                view,
                                pager_notice,
                                size,
                                session.max_height,
                                &faint,
                                profile,
                                &history,
                            )?);
                        }
                        WatchAction::Scroll(step) => {
                            let Some(live) = live.as_ref() else { continue };
                            // The repaint happens here, in place —
                            // re-entering the tick loop would re-run the
                            // child per keypress. A frozen window scrolls
                            // its copy; otherwise scrolling is always a
                            // live viewport — freezing is explicit (p or
                            // <), never a side effect of navigation.
                            let size = crossterm::terminal::size().unwrap_or((80, 24));
                            let window = usize::from(window_rows(session.max_height, size.1));
                            if let Some(p) = pause.as_mut() {
                                p.scroll = p.scroll.step(step, p.frozen.len(), window);
                            } else if let Some(ls) = live_scroll {
                                let stepped = ls.step(step, live.lines.len(), window);
                                live_scroll = (!stepped.at_top()).then_some(stepped);
                            } else {
                                let ls = LiveScroll::start(step, live.lines.len(), window);
                                if ls.at_top() {
                                    // A top-reaching entry never enters
                                    // the mode — including any scroll
                                    // over a frame that fits the window:
                                    // stay Live, nothing to paint.
                                    continue;
                                }
                                live_scroll = Some(ls);
                            }
                            previous_key = Some(repaint(
                                &mut renderer,
                                pause.as_ref(),
                                live_scroll,
                                live,
                                &live_tail,
                                &palette,
                                view,
                                None,
                                size,
                                session.max_height,
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
                                runtime[0].schedule.request_now();
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
                                None,
                                size,
                                session.max_height,
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
                                None,
                                size,
                                session.max_height,
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
                                // At the newest entry this is a no-op,
                                // not a resume: > is a step, not a
                                // homing key.
                                history.next(anchor)
                            };
                            let Some(entry) = entry else { continue };
                            let size = crossterm::terminal::size().unwrap_or((80, 24));
                            let window = usize::from(window_rows(session.max_height, size.1));
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
                                None,
                                size,
                                session.max_height,
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
                            let Some(live) = live.as_ref() else { continue };
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
                            let size = crossterm::terminal::size().unwrap_or((80, 24));
                            previous_key = Some(repaint(
                                &mut renderer,
                                pause.as_ref(),
                                live_scroll,
                                live,
                                &live_tail,
                                &palette,
                                view,
                                None,
                                size,
                                session.max_height,
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
                                Some(text),
                                size,
                                session.max_height,
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
                        runtime[0].schedule.request_respawn();
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
                                debug_notice,
                                size,
                                session.max_height,
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
    ToggleWrap,
    ShiftLeft,
    ShiftRight,
    ToggleGutter,
    ToggleHighlight,
    ToggleTime,
    Ignore,
}

/// The whole binding table, for both input paths and every mode — unix and
/// Windows read their keys differently but mean the same things by them.
/// What a Scroll action does while live is the loop's business, not the
/// table's. View keys never freeze.
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
        Key::Esc | Key::Char('F') if mode != FrameMode::Live => WatchAction::Resume,
        Key::Char('p') if mode != FrameMode::Paused => WatchAction::Freeze,
        Key::Char('<') | Key::Char(',') => WatchAction::ScrubBack,
        Key::Char('>') | Key::Char('.') if mode == FrameMode::Paused => WatchAction::ScrubForward,
        _ => WatchAction::Ignore,
    }
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
    /// Whole seconds since the viewed frame was current; 0 while live.
    /// Advancing once per second is what lets the paused age repaint.
    age_secs: u64,
}

/// The one construction of the repaint gate's key: while paused it holds
/// the freeze-time content/appearance and the window's offset; live it
/// holds this tick's values.
fn paint_key(
    pause: Option<&PauseState>,
    live_scroll: Option<LiveScroll>,
    live_content: u64,
    live_appearance: Appearance,
    size: (u16, u16),
    view: ViewState,
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
/// an absolute stamp or no time at all. ONE style at a time: both rows
/// stamp by default and count only when flipped — a frozen frame must
/// never read differently from the live one beside it. The
/// live-scrolled row carries no time in either state.
fn displayed_age(
    pause: Option<&PauseState>,
    live_scroll: Option<LiveScroll>,
    alt_time: bool,
    changed_at: jiff::Timestamp,
) -> u64 {
    match (alt_time, pause, live_scroll) {
        (true, Some(p), _) => age_seconds(p.viewed_at),
        (true, None, None) => age_seconds(changed_at),
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
/// are learned, plus the configured trigger sources when any exist.
/// Content only — the pager owns presentation.
fn help_lines(triggers: &[TriggerSpec]) -> Vec<String> {
    let mut lines: Vec<String> = [
        "rat watch — keys",
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
        "  >, .               step forward again",
        "",
        "  w                  wrap or chop long lines",
        "  h/l, Left/Right    shift the view horizontally",
        "  D                  toggle the change gutter",
        "  c                  toggle the change highlights",
        "  t                  time style: wall-clock stamps or counting ages",
    ]
    .into_iter()
    .map(str::to_string)
    .collect();
    if !triggers.is_empty() {
        lines.push(String::new());
        lines.push("  refresh triggers:".to_string());
        for spec in triggers {
            lines.push(format!("    {spec}"));
        }
    }
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
/// carrying the pre-formatted time segment.
fn live_notice(hidden: usize, time_seg: &str) -> String {
    if hidden > 0 {
        format!("… {hidden} more lines · {time_seg}")
    } else {
        time_seg.to_string()
    }
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
    notice: Option<String>,
    size: (u16, u16),
    max_height: Option<u16>,
    faint: &StyleSpec,
    profile: ColorProfile,
    history: &History,
) -> anyhow::Result<PaintKey> {
    let age_secs = displayed_age(pause, live_scroll, view.alt_time, live.changed_at);
    let key = paint_key(
        pause,
        live_scroll,
        live.hash,
        palette.appearance,
        size,
        view,
        age_secs,
    );
    let (source, offset, mode) = match (pause, live_scroll) {
        (Some(p), _) => (p.frozen.as_slice(), p.scroll.offset(), FrameMode::Paused),
        (None, Some(ls)) => (live.lines.as_slice(), ls.offset(), FrameMode::LiveScrolled),
        (None, None) => (live.lines.as_slice(), 0, FrameMode::Live),
    };
    // Marks compare the viewed frame against the previous DISTINCT
    // frame — the pause's anchored entry (re-resolved through `nearest`
    // when evicted), else the newest. No predecessor means no marks.
    let marks: Option<Vec<LineMark>> = (view.gutter || view.highlight).then(|| {
        let anchor = match pause {
            Some(p) => p
                .history_seq
                .and_then(|seq| history.nearest(seq).map(|e| e.seq)),
            None => history.newest_seq(),
        };
        let prev = anchor.and_then(|seq| history.prev(seq));
        changed_marks(prev.map(|e| e.frame.as_slice()), source)
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
    let time_live = format!(
        "{}{live_tail}",
        live_time_segment(view.alt_time, &live.since, age_seconds(live.changed_at))
    );
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
        faint,
        profile,
        &time_live,
        &time_paused,
        marks.as_deref(),
        &mark_cell,
    )?;
    Ok(key)
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
    let body = String::from_utf8_lossy(stdout);
    let mut lines: Vec<String> = Vec::new();
    if let Some(title) = title {
        lines.push(title.clone());
    }
    lines.extend(body.trim_end_matches('\n').split('\n').map(str::to_string));
    if join_stderr && !stderr.is_empty() {
        let err_body = String::from_utf8_lossy(stderr);
        lines.extend(
            err_body
                .trim_end_matches('\n')
                .split('\n')
                .map(str::to_string),
        );
    }
    lines
}

/// The single place a tty frame body is painted: truncate the window's
/// slice, append the paused row or the truncation notice, then the
/// one-shot notice row, draw.
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
    faint: &StyleSpec,
    profile: ColorProfile,
    time_live: &str,
    time_paused: &str,
    marks: Option<&[LineMark]>,
    mark_cell: &str,
) -> anyhow::Result<()> {
    let (cols, rows) = size;
    let max_rows = window_rows(max_height, rows);
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
            (true, Some(ms)) => mark_cells(
                line,
                ms.get(start + i).map_or(&[][..], |m| m.cells.as_slice()),
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
    let status = match mode {
        FrameMode::Paused => paused_notice(time_paused, offset, kept.len(), lines.len()),
        FrameMode::LiveScrolled => scrolled_notice(offset, kept.len(), lines.len()),
        FrameMode::Live => live_notice(hidden, time_live),
    };
    kept.push(faint.render(&status, profile));
    if let Some(text) = notice {
        kept.push(faint.render(&text, profile));
    }
    renderer.draw(&kept, cols).context("writing frame")?;
    Ok(())
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
) -> Option<String> {
    let pagers = resolve_pagers(&SystemEnv);
    let mut used = pagers.first().map(|p| p.bin.clone()).unwrap_or_default();
    let _ = crossterm::terminal::disable_raw_mode();
    let _ = renderer.finish();
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
    renderer.resume_over_own_frame();
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
    interactive: bool,
    appearance: Appearance,
    geom: PaneGeometry,
) -> std::process::Command {
    let mut command = if spec.shell {
        // A shell spec holds the raw script as one element, so the
        // join reproduces it byte for byte.
        shell_command(&spec.command.join(" "))
    } else {
        let mut cmd = std::process::Command::new(&spec.command[0]);
        cmd.args(&spec.command[1..]);
        cmd
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

/// `build_source_command` plus the pane identity, which only exists
/// under a declared layout: `Registry::pane` answers `None` without
/// one, so no RAT_PANE is ever exported to a plain watch child.
fn source_command(
    registry: &Registry,
    id: SourceId,
    interactive: bool,
    appearance: Appearance,
    geom: PaneGeometry,
) -> std::process::Command {
    let mut command = build_source_command(registry.spec(id), interactive, appearance, geom);
    if registry.pane(id).is_some() {
        command.env("RAT_PANE", &registry.spec(id).name);
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
    let crossterm::event::Event::Key(key_event) =
        crossterm::event::read().context("reading event")?
    else {
        return Ok(Vec::new());
    };
    match from_crossterm(key_event) {
        Some(key) => Ok(vec![TapEvent::Key(key)]),
        None => Ok(Vec::new()),
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

/// Runtime state per source: the schedule, the slot, and everything the
/// drain updates. The registry itself stays pure — these are the
/// resources it deliberately does not carry.
struct SourceRuntime {
    schedule: TickSchedule,
    slot: ChildSlot,
    tx: std::sync::mpsc::Sender<TickOutcome>,
    /// This source's rendered lines: without panes, the whole frame the
    /// shipped composer produced; with them, the child's own lines
    /// awaiting their box.
    output: Option<Vec<String>>,
    hash: u64,
    changed_at: jiff::Timestamp,
    /// Whether this source has completed at least once — the once-mode
    /// exit condition at N sources.
    posted: bool,
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

#[cfg(unix)]
fn shell_command(script: &str) -> std::process::Command {
    let mut cmd = std::process::Command::new("sh");
    cmd.arg("-c").arg(script);
    cmd
}

#[cfg(windows)]
fn shell_command(script: &str) -> std::process::Command {
    let shell = std::env::var("COMSPEC").unwrap_or_else(|_| "cmd".to_string());
    let mut cmd = std::process::Command::new(shell);
    cmd.arg("/C").arg(script);
    cmd
}

#[cfg(test)]
mod tests {
    use ratatui::style::Color;

    use super::*;

    const ALL_MODES: [FrameMode; 3] = [FrameMode::Live, FrameMode::LiveScrolled, FrameMode::Paused];

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
    fn esc_only_means_something_when_not_live() {
        assert_eq!(action_for(Key::Esc, FrameMode::Live), WatchAction::Ignore);
        assert_eq!(
            action_for(Key::Esc, FrameMode::LiveScrolled),
            WatchAction::Resume
        );
        assert_eq!(action_for(Key::Esc, FrameMode::Paused), WatchAction::Resume);
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
            assert_eq!(action_for(Key::Tab, mode), WatchAction::Ignore);
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
        // a live surface there is nothing newer to step to.
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
        assert_eq!(live_notice(0, "since 18:47:53"), "since 18:47:53");
        assert_eq!(
            live_notice(8, "changed 14s ago"),
            "… 8 more lines · changed 14s ago"
        );
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
        let lines = help_lines(&specs);
        assert!(
            lines
                .iter()
                .any(|line| line.contains("file:/tmp/state.json")),
            "{lines:?}"
        );
        // And stays clean when none are configured.
        assert!(
            !help_lines(&[]).iter().any(|line| line.contains("trigger")),
            "the untriggered reference must not mention triggers"
        );
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
        // The key carries the caller's displayed age verbatim; which
        // surface counts is the caller's business.
        let live = paint_key(None, None, 42, Appearance::Dark, (80, 24), view, 14);
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
        let scrolled = paint_key(None, Some(ls), 42, Appearance::Dark, (80, 24), view, 14);
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
                age_secs: 14,
            }
        );
        let paused = paint_key(Some(&p), None, 42, Appearance::Dark, (80, 24), view, 14);
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
        assert_eq!(
            displayed_age(None, Some(ls), true, old),
            0,
            "the scrolled row has no time"
        );
        assert_eq!(displayed_age(None, Some(ls), false, old), 0);
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
        let (tx, _rx) = std::sync::mpsc::channel::<TickOutcome>();
        SourceRuntime {
            schedule: TickSchedule::new(Some(Duration::from_secs(2))),
            slot: ChildSlot::default(),
            tx,
            output: None,
            hash,
            changed_at: stamp(0),
            posted: false,
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

    fn source_spec(command: &[&str], shell: bool) -> SourceSpec {
        SourceSpec {
            name: String::new(),
            command: command.iter().map(|s| s.to_string()).collect(),
            shell,
            interval: Some(Duration::from_secs(2)),
            triggers: Vec::new(),
            debounce: Duration::from_millis(250),
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
        let spec = source_spec(&["some-tool", "--flag"], false);
        let cmd = build_source_command(&spec, true, Appearance::Light, terminal_geom(100, 40));
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
        let spec = source_spec(&["some-tool", "--flag", "value"], false);
        let cmd = build_source_command(&spec, false, Appearance::Dark, terminal_geom(80, 24));
        assert_eq!(program_of(&cmd), "some-tool");
        assert_eq!(argv_of(&cmd), ["--flag", "value"]);
    }

    #[test]
    fn question_mark_pages_the_key_help() {
        for mode in ALL_MODES {
            assert_eq!(action_for(Key::Char('?'), mode), WatchAction::Help);
        }
    }

    #[test]
    fn the_help_names_the_key_families() {
        let text = help_lines(&[]).join("\n");
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
        let spec = source_spec(&["echo hi"], true);
        let cmd = build_source_command(&spec, false, Appearance::Dark, terminal_geom(80, 24));
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
}
