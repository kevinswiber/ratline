use std::io::Write;

use unicode_width::UnicodeWidthStr;

/// Terminal rows a set of lines will occupy at `term_width`, accounting for
/// wrapping. ANSI escapes are stripped from a copy for measurement only.
pub fn rendered_rows(lines: &[String], term_width: u16) -> u16 {
    let width = usize::from(term_width.max(1));
    lines
        .iter()
        .map(|line| {
            let visible = strip_ansi_escapes::strip_str(line);
            visible.as_str().width().div_ceil(width).max(1)
        })
        .sum::<usize>()
        .min(usize::from(u16::MAX)) as u16
}

/// The complete byte stream for one repaint. Both `frame` and `watch` emit
/// exactly this, so the wire format has one implementation.
///
/// Layout: BSU (if sync) - either a full screen wipe with cursor home
/// (`clear_screen`) or a move up over the previous frame - carriage return -
/// clear to end of screen - lines terminated with CRLF - ESU (if sync). CRLF
/// keeps output correct in raw mode, where LF alone does not return the column.
/// Wiping inside the synchronized frame makes the clear-plus-first-paint
/// transition atomic — no blank flash.
pub fn frame_bytes(
    prev_rows: u16,
    lines: &[String],
    _term_width: u16,
    sync: bool,
    clear_screen: bool,
) -> String {
    let mut out = String::new();
    if sync {
        out.push_str("\x1b[?2026h");
    }
    if clear_screen {
        out.push_str("\x1b[2J\x1b[H");
    } else if prev_rows > 0 {
        out.push_str(&format!("\x1b[{prev_rows}A"));
    }
    out.push_str("\r\x1b[0J");
    for line in lines {
        out.push_str(line);
        out.push_str("\r\n");
    }
    if sync {
        out.push_str("\x1b[?2026l");
    }
    out
}

/// Maximum percentage of rows that may differ before a changed-rows rewrite
/// stops paying for itself and the full repaint takes over.
const DIFF_MAX_CHANGED_PCT: usize = 60;

/// The changed-rows rewrite: `Some(bytes)` when `next` can be painted by
/// rewriting only the rows that differ from `prev`; `None` means "emit the
/// full sequence". `Some(String::new())` means byte-identical: write nothing.
///
/// Eligibility: equal line counts, every line in both frames exactly one
/// terminal row (no wrapping), and no more than `DIFF_MAX_CHANGED_PCT` of
/// rows changed. The emitted stream is purely relative — no newlines (which
/// could scroll), no absolute positioning, no queries.
fn diff_bytes(prev: &[String], next: &[String], term_width: u16, sync: bool) -> Option<String> {
    if prev.len() != next.len() {
        return None;
    }
    // 1:1 check: every line is at least one row, so the row sum equals the
    // line count only when every line occupies exactly one row.
    if usize::from(rendered_rows(prev, term_width)) != prev.len()
        || usize::from(rendered_rows(next, term_width)) != next.len()
    {
        return None;
    }
    let changed: Vec<usize> = (0..next.len()).filter(|&i| prev[i] != next[i]).collect();
    if changed.is_empty() {
        return Some(String::new());
    }
    if changed.len() * 100 > next.len() * DIFF_MAX_CHANGED_PCT {
        return None;
    }
    let mut out = String::new();
    if sync {
        out.push_str("\x1b[?2026h");
    }
    // The cursor rests on the line below the frame; row i sits len-i rows
    // above it. Walk row to row with relative moves only and finish by
    // returning to the resting line, so the net vertical displacement is
    // zero and nothing can scroll.
    let mut above = 0usize;
    for &i in &changed {
        let target = next.len() - i;
        if target > above {
            out.push_str(&format!("\x1b[{}A", target - above));
        } else {
            out.push_str(&format!("\x1b[{}B", above - target));
        }
        above = target;
        out.push_str("\r\x1b[2K");
        out.push_str(&next[i]);
        out.push('\r');
    }
    out.push_str(&format!("\x1b[{above}B"));
    if sync {
        out.push_str("\x1b[?2026l");
    }
    Some(out)
}

