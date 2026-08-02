use std::time::{Duration, Instant};

use anyhow::Context;
use ratatui::buffer::Buffer;
use ratatui::layout::Rect;

use crate::color::ColorProfile;
use crate::exit::{AppError, AppResult};
use crate::term::buffer_ansi::buffer_to_lines;
use crate::term::inline::InlineRenderer;
use crate::term::tty::{RawModeGuard, UiStream};
use crate::ui::key::{Key, from_crossterm};

#[derive(Copy, Clone, PartialEq, Eq, Debug)]
pub enum Outcome {
    Continue,
    Submit,
    Abort,
}

/// An interactive command: a pure reducer plus a widget render.
pub trait UiApp {
    fn on_key(&mut self, key: Key) -> Outcome;
    fn render(&self, area: Rect, buf: &mut Buffer);
    fn height(&self, term: (u16, u16)) -> u16;
}

/// Drive a UiApp on the UI stream: raw mode, event pump, inline repaint.
/// The UI erases itself on exit so only the result (printed by the caller
/// to stdout) remains. Esc maps to exit 1, Ctrl-C to 130, --timeout to 124.
pub fn run_ui<A: UiApp>(
    app: &mut A,
    profile: ColorProfile,
    timeout: Option<Duration>,
) -> AppResult {
    let ui = UiStream::open();
    if !ui.is_tty() {
        return Err(anyhow::anyhow!("interactive commands need a terminal").into());
    }
    let _raw_guard = RawModeGuard::enable().context("enabling raw mode")?;
    let mut renderer = InlineRenderer::new(ui)
        .with_cursor_hidden(true)
        .with_sync_output(true);

    let deadline = timeout.map(|t| Instant::now() + t);
    let outcome = loop {
        let (cols, rows) = crossterm::terminal::size().unwrap_or((80, 24));
        let height = app.height((cols, rows)).clamp(1, rows);
        let area = Rect::new(0, 0, cols, height);
        let mut buf = Buffer::empty(area);
        app.render(area, &mut buf);
        renderer
            .draw(&buffer_to_lines(&buf, profile), cols)
            .context("painting ui")?;

        let wait = match deadline {
            Some(d) => {
                let now = Instant::now();
                if now >= d {
                    break Err(AppError::Timeout(None));
                }
                (d - now).min(Duration::from_millis(250))
            }
            None => Duration::from_millis(250),
        };
        if crossterm::event::poll(wait).context("polling events")? {
            let event = crossterm::event::read().context("reading event")?;
            if let crossterm::event::Event::Key(key_event) = event {
                let Some(key) = from_crossterm(key_event) else {
                    continue;
                };
                if key == Key::CtrlC {
                    break Err(AppError::Aborted);
                }
                match app.on_key(key) {
                    Outcome::Continue => {}
                    Outcome::Submit => break Ok(()),
                    Outcome::Abort => break Err(AppError::NoSelection),
                }
            }
        }
    };

    renderer.clear().context("clearing ui")?;
    renderer.finish().context("restoring terminal")?;
    outcome
}
