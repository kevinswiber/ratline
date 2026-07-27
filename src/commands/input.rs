use ratatui::buffer::Buffer;
use ratatui::layout::Rect;
use ratatui::style::{Modifier, Style};

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
        // Byte-identity goldens: captured on v0.5.0 render code, before the
        // placeholder/cursor rewire. The mid-string frames show finding F-0:
        // the REVERSED caret emits no SGR at all (no \x1b[7m anywhere), so
        // "abc" reaches the terminal completely plain.
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

    #[test]
    fn the_prompt_takes_its_accent_from_the_palette() {
        let dark = Palette::builtin(Appearance::Dark, AppearanceSource::Default);
        let light = Palette::builtin(Appearance::Light, AppearanceSource::Default);
        assert_eq!(prompt_fg(dark), Color::Indexed(212));
        assert_eq!(prompt_fg(light), light.accent);
        assert_ne!(prompt_fg(dark), prompt_fg(light));
    }
}
