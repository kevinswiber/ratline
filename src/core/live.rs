//! The handoff between a live source's worker and the loop.
//!
//! One outbox slot, latest wins, no queue — which is the whole design.
//! The loop's termination argument says a source contributes at most one
//! body per iteration; a queue of emissions is precisely what would
//! break it, and a fast child would then starve the loop it is trying to
//! draw on.
//!
//! **The wake is gated the same way, and that half is easy to miss.**
//! `feed` returns true only when it fills an empty outbox, so while a
//! body sits unread a flooding child publishes silently. Bounding the
//! body alone would leave the channel growing one wake per read, and the
//! loop drains that channel until empty — so the starvation the outbox
//! exists to prevent would simply arrive through the wakes instead.
//!
//! Both caps live behind ONE mutex because the two reader threads feed
//! them concurrently (draining the pipes serially deadlocks) and a
//! published body must be an atomic view of both: a pane composes all
//! retained stdout followed by all retained stderr.

use std::sync::{Arc, Mutex, MutexGuard, PoisonError};

use crate::core::retain::{LineCap, Retention};

/// One cap from one policy — the single place a `Retention` becomes a
/// `LineCap` here, so a re-armed cap cannot drift from a fresh one.
fn cap(policy: Retention) -> LineCap {
    LineCap::new(policy.max_lines, policy.keep)
}

/// Which pipe a reader is feeding.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Stream {
    Stdout,
    Stderr,
}

/// One body a live source is offering the loop.
///
/// Two streams kept apart: the pane composes retained stdout then
/// retained stderr, each under its own bound. One merged collection
/// would interleave nondeterministically and change the arithmetic.
#[derive(Debug, Eq, PartialEq)]
pub struct Emission {
    pub stdout: Vec<Vec<u8>>,
    pub stderr: Vec<Vec<u8>>,
    /// Lines the bound discarded, summed across both streams — the badge
    /// is per pane, not per pipe.
    pub dropped: usize,
    /// When this body became current.
    pub at: jiff::Timestamp,
}

struct State {
    /// The policy each cap is rebuilt from. Kept because an `Emissions`
    /// OUTLIVES the child that fills it — the loop owns one per source
    /// and the scheduler may respawn — so `finish` has to hand back a
    /// fresh cap, not a spent one.
    policy: (Retention, Retention),
    stdout: LineCap,
    stderr: LineCap,
    pending: Option<Emission>,
}

impl State {
    /// The shape a reader could see: how many lines are retained on each
    /// stream, and how many have been dropped. Cheap — no cloning.
    fn shape(&self) -> (usize, usize, usize, usize) {
        (
            self.stdout.retained_len(),
            self.stdout.dropped_count(),
            self.stderr.retained_len(),
            self.stderr.dropped_count(),
        )
    }
}

/// A live source's bounded buffers and its one-slot outbox.
/// Cloning shares them, exactly as `ChildSlot` does.
#[derive(Clone)]
pub struct Emissions(Arc<Mutex<State>>);

impl Emissions {
    pub fn new(stdout: Retention, stderr: Retention) -> Emissions {
        Emissions(Arc::new(Mutex::new(State {
            policy: (stdout, stderr),
            stdout: cap(stdout),
            stderr: cap(stderr),
            pending: None,
        })))
    }

    /// Feed one chunk, and publish the whole body if the visible shape
    /// moved.
    ///
    /// Returns `true` ONLY when this call filled an empty outbox — which
    /// is exactly when the worker should send a wake. While a body sits
    /// unread, later feeds update it in place and return `false`.
    ///
    /// The publish decision lives here rather than in the worker because
    /// the "did it move?" predicate needs both caps under the same lock
    /// that mutated them; leaving it to the caller would race the two
    /// readers against each other and duplicate the rule.
    pub fn feed(&self, stream: Stream, chunk: &[u8], at: jiff::Timestamp) -> bool {
        let mut state = self.lock();
        let before = state.shape();
        match stream {
            Stream::Stdout => state.stdout.feed(chunk),
            Stream::Stderr => state.stderr.feed(chunk),
        }
        if state.shape() == before {
            // A chunk that landed mid-line changes nothing a reader can
            // see; waking for it would spend a compose on identical bytes.
            return false;
        }
        let (stdout, out_dropped) = state.stdout.snapshot();
        let (stderr, err_dropped) = state.stderr.snapshot();
        let was_empty = state.pending.is_none();
        state.pending = Some(Emission {
            stdout,
            stderr,
            dropped: out_dropped + err_dropped,
            at,
        });
        was_empty
    }

    /// Take the pending body, leaving the outbox empty.
    pub fn take(&self) -> Option<Emission> {
        self.lock().pending.take()
    }

