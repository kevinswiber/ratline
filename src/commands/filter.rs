use std::io::Read;

use anyhow::Context;
use ratatui::buffer::Buffer;
use ratatui::layout::Rect;
use ratatui::style::{Modifier, Style};
use unicode_width::UnicodeWidthStr;

use crate::cli::FilterArgs;
use crate::color::ColorProfile;
use crate::core::duration::parse_interval;
use crate::exit::AppResult;
use crate::theme::Palette;
use crate::ui::filter::FilterState;
use crate::ui::key::Key;
use crate::ui::loop_::{Outcome, UiApp, run_ui};

struct FilterApp {
    state: FilterState,
    prompt: String,
    placeholder: String,
    indicator: String,
    selected_prefix: String,
    unselected_prefix: String,
    multi: bool,
    header: Option<String>,
    palette: Palette,
}

impl UiApp for FilterApp {
    fn on_key(&mut self, key: Key) -> Outcome {
        self.state.on_key(key)
    }

    fn render(&self, _area: Rect, buf: &mut Buffer) {
        // The prompt is a prompt (accent); the marker and cursor row are a
        // selection. Same values by construction, distinct tokens by role.
        let accent = Style::default().fg(self.palette.accent);
        let selection = Style::default().fg(self.palette.selection);
        let mut y = 0;
        if let Some(header) = &self.header {
            buf.set_string(0, y, header, Style::default().add_modifier(Modifier::BOLD));
            y += 1;
        }
        buf.set_string(0, y, &self.prompt, accent);
        // Cell width, not char count — same rule as the input prompt.
        let qx = self.prompt.as_str().width() as u16;
        // The query scrolls in its own window, same as `rat input`.
        let qtext = self.state.query.visible(&self.placeholder);
        let qstyle = if self.state.query.value.is_empty() {
            Style::default()
                .add_modifier(Modifier::DIM)
                .fg(self.palette.placeholder)
        } else {
            Style::default()
        };
        buf.set_string(qx, y, &qtext, qstyle);
        y += 1;

        let visible = usize::from(self.state.height);
        let window = self
            .state
            .matches
            .iter()
            .enumerate()
            .skip(self.state.offset)
            .take(visible);
        for (match_idx, m) in window {
            let at_cursor = match_idx == self.state.cursor;
            let mut x = 0u16;
            let marker = if at_cursor { &self.indicator } else { "  " };
            buf.set_string(x, y, marker, selection);
            // x is a cell cursor: a wide glyph advances it by two.
            x += marker.width() as u16;
            if self.multi {
                let prefix = if self.state.selected[m.index] {
                    &self.selected_prefix
                } else {
                    &self.unselected_prefix
                };
                buf.set_string(x, y, prefix, Style::default());
                x += prefix.as_str().width() as u16;
            }
            let item = &self.state.items[m.index];
            let base = if at_cursor {
                selection
            } else {
                Style::default()
            };
            buf.set_string(x, y, item, base);
            // Highlight the matched characters. `positions` are char
            // indices into the item; the buffer takes cells, so the
            // offset is the width of everything before the match — a
            // char index would repaint an earlier cell.
            for &pos in &m.positions {
                if let Some(c) = item.chars().nth(pos as usize) {
                    let before: String = item.chars().take(pos as usize).collect();
                    buf.set_string(
                        x + before.width() as u16,
                        y,
                        c.to_string(),
                        base.add_modifier(Modifier::BOLD).fg(self.palette.r#match),
                    );
                }
            }
            y += 1;
        }
    }

    fn height(&self, _term: (u16, u16)) -> u16 {
        let rows = (self.state.matches.len() as u16).min(self.state.height);
        1 + rows + u16::from(self.header.is_some())
    }

    fn cursor_pos(&self) -> Option<(u16, u16)> {
        // Cells, not chars, and measured inside the query's window.
        let row = u16::from(self.header.is_some());
        let col = self.prompt.as_str().width() + self.state.query.caret_col();
        Some((col as u16, row))
    }

    fn prepare(&mut self, term: (u16, u16)) {
        let avail = usize::from(term.0).saturating_sub(self.prompt.as_str().width());
        self.state.query.follow(avail);
    }
}

pub fn run(args: FilterArgs, profile: ColorProfile, palette: Palette) -> AppResult {
    let mut stdin = String::new();
    std::io::stdin()
        .read_to_string(&mut stdin)
        .context("reading stdin")?;
    let delim = if args.input_delimiter == "\\n" {
        "\n"
    } else {
        &args.input_delimiter
    };
    let items: Vec<String> = stdin
        .trim_end_matches('\n')
        .split(delim)
        .filter(|s| !s.is_empty())
        .map(str::to_string)
        .collect();

    if args.select_if_one && items.len() == 1 {
        println!("{}", items[0]);
        return Ok(());
    }

    let out_delim = if args.output_delimiter == "\\n" {
        "\n"
    } else {
        &args.output_delimiter
    };
    let limit = if args.no_limit {
        None
    } else {
        Some(args.limit)
    };
    let fuzzy = !args.no_fuzzy;
    let sort = !args.no_fuzzy_sort;
    let strict = !args.no_strict;
    let state = FilterState::new(items, limit, args.height, fuzzy, sort, args.value.clone());
    let mut app = FilterApp {
        state,
        prompt: args.prompt.clone(),
        placeholder: args.placeholder.clone(),
        indicator: args.indicator.clone(),
        selected_prefix: args.selected_prefix.clone(),
        unselected_prefix: args.unselected_prefix.clone(),
        multi: limit != Some(1),
        header: args.header.clone(),
        palette,
    };
    let timeout = args.timeout.as_deref().map(parse_interval).transpose()?;
    run_ui(&mut app, profile, timeout)?;

    let results = app.state.results();
    if results.is_empty() {
        // Matching gum: strict mode exits 0 with empty output; non-strict
        // returns the query itself.
        if !strict && !app.state.query.value.is_empty() {
            println!("{}", app.state.query.value);
        }
        return Ok(());
    }
    println!("{}", results.join(out_delim));
    Ok(())
}

#[cfg(test)]
mod tests {
    use ratatui::style::Color;

