//! Run one configured watch child to completion — inline or on a
//! worker thread — capturing both streams without deadlock, parking
//! the process in a shutdown-barred slot, and posting exactly one
//! outcome.
//!
//! The outcome carries its source tag and exit status, because one
//! loop drains N sources and a pane names the code its command exited
//! with.
//!
//! The slot's protocol is the heart: the worker checks the shutdown
//! bar and spawns/parks under ONE critical section (the lock spans
//! the spawn), while `shutdown` takes the same lock — so it either
//! kills a parked child or bars a spawn that has not happened yet.
//! There is no window in between.

use std::io::Read;
use std::process::Stdio;
use std::sync::{Arc, Mutex, MutexGuard, PoisonError};

use crate::core::live::{Emissions, Stream};
use crate::core::registry::SourceId;
use crate::core::retain::{Retention, read_all};

/// The slot the tick in flight parks its child in, plus the shutdown
/// bar. Cloning shares the slot.
#[derive(Clone, Default)]
pub struct ChildSlot(Arc<Mutex<SlotState>>);

#[derive(Default)]
struct SlotState {
    child: Option<std::process::Child>,
    shutdown: bool,
}

impl ChildSlot {
    /// Kill whatever is parked and bar any spawn that has not
    /// happened yet. Kill-only, no escalation: this runs on the way
    /// out and must not block. A child that already exited, or one
    /// its worker already reclaimed to reap, is a no-op.
    pub fn shutdown(&self) {
        let mut state = self.lock();
        state.shutdown = true;
        if let Some(child) = state.child.as_mut() {
            let _ = child.kill();
        }
    }

    /// A Drop guard: hold one in `run()` (`let _shutdown =
    /// slot.guard();` — a NAMED binding; a bare `let _ =` drops at
    /// once) so every exit — returns, `?`, panics — shuts the slot
    /// down.
    pub fn guard(&self) -> ShutdownGuard {
        ShutdownGuard(self.clone())
    }

    /// The state holds no invariant a panicking worker can break, so
    /// poisoning is recovered, not propagated.
    fn lock(&self) -> MutexGuard<'_, SlotState> {
        self.0.lock().unwrap_or_else(PoisonError::into_inner)
    }
}

/// Calls `shutdown()` on its slot when dropped.
pub struct ShutdownGuard(ChildSlot);

impl Drop for ShutdownGuard {
    fn drop(&mut self) {
        self.0.shutdown();
    }
}

/// What a source hands the loop.
///
/// Two variants rather than one struct with optional fields, because a
/// progress event has no exit status, no close instant and no close
/// stamps — those describe a COMPLETION. Making them `Option` would let
/// a caller build "progress that exited 3", and would push the question
/// of which fields are meaningful onto every reader.
pub enum TickEvent {
    /// A long-lived source has new content waiting in its outbox.
    ///
    /// Deliberately carries no body: the outbox is latest-wins, so N of
    /// these collapse into one render while the wakes themselves stay
    /// cheap. Which source moved is all the loop needs to go look.
    Progress { source: SourceId },
    /// A child ran to completion — the shipped path, unchanged.
    Completed(TickOutcome),
}

/// One finished tick, as it travels from the worker to the loop.
pub struct TickOutcome {
    /// Which source finished — the index of every per-source resource.
    pub source: SourceId,
    /// The retained lines of each stream, terminators kept, exactly as
    /// the child wrote them. Bytes rather than text because one consumer
    /// writes a child's stderr straight through: decoding here would
    /// replace invalid UTF-8 irreversibly and lose the framing. A
    /// consumer that wants a body concatenates and decodes the whole
    /// stream, which is what the renderers already do.
    pub stdout: Vec<Vec<u8>>,
    pub stderr: Vec<Vec<u8>>,
    /// When the child finished — the instant this content became
    /// current. Read on the worker, because a completion can wait
    /// (behind a pager, say) before the loop composes it.
    pub at: jiff::Timestamp,
    /// Set when the command could not be started at all. The wording
    /// is the caller's: frame content while looping, a hard error
    /// under `--once`.
    pub spawn_error: Option<std::io::Error>,
    /// The watched union, stamped on the WORKER immediately after the child
    /// is reaped — the closing side of its bracket. Empty when the source
    /// watches nothing, which is every source under `--once`.
    ///
    /// Taken here rather than in the loop's drain on purpose: the loop can
    /// sleep up to one slice between the child's exit and the drain, and that
    /// slop widens the attribution window enough to matter.
    pub close_stamps: Vec<(std::path::PathBuf, crate::core::trigger::PathStamp)>,
    /// The monotonic instant the child was reaped — the closing side of its
    /// bracket, taken beside `close_stamps` and for the same reason. The
    /// bracket's WIDTH and its overlap window are both measured from this,
    /// so a drain-time instant would inflate every child's apparent width
    /// and widen who is credited with running over it.
    pub closed_at: std::time::Instant,
    /// The child's exit status, for the per-pane failure row. Absent
    /// when nothing ran: a spawn error, or a child reclaimed by a
    /// shutdown kill — a defaulted `exit 0` would read as a healthy
    /// source that printed nothing. `rat watch` still ignores it,
    /// exactly as `output()`'s status was ignored.
    pub status: Option<std::process::ExitStatus>,
    /// How many LINES this tick discarded to stay inside its bound,
    /// summed across both pipes. Zero for every command whose output
    /// fits, which is nearly all of them, and zero when nothing ran.
    /// Read by the loop, which is what makes a truncation visible
    /// instead of silent.
    pub dropped: usize,
}

/// How a tick's two pipes are drained — the ONLY thing that differs
/// between a batch source and a live one.
///
/// Everything else about running a child is one implementation on
/// purpose: the lock that spans the spawn, the concurrent drain of both
/// pipes, the reap, and the closing stamps. A second copy of that
/// critical section would be a second chance to reopen the window the
/// first one exists to close.
enum Sink {
    /// A private accumulator per pipe, yielded once at EOF — the shipped
    /// path, and the only one a source without `live` ever takes.
    Batch(Retention),
    /// A live source's shared caps and outbox. Each reader feeds them and
    /// wakes the loop as complete lines arrive; the completion path takes
    /// whatever is left.
    Live(Emissions, std::sync::mpsc::Sender<TickEvent>),
}