    /// Consume both caps at the child's EOF and yield its final body:
    /// all retained stdout, all retained stderr, and what the two bounds
    /// discarded between them. The completion path's call.
    ///
    /// This also flushes an unterminated trailing line, which `feed`
    /// deliberately withholds — mid-write while the child lives, but a
    /// real line once it is gone.
    ///
    /// **It RE-ARMS rather than spends, and that is not tidiness.** The
    /// caps are replaced with fresh ones built from the same policy,
    /// because this type outlives the child that fills it: the loop owns
    /// one per source and the scheduler may respawn after an exit.
    /// Leaving the old caps in place would append the next child's
    /// output to a dead child's lines; leaving spent ones would retain
    /// nothing at all, silently.
    ///
    /// A body already sitting in the outbox is left alone. It is a
    /// strictly earlier view of the same stream, and the events are
    /// ordered — every wake this child sent was sent before the
    /// completion — so the loop records it first and the completion
    /// replaces it in the same drain.
    pub fn finish(&self) -> (Vec<Vec<u8>>, Vec<Vec<u8>>, usize) {
        let mut state = self.lock();
        let (out_policy, err_policy) = state.policy;
        let stdout = std::mem::replace(&mut state.stdout, cap(out_policy));
        let stderr = std::mem::replace(&mut state.stderr, cap(err_policy));
        let (out, out_dropped) = stdout.finish();
        let (err, err_dropped) = stderr.finish();
        (out, err, out_dropped + err_dropped)
    }

    /// The state holds no invariant a panicking worker can break, so
    /// poisoning is recovered, not propagated — `ChildSlot`'s rule and
    /// for the same reason.
    fn lock(&self) -> MutexGuard<'_, State> {
        self.0.lock().unwrap_or_else(PoisonError::into_inner)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::core::retain::Keep;

    fn ample() -> Retention {
        Retention {
            max_lines: 100,
            keep: Keep::Bottom,
        }
    }

    fn bound(max_lines: usize) -> Retention {
        Retention {
            max_lines,
            keep: Keep::Bottom,
        }
    }

    fn at() -> jiff::Timestamp {
        jiff::Timestamp::now()
    }

    #[test]
    fn the_first_visible_line_transitions_the_outbox_and_says_so() {
        // The RETURN VALUE is the wake signal: true means "the loop is
        // not already holding a body from me".
        let e = Emissions::new(ample(), ample());
        assert!(e.feed(Stream::Stdout, b"one\n", at()), "empty -> occupied");
        let body = e.take().expect("a body");
        assert_eq!(body.stdout, vec![b"one\n".to_vec()]);
        assert!(body.stderr.is_empty());
    }

    #[test]
    fn feeding_while_a_body_is_unread_updates_it_and_sends_no_second_wake() {
        // Bounding the body alone leaves the channel growing one wake per
        // read, and the loop drains that channel until empty — so the
        // starvation the outbox prevents arrives through the wakes.
        let e = Emissions::new(ample(), ample());
        assert!(e.feed(Stream::Stdout, b"one\n", at()));
        for i in 0..500 {
            assert!(
                !e.feed(Stream::Stdout, format!("line-{i}\n").as_bytes(), at()),
                "a wake was sent while a body was still pending"
            );
        }
        let body = e.take().expect("a body");
        assert_eq!(body.stdout.last().unwrap(), b"line-499\n");
        assert!(e.take().is_none(), "one slot, never a backlog");
    }

    #[test]
    fn taking_re_arms_the_wake() {
        let e = Emissions::new(ample(), ample());
        assert!(e.feed(Stream::Stdout, b"a\n", at()));
        e.take();
        assert!(e.feed(Stream::Stdout, b"b\n", at()), "a taken slot re-arms");
    }

    #[test]
    fn a_partial_line_moves_nothing_and_wakes_nobody() {
        let e = Emissions::new(ample(), ample());
        assert!(!e.feed(Stream::Stdout, b"no-terminator", at()));
        assert!(e.take().is_none());
        assert!(
            e.feed(Stream::Stdout, b"-yet\n", at()),
            "the terminator moves it"
        );
    }

    #[test]
    fn the_two_streams_stay_apart_and_a_published_body_carries_both() {
        let e = Emissions::new(ample(), ample());
        e.feed(Stream::Stdout, b"out-1\n", at());
        e.feed(Stream::Stderr, b"err-1\n", at());
        let body = e.take().expect("a body");
        assert_eq!(body.stdout, vec![b"out-1\n".to_vec()]);
        assert_eq!(body.stderr, vec![b"err-1\n".to_vec()]);
    }

