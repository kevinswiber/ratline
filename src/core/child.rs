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

use crate::core::registry::SourceId;

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

/// One finished tick, as it travels from the worker to the loop.
pub struct TickOutcome {
    /// Which source finished — the index of every per-source resource.
    pub source: SourceId,
    pub stdout: Vec<u8>,
    pub stderr: Vec<u8>,
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
    /// Staged: the loop's drain reads this in the next task, which removes
    /// the allow. A bin crate counts a field nobody in the binary reads.
    #[allow(dead_code)]
    pub close_stamps: Vec<(std::path::PathBuf, crate::core::trigger::PathStamp)>,
    /// The child's exit status, for the per-pane failure row. Absent
    /// when nothing ran: a spawn error, or a child reclaimed by a
    /// shutdown kill — a defaulted `exit 0` would read as a healthy
    /// source that printed nothing. `rat watch` still ignores it,
    /// exactly as `output()`'s status was ignored.
    pub status: Option<std::process::ExitStatus>,
}

/// Run one configured child to completion on this thread.
pub fn run_tick(
    command: std::process::Command,
    source: SourceId,
    union: Vec<std::path::PathBuf>,
) -> TickOutcome {
    run_parked(command, source, &ChildSlot::default(), &union)
}

/// Run one configured child on a worker thread, which posts exactly
/// one outcome and exits. The handle is dropped on purpose: nothing
/// ever joins a tick. Err only when the OS refuses a thread.
pub fn spawn_tick(
    command: std::process::Command,
    source: SourceId,
    slot: ChildSlot,
    tx: std::sync::mpsc::Sender<TickOutcome>,
    union: Vec<std::path::PathBuf>,
) -> std::io::Result<()> {
    std::thread::Builder::new()
        .name("rat-watch-child".into())
        .spawn(move || {
            // On a shutdown race the receiver is already gone; the
            // failed send is the no-op it should be.
            let _ = tx.send(run_parked(command, source, &slot, &union));
        })?;
    Ok(())
}

fn run_parked(
    mut command: std::process::Command,
    source: SourceId,
    slot: &ChildSlot,
    union: &[std::path::PathBuf],
) -> TickOutcome {
    command.stdout(Stdio::piped()).stderr(Stdio::piped());
    // The lock spans the spawn: shutdown() takes the same lock, so it
    // either kills a parked child or bars this spawn — no window.
    let (stdout, stderr) = {
        let mut state = slot.lock();
        if state.shutdown {
            return outcome_err(source, std::io::ErrorKind::Interrupted.into());
        }
        let mut child = match command.spawn() {
            Ok(child) => child,
            Err(err) => return outcome_err(source, err),
        };
        let pipes = (child.stdout.take(), child.stderr.take());
        state.child = Some(child);
        pipes
    };
    // Both pipes drain at once: a child filling both buffers deadlocks
    // a serial reader. The helper failing to spawn drops its pipe, so
    // the child's stderr writes fail fast and the tick still finishes.
    let err_reader = std::thread::Builder::new()
        .name("rat-watch-stderr".into())
        .spawn(move || read_all(stderr));
    let out = read_all(stdout);
    let err = err_reader.map_or_else(|_| Vec::new(), |h| h.join().unwrap_or_default());
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
        spawn_error: None,
        status,
        close_stamps,
    }
}

fn outcome_err(source: SourceId, err: std::io::Error) -> TickOutcome {
    TickOutcome {
        source,
        stdout: Vec::new(),
        stderr: Vec::new(),
        at: jiff::Timestamp::now(),
        close_stamps: Vec::new(),
        spawn_error: Some(err),
        status: None,
    }
}

fn read_all<R: Read>(pipe: Option<R>) -> Vec<u8> {
    let mut buf = Vec::new();
    if let Some(mut pipe) = pipe {
        // A read error yields whatever arrived: a partial frame beats
        // tearing the dashboard down.
        let _ = pipe.read_to_end(&mut buf);
    }
    buf
}

#[cfg(test)]
mod tests {
    use std::sync::mpsc;
    use std::time::{Duration, Instant};

    use super::*;

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

