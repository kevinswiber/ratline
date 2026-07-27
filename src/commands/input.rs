use ratatui::buffer::Buffer;
use ratatui::layout::Rect;
use ratatui::style::{Color, Modifier, Style};

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
        let accent = Style::default().fg(Color::Indexed(212));
        buf.set_string(0, y, &self.prompt, accent);
        let x = self.prompt.chars().count() as u16;
        let text = self.state.display(&self.placeholder);
        let style = if self.state.value.is_empty() {
            Style::default().add_modifier(Modifier::DIM)
        } else {
            Style::default()
        };
        buf.set_string(x, y, &text, style);
        // Mark the cursor with reverse video (on the char or one past the end).
        if !self.state.value.is_empty() || self.state.cursor > 0 {
            let cx = x + self.state.cursor as u16;
            let under = text.chars().nth(self.state.cursor).unwrap_or(' ');
            buf.set_string(
                cx,
                y,
                under.to_string(),
                Style::default().add_modifier(Modifier::REVERSED),
            );
        }
    }

    fn height(&self, _term: (u16, u16)) -> u16 {
        1 + u16::from(self.header.is_some())
    }
}

pub fn run(args: InputArgs, profile: ColorProfile, _palette: Palette) -> AppResult {
    let mut app = InputApp {
        state: InputState::new(args.value.clone(), args.password, args.char_limit),
        prompt: args.prompt.clone(),
        placeholder: args.placeholder.clone(),
        header: args.header.clone(),
    };
    let timeout = args.timeout.as_deref().map(parse_interval).transpose()?;
    run_ui(&mut app, profile, timeout)?;
    println!("{}", app.state.value);
    Ok(())
}
