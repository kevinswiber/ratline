use std::io::Read;

use anyhow::Context;
use ratatui::buffer::Buffer;
use ratatui::layout::Rect;
use ratatui::style::{Modifier, Style};

use crate::cli::ChooseArgs;
use crate::color::ColorProfile;
use crate::core::duration::parse_interval;
use crate::exit::AppResult;
use crate::theme::Palette;
use crate::ui::choose::ChooseState;
use crate::ui::key::Key;
use crate::ui::loop_::{Outcome, UiApp, run_ui};

struct ChooseApp {
    state: ChooseState,
    header: String,
    cursor: String,
    selected_prefix: String,
    unselected_prefix: String,
    multi: bool,
    show_help: bool,
    palette: Palette,
}

impl UiApp for ChooseApp {
    fn on_key(&mut self, key: Key) -> Outcome {
        self.state.on_key(key)
    }

    fn render(&self, _area: Rect, buf: &mut Buffer) {
        let accent = Style::default().fg(self.palette.accent);
        buf.set_string(
            0,
            0,
            &self.header,
            Style::default().add_modifier(Modifier::BOLD),
        );
        let visible = usize::from(self.state.height);
        let mut y = 1;
        for idx in self.state.offset..(self.state.offset + visible).min(self.state.items.len()) {
            let at_cursor = idx == self.state.cursor;
            let mut line = String::new();
            line.push_str(if at_cursor {
                &self.cursor
            } else {
                // Keep columns aligned under the cursor marker.
                "  "
            });
            if self.multi {
                line.push_str(if self.state.selected[idx] {
                    &self.selected_prefix
                } else {
                    &self.unselected_prefix
                });
            }
            line.push_str(&self.state.items[idx]);
            let style = if at_cursor { accent } else { Style::default() };
            buf.set_string(0, y, line, style);
            y += 1;
        }
        if self.show_help {
            let help = if self.multi {
                "↑/↓ move · space select · enter confirm · esc cancel"
            } else {
                "↑/↓ move · enter choose · esc cancel"
            };
            buf.set_string(0, y, help, Style::default().add_modifier(Modifier::DIM));
        }
    }

    fn height(&self, _term: (u16, u16)) -> u16 {
        let rows = (self.state.items.len() as u16).min(self.state.height);
        1 + rows + u16::from(self.show_help)
    }
}

pub fn run(args: ChooseArgs, profile: ColorProfile, palette: Palette) -> AppResult {
    let options = if args.options.is_empty() {
        let mut buf = String::new();
        std::io::stdin()
            .read_to_string(&mut buf)
            .context("reading stdin")?;
        let delim = if args.input_delimiter == "\\n" {
            "\n"
        } else {
            &args.input_delimiter
        };
        buf.trim_end_matches('\n')
            .split(delim)
            .filter(|s| !s.is_empty())
            .map(str::to_string)
            .collect()
    } else {
        args.options.clone()
    };

    let out_delim = if args.output_delimiter == "\\n" {
        "\n"
    } else {
        &args.output_delimiter
    };

    if args.select_if_one && options.len() == 1 {
        println!("{}", options[0]);
        return Ok(());
    }

    let limit = if args.no_limit {
        None
    } else {
        Some(args.limit)
    };
    let multi = limit != Some(1);
    let mut state = ChooseState::new(options, limit, args.height);
    state.preselect(&args.selected);
    let mut app = ChooseApp {
        state,
        header: args.header.clone(),
        cursor: args.cursor.clone(),
        selected_prefix: args.selected_prefix.clone(),
        unselected_prefix: args.unselected_prefix.clone(),
        multi,
        show_help: !args.no_show_help,
        palette,
    };
    let timeout = args.timeout.as_deref().map(parse_interval).transpose()?;
    run_ui(&mut app, profile, timeout)?;

    let results = app.state.results(args.ordered || !multi);
    if !results.is_empty() {
        println!("{}", results.join(out_delim));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use ratatui::style::Color;

    use super::*;
    use crate::theme::{Appearance, AppearanceSource};

    fn cursor_row_fg(palette: Palette) -> Color {
        let mut app = ChooseApp {
            state: ChooseState::new(vec!["one".into(), "two".into()], Some(1), 5),
            header: "pick".into(),
            cursor: "> ".into(),
            selected_prefix: "[x] ".into(),
            unselected_prefix: "[ ] ".into(),
            multi: false,
            show_help: false,
            palette,
        };
        let area = Rect::new(0, 0, 40, 6);
        let mut buf = Buffer::empty(area);
        app.state.cursor = 0;
        app.render(area, &mut buf);
        buf.cell((0, 1)).expect("first row is painted").fg
    }

    fn rendered(palette: Palette) -> Vec<String> {
        let mut app = ChooseApp {
            state: ChooseState::new(vec!["one".into(), "two".into()], Some(1), 5),
            header: "pick".into(),
            cursor: "> ".into(),
            selected_prefix: "[x] ".into(),
            unselected_prefix: "[ ] ".into(),
            multi: false,
            show_help: false,
            palette,
        };
        let area = Rect::new(0, 0, 40, 6);
        let mut buf = Buffer::empty(area);
        app.state.cursor = 0;
        app.render(area, &mut buf);
        crate::term::buffer_ansi::buffer_to_lines(&buf, ColorProfile::Ansi256)
    }

    #[test]
    fn the_rendered_choose_frame_is_pinned() {
        // Byte-identity golden: captured on v0.5.0 render code, before the
        // selection-token rewire. If this changes, emitted bytes changed.
        let dark = rendered(Palette::builtin(
            Appearance::Dark,
            AppearanceSource::Default,
        ));
        assert_eq!(
            dark,
            [
                "\u{1b}[1mpick\u{1b}[0m",
                "\u{1b}[38;5;212m> one\u{1b}[0m",
                "  two",
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
                "\u{1b}[1mpick\u{1b}[0m",
                "\u{1b}[38;5;129m> one\u{1b}[0m",
                "  two",
                "",
                "",
                ""
            ]
        );
    }

    #[test]
    fn the_cursor_row_takes_its_accent_from_the_palette() {
        let dark = Palette::builtin(Appearance::Dark, AppearanceSource::Default);
        let light = Palette::builtin(Appearance::Light, AppearanceSource::Default);
        assert_eq!(cursor_row_fg(dark), dark.accent);
        assert_eq!(cursor_row_fg(dark), Color::Indexed(212));
        assert_eq!(cursor_row_fg(light), light.accent);
        assert_ne!(cursor_row_fg(dark), cursor_row_fg(light));
    }
}
