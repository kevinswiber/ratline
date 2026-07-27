use std::io::Read;

use anyhow::Context;
use ratatui::buffer::Buffer;
use ratatui::layout::Rect;
use ratatui::style::{Color, Modifier, Style};

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
}

impl UiApp for FilterApp {
    fn on_key(&mut self, key: Key) -> Outcome {
        self.state.on_key(key)
    }

    fn render(&self, _area: Rect, buf: &mut Buffer) {
        let accent = Style::default().fg(Color::Indexed(212));
        let mut y = 0;
        if let Some(header) = &self.header {
            buf.set_string(0, y, header, Style::default().add_modifier(Modifier::BOLD));
            y += 1;
        }
        buf.set_string(0, y, &self.prompt, accent);
        let qx = self.prompt.chars().count() as u16;
        let qtext = self.state.query.display(&self.placeholder);
        let qstyle = if self.state.query.value.is_empty() {
            Style::default().add_modifier(Modifier::DIM)
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
            buf.set_string(x, y, marker, accent);
            x += marker.chars().count() as u16;
            if self.multi {
                let prefix = if self.state.selected[m.index] {
                    &self.selected_prefix
                } else {
                    &self.unselected_prefix
                };
                buf.set_string(x, y, prefix, Style::default());
                x += prefix.chars().count() as u16;
            }
            let item = &self.state.items[m.index];
            let base = if at_cursor { accent } else { Style::default() };
            buf.set_string(x, y, item, base);
            // Highlight the matched characters.
            for &pos in &m.positions {
                if let Some(c) = item.chars().nth(pos as usize) {
                    buf.set_string(
                        x + pos as u16,
                        y,
                        c.to_string(),
                        base.add_modifier(Modifier::BOLD).fg(Color::Indexed(212)),
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
}

pub fn run(args: FilterArgs, profile: ColorProfile, _palette: Palette) -> AppResult {
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