/// Repaints blocks of pre-rendered ANSI lines in place. Generic over the
/// writer so tests assert exact bytes against a Vec<u8>.
pub struct InlineRenderer<W: Write> {
    out: W,
    prev_rows: u16,
    prev_lines: Vec<String>,
    diff_invalid: bool,
    hide_cursor: bool,
    sync: bool,
    clear_screen: bool,
    screen_cleared: bool,
    cursor_hidden: bool,
    finished: bool,
    last_width: Option<u16>,
}

impl<W: Write> InlineRenderer<W> {
    pub fn new(out: W) -> Self {
        InlineRenderer {
            out,
            prev_rows: 0,
            prev_lines: Vec::new(),
            diff_invalid: false,
            hide_cursor: false,
            sync: true,
            clear_screen: false,
            screen_cleared: false,
            cursor_hidden: false,
            finished: false,
            last_width: None,
        }
    }

    pub fn with_cursor_hidden(mut self, hide: bool) -> Self {
        self.hide_cursor = hide;
        self
    }

    pub fn with_sync_output(mut self, sync: bool) -> Self {
        self.sync = sync;
        self
    }

    /// Wipe the screen and home the cursor as part of the first frame.
    pub fn with_clear_screen(mut self, clear: bool) -> Self {
        self.clear_screen = clear;
        self
    }

    /// Repaint: one assembled string, one write, one flush (I8 — the
    /// synchronized frame never spans a blocking operation).
    pub fn draw(&mut self, lines: &[String], term_width: u16) -> std::io::Result<()> {
        let width_unchanged = self.last_width == Some(term_width);
        // A resize invalidates the wrap-derived row count (reflowing
        // terminals change how many rows the old frame occupies), so a
        // relative move-up can no longer be trusted: repaint from scratch,
        // re-wiping when this renderer owns the whole screen.
        if self.last_width.is_some_and(|w| w != term_width) {
            self.prev_rows = 0;
            if self.clear_screen {
                self.screen_cleared = false;
            }
        }
        self.last_width = Some(term_width);
        let mut bytes = String::new();
        if self.hide_cursor && !self.cursor_hidden {
            bytes.push_str("\x1b[?25l");
            self.cursor_hidden = true;
        }
        let wipe = self.clear_screen && !self.screen_cleared;
        self.screen_cleared = true;
        let diff = if width_unchanged
            && !wipe
            && !self.diff_invalid
            && self.prev_lines.len() == usize::from(self.prev_rows)
        {
            diff_bytes(&self.prev_lines, lines, term_width, self.sync)
        } else {
            None
        };
        match diff {
            Some(rewrite) => bytes.push_str(&rewrite),
            None => {
                bytes.push_str(&frame_bytes(
                    self.prev_rows,
                    lines,
                    term_width,
                    self.sync,
                    wipe,
                ));
                self.diff_invalid = false;
            }
        }
        self.out.write_all(bytes.as_bytes())?;
        self.out.flush()?;
        self.prev_rows = rendered_rows(lines, term_width);
        self.prev_lines = lines.to_vec();
        self.finished = false;
        Ok(())
    }

    #[cfg(test)]
    fn prev_lines(&self) -> &[String] {
        &self.prev_lines
    }

    /// Erase the current frame and forget it.
    // Consumed by the interactive UI loop, which lands with those commands.
    #[allow(dead_code)]
    pub fn clear(&mut self) -> std::io::Result<()> {
        let mut bytes = String::new();
        if self.prev_rows > 0 {
            bytes.push_str(&format!("\x1b[{}A", self.prev_rows));
        }
        bytes.push_str("\r\x1b[0J");
        self.out.write_all(bytes.as_bytes())?;
        self.out.flush()?;
        self.prev_rows = 0;
        self.prev_lines.clear();
        Ok(())
    }

    /// The pager returned and its alternate screen restored this
    /// renderer's own last frame, cursor below it. Keep the frame
    /// geometry so the next draw climbs over and replaces the restored
    /// copy inside one synchronized frame — forgetting it would paint a
    /// duplicate underneath — but re-arm the cursor and wipe state the
    /// foreign program disturbed.
    pub fn resume_over_own_frame(&mut self) {
        self.screen_cleared = false;
        self.cursor_hidden = false;
        self.finished = false;
        // The pager may have left anything on the frame's rows, so a
        // changed-rows rewrite cannot be trusted until a full repaint
        // reclaims them.
        self.diff_invalid = true;
    }

