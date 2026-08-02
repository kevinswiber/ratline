use ratatui::buffer::Buffer;
use ratatui::style::{Color, Modifier};

use crate::color::ColorProfile;
use crate::style_spec::StyleSpec;

fn cell_spec(cell: &ratatui::buffer::Cell) -> StyleSpec {
    StyleSpec {
        bold: cell.modifier.contains(Modifier::BOLD),
        faint: cell.modifier.contains(Modifier::DIM),
        italic: cell.modifier.contains(Modifier::ITALIC),
        underline: cell.modifier.contains(Modifier::UNDERLINED),
        reverse: cell.modifier.contains(Modifier::REVERSED),
        strikethrough: cell.modifier.contains(Modifier::CROSSED_OUT),
        foreground: (cell.fg != Color::Reset).then_some(cell.fg),
        background: (cell.bg != Color::Reset).then_some(cell.bg),
    }
}

/// Convert a rendered ratatui buffer into ANSI lines for the inline
/// renderer: SGR emitted only on style change, reset closed per line,
/// trailing unstyled blanks trimmed.
pub fn buffer_to_lines(buf: &Buffer, profile: ColorProfile) -> Vec<String> {
    let area = buf.area;
    let mut lines = Vec::with_capacity(usize::from(area.height));
    for y in area.top()..area.bottom() {
        let mut line = String::new();
        let mut open_prefix: Option<String> = None;
        let mut pending_blanks = String::new();
        for x in area.left()..area.right() {
            let Some(cell) = buf.cell((x, y)) else {
                continue;
            };
            let spec = cell_spec(cell);
            let prefix = spec.sgr_prefix(profile);
            let prefix = (!prefix.is_empty()).then_some(prefix);
            // Trailing unstyled spaces are held back and dropped at line end.
            if cell.symbol() == " " && prefix.is_none() {
                if open_prefix.is_some() {
                    line.push_str("\x1b[0m");
                    open_prefix = None;
                }
                pending_blanks.push(' ');
                continue;
            }
            if prefix != open_prefix {
                if open_prefix.is_some() {
                    line.push_str("\x1b[0m");
                }
                line.push_str(&pending_blanks);
                pending_blanks.clear();
                if let Some(p) = &prefix {
                    line.push_str(&format!("\x1b[{p}m"));
                }
                open_prefix = prefix;
            } else {
                line.push_str(&pending_blanks);
                pending_blanks.clear();
            }
            line.push_str(cell.symbol());
        }
        if open_prefix.is_some() {
            line.push_str("\x1b[0m");
        }
        lines.push(line);
    }
    lines
}

#[cfg(test)]
mod tests {
    use ratatui::buffer::Buffer;
    use ratatui::layout::Rect;
    use ratatui::style::{Color, Modifier, Style};

    use super::*;
    use crate::color::ColorProfile;

    fn buf_with(text: &str, style: Option<Style>) -> Buffer {
        let area = Rect::new(0, 0, text.len() as u16, 1);
        let mut buf = Buffer::empty(area);
        buf.set_string(0, 0, text, style.unwrap_or_default());
        buf
    }

    #[test]
    fn plain_buffer_renders_plain_lines() {
        let buf = buf_with("abc", None);
        assert_eq!(buffer_to_lines(&buf, ColorProfile::Ascii), vec!["abc"]);
        assert_eq!(buffer_to_lines(&buf, ColorProfile::TrueColor), vec!["abc"]);
    }

    #[test]
    fn styled_cells_emit_sgr_with_reset() {
        let style = Style::default().fg(Color::Red).add_modifier(Modifier::BOLD);
        let buf = buf_with("hi", Some(style));
        let lines = buffer_to_lines(&buf, ColorProfile::Ansi256);
        assert_eq!(lines, vec!["\x1b[1;31mhi\x1b[0m"]);
    }

    #[test]
    fn adjacent_same_style_cells_share_one_prefix() {
        let style = Style::default().fg(Color::Red);
        let buf = buf_with("aaa", Some(style));
        let lines = buffer_to_lines(&buf, ColorProfile::Ansi256);
        assert_eq!(
            lines[0].matches("\x1b[31m").count(),
            1,
            "got: {:?}",
            lines[0]
        );
    }

    #[test]
    fn style_changes_mid_line_reset_and_reopen() {
        let area = Rect::new(0, 0, 2, 1);
        let mut buf = Buffer::empty(area);
        buf.set_string(0, 0, "a", Style::default().fg(Color::Red));
        buf.set_string(1, 0, "b", Style::default());
        let lines = buffer_to_lines(&buf, ColorProfile::Ansi256);
        assert_eq!(lines, vec!["\x1b[31ma\x1b[0mb"]);
    }

    #[test]
    fn ascii_profile_emits_zero_escapes() {
        let style = Style::default().fg(Color::Red).add_modifier(Modifier::BOLD);
        let buf = buf_with("x", Some(style));
        let lines = buffer_to_lines(&buf, ColorProfile::Ascii);
        assert_eq!(lines, vec!["x"]);
    }

    #[test]
    fn trailing_blank_cells_are_trimmed() {
        let area = Rect::new(0, 0, 10, 1);
        let mut buf = Buffer::empty(area);
        buf.set_string(0, 0, "hi", ratatui::style::Style::default());
        let lines = buffer_to_lines(&buf, ColorProfile::Ascii);
        assert_eq!(lines, vec!["hi"]);
    }

    #[test]
    fn a_reversed_cell_emits_sgr_7() {
        let style = Style::default().add_modifier(Modifier::REVERSED);
        let buf = buf_with("x", Some(style));
        let lines = buffer_to_lines(&buf, ColorProfile::Ansi256);
        assert_eq!(lines, vec!["\x1b[7mx\x1b[0m"]);
    }

    #[test]
    fn a_trailing_reversed_blank_survives_the_trim() {
        // The caret one past the end of the line is a reversed space; it
        // must reach the terminal, not fall to the trailing-blank trim.
        let area = Rect::new(0, 0, 10, 1);
        let mut buf = Buffer::empty(area);
        buf.set_string(0, 0, "hi", Style::default());
        buf.set_string(2, 0, " ", Style::default().add_modifier(Modifier::REVERSED));
        let lines = buffer_to_lines(&buf, ColorProfile::Ansi256);
        assert_eq!(lines, vec!["hi\x1b[7m \x1b[0m"]);
    }
}
