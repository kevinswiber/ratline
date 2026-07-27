// Consumed by the layout commands, which land with their wiring.
#![allow(dead_code)]

use unicode_width::UnicodeWidthChar;

/// Default truncation marker: one display cell.
pub const ELLIPSIS: &str = "…";

/// Horizontal placement inside a fixed-width column.
#[derive(Copy, Clone, PartialEq, Eq, Debug, Default, clap::ValueEnum)]
pub enum Align {
    #[default]
    Left,
    Right,
    Center,
}

/// One step of an ANSI-aware walk over a string.
#[derive(Copy, Clone, PartialEq, Eq, Debug)]
pub enum Chunk<'a> {
    /// An escape sequence: zero display cells, never split.
    Escape(&'a str),
    /// A printable character and the cells it occupies.
    Text(&'a str, usize),
}

/// Iterator over [`Chunk`]s. Returned by [`chunks`].
pub struct Chunks<'a> {
    rest: &'a str,
}

impl<'a> Iterator for Chunks<'a> {
    type Item = Chunk<'a>;

    fn next(&mut self) -> Option<Chunk<'a>> {
        if self.rest.is_empty() {
            return None;
        }
        let bytes = self.rest.as_bytes();
        if bytes[0] == 0x1b {
            let end = escape_len(bytes);
            let (escape, rest) = self.rest.split_at(end);
            self.rest = rest;
            return Some(Chunk::Escape(escape));
        }
        let c = self.rest.chars().next().expect("non-empty");
        let (text, rest) = self.rest.split_at(c.len_utf8());
        self.rest = rest;
        Some(Chunk::Text(text, UnicodeWidthChar::width(c).unwrap_or(0)))
    }
}

/// Bytes an escape sequence starting at `bytes[0] == ESC` occupies. An
/// unterminated sequence swallows the remainder rather than leaking bytes
/// into width math.
fn escape_len(bytes: &[u8]) -> usize {
    match bytes.get(1) {
        // CSI: parameters and intermediates end at a final byte in @..=~.
        Some(b'[') => {
            let mut i = 2;
            while i < bytes.len() {
                if (0x40..=0x7e).contains(&bytes[i]) {
                    return i + 1;
                }
                i += 1;
            }
            bytes.len()
        }
        // OSC: terminated by BEL or ST (ESC \).
        Some(b']') => {
            let mut i = 2;
            while i < bytes.len() {
                if bytes[i] == 0x07 {
                    return i + 1;
                }
                if bytes[i] == 0x1b && bytes.get(i + 1) == Some(&b'\\') {
                    return i + 2;
                }
                i += 1;
            }
            bytes.len()
        }
        Some(_) => 2,
        None => 1,
    }
}

/// Walk a string as escape sequences and printable characters.
pub fn chunks(s: &str) -> Chunks<'_> {
    Chunks { rest: s }
}

/// Display cells a string occupies; escape sequences count as zero.
pub fn display_width(s: &str) -> usize {
    chunks(s)
        .map(|chunk| match chunk {
            Chunk::Escape(_) => 0,
            Chunk::Text(_, w) => w,
        })
        .sum()
}

/// Split at the last position fitting in `max` display cells. Escape
/// sequences never straddle the split and trailing zero-width escapes stay
/// with the head; a wide character that would straddle goes to the tail.
pub fn split_at_width(s: &str, max: usize) -> (&str, &str) {
    let mut used = 0;
    let mut end = 0;
    for chunk in chunks(s) {
        match chunk {
            // Zero-width escapes between kept text ride with the head; one
            // after the overflow point is never reached.
            Chunk::Escape(e) => end += e.len(),
            Chunk::Text(t, w) => {
                if used + w > max {
                    return s.split_at(end);
                }
                used += w;
                end += t.len();
            }
        }
    }
    s.split_at(end)
}

/// Pad to `width` display cells with spaces. Strings already at or over
/// `width` are returned untouched — padding never truncates.
pub fn pad_display(s: &str, width: usize, align: Align) -> String {
    let current = display_width(s);
    if current >= width {
        return s.to_string();
    }
    let missing = width - current;
    match align {
        Align::Left => format!("{s}{}", " ".repeat(missing)),
        Align::Right => format!("{}{s}", " ".repeat(missing)),
        Align::Center => {
            let left = missing / 2;
            format!("{}{s}{}", " ".repeat(left), " ".repeat(missing - left))
        }
    }
}