    /// Restore the cursor. Idempotent; also runs on drop.
    pub fn finish(&mut self) -> std::io::Result<()> {
        if self.finished {
            return Ok(());
        }
        self.finished = true;
        if self.cursor_hidden {
            self.out.write_all(b"\x1b[?25h")?;
            self.out.flush()?;
            self.cursor_hidden = false;
        }
        Ok(())
    }
}

impl<W: Write> Drop for InlineRenderer<W> {
    fn drop(&mut self) {
        let _ = self.finish();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn lines(items: &[&str]) -> Vec<String> {
        items.iter().map(|s| s.to_string()).collect()
    }

    #[test]
    fn rendered_rows_counts_wrapping() {
        assert_eq!(rendered_rows(&lines(&["x"]), 80), 1);
        assert_eq!(rendered_rows(&lines(&[&"a".repeat(200)]), 80), 3);
        assert_eq!(rendered_rows(&lines(&["a", "b", "c"]), 80), 3);
        assert_eq!(rendered_rows(&[], 80), 0);
    }

    #[test]
    fn rendered_rows_ignores_ansi_and_counts_display_width() {
        // Pure ANSI still occupies one row.
        assert_eq!(rendered_rows(&lines(&["\x1b[31m\x1b[0m"]), 80), 1);
        // Escapes are invisible: 5 visible chars in 80 cols is one row.
        assert_eq!(
            rendered_rows(&lines(&["\x1b[1;38;5;212mhello\x1b[0m"]), 80),
            1
        );
        // 50 CJK chars are 100 cells: two rows at 80.
        assert_eq!(rendered_rows(&lines(&[&"日".repeat(50)]), 80), 2);
        // Empty line still occupies a row.
        assert_eq!(rendered_rows(&lines(&[""]), 80), 1);
    }

    #[test]
    fn first_frame_has_no_move_up() {
        let bytes = frame_bytes(0, &lines(&["hi"]), 80, true, false);
        assert_eq!(bytes, "\x1b[?2026h\r\x1b[0Jhi\r\n\x1b[?2026l");
    }

    #[test]
    fn later_frames_move_up_by_previous_rows() {
        let bytes = frame_bytes(3, &lines(&["a", "b"]), 80, true, false);
        assert_eq!(bytes, "\x1b[?2026h\x1b[3A\r\x1b[0Ja\r\nb\r\n\x1b[?2026l");
    }

    #[test]
    fn no_sync_omits_2026() {
        let bytes = frame_bytes(1, &lines(&["a"]), 80, false, false);
        assert!(!bytes.contains("\x1b[?2026"));
        assert_eq!(bytes, "\x1b[1A\r\x1b[0Ja\r\n");
    }

    #[test]
    fn renderer_tracks_rows_across_frames_including_shrink() {
        let mut out: Vec<u8> = Vec::new();
        {
            let mut r = InlineRenderer::new(&mut out)
                .with_cursor_hidden(true)
                .with_sync_output(true);
            r.draw(&lines(&["a", "b", "c"]), 80).unwrap();
            r.draw(&lines(&["only"]), 80).unwrap();
            r.finish().unwrap();
            r.finish().unwrap(); // idempotent
        }
        let s = String::from_utf8(out).unwrap();
        let expected = concat!(
            "\x1b[?25l",
            "\x1b[?2026h\r\x1b[0Ja\r\nb\r\nc\r\n\x1b[?2026l",
            "\x1b[?2026h\x1b[3A\r\x1b[0Jonly\r\n\x1b[?2026l",
            "\x1b[?25h",
        );
        assert_eq!(s, expected);
    }

    #[test]
    fn resuming_over_the_restored_frame_replaces_it_in_place() {
        let mut out: Vec<u8> = Vec::new();
        {
            let mut r = InlineRenderer::new(&mut out)
                .with_cursor_hidden(true)
                .with_sync_output(true);
            r.draw(&lines(&["a", "b"]), 80).unwrap();
            r.finish().unwrap(); // the pager is about to own the screen
            r.resume_over_own_frame(); // its alternate screen restored our frame
            r.draw(&lines(&["c"]), 80).unwrap();
        }
        let s = String::from_utf8(out).unwrap();
        // The post-pager frame re-hides the cursor and climbs over the
        // restored two-row copy instead of painting below it.
        let expected = concat!(
            "\x1b[?25l",
            "\x1b[?2026h\r\x1b[0Ja\r\nb\r\n\x1b[?2026l",
            "\x1b[?25h",
            "\x1b[?25l",
            "\x1b[?2026h\x1b[2A\r\x1b[0Jc\r\n\x1b[?2026l",
            "\x1b[?25h", // drop restores the cursor
        );
        assert_eq!(s, expected);
    }

    #[test]
    fn the_renderer_retains_what_it_painted() {
        let mut out: Vec<u8> = Vec::new();
        let mut r = InlineRenderer::new(&mut out).with_sync_output(false);
        r.draw(&lines(&["a", "b"]), 80).unwrap();
        assert_eq!(r.prev_lines(), lines(&["a", "b"]).as_slice());
        r.draw(&lines(&["a", "b", "c"]), 80).unwrap();
        assert_eq!(r.prev_lines(), lines(&["a", "b", "c"]).as_slice());
    }

    #[test]
    fn a_width_change_still_resets_the_row_count() {
        let mut out: Vec<u8> = Vec::new();
        {
            let mut r = InlineRenderer::new(&mut out).with_sync_output(false);
            r.draw(&lines(&["a", "b"]), 80).unwrap();
            r.draw(&lines(&["x"]), 60).unwrap();
            assert_eq!(r.prev_lines(), lines(&["x"]).as_slice());
        }
        let s = String::from_utf8(out).unwrap();
        assert!(
            !s.contains("\x1b[2A"),
            "resize must repaint without a stale move-up: {s:?}"
        );
    }

    /// Net vertical cursor displacement across all CSI `A`/`B` moves.
    fn net_vertical(s: &str) -> i64 {
        let mut net = 0i64;
        let mut rest = s;
        while let Some(idx) = rest.find("\x1b[") {
            rest = &rest[idx + 2..];
            let digits: String = rest.chars().take_while(char::is_ascii_digit).collect();
            let n: i64 = digits.parse().unwrap_or(1);
            match rest[digits.len()..].chars().next() {
                Some('A') => net += n,
                Some('B') => net -= n,
                _ => {}
            }
        }
        net
    }

    #[test]
    fn an_identical_frame_writes_nothing() {
        let f = lines(&["a", "b", "c"]);
        assert_eq!(diff_bytes(&f, &f, 80, false), Some(String::new()));
    }

    #[test]
    fn one_changed_row_rewrites_one_row() {
        let prev = lines(&["a", "b", "c"]);
        let next = lines(&["a", "X", "c"]);
        let s = diff_bytes(&prev, &next, 80, false).expect("eligible");
        assert_eq!(s.matches("\x1b[2K").count(), 1, "got: {s:?}");
        assert!(s.contains('X'), "got: {s:?}");
        assert!(!s.contains('\n'), "a newline could scroll: {s:?}");
        assert!(!s.contains("\x1b[0J"), "got: {s:?}");
        assert_eq!(net_vertical(&s), 0, "must land on the resting line: {s:?}");
    }

    #[test]
    fn sync_markers_wrap_the_rewrite() {
        let prev = lines(&["a", "b"]);
        let next = lines(&["a", "X"]);
        let s = diff_bytes(&prev, &next, 80, true).expect("eligible");
        assert!(s.starts_with("\x1b[?2026h"), "got: {s:?}");
        assert!(s.ends_with("\x1b[?2026l"), "got: {s:?}");
    }

    #[test]
    fn too_much_change_declines() {
        // 2 of 3 rows changed: 67% > DIFF_MAX_CHANGED_PCT.
        let prev = lines(&["a", "b", "c"]);
        let next = lines(&["x", "y", "c"]);
        assert_eq!(diff_bytes(&prev, &next, 80, false), None);
    }

    #[test]
    fn unequal_lengths_decline() {
        let prev = lines(&["a", "b"]);
        let next = lines(&["a", "b", "c"]);
        assert_eq!(diff_bytes(&prev, &next, 80, false), None);
        assert_eq!(diff_bytes(&next, &prev, 80, false), None);
    }

    #[test]
    fn a_wrapped_row_declines() {
        // A line wider than the terminal occupies >1 row: not 1:1.
        let wide = "a".repeat(100);
        let prev = lines(&["a", "b"]);
        let next = lines(&[&wide, "b"]);
        assert_eq!(diff_bytes(&prev, &next, 80, false), None);
        assert_eq!(diff_bytes(&next, &prev, 80, false), None);
    }

    #[test]
    fn an_eligible_draw_takes_the_diff_path() {
        let mut out: Vec<u8> = Vec::new();
        {
            let mut r = InlineRenderer::new(&mut out).with_sync_output(false);
            r.draw(&lines(&["a", "b", "c"]), 80).unwrap();
            r.draw(&lines(&["a", "X", "c"]), 80).unwrap();
        }
        let s = String::from_utf8(out).unwrap();
        assert_eq!(
            s.matches("\x1b[0J").count(),
            1,
            "only the first draw wipes: {s:?}"
        );
        assert!(s.contains("\x1b[2K"), "got: {s:?}");
    }

    #[test]
    fn an_ineligible_draw_falls_back() {
        let mut out: Vec<u8> = Vec::new();
        {
            let mut r = InlineRenderer::new(&mut out).with_sync_output(false);
            r.draw(&lines(&["a", "b", "c"]), 80).unwrap();
            r.draw(&lines(&["a", "b"]), 80).unwrap();
        }
        let s = String::from_utf8(out).unwrap();
        assert_eq!(
            s.matches("\x1b[0J").count(),
            2,
            "a height change takes the full path: {s:?}"
        );
        assert!(!s.contains("\x1b[2K"), "got: {s:?}");
    }

    #[test]
    fn drop_restores_cursor() {
        let mut out: Vec<u8> = Vec::new();
        {
            let mut r = InlineRenderer::new(&mut out).with_cursor_hidden(true);
            r.draw(&lines(&["x"]), 80).unwrap();
        }
        let s = String::from_utf8(out).unwrap();
        assert!(s.ends_with("\x1b[?25h"), "drop must show cursor: {s:?}");
    }

    #[test]
    fn clear_erases_the_frame() {
        let mut out: Vec<u8> = Vec::new();
        {
            let mut r = InlineRenderer::new(&mut out).with_sync_output(false);
            r.draw(&lines(&["a", "b"]), 80).unwrap();
            r.clear().unwrap();
        }
        let s = String::from_utf8(out).unwrap();
        assert!(s.ends_with("\x1b[2A\r\x1b[0J"), "got: {s:?}");
    }

    #[test]
    fn wrapped_lines_move_up_by_rendered_rows_not_line_count() {
        let mut out: Vec<u8> = Vec::new();
        {
            let mut r = InlineRenderer::new(&mut out).with_sync_output(false);
            r.draw(&lines(&[&"a".repeat(200)]), 80).unwrap(); // 3 rows
            r.draw(&lines(&["short"]), 80).unwrap();
        }
        let s = String::from_utf8(out).unwrap();
        assert!(s.contains("\x1b[3A"), "second frame must move up 3: {s:?}");
    }
}

#[cfg(test)]
mod truncate_tests {
    use super::*;

