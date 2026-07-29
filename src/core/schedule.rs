//! When the next watch child may start.
//!
//! Fixed-delay, not fixed-rate: the deadline is set when a tick
//! COMPOSES, so the gap between one child exiting and the next
//! starting is always one interval, and two children never overlap.
//! A schedule without an interval never sets its own deadline: it
//! spawns once at startup and then only when a request collapses it.
//! Pure: no I/O, no processes, no clock reads — every method takes
//! `now`, which is what makes the machine testable without sleeping.

use std::time::{Duration, Instant};

/// The cadence for one child source: at most one in flight, the next
/// start due one interval after the last completion.
pub struct TickSchedule {
    interval: Option<Duration>,
    due: Deadline,
    in_flight: bool,
    /// A request an in-flight child cannot satisfy, because that child
    /// was started under an environment this request supersedes.
    respawn: bool,
}

/// When the next child may spawn.
#[derive(Copy, Clone, PartialEq, Eq, Debug)]
pub enum Deadline {
    /// Spawn at the next poll.
    Now,
    /// Spawn once `now` reaches the instant.
    At(Instant),
    /// No deadline of its own: only a request collapses it.
    Never,
}

/// What the loop should do about the child on this turn.
#[derive(Copy, Clone, PartialEq, Eq, Debug)]
pub enum Due {
    /// Start a child now. Answering this records one as in flight —
    /// the caller must start it.
    Spawn,
    /// The interval has not elapsed.
    Wait,
    /// A child is already running; only its completion moves this on.
    Running,
}

impl TickSchedule {
    /// A fresh schedule starts due: the first child spawns at once,
    /// interval or not.
    pub fn new(interval: Option<Duration>) -> TickSchedule {
        TickSchedule {
            interval,
            due: Deadline::Now,
            in_flight: false,
            respawn: false,
        }
    }

    pub fn poll(&mut self, now: Instant) -> Due {
        if self.in_flight {
            return Due::Running;
        }
        match self.due {
            Deadline::At(due) if now < due => Due::Wait,
            Deadline::Never => Due::Wait,
            Deadline::Now | Deadline::At(_) => {
                self.in_flight = true;
                Due::Spawn
            }
        }
    }

    /// A child finished and its frame has been composed.
    pub fn completed(&mut self, now: Instant) {
        self.due = if self.respawn {
            Deadline::Now
        } else {
            self.interval
                .map_or(Deadline::Never, |interval| Deadline::At(now + interval))
        };
        self.respawn = false;
        self.in_flight = false;
    }

    /// Ask for a tick as soon as possible. While a child is in flight
    /// this needs no special arm: `completed` overwrites the deadline,
    /// so the in-flight child's completion discharges the request —
    /// its output is no older than a spawn made at request time.
    pub fn request_now(&mut self) {
        self.due = Deadline::Now;
    }

    /// Ask for a NEW child. An in-flight one cannot satisfy this — it
    /// was started under an environment this request supersedes — so
    /// the request survives that completion and spawns a fresh child
    /// at once. Spent after one spawn.
    pub fn request_respawn(&mut self) {
        self.due = Deadline::Now;
        self.respawn = self.in_flight;
    }

    /// The longest the caller may sleep before it must ask again.
    pub fn nap(&self, now: Instant, cap: Duration) -> Duration {
        if self.in_flight {
            return cap;
        }
        match self.due {
            Deadline::Now => Duration::ZERO,
            Deadline::At(due) => due.saturating_duration_since(now).min(cap),
            Deadline::Never => cap,
        }
    }
}

#[cfg(test)]
mod tests {
    use std::time::{Duration, Instant};

    use super::*;

    const IVL: Duration = Duration::from_secs(2);
    const CAP: Duration = Duration::from_millis(50);

    fn base() -> Instant {
        Instant::now()
    }

    #[test]
    fn a_fresh_schedule_spawns_at_once() {
        let mut s = TickSchedule::new(Some(IVL));
        assert_eq!(s.poll(base()), Due::Spawn);
    }

    #[test]
    fn only_one_child_runs_at_a_time() {
        let t = base();
        let mut s = TickSchedule::new(Some(IVL));
        assert_eq!(s.poll(t), Due::Spawn);
        // However long we wait, a second Spawn needs a completion first.
        assert_eq!(s.poll(t + IVL * 10), Due::Running);
    }

    #[test]
    fn a_finished_tick_waits_one_interval() {
        let t = base();
        let mut s = TickSchedule::new(Some(IVL));
        s.poll(t);
        s.completed(t + Duration::from_secs(30)); // a slow child
        // Fixed-delay: the interval counts from COMPLETION, not spawn.
        assert_eq!(s.poll(t + Duration::from_secs(31)), Due::Wait);
        assert_eq!(s.poll(t + Duration::from_secs(32)), Due::Spawn);
    }