    #[test]
    fn a_stderr_feed_publishes_the_current_stdout_too() {
        // The atomicity requirement: whichever reader moves last
        // publishes a view of BOTH caps, or a body would show one
        // stream frozen at the other's last write.
        let e = Emissions::new(ample(), ample());
        e.feed(Stream::Stdout, b"out-1\n", at());
        e.take();
        e.feed(Stream::Stderr, b"err-1\n", at());
        let body = e.take().expect("a body");
        assert_eq!(
            body.stdout,
            vec![b"out-1\n".to_vec()],
            "stdout must not vanish"
        );
        assert_eq!(body.stderr, vec![b"err-1\n".to_vec()]);
    }

    #[test]
    fn each_stream_keeps_its_own_bound_and_the_drop_count_is_the_sum() {
        // The badge is per PANE, not per pipe.
        let e = Emissions::new(bound(1), bound(1));
        e.feed(Stream::Stdout, b"o1\no2\no3\n", at());
        e.feed(Stream::Stderr, b"e1\ne2\n", at());
        let body = e.take().expect("a body");
        assert_eq!(body.stdout, vec![b"o3\n".to_vec()]);
        assert_eq!(body.stderr, vec![b"e2\n".to_vec()]);
        assert_eq!(body.dropped, 3, "2 dropped from stdout + 1 from stderr");
    }

    #[test]
    fn eviction_alone_counts_as_movement() {
        // A FULL cap that evicts one line and gains one has the same
        // length and different content. A predicate on length alone
        // would go blind exactly when a follower is busiest.
        let e = Emissions::new(bound(1), bound(1));
        assert!(e.feed(Stream::Stdout, b"first\n", at()));
        e.take();
        assert!(
            e.feed(Stream::Stdout, b"second\n", at()),
            "eviction is movement"
        );
    }

    #[test]
    fn finishing_flushes_the_line_the_child_never_terminated() {
        // The one difference between a snapshot and a finish. `feed`
        // withholds a partial line because it is mid-write and would
        // repaint under the reader; once the child is gone no terminator
        // is coming, and dropping it would eat output that fits.
        let e = Emissions::new(ample(), ample());
        e.feed(Stream::Stdout, b"whole\npartial", at());
        let body = e.take().expect("a body");
        assert_eq!(body.stdout, vec![b"whole\n".to_vec()], "not while it lives");
        let (stdout, _, _) = e.finish();
        assert_eq!(
            stdout,
            vec![b"whole\n".to_vec(), b"partial".to_vec()],
            "but yes once it is gone, and verbatim — no terminator invented"
        );
    }

    #[test]
    fn finishing_reports_what_both_bounds_dropped() {
        let e = Emissions::new(bound(1), bound(1));
        e.feed(Stream::Stdout, b"o1\no2\no3\n", at());
        e.feed(Stream::Stderr, b"e1\ne2\n", at());
        let (stdout, stderr, dropped) = e.finish();
        assert_eq!(stdout, vec![b"o3\n".to_vec()]);
        assert_eq!(stderr, vec![b"e2\n".to_vec()]);
        assert_eq!(dropped, 3, "2 dropped from stdout + 1 from stderr");
    }

    #[test]
    fn finishing_re_arms_the_caps_for_the_next_child() {
        // An Emissions OUTLIVES its child: the loop owns one per source
        // and the scheduler may respawn after an exit. Two ways to get
        // this wrong and neither one is loud — leaving the old caps in
        // place appends the next child's output to a dead child's lines,
        // and leaving SPENT caps retains nothing at all.
        let e = Emissions::new(bound(10), bound(10));
        e.feed(Stream::Stdout, b"first-child\n", at());
        assert_eq!(e.finish().0, vec![b"first-child\n".to_vec()]);
        e.take();

        assert!(
            e.feed(Stream::Stdout, b"second-child\n", at()),
            "a re-armed cap still wakes the loop"
        );
        let body = e.take().expect("a body");
        assert_eq!(
            body.stdout,
            vec![b"second-child\n".to_vec()],
            "the next child starts clean, neither appending nor bounded to nothing"
        );
        assert_eq!(body.dropped, 0, "and its drop count starts at zero");
    }

    #[test]
    fn a_clone_shares_the_state() {
        let a = Emissions::new(ample(), ample());
        let b = a.clone();
        b.feed(Stream::Stdout, b"through\n", at());
        assert!(a.take().is_some(), "a clone must share, not copy");
    }

    #[test]
    fn a_panicking_feeder_does_not_poison_the_state() {
        let e = Emissions::new(ample(), ample());
        let c = e.clone();
        let _ = std::thread::spawn(move || {
            c.feed(Stream::Stdout, b"before the panic\n", at());
            panic!("worker died");
        })
        .join();
        assert!(e.take().is_some(), "a panic must not wedge the state");
    }
}
