use std::io::Read;

use anyhow::Context;
use ratatui::buffer::Buffer;
use ratatui::layout::Rect;
use ratatui::style::{Color, Modifier, Style};

use crate::cli::ChooseArgs;
use crate::color::ColorProfile;
use crate::core::duration::parse_interval;
use crate::exit::AppResult;
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
}

impl UiApp for ChooseApp {
    fn on_key(&mut self, key: Key) -> Outcome {
        self.state.on_key(key)
    }

    fn render(&self, _area: Rect, buf: &mut Buffer) {
        let accent = Style::default().fg(Color::Indexed(212));
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

pub fn run(args: ChooseArgs, profile: ColorProfile) -> AppResult {
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
    };
    let timeout = args.timeout.as_deref().map(parse_interval).transpose()?;
    run_ui(&mut app, profile, timeout)?;

    let results = app.state.results(args.ordered || !multi);
    if !results.is_empty() {
        println!("{}", results.join(out_delim));
    }
    Ok(())
}
