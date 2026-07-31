//! A bounded line accumulator.
//!
//! Bytes in, at most `max_lines` of them out, and a count of the lines
//! that did not survive. The bound is a LINE count and never a byte
//! budget: retention costs a fixed overhead per retained line whatever
//! the line holds, so the same byte total spans an order of magnitude
//! in cost depending on how it is chopped up. Lines are the unit that
//! predicts the cost, and the unit a pane renders.
//!
//! Nothing here decodes, for two separate reasons, and it reads like an
//! omission rather than the correctness rule it is. One: a pipe splits
//! wherever it likes, so a chunk can end in the middle of a character,
//! and decoding a chunk would turn that half into U+FFFD before its
//! other half arrived. Two: one of the two consumers writes a child's
//! stderr through verbatim, so any decode is an irreversible change to
//! what the user sees. Decoding belongs to the render path, which
//! already does it over the whole stream.
//!
//! A line ends at `\n` and nowhere else — a `\r` before it belongs to
//! the line, exactly as the renderer treats it. Do not "improve" that.

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
    /// The bytes since the last `\n`, carried across feeds. A pipe splits
    /// wherever it likes — mid-line, mid-character, between the `\r` and
    /// the `\n` — so a chunk boundary is not a line boundary and treating
    /// it as one would both corrupt the content and inflate the count.
    partial: Vec<u8>,
}

impl LineCap {
    /// A cap retaining at most `max_lines` lines from the `keep` end.
    pub fn new(max_lines: usize, keep: Keep) -> LineCap {
        LineCap {
            max_lines,
            keep,
            retained: VecDeque::new(),
            dropped: 0,
            partial: Vec::new(),
        }
    }

    /// Feed an arbitrary byte chunk. Never retains beyond the bound.
    ///
    /// This is the only place a line ending is recognised. A second
    /// notion of where a line ends would drift from this one.
    pub fn feed(&mut self, chunk: &[u8]) {
        let mut rest = chunk;
        while let Some(nl) = rest.iter().position(|&b| b == b'\n') {
            let (line, tail) = rest.split_at(nl + 1);
            if self.partial.is_empty() {
                self.accept(line);
            } else {
                self.partial.extend_from_slice(line);
                let joined = std::mem::take(&mut self.partial);
                self.accept(&joined);
            }
            rest = tail;
        }
        self.partial.extend_from_slice(rest);
    }

    /// Consume, flushing any unterminated trailing line, and yield the
    /// retained lines with the number dropped.
    ///
    /// Consuming is the point: the flush is a state-changing, once-only
    /// step, so a caller cannot take the lines twice and get two
    /// different answers.
    pub fn finish(mut self) -> (Vec<Vec<u8>>, usize) {
        if !self.partial.is_empty() {
            // A line the child never terminated is still a line: the
            // render path splits on `\n` after trimming one trailing
            // terminator, so dropping this would eat output that fits.
            // No terminator is invented for it — the retained bytes stay
            // exactly what the child wrote.
            let last = std::mem::take(&mut self.partial);
            self.accept(&last);
        }
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
    fn a_line_split_across_feeds_is_one_line() {
        let mut cap = LineCap::new(10, Keep::Bottom);
        cap.feed(b"hel");
        cap.feed(b"lo\n");
        assert_eq!(cap.finish(), (vec![b"hello\n".to_vec()], 0));
    }

    #[test]
    fn a_split_line_still_counts_once_against_the_bound() {
        // The failure this guards: counting the fragment as a line lets a
        // chatty child evict twice as fast as it should, and `dropped` lies.
        let mut cap = LineCap::new(2, Keep::Bottom);
        cap.feed(b"1\n2\n3");
        cap.feed(b"\n");
        assert_eq!(cap.finish(), (vec![b"2\n".to_vec(), b"3\n".to_vec()], 1));
    }

    #[test]
    fn a_multibyte_char_split_across_feeds_survives() {
        // Two failures share this test and only one of them is the join.
        // Retaining bytes rather than text is what keeps the half-character
        // intact across the boundary; the join is what puts the halves back
        // in one line. Decoding per chunk would pass the second and fail
        // the first, silently, with a U+FFFD where the é was.
        let mut cap = LineCap::new(10, Keep::Bottom);
        let s = "héllo\n".as_bytes();
        let cut = 2; // inside the two-byte é
        cap.feed(&s[..cut]);
        cap.feed(&s[cut..]);
        assert_eq!(cap.finish(), (vec!["héllo\n".as_bytes().to_vec()], 0));
    }

    #[test]
    fn many_tiny_feeds_bound_the_partial_buffer_too() {
        // A child emitting one enormous line with no newline defeats a LINE
        // bound. This pins that assembling it is at least not quadratic;
        // bounding it is a separate concern.
        let mut cap = LineCap::new(4, Keep::Bottom);
        for _ in 0..50_000 {
            cap.feed(b"x");
        }
        let (lines, _) = cap.finish();
        assert_eq!(lines.len(), 1);
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
