use std::collections::VecDeque;

/// Byte budget for retained frames. Bytes, not frames: one dashboard's
/// frame is another's screenful, so a frame count caps nothing useful.
pub const HISTORY_MAX_BYTES: usize = 4 * 1024 * 1024;

/// One distinct composed frame. Consecutive identical ticks collapse into
/// the newest entry, which counts them instead.
pub struct Entry {
    pub seq: u64,
    pub at: jiff::Timestamp,
    pub sig: u64,
    pub ticks: u32,
    pub frame: Vec<String>,
}

impl Entry {
    fn bytes(&self) -> usize {
        self.frame
            .iter()
            .map(|l| l.len() + std::mem::size_of::<String>())
            .sum()
    }
}

/// A byte-capped ring of distinct composed frames. Entries are addressed
/// by a monotonic `seq` that survives eviction, so a held cursor can
/// always be resolved to the closest surviving entry.
#[derive(Default)]
pub struct History {
    entries: VecDeque<Entry>,
    next_seq: u64,
    total_bytes: usize,
}

impl History {
    pub fn new() -> History {
        History::default()
    }

    /// Record one tick. Returns true when a NEW distinct frame was
    /// recorded; false means consecutive dedupe — the newest entry's tick
    /// count grew instead. Eviction drops the oldest entries once the
    /// byte cap is exceeded, but never below two (a scrub with nothing to
    /// step between is no scrub at all).
    pub fn record(&mut self, sig: u64, lines: &[String], at: jiff::Timestamp) -> bool {
        if let Some(newest) = self.entries.back_mut()
            && newest.sig == sig
        {
            newest.ticks += 1;
            return false;
        }
        let entry = Entry {
            seq: self.next_seq,
            at,
            sig,
            ticks: 1,
            frame: lines.to_vec(),
        };
        self.next_seq += 1;
        self.total_bytes += entry.bytes();
        self.entries.push_back(entry);
        while self.total_bytes > HISTORY_MAX_BYTES && self.entries.len() > 2 {
            if let Some(evicted) = self.entries.pop_front() {
                self.total_bytes -= evicted.bytes();
            }
        }
        true
    }

    pub fn newest_seq(&self) -> Option<u64> {
        self.entries.back().map(|e| e.seq)
    }

    /// The surviving entry closest to `seq`, clamped into the surviving
    /// range: at-or-before `seq` when one survives, else the oldest
    /// survivor. `None` only when empty.
    pub fn nearest(&self, seq: u64) -> Option<&Entry> {
        self.entries
            .iter()
            .rev()
            .find(|e| e.seq <= seq)
            .or_else(|| self.entries.front())
    }

    /// The newest surviving entry older than `seq`.
    pub fn prev(&self, seq: u64) -> Option<&Entry> {
        self.entries.iter().rev().find(|e| e.seq < seq)
    }

    /// The oldest surviving entry newer than `seq`.
    pub fn next(&self, seq: u64) -> Option<&Entry> {
        self.entries.iter().find(|e| e.seq > seq)
    }

    // Cap enforcement is internal; the accounting is observed by tests.
    #[cfg_attr(not(test), allow(dead_code))]
    pub fn bytes(&self) -> usize {
        self.total_bytes
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn at() -> jiff::Timestamp {
        jiff::Timestamp::from_second(1_785_067_200).expect("fixed timestamp")
    }

    fn frame(items: &[&str]) -> Vec<String> {
        items.iter().map(|s| s.to_string()).collect()
    }

    #[test]
    fn a_changed_frame_records_and_a_repeat_dedupes() {
        let mut h = History::new();
        assert!(h.record(1, &frame(&["a"]), at()));
        assert!(!h.record(1, &frame(&["a"]), at()), "a repeat sig dedupes");
        let entry = h.nearest(0).expect("one entry");
        assert_eq!(entry.ticks, 2, "the deduped entry counts its ticks");
        assert_eq!(h.newest_seq(), Some(0));
    }

    #[test]
    fn entries_are_seq_addressed_monotonically() {
        let mut h = History::new();
        assert!(h.record(1, &frame(&["a"]), at()));
        assert!(h.record(2, &frame(&["b"]), at()));
        assert!(h.record(3, &frame(&["c"]), at()));
        assert_eq!(h.newest_seq(), Some(2));
        for seq in 0..=2 {
            assert_eq!(h.nearest(seq).expect("present").seq, seq);
        }
    }

    #[test]
    fn prev_and_next_walk_the_ring() {
        let mut h = History::new();
        h.record(1, &frame(&["a"]), at());
        h.record(2, &frame(&["b"]), at());
        h.record(3, &frame(&["c"]), at());
        assert_eq!(h.prev(2).expect("prev of newest").seq, 1);
        assert_eq!(h.prev(1).expect("prev of middle").seq, 0);
        assert!(h.prev(0).is_none(), "nothing before the oldest");
        assert_eq!(h.next(0).expect("next of oldest").seq, 1);
        assert_eq!(h.next(1).expect("next of middle").seq, 2);
    }

    #[test]
    fn next_past_newest_is_none() {
        let mut h = History::new();
        h.record(1, &frame(&["a"]), at());
        h.record(2, &frame(&["b"]), at());
        assert!(h.next(1).is_none());
        assert!(h.next(99).is_none());
    }

    #[test]
    fn the_byte_cap_evicts_from_the_front_but_keeps_two() {
        // Each frame is 3 MiB: any two together exceed the 4 MiB cap, so
        // eviction pressure is constant — the 2-entry floor must hold.
        let big = "x".repeat(3 * 1024 * 1024);
        let mut h = History::new();
        assert!(h.record(1, &frame(&[&big]), at()));
        assert!(h.record(2, &frame(&[&big]), at()));
        assert!(h.record(3, &frame(&[&big]), at()));
        assert_eq!(h.newest_seq(), Some(2));
        assert_eq!(
            h.nearest(0).expect("oldest survivor").seq,
            1,
            "the front entry was evicted"
        );
        assert!(h.prev(1).is_none(), "seq 0 is gone");
        assert_eq!(h.next(1).expect("newest").seq, 2);
        assert!(
            h.bytes() > HISTORY_MAX_BYTES,
            "the floor outranks the cap: two big entries stay"
        );
    }

    #[test]
    fn nearest_survives_eviction() {
        let big = "x".repeat(3 * 1024 * 1024);
        let mut h = History::new();
        h.record(1, &frame(&[&big]), at());
        h.record(2, &frame(&[&big]), at());
        h.record(3, &frame(&[&big]), at());
        // Survivors are seqs 1 and 2.
        assert_eq!(
            h.nearest(0).expect("clamped").seq,
            1,
            "older than every survivor"
        );
        assert_eq!(h.nearest(1).expect("exact").seq, 1);
        assert_eq!(h.nearest(2).expect("exact").seq, 2);
        assert_eq!(h.nearest(99).expect("clamped").seq, 2, "past the newest");
        assert!(History::new().nearest(0).is_none(), "empty history");
    }

    #[test]
    fn bytes_accounts_frames() {
        let lines = frame(&["abc", "defgh"]);
        let mut h = History::new();
        h.record(1, &lines, at());
        let expected: usize = lines
            .iter()
            .map(|l| l.len() + std::mem::size_of::<String>())
            .sum();
        assert_eq!(h.bytes(), expected);
    }
}