/// Keep the leading `max` display cells, appending `marker` inside the
/// budget when anything is dropped ("" for a hard cut) and a reset when SGR
/// was left open.
pub fn truncate_display(s: &str, max: usize, marker: &str) -> String {
    if display_width(s) <= max {
        return s.to_string();
    }
    let marker_width = display_width(marker);
    if marker_width >= max {
        let (head, _) = split_at_width(s, max);
        return head.to_string();
    }
    let (head, _) = split_at_width(s, max - marker_width);
    let mut out = format!("{head}{marker}");
    if sgr_left_open(head) {
        out.push_str("\x1b[0m");
    }
    out
}

/// The SGR sequences a prefix of a string leaves open.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct SgrState {
    open: Vec<String>,
}

impl SgrState {
    /// Feed one escape sequence; non-SGR escapes are ignored.
    pub fn apply(&mut self, escape: &str) {
        if escape == "\x1b[0m" || escape == "\x1b[m" {
            self.open.clear();
        } else if escape.starts_with("\x1b[") && escape.ends_with('m') {
            self.open.push(escape.to_string());
        }
    }

    /// Feed every escape sequence in a string.
    fn apply_all(&mut self, s: &str) {
        for chunk in chunks(s) {
            if let Chunk::Escape(e) = chunk {
                self.apply(e);
            }
        }
    }

    /// The sequence that reopens this state, or "" when nothing is open.
    pub fn prefix(&self) -> String {
        self.open.concat()
    }

    /// Whether a reset is needed to close what is open.
    pub fn is_open(&self) -> bool {
        !self.open.is_empty()
    }
}

fn sgr_left_open(s: &str) -> bool {
    let mut state = SgrState::default();
    state.apply_all(s);
    state.is_open()
}

/// Break one line into lines of at most `width` display cells, preferring
/// spaces and hard-breaking over-long words. SGR open at a break is closed
/// at the line end and reopened on the next line.
pub fn wrap_display(s: &str, width: usize) -> Vec<String> {
    if width == 0 {
        return vec![s.to_string()];
    }
    let mut out = Vec::new();
    let mut state = SgrState::default();
    let mut emit = |line: &str, state: &mut SgrState| {
        let mut built = format!("{}{line}", state.prefix());
        state.apply_all(line);
        if state.is_open() {
            built.push_str("\x1b[0m");
        }
        out.push(built);
    };
    let mut rest = s;
    loop {
        if display_width(rest) <= width {
            emit(rest, &mut state);
            break;
        }
        let (head, tail) = split_at_width(rest, width);
        let tail_starts_with_space = matches!(chunks(tail).next(), Some(Chunk::Text(" ", _)));
        let line_end = if tail_starts_with_space {
            head.len()
        } else {
            last_space_offset(head)
                .filter(|&pos| pos > 0)
                .unwrap_or(head.len())
        };
        emit(rest[..line_end].trim_end_matches(' '), &mut state);
        rest = skip_leading_spaces(&rest[line_end..]);
    }
    out
}

/// Byte offset of the last plain-space chunk, if any.
fn last_space_offset(s: &str) -> Option<usize> {
    let mut offset = 0;
    let mut last = None;
    for chunk in chunks(s) {
        match chunk {
            Chunk::Escape(e) => offset += e.len(),
            Chunk::Text(t, _) => {
                if t == " " {
                    last = Some(offset);
                }
                offset += t.len();
            }
        }
    }
    last
}

