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
/// Layout: BSU (if sync) - move up over the previous frame - carriage return -
/// clear to end of screen - lines terminated with CRLF - ESU (if sync). CRLF
/// keeps output correct in raw mode, where LF alone does not return the column.
pub fn frame_bytes(prev_rows: u16, lines: &[String], _term_width: u16, sync: bool) -> String {
    let mut out = String::new();
    if sync {
        out.push_str("\x1b[?2026h");
    }
    if prev_rows > 0 {
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

/// Repaints blocks of pre-rendered ANSI lines in place. Generic over the
/// writer so tests assert exact bytes against a Vec<u8>.
pub struct InlineRenderer<W: Write> {
    out: W,
    prev_rows: u16,
    hide_cursor: bool,
    sync: bool,
    cursor_hidden: bool,
    finished: bool,
}

impl<W: Write> InlineRenderer<W> {
    pub fn new(out: W) -> Self {
        InlineRenderer {
            out,
            prev_rows: 0,
            hide_cursor: false,
            sync: true,
            cursor_hidden: false,
            finished: false,
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

    /// Repaint: one assembled string, one write, one flush (I8 — the
    /// synchronized frame never spans a blocking operation).
    pub fn draw(&mut self, lines: &[String], term_width: u16) -> std::io::Result<()> {
        let mut bytes = String::new();
        if self.hide_cursor && !self.cursor_hidden {
            bytes.push_str("\x1b[?25l");
            self.cursor_hidden = true;
        }
        bytes.push_str(&frame_bytes(self.prev_rows, lines, term_width, self.sync));
        self.out.write_all(bytes.as_bytes())?;
        self.out.flush()?;
        self.prev_rows = rendered_rows(lines, term_width);
        self.finished = false;
        Ok(())
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
        Ok(())
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
        let bytes = frame_bytes(0, &lines(&["hi"]), 80, true);
        assert_eq!(bytes, "\x1b[?2026h\r\x1b[0Jhi\r\n\x1b[?2026l");
    }

    #[test]
    fn later_frames_move_up_by_previous_rows() {
        let bytes = frame_bytes(3, &lines(&["a", "b"]), 80, true);
        assert_eq!(bytes, "\x1b[?2026h\x1b[3A\r\x1b[0Ja\r\nb\r\n\x1b[?2026l");
    }

    #[test]
    fn no_sync_omits_2026() {
        let bytes = frame_bytes(1, &lines(&["a"]), 80, false);
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
