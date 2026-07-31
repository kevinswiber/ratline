//! A bounded line accumulator.
//!
//! Bytes in, at most `max_lines` of them out, and a count of the lines
//! that did not survive. The bound is a LINE count and never a byte
//! budget: retention costs a fixed overhead per retained line whatever
//! the line holds, so the same byte total spans an order of magnitude
//! in cost depending on how it is chopped up. Lines are the unit that
//! predicts the cost, and the unit a pane renders.
//!
//! Nothing here decodes. The retained bytes are handed back exactly as
//! the child wrote them, terminators included, because one of the two
//! consumers writes a child's stderr through verbatim — decoding would
//! replace invalid UTF-8 with U+FFFD irreversibly and lose the stream's
//! framing. Decoding belongs to the render path, which already does it.

use std::collections::VecDeque;

/// Which end of an over-long stream survives.
///
/// The two are not symmetric implementations of one idea:
/// `Bottom` is a **ring** — every line is retained and the oldest is
/// evicted — while `Top` is a **gate** that shuts once it is full and
/// retains nothing after. Both keep counting, and neither ever asks
/// its caller to stop reading.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Keep {
    /// Keep the oldest lines; discard everything after the bound.
    Top,
    /// Keep the newest lines; evict the oldest to make room.
    Bottom,
}

/// A bounded window over a stream of lines.
///
/// `dropped` counts **lines**, not bytes.
pub struct LineCap {
    max_lines: usize,
    keep: Keep,
    retained: VecDeque<Vec<u8>>,
    dropped: usize,
}

impl LineCap {
    /// A cap retaining at most `max_lines` lines from the `keep` end.
    pub fn new(max_lines: usize, keep: Keep) -> LineCap {
        LineCap {
            max_lines,
            keep,
            retained: VecDeque::new(),
            dropped: 0,
        }
    }

    /// Feed an arbitrary byte chunk. Never retains beyond the bound.
    pub fn feed(&mut self, chunk: &[u8]) {
        let mut rest = chunk;
        while let Some(nl) = rest.iter().position(|&b| b == b'\n') {
            let (line, tail) = rest.split_at(nl + 1);
            self.accept(line);
            rest = tail;
        }
        if !rest.is_empty() {
            // A line the child has not terminated is still a line: the
            // render path splits on `\n` after trimming one trailing
            // terminator, so dropping this would eat output that fits.
            self.accept(rest);
        }
    }

    /// Consume, yielding the retained lines and the number dropped.
    ///
    /// Consuming is the point: retaining is a state-changing, once-only
    /// step, so a caller cannot take the lines twice and get two
    /// different answers.
    pub fn finish(self) -> (Vec<Vec<u8>>, usize) {
        (self.retained.into(), self.dropped)
    }

    /// Offer one line, terminator included, to the retained window.
    fn accept(&mut self, line: &[u8]) {
        match self.keep {
            Keep::Bottom => {
                self.retained.push_back(line.to_vec());
                // A deque so eviction is O(1). A `Vec` with `remove(0)`
                // is O(n) per line on exactly the hot path this type
                // exists for.
                while self.retained.len() > self.max_lines {
                    self.retained.pop_front();
                    self.dropped += 1;
                }
            }
            Keep::Top => {
                if self.retained.len() < self.max_lines {
                    self.retained.push_back(line.to_vec());
                } else {
                    self.dropped += 1;
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn under_the_bound_nothing_is_dropped_and_the_lines_are_verbatim() {
        let mut cap = LineCap::new(10, Keep::Bottom);
        cap.feed(b"a\nb\nc\n");
        assert_eq!(
            cap.finish(),
            (vec![b"a\n".to_vec(), b"b\n".to_vec(), b"c\n".to_vec()], 0)
        );
    }

    #[test]
    fn keep_bottom_retains_the_newest_and_counts_the_rest() {
        let mut cap = LineCap::new(2, Keep::Bottom);
        cap.feed(b"1\n2\n3\n4\n");
        assert_eq!(cap.finish(), (vec![b"3\n".to_vec(), b"4\n".to_vec()], 2));
    }

    #[test]
    fn keep_top_retains_the_oldest_and_counts_the_rest() {
        let mut cap = LineCap::new(2, Keep::Top);
        cap.feed(b"1\n2\n3\n4\n");
        assert_eq!(cap.finish(), (vec![b"1\n".to_vec(), b"2\n".to_vec()], 2));
    }

    #[test]
    fn a_trailing_line_without_a_newline_is_still_a_line() {
        // `output_lines` trims one trailing newline and splits; a command
        // that does not end with one still has a last line, and dropping it
        // would silently eat output that fits.
        let mut cap = LineCap::new(10, Keep::Bottom);
        cap.feed(b"a\nb");
        // The last line has no terminator, and that is preserved:
        // concat() must reproduce exactly what the child wrote.
        assert_eq!(cap.finish(), (vec![b"a\n".to_vec(), b"b".to_vec()], 0));
    }

    #[test]
    fn invalid_utf8_survives_the_accumulator_untouched() {
        // Decoding here would replace this byte with U+FFFD and the plain
        // path would write different bytes than the child produced.
        let mut cap = LineCap::new(10, Keep::Bottom);
        cap.feed(b"\xff\n");
        assert_eq!(cap.finish(), (vec![b"\xff\n".to_vec()], 0));
    }

    #[test]
    fn the_retained_set_never_exceeds_the_bound_however_much_is_fed() {
        // The property that makes this a BOUND rather than a hint.
        let mut cap = LineCap::new(4, Keep::Bottom);
        for _ in 0..10_000 {
            cap.feed(b"x\n");
        }
        let (lines, dropped) = cap.finish();
        assert_eq!(lines.len(), 4);
        assert_eq!(dropped, 9_996);
    }

    #[test]
    fn a_zero_bound_retains_nothing_and_counts_everything() {
        // Not a configuration anyone should write, but it must not panic
        // and must not silently behave as unbounded.
        let mut cap = LineCap::new(0, Keep::Bottom);
        cap.feed(b"a\nb\n");
        assert_eq!(cap.finish(), (Vec::<Vec<u8>>::new(), 2));
    }
}