/// Drop leading plain-space chunks. Stops at the first escape so a style
/// change opening the next word is never discarded.
fn skip_leading_spaces(s: &str) -> &str {
    let mut offset = 0;
    for chunk in chunks(s) {
        match chunk {
            Chunk::Text(" ", _) => offset += 1,
            _ => break,
        }
    }
    &s[offset..]
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn display_width_ignores_escapes() {
        assert_eq!(display_width("hello"), 5);
        assert_eq!(display_width("\x1b[1;38;5;212mhello\x1b[0m"), 5);
        assert_eq!(display_width("日本"), 4);
        assert_eq!(display_width(""), 0);
        assert_eq!(display_width("\x1b[31m\x1b[0m"), 0);
    }

    #[test]
    fn display_width_agrees_with_stripping() {
        use unicode_width::UnicodeWidthStr;
        for s in [
            "plain",
            "\x1b[1mbold\x1b[0m",
            "\x1b[38;2;255;0;0mrgb\x1b[0m tail",
            "日本語\x1b[0m",
            "\x1b]0;title\x07after",
        ] {
            assert_eq!(
                display_width(s),
                strip_ansi_escapes::strip_str(s).as_str().width(),
                "mismatch for {s:?}"
            );
        }
    }

    #[test]
    fn pad_display_fills_to_display_cells() {
        assert_eq!(pad_display("ab", 5, Align::Left), "ab   ");
        assert_eq!(pad_display("ab", 5, Align::Right), "   ab");
        assert_eq!(pad_display("ab", 5, Align::Center), " ab  ");
        assert_eq!(pad_display("日本", 6, Align::Left), "日本  ");
        assert_eq!(
            pad_display("\x1b[31mab\x1b[0m", 4, Align::Left),
            "\x1b[31mab\x1b[0m  "
        );
        assert_eq!(pad_display("abcdef", 3, Align::Left), "abcdef");
    }

    #[test]
    fn split_at_width_never_splits_an_escape() {
        assert_eq!(
            split_at_width("\x1b[31mabcd\x1b[0m", 2),
            ("\x1b[31mab", "cd\x1b[0m")
        );
        // A closing escape rides along with the text it closes.
        assert_eq!(split_at_width("ab\x1b[0mcd", 2), ("ab\x1b[0m", "cd"));
    }

    #[test]
    fn split_at_width_puts_a_straddling_wide_char_in_the_tail() {
        assert_eq!(split_at_width("a日本", 2), ("a", "日本"));
    }

    #[test]
    fn truncate_display_adds_the_marker_inside_the_budget() {
        assert_eq!(truncate_display("abcdef", 4, "…"), "abc…");
        assert_eq!(truncate_display("abc", 4, "…"), "abc");
        assert_eq!(truncate_display("abcdef", 4, ""), "abcd");
        assert_eq!(truncate_display("abcdef", 1, "…"), "a");
    }

    #[test]
    fn truncate_display_closes_open_styling() {
        assert_eq!(
            truncate_display("\x1b[31mabcdef\x1b[0m", 4, "…"),
            "\x1b[31mabc…\x1b[0m"
        );
    }

    #[test]
    fn sgr_state_tracks_open_codes() {
        let mut state = SgrState::default();
        state.apply("\x1b[31m");
        assert!(state.is_open());
        assert_eq!(state.prefix(), "\x1b[31m");
        state.apply("\x1b[1m");
        assert_eq!(state.prefix(), "\x1b[31m\x1b[1m");
        state.apply("\x1b[0m");
        assert!(!state.is_open());
        assert_eq!(state.prefix(), "");
        state.apply("\x1b]0;title\x07"); // non-SGR escapes are ignored
        assert!(!state.is_open());
    }

    #[test]
    fn wrap_breaks_at_spaces() {
        assert_eq!(
            wrap_display("the quick brown fox", 10),
            vec!["the quick", "brown fox"]
        );
    }

    #[test]
    fn wrap_hard_breaks_a_long_word() {
        assert_eq!(wrap_display("abcdefghij", 4), vec!["abcd", "efgh", "ij"]);
    }

    #[test]
    fn wrap_lines_never_exceed_the_width() {
        for line in wrap_display("日本語 mixed ちゃんと wrapping", 7) {
            assert!(display_width(&line) <= 7, "too wide: {line:?}");
        }
    }

    #[test]
    fn wrap_reopens_styling_on_each_line() {
        assert_eq!(
            wrap_display("\x1b[31mthe quick brown\x1b[0m", 9),
            vec!["\x1b[31mthe quick\x1b[0m", "\x1b[31mbrown\x1b[0m"]
        );
    }

    #[test]
    fn wrap_keeps_an_escape_that_starts_the_next_word() {
        assert_eq!(
            wrap_display("aa \x1b[31mbb\x1b[0m", 2),
            vec!["aa", "\x1b[31mbb\x1b[0m"]
        );
    }

    #[test]
    fn wrap_of_empty_and_zero_width_terminates() {
        assert_eq!(wrap_display("", 5), vec![String::new()]);
        assert_eq!(wrap_display("abc", 0), vec!["abc".to_string()]);
    }
}