    fn lines(items: &[&str]) -> Vec<String> {
        items.iter().map(|s| s.to_string()).collect()
    }

    #[test]
    fn truncate_keeps_leading_rows_and_counts_hidden() {
        let input = lines(&["a", "b", "c", "d"]);
        let (kept, hidden) = truncate_to_rows(input.clone(), 2, 80);
        assert_eq!(kept, lines(&["a", "b"]));
        assert_eq!(hidden, 2);
        let (kept, hidden) = truncate_to_rows(input, 10, 80);
        assert_eq!(hidden, 0);
        assert_eq!(kept.len(), 4);
    }

    #[test]
    fn truncate_is_wrap_aware() {
        // One 200-char line is 3 rows at width 80: it alone exceeds 2 rows.
        let input = lines(&[&"a".repeat(200), "tail"]);
        let (kept, hidden) = truncate_to_rows(input, 2, 80);
        assert!(kept.is_empty());
        assert_eq!(hidden, 2);
    }
}

/// Keep the leading lines that fit in `max_rows` rendered rows; return the
/// kept lines and how many were hidden. Dashboards lead with the headline,
/// so the head is kept rather than the tail.
pub fn truncate_to_rows(
    lines: Vec<String>,
    max_rows: u16,
    term_width: u16,
) -> (Vec<String>, usize) {
    let mut used: u16 = 0;
    let mut kept = Vec::new();
    let total = lines.len();
    for line in lines {
        let rows = rendered_rows(std::slice::from_ref(&line), term_width);
        if used.saturating_add(rows) > max_rows {
            break;
        }
        used += rows;
        kept.push(line);
    }
    let hidden = total - kept.len();
    (kept, hidden)
}

#[cfg(test)]
mod clear_screen_tests {
    use super::*;

