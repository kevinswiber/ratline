use std::io::Write;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::{Duration, Instant};

use anyhow::{Context, anyhow};
use crossterm::tty::IsTty;

use crate::cli::WatchArgs;
use crate::color::{ColorProfile, SystemEnv};
use crate::core::child::{ChildSlot, TickOutcome, run_tick, spawn_tick};
use crate::core::duration::parse_interval;
use crate::core::measure::shift_chop;
use crate::core::pager::{PagerCommand, resolve_pagers};
use crate::core::schedule::{Due, TickSchedule};
use crate::core::snapshot::{snapshot_body, snapshot_stamp, write_snapshot};
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
use crate::term::tap::{TapScanner, TtyTap};
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

// The palette follows the terminal only where the reader can see its
// reports; elsewhere it stays the startup verdict for the whole run.
#[cfg_attr(windows, allow(unused_mut))]
pub fn run(args: WatchArgs, profile: ColorProfile, mut palette: Palette) -> AppResult {
    let interval = parse_interval(&args.interval)?;
    let (interrupted, terminated) = register_signals()?;

    let stdout = std::io::stdout();
    let is_tty = stdout.is_tty();
    // Framing only makes sense on a terminal; piped output gets the plain
    // content so `rat watch | tee log` stays readable.
    let mut renderer = InlineRenderer::new(stdout.lock())
        .with_cursor_hidden(is_tty && !args.no_hide_cursor)
        .with_sync_output(is_tty && !args.no_sync)
        .with_clear_screen(is_tty && args.clear);

    // The looping tty mode reads keys (q quits, v pages the full frame), so
    // it owns the terminal input; children must not compete for it.
    let interactive = is_tty && !args.once;
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

    let title_line = args.title.as_ref().map(|title| {
        StyleSpec {
            bold: true,
            ..StyleSpec::default()
        }
        .render(title, profile)
    });
    let faint = StyleSpec {
        faint: true,
        ..StyleSpec::default()
    };

    let mut schedule = TickSchedule::new(interval);
    let slot = ChildSlot::default();
    // Every exit from run() — return, `?`, panic — kills the in-flight
    // child through this guard's Drop. A NAMED binding: `let _ =`
    // would drop it here and now.
    let _shutdown = slot.guard();
    let (tx, rx) = std::sync::mpsc::channel::<TickOutcome>();
    let mut previous_key: Option<PaintKey> = None;
    let mut live: Option<Live> = None;
    let mut pause: Option<PauseState> = None;
    let mut live_scroll: Option<LiveScroll> = None;
    let mut history = History::new();
    let mut view = ViewState {
        wrap: !args.no_wrap,
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
        // 2. Start a child if one is due. At most one is ever in
        // flight, and its environment is measured HERE, at the spawn
        // instant, on this thread.
        if schedule.poll(Instant::now()) == Due::Spawn {
            let size = crossterm::terminal::size().unwrap_or((80, 24));
            // Once mode has no loop to keep responsive: it runs the
            // tick on this thread and posts it to the channel a worker
            // would have used, so both modes share one completion
            // handler. The same detour catches an OS that refuses a
            // thread — that costs a stall, never a tick.
            let mut inline = args.once;
            if !inline {
                let command = build_command(&args, interactive, palette.appearance, size);
                inline = spawn_tick(command, slot.clone(), tx.clone()).is_err();
            }
            if inline {
                let command = build_command(&args, interactive, palette.appearance, size);
                let _ = tx.send(run_tick(command));
            }
        }
        // 3. Collect a finished tick — at most one can be queued.
        if let Ok(outcome) = rx.try_recv() {
            let (stdout, stderr) = match outcome.spawn_error {
                // Once mode fails loudly, before any paint; loop mode
                // renders the failure as content so a transient error
                // does not tear down the dashboard.
                Some(err) if args.once => {
                    return Err(anyhow!("running {:?}: {err}", args.command[0]).into());
                }
                Some(err) => (
                    format!("watch: {:?}: {err}", args.command[0]).into_bytes(),
                    Vec::new(),
                ),
                None => (outcome.stdout, outcome.stderr),
            };
            let mut combined = stdout.clone();
            combined.extend_from_slice(&stderr);
            let hash = signature(&combined);
            // One clock per tick, stamped at COMPLETION on the worker:
            // the absolute stamp, the counting form, and the history
            // entry a scrub later shows all name the instant the
            // content became current — even when the completion waited
            // (behind a pager, say) to be collected.
            let now = outcome.at;
            // The live status row names the ABSOLUTE local time of the
            // last content change: a counting age would change every
            // tick and defeat the repaint gate. Tracked against the
            // tick hash, independent of whether the gate repaints.
            let (changed_at, since) = match live.take() {
                Some(prev) if prev.hash == hash => (prev.changed_at, prev.since),
                _ => (now, local_hms(now)),
            };
            // The terminal size joins the change key: a resize must
            // repaint even when the content is unchanged. So does the
            // appearance: a palette swap must repaint even when the
            // child prints the same bytes.
            let size = crossterm::terminal::size().unwrap_or((80, 24));
            // Composed above the repaint gate: the newest frame is
            // tracked on every completion, so paging always acts on
            // the newest content.
            let current = Live {
                lines: compose_frame(title_line.as_ref(), &stdout, &stderr, is_tty),
                hash,
                changed_at,
                since,
            };
            if is_tty {
                // Every distinct frame is retained (byte-capped,
                // deduped) so the scrub keys can walk back through it.
                history.record(hash, &current.lines, now);
            }
            // A resize while frozen must not leave the window past the frame.
            if let Some(p) = pause.as_mut() {
                let window = usize::from(window_rows(args.max_height, size.1));
                p.scroll = p.scroll.clamp(p.frozen.len(), window);
            }
            // A live window rides the tail whatever shape the frame takes:
            // a pinned window tracks the end, an unpinned one holds its
            // offset clamped into the new shape, and reaching the top
            // collapses to the live view. Freezing is never implicit — the
            // history ring holds any moment that slides away.
            if let Some(ls) = live_scroll {
                let window = usize::from(window_rows(args.max_height, size.1));
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
                        &palette,
                        view,
                        None,
                        size,
                        args.max_height,
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
                    if !stderr.is_empty() {
                        let mut err = std::io::stderr().lock();
                        err.write_all(&stderr).context("writing stderr")?;
                        err.flush().context("flushing stderr")?;
                    }
                }
            }
            live = Some(current);
            if args.once {
                break;
            }
            schedule.completed(Instant::now());
        }
        // 4. How long we may sleep: never past the next spawn, never
        // past one slice, so a signal, a key, and a completing child
        // are all noticed promptly.
        let nap = schedule.nap(Instant::now(), SLICE);
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
                    &palette,
                    view,
                    None,
                    crossterm::terminal::size().unwrap_or((80, 24)),
                    args.max_height,
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
            Some(tap) => match tap.recv_timeout(nap) {
                Some(chunk) => scanner.feed(&chunk),
                None => scanner.idle(nap),
            },
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
                        WatchAction::Page => {
                            let Some(live) = live.as_ref() else { continue };
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
                                page_frame(
                                    pause.as_ref().map_or(&live.lines, |p| &p.frozen),
                                    &mut renderer,
                                )
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
                                &palette,
                                view,
                                pager_notice,
                                size,
                                args.max_height,
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
                            let window = usize::from(window_rows(args.max_height, size.1));
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
                                &palette,
                                view,
                                None,
                                size,
                                args.max_height,
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
                                schedule.request_now();
                            }
                            live_scroll = None;
                            let size = crossterm::terminal::size().unwrap_or((80, 24));
                            previous_key = Some(repaint(
                                &mut renderer,
                                pause.as_ref(),
                                live_scroll,
                                live,
                                &palette,
                                view,
                                None,
                                size,
                                args.max_height,
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
                            let window = usize::from(window_rows(args.max_height, size.1));
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
                                &palette,
                                view,
                                None,
                                size,
                                args.max_height,
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
                            let window = usize::from(window_rows(args.max_height, size.1));
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
                                &palette,
                                view,
                                None,
                                size,
                                args.max_height,
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
                                &palette,
                                view,
                                None,
                                size,
                                args.max_height,
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
                                &args,
                            );
                            let size = crossterm::terminal::size().unwrap_or((80, 24));
                            previous_key = Some(repaint(
                                &mut renderer,
                                pause.as_ref(),
                                live_scroll,
                                live,
                                &palette,
                                view,
                                Some(text),
                                size,
                                args.max_height,
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
                        schedule.request_respawn();
                        if let Some(live) = live.as_ref() {
                            let size = crossterm::terminal::size().unwrap_or((80, 24));
                            previous_key = Some(repaint(
                                &mut renderer,
                                pause.as_ref(),
                                live_scroll,
                                live,
                                &palette,
                                view,
                                debug_notice,
                                size,
                                args.max_height,
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

/// `t` as local wall-clock HH:MM:SS — the `since` stamp's format.
fn local_hms(t: jiff::Timestamp) -> String {
    t.to_zoned(jiff::tz::TimeZone::system())
        .strftime("%H:%M:%S")
        .to_string()
}

/// The live status row: the truncation notice when rows are hidden,
/// carrying the pre-formatted time segment.
fn live_notice(hidden: usize, time_seg: &str) -> String {
    if hidden > 0 {
        format!("… {hidden} more lines · {time_seg} · v views all · q quits")
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
    let time_live = live_time_segment(view.alt_time, &live.since, age_seconds(live.changed_at));
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
fn snapshot_frame(lines: &[String], args: &WatchArgs) -> String {
    let dir = args
        .snapshot_dir
        .clone()
        .or_else(|| std::env::current_dir().ok())
        .unwrap_or_else(|| std::path::PathBuf::from("."));
    let stamp = snapshot_stamp(&jiff::Timestamp::now().to_zoned(jiff::tz::TimeZone::system()));
    let body = snapshot_body(lines, args.snapshot_ansi);
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
/// command never reads the terminal and never sees the palette.
fn build_command(
    args: &WatchArgs,
    interactive: bool,
    appearance: Appearance,
    size: (u16, u16),
) -> std::process::Command {
    let mut command = if args.shell {
        shell_command(&args.command.join(" "))
    } else {
        let mut cmd = std::process::Command::new(&args.command[0]);
        cmd.args(&args.command[1..]);
        cmd
    };
    if interactive {
        command.stdin(std::process::Stdio::null());
    }
    // Children lay out against the frame without a tty side channel: the
    // size is re-measured every tick, so scripts adapt to resizes live.
    let (cols, rows) = size;
    command.env("RAT_WIDTH", cols.to_string());
    command.env("RAT_HEIGHT", rows.to_string());
    // Children inherit the controlling terminal, so a child that resolved its
    // own appearance would query a terminal this process is reading from.
    // Hand it the verdict instead.
    command.env("RAT_APPEARANCE", appearance.as_str());
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
            "… 8 more lines · changed 14s ago · v views all · q quits"
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

    fn watch_args(command: &[&str], shell: bool) -> WatchArgs {
        WatchArgs {
            interval: "2s".to_string(),
            once: false,
            clear: false,
            no_hide_cursor: false,
            no_sync: false,
            shell,
            title: None,
            max_height: None,
            snapshot_dir: None,
            snapshot_ansi: false,
            no_wrap: false,
            command: command.iter().map(|s| s.to_string()).collect(),
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
        let args = watch_args(&["some-tool", "--flag"], false);
        let cmd = build_command(&args, true, Appearance::Light, (100, 40));
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
        let args = watch_args(&["some-tool", "--flag", "value"], false);
        let cmd = build_command(&args, false, Appearance::Dark, (80, 24));
        assert_eq!(program_of(&cmd), "some-tool");
        assert_eq!(argv_of(&cmd), ["--flag", "value"]);
    }

    #[test]
    fn shell_mode_goes_through_the_platform_shell() {
        let args = watch_args(&["echo hi"], true);
        let cmd = build_command(&args, false, Appearance::Dark, (80, 24));
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
