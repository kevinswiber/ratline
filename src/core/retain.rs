//! A bounded line accumulator.
//!
//! Bytes in, at most `max_lines` of them out, and a count of the lines
//! that did not survive. The bound is a LINE count and never a byte
//! budget: retention costs a fixed overhead per retained line whatever
//! the line holds, so the same byte total spans an order of magnitude
//! in cost depending on how it is chopped up. Lines are the unit that
//! predicts the cost, and the unit a pane renders.
//!
//! # What a line count costs in bytes
//!
//! **A line bound is not a memory bound, and the difference is three
//! orders of magnitude.** Since a line survives up to [`MAX_LINE_BYTES`]
//! (64 KiB), the true ceiling is `max_lines x 64 KiB` per stream:
//!
//! | caller | lines | typical | ceiling per stream |
//! |---|---|---|---|
//! | `rat watch`, dashboard panes | 1,000 | ~100 KiB | **62.5 MiB** |
//! | `rat spin` | 10,000 | ~1 MiB | **625 MiB** |
//!
//! Both streams are capped separately, so a child flooding both doubles
//! it. Terminal lines run well under 100 bytes, so ordinary output sits
//! at the typical column; reaching the ceiling takes output that is
//! uniformly enormous — hundreds of MB of ≥64 KiB lines — which is
//! pathological rather than merely large, and which consumed UNBOUNDED
//! memory before this type existed.
//!
//! This is a deliberate, documented ceiling rather than an oversight,
//! but it is the number to reason with: **anyone raising `max_lines` is
//! multiplying it.** Bounding bytes as well would mean carrying a second
//! budget through eviction, and the line count would stop predicting the
//! cost — the property the paragraph above exists to buy.
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
use std::io::Read;