/// Run one configured child to completion on this thread.
pub fn run_tick(
    command: std::process::Command,
    source: SourceId,
    union: Vec<std::path::PathBuf>,
    retention: Retention,
) -> TickOutcome {
    run_parked(
        command,
        source,
        &ChildSlot::default(),
        &union,
        Sink::Batch(retention),
    )
}

/// Run one configured child on a worker thread, which posts exactly
/// one completion and exits. The handle is dropped on purpose: nothing
/// ever joins a tick. Err only when the OS refuses a thread.
pub fn spawn_tick(
    command: std::process::Command,
    source: SourceId,
    slot: ChildSlot,
    tx: std::sync::mpsc::Sender<TickEvent>,
    union: Vec<std::path::PathBuf>,
    retention: Retention,
) -> std::io::Result<()> {
    std::thread::Builder::new()
        .name("rat-watch-child".into())
        .spawn(move || {
            // On a shutdown race the receiver is already gone; the
            // failed send is the no-op it should be.
            let _ = tx.send(TickEvent::Completed(run_parked(
                command,
                source,
                &slot,
                &union,
                Sink::Batch(retention),
            )));
        })?;
    Ok(())
}

/// Run one LONG-LIVED child on a worker thread: same slot protocol and
/// the same exactly-one-completion contract as `spawn_tick`, except that
/// this one offers its body as it arrives instead of only at EOF.
///
/// No `Retention` parameter, and that absence is the design: the caps
/// live inside the `Emissions` the loop owns, because they must outlive
/// any single read. A child that never exits has no "at EOF".
pub fn spawn_live_tick(
    command: std::process::Command,
    source: SourceId,
    slot: ChildSlot,
    emissions: Emissions,
    tx: std::sync::mpsc::Sender<TickEvent>,
    union: Vec<std::path::PathBuf>,
) -> std::io::Result<()> {
    std::thread::Builder::new()
        .name("rat-watch-child".into())
        .spawn(move || {
            let sink = Sink::Live(emissions, tx.clone());
            // A live child MAY still exit — a follower whose file was
            // rotated, one killed upstream — and when it does the shipped
            // completion path runs unchanged, so the exit badge and the
            // final body are right.
            let _ = tx.send(TickEvent::Completed(run_parked(
                command, source, &slot, &union, sink,
            )));
        })?;
    Ok(())
}

/// Drain one pipe into a live source's shared caps, offering what it has
/// as it goes.
///
/// **Always runs to EOF**, exactly as `read_all` does and for the reason
/// plan 0016 measured rather than reasoned about: an early exit does not
/// hang, it drops the pipe, the child dies of SIGPIPE, and the drop count
/// comes back ZERO while everything past the bound is silently lost.
fn pump_live<R: Read>(
    pipe: Option<R>,
    stream: Stream,
    source: SourceId,
    emissions: &Emissions,
    tx: &std::sync::mpsc::Sender<TickEvent>,
) {
    let Some(mut pipe) = pipe else { return };
    // A read granularity, not a bound. The bound is the cap's.
    let mut buf = [0u8; 8 * 1024];
    loop {
        match pipe.read(&mut buf) {
            Ok(0) => break,
            Ok(n) => {
                // `feed` owns the bound AND the publish decision, because
                // the "did anything visible move?" predicate needs both
                // caps under the lock that mutated them. It answers true
                // only when this call filled an EMPTY outbox, which is
                // exactly when a wake is owed — so a child flooding an
                // unread slot publishes silently.
                if emissions.feed(stream, &buf[..n], jiff::Timestamp::now()) {
                    let _ = tx.send(TickEvent::Progress { source });
                }
            }
            Err(err) if err.kind() == std::io::ErrorKind::Interrupted => continue,
            // A read error yields whatever arrived: a partial frame beats
            // tearing the dashboard down.
            Err(_) => break,
        }
    }
}

fn run_parked(
    mut command: std::process::Command,
    source: SourceId,
    slot: &ChildSlot,
    union: &[std::path::PathBuf],
    sink: Sink,
) -> TickOutcome {
    command.stdout(Stdio::piped()).stderr(Stdio::piped());
    // The lock spans the spawn: shutdown() takes the same lock, so it
    // either kills a parked child or bars this spawn — no window.
    let (stdout, stderr) = {
        let mut state = slot.lock();
        if state.shutdown {
            return not_started(source, std::io::ErrorKind::Interrupted.into());
        }
        let mut child = match command.spawn() {
            Ok(child) => child,
            Err(err) => return not_started(source, err),
        };
        let pipes = (child.stdout.take(), child.stderr.take());
        state.child = Some(child);
        pipes
    };
    // Both pipes drain at once: a child filling both buffers deadlocks
    // a serial reader. The helper failing to spawn drops its pipe, so
    // the child's stderr writes fail fast and the tick still finishes.
    let (out, err, dropped) = match sink {
        Sink::Batch(retention) => {
            let err_reader = std::thread::Builder::new()
                .name("rat-watch-stderr".into())
                .spawn(move || read_all(stderr, retention));
            let (out, out_dropped) = read_all(stdout, retention);
            let (err, err_dropped) =
                err_reader.map_or_else(|_| (Vec::new(), 0), |h| h.join().unwrap_or_default());
            (out, err, out_dropped + err_dropped)
        }
        Sink::Live(emissions, tx) => {
            let (their_emissions, their_tx) = (emissions.clone(), tx.clone());
            let err_reader = std::thread::Builder::new()
                .name("rat-watch-stderr".into())
                .spawn(move || {
                    pump_live(stderr, Stream::Stderr, source, &their_emissions, &their_tx);
                });
            pump_live(stdout, Stream::Stdout, source, &emissions, &tx);
            if let Ok(handle) = err_reader {
                // Joined before the caps are consumed, or the final body
                // could be taken while a reader is still feeding it.
                let _ = handle.join();
            }
            emissions.finish()
        }
    };
    let status = if let Some(mut child) = slot.lock().child.take() {
        // The status is no longer discarded: a pane names the code its
        // command exited with. Nobody FAILS on it — a failing child is
        // still frame content, exactly as `output()`'s status was.
        child.wait().ok()
    } else {
        // Reclaimed by a shutdown kill: there is no status to report.
        None
    };
    // The bracket closes HERE, not in the loop's drain: a change the child
    // made is attributable to it only while the window is still this tight.
    let close_stamps = crate::core::trigger::stamps(union);
    TickOutcome {
        source,
        stdout: out,
        stderr: err,
        at: jiff::Timestamp::now(),
        closed_at: std::time::Instant::now(),
        spawn_error: None,
        status,
        close_stamps,
        // Both pipes are capped separately, so what the tick lost is
        // their sum.
        dropped,
    }
}

