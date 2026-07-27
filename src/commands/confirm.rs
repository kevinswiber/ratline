use ratatui::buffer::Buffer;
use ratatui::layout::Rect;
use ratatui::style::{Modifier, Style};

use crate::cli::ConfirmArgs;
use crate::color::ColorProfile;
use crate::core::duration::parse_interval;
use crate::exit::{AppError, AppResult};
use crate::theme::Palette;
use crate::ui::confirm::ConfirmState;
use crate::ui::key::Key;
use crate::ui::loop_::{Outcome, UiApp, run_ui};

struct ConfirmApp {
    state: ConfirmState,
    prompt: String,
    affirmative: String,
    negative: String,
    palette: Palette,
}

impl UiApp for ConfirmApp {
    fn on_key(&mut self, key: Key) -> Outcome {
        self.state.on_key(key)
    }

    fn render(&self, _area: Rect, buf: &mut Buffer) {
        buf.set_string(
            0,
            0,
            &self.prompt,
            Style::default().add_modifier(Modifier::BOLD),
        );
        let active = Style::default()
            .fg(self.palette.on_accent)
            .bg(self.palette.accent)
            .add_modifier(Modifier::BOLD);
        let inactive = Style::default().add_modifier(Modifier::DIM);
        let yes = format!(" {} ", self.affirmative);
        let no = format!(" {} ", self.negative);
        let (yes_style, no_style) = if self.state.affirmative {
            (active, inactive)
        } else {
            (inactive, active)
        };
        buf.set_string(2, 1, &yes, yes_style);
        let no_x = 2 + yes.len() as u16 + 3;
        buf.set_string(no_x, 1, &no, no_style);
    }

    fn height(&self, _term: (u16, u16)) -> u16 {
        2
    }
}

pub fn run(args: ConfirmArgs, profile: ColorProfile, palette: Palette) -> AppResult {
    let mut app = ConfirmApp {
        state: ConfirmState {
            affirmative: args.default_yes,
        },
        prompt: args.prompt.clone(),
        affirmative: args.affirmative.clone(),
        negative: args.negative.clone(),
        palette,
    };
    let timeout = args.timeout.as_deref().map(parse_interval).transpose()?;
    run_ui(&mut app, profile, timeout)?;

    let label = if app.state.affirmative {
        &args.affirmative
    } else {
        &args.negative
    };
    if args.show_output {
        println!("{label}");
    }
    if app.state.affirmative {
        Ok(())
    } else {
        Err(AppError::NoSelection)
    }
}

#[cfg(test)]
mod tests {
    use ratatui::style::Color;

    use super::*;
    use crate::theme::{Appearance, AppearanceSource};

    fn active_button(palette: Palette) -> (Color, Color) {
        let app = ConfirmApp {
            state: ConfirmState { affirmative: true },
            prompt: "Ship it?".into(),
            affirmative: "Yes".into(),
            negative: "No".into(),
            palette,
        };
        let area = Rect::new(0, 0, 40, 2);
        let mut buf = Buffer::empty(area);
        app.render(area, &mut buf);
        let cell = buf.cell((2, 1)).expect("the active button is painted");
        (cell.fg, cell.bg)
    }

    fn rendered(palette: Palette) -> Vec<String> {
        let app = ConfirmApp {
            state: ConfirmState { affirmative: true },
            prompt: "Ship it?".into(),
            affirmative: "Yes".into(),
            negative: "No".into(),
            palette,
        };
        let area = Rect::new(0, 0, 40, 2);
        let mut buf = Buffer::empty(area);
        app.render(area, &mut buf);
        crate::term::buffer_ansi::buffer_to_lines(&buf, ColorProfile::Ansi256)
    }

    #[test]
    fn the_rendered_confirm_frame_is_pinned() {
        // Byte-identity golden: captured on v0.5.0 render code. The active
        // button's foreground is the NAMED on-accent (SGR 30 dark / 97
        // light), not an indexed sequence; the background is the indexed
        // accent.
        let dark = rendered(Palette::builtin(
            Appearance::Dark,
            AppearanceSource::Default,
        ));
        assert_eq!(
            dark,
            [
                "\u{1b}[1mShip it?\u{1b}[0m",
                "  \u{1b}[1;30;48;5;212m Yes \u{1b}[0m   \u{1b}[2m No \u{1b}[0m"
            ]
        );
        let light = rendered(Palette::builtin(
            Appearance::Light,
            AppearanceSource::Default,
        ));
        assert_eq!(
            light,
            [
                "\u{1b}[1mShip it?\u{1b}[0m",
                "  \u{1b}[1;97;48;5;129m Yes \u{1b}[0m   \u{1b}[2m No \u{1b}[0m"
            ]
        );
    }

    #[test]
    fn the_active_button_pairs_on_accent_over_accent() {
        let dark = Palette::builtin(Appearance::Dark, AppearanceSource::Default);
        let light = Palette::builtin(Appearance::Light, AppearanceSource::Default);
        assert_eq!(active_button(dark), (Color::Black, Color::Indexed(212)));
        assert_eq!(active_button(light), (light.on_accent, light.accent));
    }
}