    fn lines(items: &[&str]) -> Vec<String> {
        items.iter().map(|s| s.to_string()).collect()
    }

    #[test]
    fn clear_screen_frame_wipes_and_homes_inside_the_sync_frame() {
        let bytes = frame_bytes(0, &lines(&["hi"]), 80, true, true);
        assert_eq!(bytes, "\x1b[?2026h\x1b[2J\x1b[H\r\x1b[0Jhi\r\n\x1b[?2026l");
    }

    #[test]
    fn clear_screen_skips_move_up() {
        let bytes = frame_bytes(5, &lines(&["hi"]), 80, true, true);
        assert!(!bytes.contains("\x1b[5A"), "got: {bytes:?}");
        assert!(bytes.contains("\x1b[2J\x1b[H"));
    }

    #[test]
    fn renderer_clears_screen_only_on_the_first_frame() {
        let mut out: Vec<u8> = Vec::new();
        {
            let mut r = InlineRenderer::new(&mut out)
                .with_sync_output(true)
                .with_clear_screen(true);
            r.draw(&lines(&["a"]), 80).unwrap();
            r.draw(&lines(&["b"]), 80).unwrap();
        }
        let s = String::from_utf8(out).unwrap();
        assert_eq!(s.matches("\x1b[2J").count(), 1, "got: {s:?}");
        assert!(s.contains("\x1b[1A"), "second frame still moves up: {s:?}");
    }
}

#[cfg(test)]
mod resize_tests {
    use super::*;