/// The outcome of a tick that never ran: a spawn the OS refused, a spawn
/// barred by shutdown, or a worker thread that could not be started.
///
/// Public because a live source has **no inline fallback**. A batch
/// source whose worker thread cannot start runs its child on the loop
/// thread instead; a live child never exits, so doing that would wedge
/// the loop forever. The caller reports the failure rather than becoming
/// it.
pub fn not_started(source: SourceId, err: std::io::Error) -> TickOutcome {
    TickOutcome {
        closed_at: std::time::Instant::now(),
        source,
        stdout: Vec::new(),
        stderr: Vec::new(),
        at: jiff::Timestamp::now(),
        close_stamps: Vec::new(),
        spawn_error: Some(err),
        status: None,
        // Nothing ran, so nothing was read and nothing was discarded.
        dropped: 0,
    }
}

#[cfg(test)]
mod tests {
    use std::io::{Read, Write as _};
    use std::sync::mpsc;
    use std::time::{Duration, Instant};

    use super::*;
    use crate::core::live::Emission;
    use crate::core::retain::{Keep, read_all};

    #[cfg(unix)]
    fn script(body: &str) -> std::process::Command {
        let mut cmd = std::process::Command::new("sh");
        cmd.arg("-c").arg(body);
        cmd
    }

    #[cfg(windows)]
    fn script(body: &str) -> std::process::Command {
        let mut cmd = std::process::Command::new("cmd");
        cmd.arg("/C").arg(body);
        cmd
    }

    /// The long-running fixture whose spawned process IS the one that
    /// must die — spawned DIRECTLY, never through a shell: a shell
    /// that forks instead of execing (dash does) would absorb the
    /// kill while its child kept the pipes open. The same rule puts
    /// ping, not cmd.exe, in the slot on Windows.
    fn sleeper() -> std::process::Command {
        #[cfg(unix)]
        {
            let mut cmd = std::process::Command::new("sleep");
            cmd.arg("30");
            cmd
        }
        #[cfg(windows)]
        {
            let mut cmd = std::process::Command::new("ping");
            cmd.args(["-n", "31", "127.0.0.1"]);
            cmd
        }
    }

    fn parked(slot: &ChildSlot) -> bool {
        slot.lock().child.is_some()
    }

    fn wait_until_parked(slot: &ChildSlot) {
        let deadline = Instant::now() + Duration::from_secs(2);
        while !parked(slot) {
            assert!(Instant::now() < deadline, "the child never parked");
            std::thread::sleep(Duration::from_millis(10));
        }
    }

    fn contains(haystack: &[u8], needle: &[u8]) -> bool {
        needle.len() <= haystack.len() && haystack.windows(needle.len()).any(|w| w == needle)
    }

    /// A bound no fixture in this module comes near, for the tests that
    /// are about something other than the bound. Naming it keeps those
    /// tests from reading as if the number mattered to them.
    fn ample() -> Retention {
        Retention {
            max_lines: 10_000,
            keep: Keep::Bottom,
        }
    }

    /// A child that prints `n` lines and exits: the decimal `i` for `i`
    /// in `0..n`, each with the platform's native terminator, then
    /// exit 0. `n == 0` has no contract — every caller wants output.
    ///
    /// Built on `script`, unlike `sleeper`, and the difference matters
    /// before anyone unifies them: `sleeper` is spawned directly because
    /// a shell that forks instead of execing would absorb the kill while
    /// its child held the pipes open. This one is never killed — it runs
    /// to completion — so the shell is harmless.
    fn flooder(n: usize) -> std::process::Command {
        assert!(
            n > 0,
            "flooder(0) has no contract; the tests all want output"
        );
        #[cfg(unix)]
        {
            // A POSIX shell counter rather than `seq`: `seq` is present
            // on macOS and the runners, but it is not POSIX and the
            // shell can count.
            script(&format!(
                "i=0; while [ $i -lt {n} ]; do echo $i; i=$((i+1)); done"
            ))
        }
        #[cfg(windows)]
        {
            script(&format!("for /l %i in (0,1,{}) do @echo %i", n - 1))
        }
    }

    /// Path to the rat binary — the only fixture the live tests can use.
    /// They need a child that prints and then DOES NOT EXIT, and the one
    /// tool that would give them that for free is `tail -f`, which the
    /// Windows leg does not have. So the follower is rat itself.
    fn rat_bin() -> std::path::PathBuf {
        assert_cmd::cargo::cargo_bin("rat")
    }

    /// A temp dir holding `log`, seeded with `contents`. The dir comes
    /// back with it because dropping the dir deletes the file.
    fn seeded_log(contents: &str) -> (tempfile::TempDir, std::path::PathBuf) {
        let dir = tempfile::tempdir().expect("tempdir");
        let log = dir.path().join("log");
        std::fs::write(&log, contents).expect("seed the log");
        (dir, log)
    }

    /// `rat __follow <log>`: print what is there, then keep running and
    /// print whatever is appended.
    fn follow_cmd(log: &std::path::Path) -> std::process::Command {
        let mut cmd = std::process::Command::new(rat_bin());
        cmd.arg("__follow").arg(log);
        cmd
    }