/// The most bytes one line may hold, terminator aside, before it is
/// dropped whole.
///
/// A line bound does not bound memory on its own: a child emitting one
/// endless line with no newline in it defeats a count of lines, and the
/// buffer assembling that line grows until the process dies — the same
/// exhaustion by a different door.
///
/// **The over-long line is dropped whole, never truncated**, and that is
/// a correctness rule rather than a preference. The change marks index
/// into the retained body by character position, so a body holding half
/// a line would carry marks pointing into text the user cannot see.
/// Truncating within a line means changing that contract first.
///
/// 64 KiB sits far above any line a terminal renders — a thousand-column
/// line with a style sequence on every cell is an order of magnitude
/// under it, and 64 KiB is some eight hundred rows of an 80-column
/// terminal. It is a **secondary** ceiling: the line count is the bound,
/// and this only catches the shape the line count cannot express.
const MAX_LINE_BYTES: usize = 64 * 1024;

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
    /// The line in progress passed the byte backstop, so it is being
    /// discarded whole and its bytes are being skipped until the next
    /// `\n`. It has already been counted.
    overlong: bool,
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
            overlong: false,
        }
    }

    /// Feed an arbitrary byte chunk. Never retains beyond the bound.
    ///
    /// This is the only place a line ending is recognised. A second
    /// notion of where a line ends would drift from this one.
    pub fn feed(&mut self, chunk: &[u8]) {
        let mut rest = chunk;
        while let Some(nl) = rest.iter().position(|&b| b == b'\n') {
            self.extend_line(&rest[..nl]);
            self.end_line();
            rest = &rest[nl + 1..];
        }
        self.extend_line(rest);
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
            // exactly what the child wrote. An over-long line cannot
            // reach here: it was counted and released when it passed the
            // backstop.
            let last = std::mem::take(&mut self.partial);
            self.accept(&last);
        }
        (self.retained.into(), self.dropped)
    }

    /// The retained lines and the drop count, WITHOUT consuming.
    ///
    /// `finish()` stays consuming and is still the batch path's only
    /// exit: it was made so on purpose, since a caller that could take
    /// the lines twice could get two different answers. Nothing here
    /// relaxes that — a live source reads repeatedly precisely because
    /// its child has not finished.
    ///
    /// **The in-progress line is excluded, and that is the difference
    /// that matters.** `finish()` flushes it because the child is gone
    /// and no terminator is ever coming. A live child's partial line is
    /// mid-write, and a snapshot including it would paint half a line
    /// that changes under the reader on the next chunk.
    pub fn snapshot(&self) -> (Vec<Vec<u8>>, usize) {
        (self.retained.iter().cloned().collect(), self.dropped)
    }

    /// How many lines are retained, without cloning any of them.
    ///
    /// The live worker asks this on every read to decide whether
    /// anything a reader could see has changed; asking `snapshot()`
    /// instead would clone the whole retained set per chunk to answer a
    /// question about its length.
    pub fn retained_len(&self) -> usize {
        self.retained.len()
    }

    /// How many lines the bound has discarded, without cloning.
    ///
    /// Paired with `retained_len` because a body can change by DROPPING
    /// alone: a full cap that evicts one line and gains one has the same
    /// length and different content.
    pub fn dropped_count(&self) -> usize {
        self.dropped
    }

    /// Add bytes to the line in progress. `bytes` never holds a `\n`.
    ///
    /// This is the one place that decides a line is over: past the
    /// backstop it is counted once, released, and its remaining bytes
    /// are skipped rather than buffered.
    fn extend_line(&mut self, bytes: &[u8]) {
        if self.overlong {
            return;
        }
        if self.partial.len() + bytes.len() > MAX_LINE_BYTES {
            self.overlong = true;
            self.dropped += 1;
            // Release rather than clear: a monster's capacity is the
            // memory this backstop exists to give back.
            self.partial = Vec::new();
            return;
        }
        self.partial.extend_from_slice(bytes);
    }

    /// The line in progress ended at a `\n`.
    ///
    /// A `\r` immediately before that `\n` is the other half of a CRLF
    /// terminator rather than content, and it is dropped here — the one
    /// place a line ending is decided, so the two halves are recognised
    /// together or the split drifts.
    ///
    /// Keeping it was not harmless. The paint path writes a row and then
    /// pads it to the pane's width, so a `\r` riding along returns the
    /// cursor to column 0 and the padding erases the row it just drew: a
    /// line shorter than its pane vanished outright, and a longer one
    /// kept only the tail the padding could not reach. Every Windows
    /// child ends its lines this way, so on Windows this was most of
    /// them.
    ///
    /// A BARE `\r` is content and stays: a child that rewrites its own
    /// line without ever sending `\n` — a progress meter — means it.
    fn end_line(&mut self) {
        if self.overlong {
            // Already counted; the split resynchronizes here, so the
            // next line is an ordinary line.
            self.overlong = false;
            return;
        }
        let mut line = std::mem::take(&mut self.partial);
        if line.last() == Some(&b'\r') {
            line.pop();
        }
        line.push(b'\n');
        self.accept(&line);
        // Hand the buffer back so the next line reuses its capacity.
        line.clear();
        self.partial = line;
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

/// How much of one child's output is retained, and from which end.
///
/// Carried by value, and deliberately not an `Option`: every caller has
/// a policy, so "unbounded" is not representable.
#[derive(Clone, Copy)]
pub struct Retention {
    pub max_lines: usize,
    pub keep: Keep,
}

/// Drain one pipe to EOF, retaining at most what the policy allows.
///
/// **The loop always runs to EOF.** There is no early exit when the
/// bound fills — the accumulator discards, the reader keeps reading.
///
/// What an early exit actually costs was measured rather than reasoned
/// about, and it is not the deadlock one would expect: this function
/// takes the pipe by value and drops it on return, so the read end
/// closes before anyone waits and the child dies of SIGPIPE in
/// milliseconds. The damage is quieter than a hang and harder to
/// notice — the drop count comes back ZERO while everything past the
/// bound is lost, so the one number that makes a truncation visible
/// would report nothing was truncated, and the child's exit stops
/// being clean. See the proof in `core::child`'s tests.
///
/// Each pipe gets its own accumulator, which is the shipped shape, so a
/// child flooding both is bounded at twice the line count. Bounded is
/// the requirement; a single shared ceiling is not.
pub fn read_all<R: Read>(pipe: Option<R>, retention: Retention) -> (Vec<Vec<u8>>, usize) {
    let mut cap = LineCap::new(retention.max_lines, retention.keep);
    if let Some(mut pipe) = pipe {
        // A read granularity, not a bound. The bound is the cap's.
        let mut buf = [0u8; 8 * 1024];
        loop {
            match pipe.read(&mut buf) {
                Ok(0) => break,
                Ok(n) => cap.feed(&buf[..n]),
                Err(err) if err.kind() == std::io::ErrorKind::Interrupted => continue,
                // A read error yields whatever arrived: a partial frame
                // beats tearing the dashboard down.
                Err(_) => break,
            }
        }
    }
    cap.finish()
}

/// A dropped-line count as a person reads it: `90`, `1.2k`, `2.5M`.
///
/// One spelling for one number, shared by every surface that reports a
/// truncation — a watch pane's chrome badge and `spin`'s stderr notice.
/// Two spellings of the same count would read as two different
/// mechanisms.
pub fn compact_count(n: usize) -> String {
    const K: usize = 1_000;
    const M: usize = 1_000_000;
    match n {
        n if n < K => n.to_string(),
        n if n < M => format!("{}.{}k", n / K, (n % K) / 100),
        n => format!("{}.{}M", n / M, (n % M) / 100_000),
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
    fn a_crlf_terminator_is_recognised_whole() {
        // The `\r` is half a terminator, not content. Retaining it put a
        // carriage return inside the row the pane paints, and the padding
        // that follows the row to the pane's width then erased the row —
        // blank panes for every Windows child.
        let mut cap = LineCap::new(10, Keep::Bottom);
        cap.feed(b"a\r\nb\r\n");
        assert_eq!(cap.finish(), (vec![b"a\n".to_vec(), b"b\n".to_vec()], 0));
    }

    #[test]
    fn a_crlf_split_across_two_reads_is_still_one_terminator() {
        // A pipe breaks where it likes, and the `\r` landing at the end of
        // one chunk is the case a strip inside `feed` would miss: by the
        // time the `\n` arrives the `\r` is already buffered.
        let mut cap = LineCap::new(10, Keep::Bottom);
        cap.feed(b"a\r");
        cap.feed(b"\nb\r\n");
        assert_eq!(cap.finish(), (vec![b"a\n".to_vec(), b"b\n".to_vec()], 0));
    }

    #[test]
    fn a_bare_carriage_return_is_content_and_survives() {
        // A progress meter rewrites its own line with `\r` and sends no
        // `\n` at all. Only the `\r` that a `\n` follows is a terminator;
        // stripping any other would rewrite what the child wrote.
        let mut cap = LineCap::new(10, Keep::Bottom);
        cap.feed(b"10%\r50%\r100%\n");
        assert_eq!(cap.finish(), (vec![b"10%\r50%\r100%\n".to_vec()], 0));
    }

    #[test]
    fn an_unterminated_line_keeps_its_trailing_carriage_return() {
        // No `\n` ever arrives, so nothing here is a CRLF terminator and
        // the flush must not invent one to strip.
        let mut cap = LineCap::new(10, Keep::Bottom);
        cap.feed(b"a\r\nb\r");
        assert_eq!(cap.finish(), (vec![b"a\n".to_vec(), b"b\r".to_vec()], 0));
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
        // bound. This pins that assembling it is at least not quadratic.
        // The 50 KB it assembles sits deliberately under the byte
        // backstop, so this stays a test about assembling rather than
        // about dropping.
        let mut cap = LineCap::new(4, Keep::Bottom);
        for _ in 0..50_000 {
            cap.feed(b"x");
        }
        let (lines, _) = cap.finish();
        assert_eq!(lines.len(), 1);
    }

    #[test]
    fn one_enormous_line_does_not_grow_without_limit() {
        let mut cap = LineCap::new(10, Keep::Bottom);
        for _ in 0..(MAX_LINE_BYTES / 8 + 100) {
            cap.feed(b"xxxxxxxx");
        }
        let (lines, dropped) = cap.finish();
        assert!(
            lines.iter().all(|l| l.len() <= MAX_LINE_BYTES + 1),
            "a line grew past the backstop"
        );
        assert!(dropped > 0, "an over-long line must be counted as dropped");
    }

    #[test]
    fn an_over_long_line_counts_once_however_many_reads_it_spans() {
        // `dropped` is a line count everywhere, so a monster weighs one —
        // not its byte weight, and not once per read that carried it.
        let mut cap = LineCap::new(10, Keep::Bottom);
        for _ in 0..40 {
            cap.feed(&vec![b'x'; MAX_LINE_BYTES / 2]);
        }
        cap.feed(b"\n");
        let (lines, dropped) = cap.finish();
        assert!(lines.is_empty());
        assert_eq!(dropped, 1);
    }

    #[test]
    fn a_line_at_exactly_the_backstop_is_kept_whole() {
        // The eviction unit is a LINE. A line that fits is never cut.
        let mut cap = LineCap::new(10, Keep::Bottom);
        cap.feed(&vec![b'x'; MAX_LINE_BYTES]);
        cap.feed(b"\n");
        let (lines, dropped) = cap.finish();
        assert_eq!(lines.len(), 1);
        assert_eq!(lines[0].len(), MAX_LINE_BYTES + 1); // + its terminator
        assert_eq!(dropped, 0);
    }

    #[test]
    fn the_line_after_an_over_long_one_is_unaffected() {
        // Dropping a monster must not desynchronize the split — the next
        // line is an ordinary line.
        let mut cap = LineCap::new(10, Keep::Bottom);
        cap.feed(&vec![b'x'; MAX_LINE_BYTES * 2]);
        cap.feed(b"\nafter\n");
        let (lines, _) = cap.finish();
        assert_eq!(lines.last().unwrap(), b"after\n");
    }

    #[test]
    fn a_zero_bound_retains_nothing_and_counts_everything() {
        // Not a configuration anyone should write, but it must not panic
        // and must not silently behave as unbounded.
        let mut cap = LineCap::new(0, Keep::Bottom);
        cap.feed(b"a\nb\n");
        assert_eq!(cap.finish(), (Vec::<Vec<u8>>::new(), 2));
    }

    #[test]
    fn a_snapshot_reads_the_retained_set_without_consuming_it() {
        // The live path's whole reason for existing: read now, keep reading.
        let mut cap = LineCap::new(10, Keep::Bottom);
        cap.feed(b"a\nb\n");
        assert_eq!(cap.snapshot(), (vec![b"a\n".to_vec(), b"b\n".to_vec()], 0));
        // Again, unchanged — a snapshot is not a take.
        assert_eq!(cap.snapshot(), (vec![b"a\n".to_vec(), b"b\n".to_vec()], 0));
        // And feeding continues from where it was.
        cap.feed(b"c\n");
        assert_eq!(cap.snapshot().0.len(), 3);
    }

    #[test]
    fn a_snapshot_respects_the_bound_and_reports_the_same_drop_count() {
        let mut cap = LineCap::new(2, Keep::Bottom);
        cap.feed(b"1\n2\n3\n4\n");
        assert_eq!(cap.snapshot(), (vec![b"3\n".to_vec(), b"4\n".to_vec()], 2));
    }

    #[test]
    fn a_snapshot_excludes_the_unterminated_line_that_finish_would_flush() {
        // THE ONE ASYMMETRY, and it is deliberate. `finish()` flushes a
        // trailing line the child never terminated because the child is
        // gone and that is all there will ever be. A live child's partial
        // line is not finished — it is mid-write — and showing it would
        // paint half a line that changes under the reader.
        let mut cap = LineCap::new(10, Keep::Bottom);
        cap.feed(b"a\nb");
        assert_eq!(cap.snapshot(), (vec![b"a\n".to_vec()], 0));
        // finish() still flushes it, exactly as it does today.
        assert_eq!(cap.finish(), (vec![b"a\n".to_vec(), b"b".to_vec()], 0));
    }

    #[test]
    fn a_snapshot_sees_a_line_only_once_its_terminator_arrives() {
        let mut cap = LineCap::new(10, Keep::Bottom);
        cap.feed(b"par");
        assert_eq!(cap.snapshot().0, Vec::<Vec<u8>>::new());
        cap.feed(b"tial\n");
        assert_eq!(cap.snapshot().0, vec![b"partial\n".to_vec()]);
    }

    #[test]
    fn the_cheap_accessors_agree_with_the_snapshot() {
        // The live worker asks "did anything a reader could see change?"
        // on EVERY chunk. Answering that by taking a snapshot would clone
        // the whole retained set per read, so these two exist to answer it
        // for free — and they must never disagree with what they stand in
        // for.
        let mut cap = LineCap::new(2, Keep::Bottom);
        cap.feed(b"1\n2\n3\n");
        let (lines, dropped) = cap.snapshot();
        assert_eq!(cap.retained_len(), lines.len());
        assert_eq!(cap.dropped_count(), dropped);
    }
}