    use super::*;
    use crate::theme::{Appearance, AppearanceSource};

    fn app(palette: Palette) -> FilterApp {
        FilterApp {
            state: FilterState::new(
                vec!["alpha".into(), "beta".into()],
                Some(1),
                5,
                true,
                true,
                "a".into(),
            ),
            prompt: "> ".into(),
            placeholder: "filter".into(),
            indicator: "> ".into(),
            selected_prefix: "[x] ".into(),
            unselected_prefix: "[ ] ".into(),
            multi: false,
            header: None,
            palette,
        }
    }

    fn prompt_and_highlight_fg(palette: Palette) -> (Color, Color) {
        let app = app(palette);
        let area = Rect::new(0, 0, 40, 6);
        let mut buf = Buffer::empty(area);
        app.render(area, &mut buf);
        let prompt = buf.cell((0, 0)).expect("prompt is painted").fg;
        let highlight = (0u16..40)
            .filter_map(|x| buf.cell((x, 1)))
            .find(|cell| cell.modifier.contains(Modifier::BOLD))
            .expect("a match highlight is painted")
            .fg;
        (prompt, highlight)
    }

    fn rendered(palette: Palette) -> Vec<String> {
        let app = app(palette);
        let area = Rect::new(0, 0, 40, 6);
        let mut buf = Buffer::empty(area);
        app.render(area, &mut buf);
        crate::term::buffer_ansi::buffer_to_lines(&buf, ColorProfile::Ansi256)
    }

