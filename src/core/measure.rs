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

/// Whether the string ends with SGR styling still open (an SGR sequence seen
/// after the last reset).
fn sgr_left_open(s: &str) -> bool {
    let mut open = false;
    for chunk in chunks(s) {
        if let Chunk::Escape(e) = chunk {
            if e == "\x1b[0m" || e == "\x1b[m" {
                open = false;
            } else if e.starts_with("\x1b[") && e.ends_with('m') {
                open = true;
            }
        }
    }
    open
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
}
