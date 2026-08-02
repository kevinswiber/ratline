use ratatui::buffer::Buffer;
use ratatui::layout::Rect;
use ratatui::style::{Modifier, Style};
use unicode_width::UnicodeWidthStr;

use crate::cli::InputArgs;
use crate::color::ColorProfile;
use crate::core::duration::parse_interval;
use crate::exit::AppResult;
use crate::theme::Palette;
use crate::ui::input::InputState;
use crate::ui::key::Key;
use crate::ui::loop_::{Outcome, UiApp, run_ui};

struct InputApp {
    state: InputState,
    prompt: String,
    placeholder: String,
    header: Option<String>,
    palette: Palette,
}

impl UiApp for InputApp {
    fn on_key(&mut self, key: Key) -> Outcome {
        self.state.on_key(key)
    }

    fn render(&self, _area: Rect, buf: &mut Buffer) {
        let mut y = 0;
        if let Some(header) = &self.header {
            buf.set_string(0, y, header, Style::default().add_modifier(Modifier::BOLD));
            y += 1;
        }
        let accent = Style::default().fg(self.palette.accent);
        buf.set_string(0, y, &self.prompt, accent);
        // Cell width, not char count: a wide prompt occupies more cells
        // than chars, and the text starts after the cells.
        let x = self.prompt.as_str().width() as u16;
        let text = self.state.display(&self.placeholder);
        let style = if self.state.value.is_empty() {
            Style::default()
                .add_modifier(Modifier::DIM)
                .fg(self.palette.placeholder)
        } else {
            Style::default()
        };
        buf.set_string(x, y, &text, style);
        // No painted caret: the terminal's own cursor, parked on the edit
        // point via cursor_pos, is the one and only caret — the doubled
        // look read as two cursors, and only the real one serves
        // assistive tech. The `cursor` theme token stays dormant.
    }

    fn height(&self, _term: (u16, u16)) -> u16 {
        1 + u16::from(self.header.is_some())
    }

    fn cursor_pos(&self) -> Option<(u16, u16)> {
        // Cells, not chars: the renderer consumes the column as terminal
        // cells, and wide glyphs occupy two. An empty value keeps the
        // caret at the text start — never inside the placeholder.
        let row = u16::from(self.header.is_some());
        let mut col = self.prompt.as_str().width();
        if !self.state.value.is_empty() {
            let text = self.state.display(&self.placeholder);
            let before: String = text.chars().take(self.state.cursor).collect();
            col += before.width();
        }
        Some((col as u16, row))
    }
}

pub fn run(args: InputArgs, profile: ColorProfile, palette: Palette) -> AppResult {
    let mut app = InputApp {
        state: InputState::new(args.value.clone(), args.password, args.char_limit),
        prompt: args.prompt.clone(),
        placeholder: args.placeholder.clone(),
        header: args.header.clone(),
        palette,
    };
    let timeout = args.timeout.as_deref().map(parse_interval).transpose()?;
    run_ui(&mut app, profile, timeout)?;
    println!("{}", app.state.value);
    Ok(())
}

#[cfg(test)]
mod tests {
    use ratatui::style::Color;

    use super::*;
    use crate::theme::{Appearance, AppearanceSource};

    fn prompt_fg(palette: Palette) -> Color {
        let app = InputApp {
            state: InputState::new(String::new(), false, 1000),
            prompt: "> ".into(),
            placeholder: "type".into(),
            header: None,
            palette,
        };
        let area = Rect::new(0, 0, 40, 2);
        let mut buf = Buffer::empty(area);
        app.render(area, &mut buf);
        buf.cell((0, 0)).expect("prompt is painted").fg
    }

    fn rendered(palette: Palette, value: &str, cursor: usize) -> Vec<String> {
        let mut state = InputState::new(value.to_string(), false, 1000);
        state.cursor = cursor;
        let app = InputApp {
            state,
            prompt: "> ".into(),
            placeholder: "type".into(),
            header: None,
            palette,
        };
        let area = Rect::new(0, 0, 40, 2);
        let mut buf = Buffer::empty(area);
        app.render(area, &mut buf);
        crate::term::buffer_ansi::buffer_to_lines(&buf, ColorProfile::Ansi256)
    }

    #[test]
    fn the_rendered_input_frame_is_pinned() {
        // Byte-identity goldens. The frame paints NO caret cell: the caret
        // is the terminal's own cursor, parked on the edit point by the
        // renderer — the only caret assistive tech can track, and one
        // caret is one caret (the doubled look read as two cursors).
        let dark = Palette::builtin(Appearance::Dark, AppearanceSource::Default);
        let light = Palette::builtin(Appearance::Light, AppearanceSource::Default);
        assert_eq!(
            rendered(dark, "", 0),
            ["\u{1b}[38;5;212m> \u{1b}[0m\u{1b}[2mtype\u{1b}[0m", ""]
        );
        assert_eq!(
            rendered(light, "", 0),
            ["\u{1b}[38;5;129m> \u{1b}[0m\u{1b}[2mtype\u{1b}[0m", ""]
        );
        assert_eq!(
            rendered(dark, "abc", 1),
            ["\u{1b}[38;5;212m> \u{1b}[0mabc", ""]
        );
        assert_eq!(
            rendered(light, "abc", 1),
            ["\u{1b}[38;5;129m> \u{1b}[0mabc", ""]
        );
    }