    #[test]
    fn the_nap_never_outruns_the_deadline_or_the_cap() {
        let t = base();
        let mut s = TickSchedule::new(Some(IVL));
        s.poll(t);
        // In flight: only the cap bounds the sleep.
        assert_eq!(s.nap(t, CAP), CAP);
        s.completed(t);
        // 2 s to the deadline, capped at a slice.
        assert_eq!(s.nap(t, CAP), CAP);
        // 10 ms to the deadline beats the cap.
        assert_eq!(
            s.nap(t + IVL - Duration::from_millis(10), CAP),
            Duration::from_millis(10)
        );
        // At or past the deadline: zero.
        assert_eq!(s.nap(t + IVL, CAP), Duration::ZERO);
    }

    #[test]
    fn an_immediate_request_collapses_the_deadline() {
        let t = base();
        let mut s = TickSchedule::new(Some(IVL));
        s.poll(t);
        s.completed(t);
        assert_eq!(s.poll(t), Due::Wait);
        s.request_now();
        assert_eq!(s.poll(t), Due::Spawn);
    }

    #[test]
    fn a_request_while_a_child_runs_is_satisfied_by_its_completion() {
        let t = base();
        let mut s = TickSchedule::new(Some(IVL));
        s.poll(t);
        s.request_now(); // e.g. F pressed mid-child
        s.completed(t);
        // The double-run guard: the in-flight child's completion
        // discharged the request; the next spawn waits an interval.
        assert_eq!(s.poll(t + Duration::from_millis(1)), Due::Wait);
        assert_eq!(s.poll(t + IVL), Due::Spawn);
    }

    #[test]
    fn a_respawn_request_outlives_the_child_it_interrupted() {
        let t = base();
        let mut s = TickSchedule::new(Some(IVL));
        s.poll(t);
        s.request_respawn(); // e.g. a theme flip mid-child
        s.completed(t);
        // The stale-env child did NOT satisfy it: spawn again at once.
        assert_eq!(s.poll(t), Due::Spawn);
    }

    #[test]
    fn a_respawn_request_is_spent_once() {
        let t = base();
        let mut s = TickSchedule::new(Some(IVL));
        s.poll(t);
        s.request_respawn();
        s.completed(t);
        assert_eq!(s.poll(t), Due::Spawn); // the respawn
        s.completed(t);
        // Back to normal cadence: no lingering respawn.
        assert_eq!(s.poll(t), Due::Wait);
        assert_eq!(s.poll(t + IVL), Due::Spawn);
    }

    #[test]
    fn a_respawn_request_with_nothing_running_spawns_at_once() {
        let t = base();
        let mut s = TickSchedule::new(Some(IVL));
        s.poll(t);
        s.completed(t);
        s.request_respawn();
        assert_eq!(s.poll(t), Due::Spawn);
        s.completed(t);
        // And it was spent by that spawn.
        assert_eq!(s.poll(t), Due::Wait);
    }

    #[test]
    fn a_trigger_only_schedule_spawns_once_then_waits_forever() {
        let t = base();
        let mut s = TickSchedule::new(None);
        // The first frame still runs at once — a dashboard with no first
        // frame is unusable in either mode.
        assert_eq!(s.poll(t), Due::Spawn);
        s.completed(t);
        // No interval: no deadline of its own, however long we wait.
        assert_eq!(s.poll(t + Duration::from_secs(3600)), Due::Wait);
    }

    #[test]
    fn a_request_still_collapses_a_never_deadline() {
        let t = base();
        let mut s = TickSchedule::new(None);
        s.poll(t);
        s.completed(t);
        s.request_now();
        assert_eq!(s.poll(t), Due::Spawn);
    }

    #[test]
    fn a_respawn_request_survives_completion_without_an_interval() {
        let t = base();
        let mut s = TickSchedule::new(None);
        s.poll(t);
        s.request_respawn();
        s.completed(t);
        // The stale child did not satisfy it: spawn again at once.
        assert_eq!(s.poll(t), Due::Spawn);
        s.completed(t);
        // Spent once, back to Never.
        assert_eq!(s.poll(t), Due::Wait);
    }

    #[test]
    fn the_nap_is_the_cap_when_no_deadline_exists() {
        let t = base();
        let mut s = TickSchedule::new(None);
        s.poll(t);
        s.completed(t);
        assert_eq!(s.nap(t, CAP), CAP);
    }
}