    /// A log seeded with `n` lines, followed — a child that floods and
    /// then stays alive, which is the shape a bound has to survive.
    fn flood_then_stay_alive(n: usize) -> (tempfile::TempDir, std::process::Command) {
        let body: String = (0..n).map(|i| format!("{i}\n")).collect();
        let (dir, log) = seeded_log(&body);
        let cmd = follow_cmd(&log);
        (dir, cmd)
    }

    /// Poll the outbox until a body satisfies `ready`, or fail.
    ///
    /// Non-matching bodies are DISCARDED on purpose: the outbox is
    /// latest-wins, so taking one and waiting for the next is exactly
    /// what the loop does, and a body that has not caught up yet holds
    /// nothing a later one lacks.
    fn wait_for_body_where(
        slot: &Emissions,
        within: Duration,
        ready: impl Fn(&Emission) -> bool,
    ) -> Emission {
        let deadline = Instant::now() + within;
        let mut seen: Vec<(usize, usize, usize)> = Vec::new();
        loop {
            if let Some(body) = slot.take() {
                if ready(&body) {
                    return body;
                }
                seen.push((body.stdout.len(), body.stderr.len(), body.dropped));
            }
            assert!(
                Instant::now() < deadline,
                "no body satisfied the predicate; \
                 saw (stdout lines, stderr lines, dropped): {seen:?}"
            );
            std::thread::sleep(Duration::from_millis(10));
        }
    }

    fn wait_for_body(slot: &Emissions, within: Duration) -> Emission {
        wait_for_body_where(slot, within, |_| true)
    }

    /// Every event a channel yields until its senders are gone.
    ///
    /// Sound only for a child that EXITS: the worker's exit drops the
    /// last sender and ends this. A follower would sit here for `within`.
    fn collect_until_closed(rx: &mpsc::Receiver<TickEvent>, within: Duration) -> Vec<TickEvent> {
        let mut events = Vec::new();
        while let Ok(event) = rx.recv_timeout(within) {
            events.push(event);
        }
        events
    }

    /// The line's text, with its platform terminator removed.
    ///
    /// Production bytes are untouched — this normalizes the ASSERTION,
    /// never the payload. Unix `echo` ends a line `\n` and `cmd`'s ends
    /// it `\r\n`, and the accumulator keeps the `\r` deliberately, so a
    /// hardcoded `b"99\n"` would pass here and fail the Windows leg.
    fn line_text(line: &[u8]) -> &str {
        std::str::from_utf8(line)
            .expect("the fixture emits ASCII")
            .trim_end_matches('\n')
            .trim_end_matches('\r')
    }