    #[test]
    fn the_rendered_filter_frame_is_pinned() {
        // Byte-identity golden: captured on v0.5.0 render code, before the
        // selection/match/placeholder rewire. Note the marker, cursor-row
        // base, and match highlight all read accent-valued styles here.
        let dark = rendered(Palette::builtin(
            Appearance::Dark,
            AppearanceSource::Default,
        ));
        assert_eq!(
            dark,
            [
                "\u{1b}[38;5;212m> \u{1b}[0ma",
                "\u{1b}[38;5;212m> \u{1b}[0m\u{1b}[1;38;5;212ma\u{1b}[0m\u{1b}[38;5;212mlpha\u{1b}[0m",
                "\u{1b}[38;5;212m  \u{1b}[0mbet\u{1b}[1;38;5;212ma\u{1b}[0m",
                "",
                "",
                ""
            ]
        );
        let light = rendered(Palette::builtin(
            Appearance::Light,
            AppearanceSource::Default,
        ));
        assert_eq!(
            light,
            [
                "\u{1b}[38;5;129m> \u{1b}[0ma",
                "\u{1b}[38;5;129m> \u{1b}[0m\u{1b}[1;38;5;129ma\u{1b}[0m\u{1b}[38;5;129mlpha\u{1b}[0m",
                "\u{1b}[38;5;129m  \u{1b}[0mbet\u{1b}[1;38;5;129ma\u{1b}[0m",
                "",
                "",
                ""
            ]
        );
    }

    #[test]
    fn the_marker_and_cursor_row_read_the_selection_token() {
        // Sentinel: selection/accent share a value by construction, so only
        // a diverging palette proves the render reads `selection`.
        let palette = Palette {
            selection: Color::Indexed(99),
            ..Palette::builtin(Appearance::Dark, AppearanceSource::Default)
        };
        let app = app(palette);
        let area = Rect::new(0, 0, 40, 6);
        let mut buf = Buffer::empty(area);
        app.render(area, &mut buf);
        assert_eq!(buf.cell((0, 1)).unwrap().fg, Color::Indexed(99), "marker");
        assert_eq!(
            buf.cell((3, 1)).unwrap().fg,
            Color::Indexed(99),
            "cursor-row item base"
        );
        // The prompt is a prompt, not a selection: it stays on accent.
        assert_eq!(buf.cell((0, 0)).unwrap().fg, Color::Indexed(212), "prompt");
    }

    #[test]
    fn a_match_highlight_lands_on_its_own_cell_past_a_wide_glyph() {
        // `positions` are CHAR indices; the buffer takes CELLS. Treating
        // one as the other repaints the match over an earlier cell — a
        // query of "o" against "🔥 hot" rewrote the 'h', so the row read
        // "🔥 oot".
        let app = FilterApp {
            state: FilterState::new(
                vec!["🔥 hot".into(), "日本 jp".into()],
                Some(1),
                5,
                true,
                true,
                "o".into(),
            ),
            prompt: "> ".into(),
            placeholder: "filter".into(),
            indicator: "> ".into(),
            selected_prefix: "[x] ".into(),
            unselected_prefix: "[ ] ".into(),
            multi: false,
            header: None,
            palette: Palette::builtin(Appearance::Dark, AppearanceSource::Default),
        };
        let area = Rect::new(0, 0, 40, 6);
        let mut buf = Buffer::empty(area);
        app.render(area, &mut buf);
        // Row 1: indicator "> " (2 cells), then 🔥 (2), ' ', 'h', 'o', 't'.
        let line = &crate::term::buffer_ansi::buffer_to_lines(&buf, ColorProfile::Ascii)[1];
        assert!(
            line.contains("🔥 hot"),
            "the item text must survive its own highlight: {line:?}"
        );
        // The 'o' sits at column 2 + 2 + 1 + 1 = 6 and carries the match.
        let cell = buf.cell((6, 1)).expect("the matched cell is painted");
        assert_eq!(cell.symbol(), "o");
        assert!(cell.modifier.contains(Modifier::BOLD));
        // And the 'h' before it is untouched.
        assert_eq!(buf.cell((5, 1)).unwrap().symbol(), "h");
    }