    fn cell_at(palette: Palette, value: &str, cursor: usize, x: u16) -> ratatui::buffer::Cell {
        let mut state = InputState::new(value.to_string(), false, 1000);
        state.cursor = cursor;
        let app = InputApp {
            state,
            prompt: "> ".into(),
            placeholder: "type".into(),
            header: None,
            palette,
        };
        let area = Rect::new(0, 0, 40, 2);
        let mut buf = Buffer::empty(area);
        app.render(area, &mut buf);
        buf.cell((x, 0)).expect("cell is painted").clone()
    }

    #[test]
    fn the_placeholder_reads_the_placeholder_token() {
        // Sentinel: at the default the token is Reset (emits nothing), so
        // only a diverging palette proves the render reads the field.
        let palette = Palette {
            placeholder: Color::Indexed(97),
            ..Palette::builtin(Appearance::Dark, AppearanceSource::Default)
        };
        let cell = cell_at(palette, "", 0, 2);
        assert_eq!(cell.fg, Color::Indexed(97));
        // DIM is what keeps the default byte-identical.
        assert!(cell.modifier.contains(Modifier::DIM));
    }

    #[test]
    fn no_frame_shape_paints_a_caret_cell() {
        // Past-end, mid-string, start: every cursor position leaves the
        // frame free of SGR 7 — the hardware cursor is the only caret.
        let dark = Palette::builtin(Appearance::Dark, AppearanceSource::Default);
        for cursor in [0, 1, 3] {
            for line in rendered(dark, "abc", cursor) {
                assert!(
                    !line.contains("\u{1b}[7m"),
                    "cursor {cursor} painted a caret: {line:?}"
                );
            }
        }
    }

    #[test]
    fn a_diverging_cursor_token_changes_nothing() {
        // The cursor token is dormant: the caret is the terminal's own
        // cursor, so no palette value may reach the frame through it.
        let default = Palette::builtin(Appearance::Dark, AppearanceSource::Default);
        let diverging = Palette {
            cursor: Color::Indexed(96),
            ..Palette::builtin(Appearance::Dark, AppearanceSource::Default)
        };
        assert_eq!(rendered(default, "abc", 1), rendered(diverging, "abc", 1));
        assert_eq!(rendered(default, "abc", 3), rendered(diverging, "abc", 3));
    }

    #[test]
    fn the_cursor_pos_tracks_the_caret_cell() {
        let mut state = InputState::new("abc".to_string(), false, 1000);
        state.cursor = 1;
        let app = InputApp {
            state,
            prompt: "> ".into(),
            placeholder: "type".into(),
            header: None,
            palette: Palette::builtin(Appearance::Dark, AppearanceSource::Default),
        };
        // Prompt is 2 wide, caret at char 1: column 3, row 0.
        assert_eq!(app.cursor_pos(), Some((3, 0)));
    }

    #[test]
    fn the_cursor_pos_starts_at_the_text_column_and_clears_a_header() {
        // Empty value: the edit point is still real — column = prompt
        // width — and a header pushes the input row down by one.
        let app = InputApp {
            state: InputState::new(String::new(), false, 1000),
            prompt: "> ".into(),
            placeholder: "type".into(),
            header: Some("H".into()),
            palette: Palette::builtin(Appearance::Dark, AppearanceSource::Default),
        };
        assert_eq!(app.cursor_pos(), Some((2, 1)));
    }

    #[test]
    fn the_cursor_pos_counts_display_cells_not_chars() {
        // "日" is one char but two terminal cells; the renderer consumes
        // the column as cells. Wide prompt + wide value both count.
        let mut state = InputState::new("日本".to_string(), false, 1000);
        state.cursor = 1;
        let app = InputApp {
            state,
            prompt: "日 ".into(),
            placeholder: "type".into(),
            header: None,
            palette: Palette::builtin(Appearance::Dark, AppearanceSource::Default),
        };
        // Prompt "日 " is 3 cells; the char before the caret is 2 cells.
        assert_eq!(app.cursor_pos(), Some((5, 0)));
        // Empty value: the caret sits at the prompt's cell width, not its
        // char count, and never reaches into the placeholder.
        let app = InputApp {
            state: InputState::new(String::new(), false, 1000),
            prompt: "日 ".into(),
            placeholder: "type".into(),
            header: None,
            palette: Palette::builtin(Appearance::Dark, AppearanceSource::Default),
        };
        assert_eq!(app.cursor_pos(), Some((3, 0)));
    }

    #[test]
    fn a_wide_glyph_paints_no_caret_either() {
        // Cells-vs-chars coverage lives on in the cursor_pos tests; here
        // the wide-glyph frame must simply stay caret-free like the rest.
        let mut state = InputState::new("日a".to_string(), false, 1000);
        state.cursor = 1;
        let app = InputApp {
            state,
            prompt: "> ".into(),
            placeholder: "type".into(),
            header: None,
            palette: Palette::builtin(Appearance::Dark, AppearanceSource::Default),
        };
        let area = Rect::new(0, 0, 40, 2);
        let mut buf = Buffer::empty(area);
        app.render(area, &mut buf);
        let cell = buf.cell((4, 0)).expect("cell is painted");
        assert_eq!(cell.symbol(), "a");
        assert!(!cell.modifier.contains(Modifier::REVERSED));
    }

    #[test]
    fn the_prompt_takes_its_accent_from_the_palette() {
        let dark = Palette::builtin(Appearance::Dark, AppearanceSource::Default);
        let light = Palette::builtin(Appearance::Light, AppearanceSource::Default);
        assert_eq!(prompt_fg(dark), Color::Indexed(212));
        assert_eq!(prompt_fg(light), light.accent);
        assert_ne!(prompt_fg(dark), prompt_fg(light));
    }
}