    #[test]
    fn the_outcome_carries_post_exit_stamps_for_the_watched_union() {
        // The bracket's closing side. Stamped on the worker, immediately
        // after the child is reaped, because the loop can sleep up to a slice
        // before it drains — and that slop measurably costs discrimination.
        let dir = tempfile::tempdir().unwrap();
        let f = dir.path().join("sa");
        std::fs::write(&f, b"0").unwrap();

        let outcome = run_tick(script("echo hi"), SourceId(0), vec![f.clone()]);
        assert_eq!(outcome.close_stamps.len(), 1);
        assert_eq!(outcome.close_stamps[0].0, f);
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
        let outcome = run_tick(cmd, SourceId(0), vec![f.clone()]);

        assert_ne!(
            outcome.close_stamps, before,
            "the child's write must be visible after it exits"
        );
    }

    #[test]
    fn an_empty_union_costs_no_stats_and_returns_empty() {
        // The common case, and every source under --once, where no trigger is
        // armed at all.
        let outcome = run_tick(script("echo hi"), SourceId(0), Vec::new());
        assert!(outcome.close_stamps.is_empty());
    }

    #[test]
    fn a_spawn_error_still_returns_well_formed_closing_stamps() {
        let outcome = run_tick(
            std::process::Command::new("definitely-not-a-program-here"),
            SourceId(0),
            vec![std::path::PathBuf::from("/sa")],
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
        let outcome = run_tick(cmd, SourceId(0), Vec::new());
        assert!(outcome.spawn_error.is_none());
        assert!(contains(&outcome.stdout, b"out"));
        assert!(contains(&outcome.stderr, b"err"));
        assert!(!contains(&outcome.stdout, b"err"));
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
        let outcome = run_tick(script(&body), SourceId(0), Vec::new());
        assert!(outcome.spawn_error.is_none());
        assert_eq!(outcome.stdout.len(), 3000 * 101);
        assert_eq!(outcome.stderr.len(), 3000 * 101);
    }

    #[test]
    fn a_command_that_cannot_start_reports_the_error() {
        let outcome = run_tick(
            std::process::Command::new("definitely-no-such-binary-xyz"),
            SourceId(0),
            Vec::new(),
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
        let outcome = run_tick(cmd, SourceId(0), Vec::new());
        assert!(outcome.spawn_error.is_none());
        assert!(contains(&outcome.stdout, b"hi"));
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
        )
        .expect("spawn worker");
        let outcome = rx
            .recv_timeout(Duration::from_secs(5))
            .expect("one outcome");
        assert!(contains(&outcome.stdout, b"once"));
        assert!(rx.recv_timeout(Duration::from_secs(5)).is_err());
    }

    #[test]
    fn a_parked_child_can_be_killed_from_another_thread() {
        let slot = ChildSlot::default();
        let (tx, rx) = mpsc::channel();
        spawn_tick(sleeper(), SourceId(0), slot.clone(), tx, Vec::new()).expect("spawn worker");
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
        let outcome = run_parked(cmd, SourceId(0), &slot, &[]);
        let err = outcome.spawn_error.expect("barred spawn reports an error");
        assert_eq!(err.kind(), std::io::ErrorKind::Interrupted);
        assert!(!marker.exists(), "the child must never have spawned");
    }

    #[test]
    fn dropping_the_guard_shuts_the_slot_down() {
        let slot = ChildSlot::default();
        let (tx, rx) = mpsc::channel();
        spawn_tick(sleeper(), SourceId(0), slot.clone(), tx, Vec::new()).expect("spawn worker");
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
        let outcome = run_tick(script("echo tagged"), SourceId(2), Vec::new());
        assert_eq!(outcome.source, SourceId(2));

        let (tx, rx) = mpsc::channel();
        spawn_tick(
            script("echo tagged"),
            SourceId(5),
            ChildSlot::default(),
            tx,
            Vec::new(),
        )
        .expect("spawn worker");
        let posted = rx
            .recv_timeout(Duration::from_secs(5))
            .expect("one outcome");
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
        let outcome = run_tick(cmd, SourceId(0), Vec::new());
        assert!(outcome.spawn_error.is_none());
        assert!(contains(&outcome.stdout, b"hi"));
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
        );
        assert!(outcome.spawn_error.is_some());
        assert!(outcome.status.is_none());
    }
}