    #[test]
    fn a_wide_indicator_or_prefix_advances_by_cells() {
        // The running x is a cell cursor, so a wide indicator must move
        // it two cells, not one char.
        let app = FilterApp {
            state: FilterState::new(vec!["ab".into()], None, 5, true, true, String::new()),
            prompt: "> ".into(),
            placeholder: "filter".into(),
            indicator: "日".into(),
            selected_prefix: "[x] ".into(),
            unselected_prefix: "[ ] ".into(),
            multi: true,
            header: None,
            palette: Palette::builtin(Appearance::Dark, AppearanceSource::Default),
        };
        let area = Rect::new(0, 0, 40, 6);
        let mut buf = Buffer::empty(area);
        app.render(area, &mut buf);
        // 日 spans columns 0-1, so the "[ ] " prefix starts at 2 and the
        // item at 6.
        assert_eq!(buf.cell((2, 1)).unwrap().symbol(), "[");
        assert_eq!(buf.cell((6, 1)).unwrap().symbol(), "a");
    }

    #[test]
    fn the_match_highlight_reads_the_match_token() {
        let palette = Palette {
            r#match: Color::Indexed(98),
            ..Palette::builtin(Appearance::Dark, AppearanceSource::Default)
        };
        let app = app(palette);
        let area = Rect::new(0, 0, 40, 6);
        let mut buf = Buffer::empty(area);
        app.render(area, &mut buf);
        let cell = buf.cell((2, 1)).unwrap();
        assert_eq!(cell.fg, Color::Indexed(98));
        // The BOLD attribute is part of the highlight contract.
        assert!(cell.modifier.contains(Modifier::BOLD));
    }

    #[test]
    fn the_empty_query_reads_the_placeholder_token() {
        let palette = Palette {
            placeholder: Color::Indexed(97),
            ..Palette::builtin(Appearance::Dark, AppearanceSource::Default)
        };
        let app = FilterApp {
            state: FilterState::new(
                vec!["alpha".into(), "beta".into()],
                Some(1),
                5,
                true,
                true,
                String::new(),
            ),
            prompt: "> ".into(),
            placeholder: "filter".into(),
            indicator: "> ".into(),
            selected_prefix: "[x] ".into(),
            unselected_prefix: "[ ] ".into(),
            multi: false,
            header: None,
            palette,
        };
        let area = Rect::new(0, 0, 40, 6);
        let mut buf = Buffer::empty(area);
        app.render(area, &mut buf);
        let cell = buf.cell((2, 0)).unwrap();
        assert_eq!(cell.fg, Color::Indexed(97));
        // DIM is what keeps the default byte-identical (Reset emits nothing).
        assert!(cell.modifier.contains(Modifier::DIM));
    }

    #[test]
    fn the_cursor_pos_tracks_the_query_caret() {
        // Initial query "a" leaves the caret at its end: prompt width 2
        // plus 1, on the query row.
        let app = app(Palette::builtin(
            Appearance::Dark,
            AppearanceSource::Default,
        ));
        assert_eq!(app.cursor_pos(), Some((3, 0)));
    }

    #[test]
    fn the_cursor_pos_counts_display_cells_not_chars() {
        // A wide query char occupies two cells; the parked column follows
        // the cells, not the char count.
        let mut app = app(Palette::builtin(
            Appearance::Dark,
            AppearanceSource::Default,
        ));
        app.state.query.value = "日".to_string();
        app.state.query.cursor = 1;
        assert_eq!(app.cursor_pos(), Some((4, 0)));
    }

    #[test]
    fn the_prompt_and_match_highlight_take_their_palette_tokens() {
        let dark = Palette::builtin(Appearance::Dark, AppearanceSource::Default);
        let light = Palette::builtin(Appearance::Light, AppearanceSource::Default);
        let (dark_prompt, dark_highlight) = prompt_and_highlight_fg(dark);
        assert_eq!(dark_prompt, dark.accent);
        // The literal is the byte-identity pin; the field name is routing.
        assert_eq!(dark_prompt, Color::Indexed(212));
        assert_eq!(dark_highlight, dark.r#match);
        assert_eq!(dark_highlight, Color::Indexed(212));
        let (light_prompt, light_highlight) = prompt_and_highlight_fg(light);
        assert_eq!(light_prompt, light.accent);
        assert_eq!(light_highlight, light.r#match);
    }
}