    /// A pipe that delivers once and then breaks, for the rule that a
    /// partial frame beats tearing the dashboard down.
    struct BreaksAfterOneRead<'a> {
        first: &'a [u8],
        delivered: bool,
    }

    impl Read for BreaksAfterOneRead<'_> {
        fn read(&mut self, buf: &mut [u8]) -> std::io::Result<usize> {
            if self.delivered {
                return Err(std::io::Error::other("the pipe broke"));
            }
            self.delivered = true;
            let n = self.first.len().min(buf.len());
            buf[..n].copy_from_slice(&self.first[..n]);
            Ok(n)
        }
    }

    #[test]
    fn the_outcome_reports_what_it_dropped() {
        let outcome = run_tick(
            flooder(100),
            SourceId(0),
            Vec::new(),
            Retention {
                max_lines: 10,
                keep: Keep::Bottom,
            },
        );
        assert_eq!(outcome.dropped, 90);
        // The fixture's contract: the last line it printed is `99`.
        assert_eq!(line_text(outcome.stdout.last().unwrap()), "99");
    }

    #[test]
    fn a_child_that_outruns_its_cap_still_exits_and_posts() {
        // Far more output than any pipe buffer holds: this is the test
        // that catches a reader which stops at its bound.
        //
        // FALSIFIED, and the result is worth recording because it is not
        // what one would predict. Giving the read loop an early exit
        // once the bound fills does NOT hang here — `read_all` drops the
        // pipe as it returns, closing the read end before anyone waits,
        // so the child dies of SIGPIPE (measured: unix wait status 13)
        // rather than blocking in `write`. What actually goes wrong is
        // quieter: `dropped` came back 0 while some 199,950 lines were
        // lost, so the count lies about the loss, and the exit is no
        // longer clean. Two of the three assertions below fire, in
        // 0.02 s.
        //
        // The rule stands whatever the symptom: never stop draining.
        // Just do not expect a hang to be how you find out.
        let outcome = run_tick(
            flooder(200_000),
            SourceId(0),
            Vec::new(),
            Retention {
                max_lines: 50,
                keep: Keep::Bottom,
            },
        );
        assert_eq!(outcome.stdout.len(), 50);
        assert!(outcome.dropped >= 199_950);
        assert!(
            outcome.status.is_some_and(|status| status.success()),
            "the child never exited cleanly"
        );
    }

    #[test]
    fn the_retained_window_is_the_tail_under_keep_bottom() {
        // Not incidental: keeping the head would silently defeat a pane
        // that declared keep-bottom, which is the mode people reach for
        // with exactly the commands that provoke this.
        let outcome = run_tick(
            flooder(1_000),
            SourceId(0),
            Vec::new(),
            Retention {
                max_lines: 3,
                keep: Keep::Bottom,
            },
        );
        assert_eq!(line_text(outcome.stdout.last().unwrap()), "999");
        assert_eq!(line_text(outcome.stdout.first().unwrap()), "997");
    }

    #[test]
    fn a_command_inside_its_bound_reports_zero_dropped() {
        // The common case, and the one that must stay boring.
        let outcome = run_tick(
            flooder(3),
            SourceId(0),
            Vec::new(),
            Retention {
                max_lines: 100,
                keep: Keep::Top,
            },
        );
        assert_eq!(outcome.dropped, 0);
        assert_eq!(outcome.stdout.len(), 3);
    }

    #[test]
    fn a_spawn_error_reports_no_drops() {
        // The outcome for a command that never started is built without
        // reading a pipe at all, so the new field has to be zero there
        // by construction rather than by accident.
        let outcome = run_tick(
            std::process::Command::new("definitely-no-such-binary-xyz"),
            SourceId(0),
            Vec::new(),
            Retention {
                max_lines: 10,
                keep: Keep::Bottom,
            },
        );
        assert!(outcome.spawn_error.is_some());
        assert_eq!(outcome.dropped, 0);
    }

    #[test]
    fn a_pipe_longer_than_the_bound_yields_only_the_bound() {
        let body: Vec<u8> = (0..1000)
            .flat_map(|i| format!("line{i}\n").into_bytes())
            .collect();
        let (lines, dropped) = read_all(
            Some(&body[..]),
            Retention {
                max_lines: 10,
                keep: Keep::Bottom,
            },
        );
        assert_eq!(lines.len(), 10);
        assert_eq!(dropped, 990);
        assert_eq!(lines[9], b"line999\n");
    }

    #[test]
    fn a_pipe_under_the_bound_is_byte_identical_to_today() {
        // The witness for every command whose output fits, which is
        // nearly all of them.
        let body = b"alpha\nbeta\n".to_vec();
        let (lines, dropped) = read_all(
            Some(&body[..]),
            Retention {
                max_lines: 100,
                keep: Keep::Top,
            },
        );
        assert_eq!(
            lines.concat(),
            body,
            "an under-cap stream must round-trip byte-for-byte"
        );
        assert_eq!(dropped, 0);
    }

    #[test]
    fn an_absent_pipe_is_empty_and_drops_nothing() {
        let (lines, dropped) = read_all(
            None::<&[u8]>,
            Retention {
                max_lines: 10,
                keep: Keep::Top,
            },
        );
        assert!(lines.is_empty());
        assert_eq!(dropped, 0);
    }

    #[test]
    fn an_under_cap_stream_round_trips_invalid_utf8_and_its_exact_framing() {
        // The reason the payload is bytes. The plain path writes a
        // child's stderr straight through, so any decode here is an
        // irreversible change to what the user sees. Both halves matter:
        // the invalid byte, and the ABSENT trailing newline.
        let body = b"warn: \xff\nno trailing newline".to_vec();
        let (lines, dropped) = read_all(
            Some(&body[..]),
            Retention {
                max_lines: 100,
                keep: Keep::Bottom,
            },
        );
        assert_eq!(lines.concat(), body);
        assert_eq!(dropped, 0);
    }

    #[test]
    fn empty_input_retains_nothing_and_drops_nothing() {
        // Pairs with the rendering regression beside `output_lines`:
        // EMPTY here must still render as ONE empty line there.
        let (lines, dropped) = read_all(
            Some(&b""[..]),
            Retention {
                max_lines: 10,
                keep: Keep::Top,
            },
        );
        assert!(lines.is_empty());
        assert_eq!(dropped, 0);
    }

    #[test]
    fn trailing_blank_lines_survive_as_bytes() {
        let body = b"a\n\n\n".to_vec();
        let (lines, _) = read_all(
            Some(&body[..]),
            Retention {
                max_lines: 10,
                keep: Keep::Bottom,
            },
        );
        assert_eq!(
            lines.concat(),
            body,
            "the bytes survive; collapsing is the renderer's job"
        );
    }

    #[test]
    fn a_read_error_yields_what_arrived_rather_than_nothing() {
        // The shipped rule, preserved: a partial frame beats tearing the
        // dashboard down. The bytes that arrived before the break are
        // kept, including the line the break left unterminated.
        let (lines, dropped) = read_all(
            Some(BreaksAfterOneRead {
                first: b"a\nb",
                delivered: false,
            }),
            Retention {
                max_lines: 10,
                keep: Keep::Bottom,
            },
        );
        assert_eq!(lines.concat(), b"a\nb");
        assert_eq!(dropped, 0);
    }

    #[test]
    fn the_outcome_carries_post_exit_stamps_for_the_watched_union() {
        // The bracket's closing side. Stamped on the worker, immediately
        // after the child is reaped, because the loop can sleep up to a slice
        // before it drains — and that slop measurably costs discrimination.
        let dir = tempfile::tempdir().unwrap();
        let f = dir.path().join("sa");
        std::fs::write(&f, b"0").unwrap();

        let outcome = run_tick(script("echo hi"), SourceId(0), vec![f.clone()], ample());
        assert_eq!(outcome.close_stamps.len(), 1);
        assert_eq!(outcome.close_stamps[0].0, f);
    }

    #[test]
    fn the_bracket_closes_on_the_worker_not_at_the_drain() {
        // The closing STAMPS were already taken here; the closing INSTANT
        // was not, and the bracket's width and overlap window are measured
        // from it. The loop can sleep up to a slice before it drains, so a
        // drain-time instant inflates every child's apparent width and
        // widens who is credited with overlapping it — the same slop the
        // stamps are taken here to avoid.
        let dir = tempfile::tempdir().unwrap();
        let f = dir.path().join("sa");
        std::fs::write(&f, b"0").unwrap();

        let before = std::time::Instant::now();
        let outcome = run_tick(script("echo hi"), SourceId(0), vec![f.clone()], ample());
        let drained = std::time::Instant::now();

        assert!(
            outcome.closed_at >= before,
            "the child cannot have finished before it started"
        );
        assert!(
            outcome.closed_at <= drained,
            "and it must be stamped by the worker, before the caller sees it"
        );
    }

    #[test]
    fn a_childs_own_write_is_visible_in_the_closing_stamps() {
        // What makes the bracket attributable at all: the child writes a
        // watched path, and the stamp taken after it exits differs from one
        // taken before.
        let dir = tempfile::tempdir().unwrap();
        let f = dir.path().join("sa");
        std::fs::write(&f, b"0").unwrap();
        let before = crate::core::trigger::stamps(std::slice::from_ref(&f));

        #[cfg(unix)]
        let cmd = script(&format!("printf 1 >> {}", f.display()));
        #[cfg(windows)]
        let cmd = script(&format!("echo 1 >> {}", f.display()));
        let outcome = run_tick(cmd, SourceId(0), vec![f.clone()], ample());

        assert_ne!(
            outcome.close_stamps, before,
            "the child's write must be visible after it exits"
        );
    }

    #[test]
    fn an_empty_union_costs_no_stats_and_returns_empty() {
        // The common case, and every source under --once, where no trigger is
        // armed at all.
        let outcome = run_tick(script("echo hi"), SourceId(0), Vec::new(), ample());
        assert!(outcome.close_stamps.is_empty());
    }

    #[test]
    fn a_spawn_error_still_returns_well_formed_closing_stamps() {
        let outcome = run_tick(
            std::process::Command::new("definitely-not-a-program-here"),
            SourceId(0),
            vec![std::path::PathBuf::from("/sa")],
            ample(),
        );
        assert!(outcome.spawn_error.is_some());
        assert!(outcome.close_stamps.is_empty());
    }

    #[test]
    fn a_tick_captures_both_streams_separately() {
        #[cfg(unix)]
        let cmd = script("echo out; echo err >&2");
        #[cfg(windows)]
        let cmd = script("echo out & echo err 1>&2");
        let outcome = run_tick(cmd, SourceId(0), Vec::new(), ample());
        assert!(outcome.spawn_error.is_none());
        assert!(contains(&outcome.stdout.concat(), b"out"));
        assert!(contains(&outcome.stderr.concat(), b"err"));
        assert!(!contains(&outcome.stdout.concat(), b"err"));
    }

    #[cfg(unix)]
    #[test]
    fn a_tick_that_floods_both_pipes_still_finishes() {
        // 300 KB to EACH stream — far past any pipe buffer. A serial
        // drain deadlocks on exactly this child; the concurrent drain
        // is what this test justifies.
        let line = "x".repeat(100);
        let body = format!(
            "i=0; while [ $i -lt 3000 ]; do echo {line}; echo {line} >&2; i=$((i+1)); done"
        );
        let outcome = run_tick(
            script(&body),
            SourceId(0),
            Vec::new(),
            Retention {
                max_lines: 1000,
                keep: Keep::Bottom,
            },
        );
        assert!(outcome.spawn_error.is_none());
        // 3000 lines arrive on each stream and the retained window holds
        // the bound. What this test is about is unchanged — the tick
        // FINISHES, which is what the concurrent drain buys — but the
        // whole 300 KB is no longer what comes back, so asserting the
        // total would now be asserting the absence of the cap.
        assert_eq!(outcome.stdout.len(), 1000);
        assert_eq!(outcome.stderr.len(), 1000);
        assert!(outcome.stdout.iter().all(|line| line.len() == 101));
        assert!(outcome.stderr.iter().all(|line| line.len() == 101));
    }

    #[test]
    fn a_command_that_cannot_start_reports_the_error() {
        let outcome = run_tick(
            std::process::Command::new("definitely-no-such-binary-xyz"),
            SourceId(0),
            Vec::new(),
            ample(),
        );
        assert!(outcome.spawn_error.is_some());
        assert!(outcome.stdout.is_empty());
        assert!(outcome.stderr.is_empty());
    }

    #[test]
    fn a_nonzero_exit_is_still_an_outcome() {
        #[cfg(unix)]
        let cmd = script("echo hi; exit 3");
        #[cfg(windows)]
        let cmd = script("echo hi & exit 3");
        let outcome = run_tick(cmd, SourceId(0), Vec::new(), ample());
        assert!(outcome.spawn_error.is_none());
        assert!(contains(&outcome.stdout.concat(), b"hi"));
    }

    #[test]
    fn a_worker_posts_exactly_one_outcome() {
        let (tx, rx) = mpsc::channel();
        // The only Sender moves in, so the worker's exit closes the
        // channel: one outcome, then Disconnected — proven together.
        spawn_tick(
            script("echo once"),
            SourceId(0),
            ChildSlot::default(),
            tx,
            Vec::new(),
            ample(),
        )
        .expect("spawn worker");
        let TickEvent::Completed(outcome) = rx
            .recv_timeout(Duration::from_secs(5))
            .expect("one outcome")
        else {
            panic!("a batch tick posts a completion");
        };
        assert!(contains(&outcome.stdout.concat(), b"once"));
        assert!(rx.recv_timeout(Duration::from_secs(5)).is_err());
    }

    #[test]
    fn a_parked_child_can_be_killed_from_another_thread() {
        let slot = ChildSlot::default();
        let (tx, rx) = mpsc::channel();
        spawn_tick(
            sleeper(),
            SourceId(0),
            slot.clone(),
            tx,
            Vec::new(),
            ample(),
        )
        .expect("spawn worker");
        wait_until_parked(&slot);
        slot.shutdown();
        // The kill closed the pipes, the drains hit EOF, the worker
        // reaped and posted. Without the kill this waits 30 s.
        assert!(rx.recv_timeout(Duration::from_secs(5)).is_ok());
    }

    #[test]
    fn a_shutdown_before_the_spawn_prevents_the_child() {
        // The race pin — the reason the lock spans the spawn. Fully
        // deterministic: the bar is set before the runner ever runs.
        let dir = tempfile::tempdir().expect("tempdir");
        let marker = dir.path().join("marker");
        #[cfg(unix)]
        let cmd = script(&format!(": > {}", marker.display()));
        #[cfg(windows)]
        let cmd = script(&format!("type nul > {}", marker.display()));
        let slot = ChildSlot::default();
        slot.shutdown();
        let outcome = run_parked(cmd, SourceId(0), &slot, &[], Sink::Batch(ample()));
        let err = outcome.spawn_error.expect("barred spawn reports an error");
        assert_eq!(err.kind(), std::io::ErrorKind::Interrupted);
        assert!(!marker.exists(), "the child must never have spawned");
    }

    #[test]
    fn dropping_the_guard_shuts_the_slot_down() {
        let slot = ChildSlot::default();
        let (tx, rx) = mpsc::channel();
        spawn_tick(
            sleeper(),
            SourceId(0),
            slot.clone(),
            tx,
            Vec::new(),
            ample(),
        )
        .expect("spawn worker");
        wait_until_parked(&slot);
        // The RAII half: a guard going out of scope is the shutdown.
        // (Which is why run() must HOLD its guard in a named binding —
        // a bare `let _ =` drops immediately, as exploited here.)
        drop(slot.guard());
        assert!(rx.recv_timeout(Duration::from_secs(5)).is_ok());
    }

    #[test]
    fn an_outcome_carries_its_source_tag() {
        // The tag every per-source resource is indexed by: it rides the
        // outcome home, so a drain never has to guess who finished.
        // Both paths carry it — the inline one and the worker's.
        let outcome = run_tick(script("echo tagged"), SourceId(2), Vec::new(), ample());
        assert_eq!(outcome.source, SourceId(2));

        let (tx, rx) = mpsc::channel();
        spawn_tick(
            script("echo tagged"),
            SourceId(5),
            ChildSlot::default(),
            tx,
            Vec::new(),
            ample(),
        )
        .expect("spawn worker");
        let TickEvent::Completed(posted) = rx
            .recv_timeout(Duration::from_secs(5))
            .expect("one outcome")
        else {
            panic!("a batch tick posts a completion");
        };
        assert_eq!(posted.source, SourceId(5));
    }

    #[test]
    fn a_nonzero_exit_is_reported_in_the_outcome() {
        // A failing source needs its CODE, not just its output: the code
        // is what a pane's status row names, and what tells a failure
        // apart from a command that legitimately prints nothing.
        #[cfg(unix)]
        let cmd = script("echo hi; exit 3");
        #[cfg(windows)]
        let cmd = script("echo hi & exit 3");
        let outcome = run_tick(cmd, SourceId(0), Vec::new(), ample());
        assert!(outcome.spawn_error.is_none());
        assert!(contains(&outcome.stdout.concat(), b"hi"));
        assert_eq!(outcome.status.and_then(|status| status.code()), Some(3));
    }

    #[test]
    fn a_spawn_error_still_has_no_status() {
        // Nothing ran, so nothing exited. A defaulted `exit 0` here would
        // read as a healthy source that printed nothing.
        let outcome = run_tick(
            std::process::Command::new("definitely-no-such-binary-xyz"),
            SourceId(0),
            Vec::new(),
            ample(),
        );
        assert!(outcome.spawn_error.is_some());
        assert!(outcome.status.is_none());
    }

    #[test]
    fn a_batch_tick_posts_exactly_one_completed_event() {
        // The shipped contract restated against the new type: one tick,
        // one completion, and NO progress event at all. The second half
        // is the one that keeps earning its keep once a live worker
        // starts publishing progress beside it.
        let (tx, rx) = mpsc::channel();
        spawn_tick(
            flooder(1),
            SourceId(0),
            ChildSlot::default(),
            tx,
            Vec::new(),
            ample(),
        )
        .expect("spawn worker");
        // The only Sender moved into the worker, so this ends at
        // Disconnected the moment the worker exits — every event this
        // tick will ever post is in hand by then.
        let mut events = Vec::new();
        while let Ok(event) = rx.recv_timeout(Duration::from_secs(5)) {
            events.push(event);
        }
        assert_eq!(events.len(), 1);
        assert!(matches!(events[0], TickEvent::Completed(_)));
    }

    #[test]
    fn a_completed_event_still_carries_everything_the_loop_reads() {
        // TickOutcome is UNCHANGED: the enum WRAPPED it rather than
        // reshaping it. Asserted over the worker's route, since that is
        // the one whose type moved.
        #[cfg(unix)]
        let cmd = script("echo hi; exit 3");
        #[cfg(windows)]
        let cmd = script("echo hi & exit 3");
        let (tx, rx) = mpsc::channel();
        spawn_tick(
            cmd,
            SourceId(4),
            ChildSlot::default(),
            tx,
            Vec::new(),
            ample(),
        )
        .expect("spawn worker");
        let TickEvent::Completed(outcome) =
            rx.recv_timeout(Duration::from_secs(5)).expect("one event")
        else {
            panic!("a batch tick posts a completion");
        };
        assert_eq!(outcome.source, SourceId(4));
        assert!(contains(&outcome.stdout.concat(), b"hi"));
        assert_eq!(outcome.status.and_then(|status| status.code()), Some(3));
    }

    #[test]
    fn a_live_source_publishes_before_its_child_exits() {
        // THE WHOLE POINT. The child prints and then stays alive, so a
        // pass cannot come from it having exited — which is exactly how
        // main manages to render nothing at all.
        let (_dir, log) = seeded_log("early\n");
        let slot = Emissions::new(ample(), ample());
        let child = ChildSlot::default();
        let _shutdown = child.guard();
        let (tx, rx) = mpsc::channel();
        spawn_live_tick(
            follow_cmd(&log),
            SourceId(0),
            child.clone(),
            slot.clone(),
            tx,
            Vec::new(),
        )
        .expect("spawn worker");

        let body = wait_for_body(&slot, Duration::from_secs(5));
        assert_eq!(body.stdout, vec![b"early\n".to_vec()]);
        assert!(body.stderr.is_empty());
        // The loop must be WOKEN, not left to poll. Read after the body
        // on purpose: `feed` fills the outbox and only then does the
        // worker send, so a `try_recv` here would race that send.
        assert!(
            matches!(
                rx.recv_timeout(Duration::from_secs(5)),
                Ok(TickEvent::Progress {
                    source: SourceId(0)
                })
            ),
            "a published body must come with a wake"
        );
    }

    #[test]
    fn appending_to_a_followed_file_publishes_again() {
        // Following, not just a first read: the second body must arrive
        // without the child exiting and without the loop asking.
        let (_dir, log) = seeded_log("first\n");
        let slot = Emissions::new(ample(), ample());
        let child = ChildSlot::default();
        let _shutdown = child.guard();
        let (tx, _rx) = mpsc::channel();
        spawn_live_tick(
            follow_cmd(&log),
            SourceId(0),
            child.clone(),
            slot.clone(),
            tx,
            Vec::new(),
        )
        .expect("spawn worker");

        // Order matters: wait for the FIRST body before appending, or one
        // read of a two-line file would satisfy this.
        let first = wait_for_body(&slot, Duration::from_secs(5));
        assert_eq!(first.stdout, vec![b"first\n".to_vec()]);
        std::fs::OpenOptions::new()
            .append(true)
            .open(&log)
            .expect("reopen the log")
            .write_all(b"second\n")
            .expect("append");
        let next =
            wait_for_body_where(&slot, Duration::from_secs(5), |body| body.stdout.len() == 2);
        // A SNAPSHOT, not a delta: the whole retained body every time,
        // which is what lets the shipped compose path take it unchanged.
        assert_eq!(next.stdout, vec![b"first\n".to_vec(), b"second\n".to_vec()]);
    }

    #[test]
    fn a_live_child_that_does_exit_still_posts_exactly_one_completion() {
        // `tail -f` on a rotated file, a follower killed upstream: a live
        // child MAY exit, and when it does the shipped completion path
        // must still run, so the exit badge and the final body are right.
        let child = ChildSlot::default();
        let _shutdown = child.guard();
        let (tx, rx) = mpsc::channel();
        spawn_live_tick(
            flooder(3),
            SourceId(0),
            child.clone(),
            Emissions::new(ample(), ample()),
            tx,
            Vec::new(),
        )
        .expect("spawn worker");
        let events = collect_until_closed(&rx, Duration::from_secs(5));
        assert_eq!(
            events
                .iter()
                .filter(|e| matches!(e, TickEvent::Completed(_)))
                .count(),
            1,
            "exactly one completion, however many wakes preceded it"
        );
        let Some(TickEvent::Completed(outcome)) = events.into_iter().next_back() else {
            panic!("the completion must come LAST — the body it carries is final");
        };
        assert_eq!(outcome.stdout.len(), 3, "the final body is the whole body");
    }

    #[test]
    fn a_batch_source_publishes_nothing_and_completes_once() {
        // The byte-identity witness at the unit level: a source without
        // `live` must not touch an outbox even when one exists.
        let slot = Emissions::new(ample(), ample());
        let (tx, rx) = mpsc::channel();
        spawn_tick(
            flooder(3),
            SourceId(0),
            ChildSlot::default(),
            tx,
            Vec::new(),
            ample(),
        )
        .expect("spawn worker");
        let events = collect_until_closed(&rx, Duration::from_secs(5));
        assert!(slot.take().is_none(), "a batch source must never publish");
        assert_eq!(events.len(), 1);
        assert!(matches!(events[0], TickEvent::Completed(_)));
    }

    #[test]
    fn a_flooding_live_child_does_not_grow_the_wake_queue() {
        // The wake gate from the PRODUCING side. Bounding the body alone
        // would leave the channel growing one event per read, and the
        // loop drains that channel until empty — so the starvation the
        // outbox exists to prevent would arrive through the wakes.
        let (_dir, cmd) = flood_then_stay_alive(5_000);
        let slot = Emissions::new(ample(), ample());
        let child = ChildSlot::default();
        let _shutdown = child.guard();
        let (tx, rx) = mpsc::channel();
        spawn_live_tick(cmd, SourceId(0), child.clone(), slot, tx, Vec::new())
            .expect("spawn worker");
        std::thread::sleep(Duration::from_millis(800));
        // Nobody has taken a body, so at most ONE wake may be queued.
        let wakes = rx
            .try_iter()
            .filter(|e| matches!(e, TickEvent::Progress { .. }))
            .count();
        assert!(wakes <= 1, "{wakes} wakes queued behind one unread body");
    }

    #[test]
    fn both_pipes_reach_the_slot_and_stay_apart() {
        // Each pipe is captured and bounded separately, so they are two
        // ROUTES: a fixture that only ever writes stdout leaves half the
        // live path unexercised.
        let dir = tempfile::tempdir().expect("tempdir");
        let (out, err) = (dir.path().join("out"), dir.path().join("err"));
        std::fs::write(&out, "to-stdout\n").expect("seed stdout");
        std::fs::write(&err, "to-stderr\n").expect("seed stderr");
        let mut cmd = follow_cmd(&out);
        cmd.arg("--stderr-file").arg(&err);

        let slot = Emissions::new(ample(), ample());
        let child = ChildSlot::default();
        let _shutdown = child.guard();
        let (tx, _rx) = mpsc::channel();
        spawn_live_tick(
            cmd,
            SourceId(0),
            child.clone(),
            slot.clone(),
            tx,
            Vec::new(),
        )
        .expect("spawn worker");
        let body = wait_for_body_where(&slot, Duration::from_secs(5), |body| {
            !body.stdout.is_empty() && !body.stderr.is_empty()
        });
        assert_eq!(body.stdout, vec![b"to-stdout\n".to_vec()]);
        assert_eq!(body.stderr, vec![b"to-stderr\n".to_vec()]);
    }

    #[test]
    fn the_bound_still_applies_to_a_live_source() {
        // A follower is the shape most likely to flood — never stopping
        // is the whole point of it — so it is the last place to skip a
        // bound.
        let (_dir, cmd) = flood_then_stay_alive(100);
        let slot = Emissions::new(
            Retention {
                max_lines: 10,
                keep: Keep::Bottom,
            },
            ample(),
        );
        let child = ChildSlot::default();
        let _shutdown = child.guard();
        let (tx, _rx) = mpsc::channel();
        spawn_live_tick(
            cmd,
            SourceId(0),
            child.clone(),
            slot.clone(),
            tx,
            Vec::new(),
        )
        .expect("spawn worker");
        let body = wait_for_body_where(&slot, Duration::from_secs(5), |body| body.dropped > 0);
        assert!(
            body.stdout.len() <= 10,
            "the cap must hold on the live path too: {} lines",
            body.stdout.len()
        );
        assert_eq!(
            line_text(body.stdout.last().expect("a retained line")),
            "99",
            "keep-bottom retains the NEWEST, and a follower's newest is what a pane wants"
        );
    }

    #[test]
    fn run_tick_still_returns_a_bare_outcome() {
        // The inline path is UNTOUCHED: `--once` and every existing
        // caller read the outcome directly, and wrapping it for the
        // channel is the caller's business, not this signature's. The
        // type annotation IS the assertion.
        let outcome: TickOutcome = run_tick(flooder(1), SourceId(0), Vec::new(), ample());
        assert!(outcome.spawn_error.is_none());
    }
}