    fn lines(items: &[&str]) -> Vec<String> {
        items.iter().map(|s| s.to_string()).collect()
    }

    #[test]
    fn width_change_repaints_without_move_up() {
        let mut out: Vec<u8> = Vec::new();
        {
            let mut r = InlineRenderer::new(&mut out).with_sync_output(false);
            r.draw(&lines(&["a", "b", "c"]), 100).unwrap();
            r.draw(&lines(&["a", "b", "c"]), 60).unwrap();
        }
        let s = String::from_utf8(out).unwrap();
        assert!(
            !s.contains("\x1b[3A"),
            "stale move-up after resize corrupts a reflowed screen: {s:?}"
        );
    }

    #[test]
    fn width_change_rewipes_in_clear_screen_mode() {
        let mut out: Vec<u8> = Vec::new();
        {
            let mut r = InlineRenderer::new(&mut out)
                .with_sync_output(false)
                .with_clear_screen(true);
            r.draw(&lines(&["a"]), 100).unwrap();
            r.draw(&lines(&["a"]), 60).unwrap();
        }
        let s = String::from_utf8(out).unwrap();
        assert_eq!(
            s.matches("\x1b[2J").count(),
            2,
            "resize must reclaim the screen with a fresh wipe: {s:?}"
        );
    }

    #[test]
    fn stable_width_keeps_normal_move_up() {
        let mut out: Vec<u8> = Vec::new();
        {
            let mut r = InlineRenderer::new(&mut out).with_sync_output(false);
            r.draw(&lines(&["a", "b"]), 80).unwrap();
            r.draw(&lines(&["c"]), 80).unwrap();
        }
        let s = String::from_utf8(out).unwrap();
        assert!(s.contains("\x1b[2A"), "got: {s:?}");
    }
}
