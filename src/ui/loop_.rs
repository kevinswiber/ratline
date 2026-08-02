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
    /// The frame cell (column, row) where the hardware cursor should
    /// rest after painting. Screen readers and braille displays track
    /// only the real cursor, so any app with an edit point must return
    /// it here; `None` keeps the cursor hidden (right for list UIs).
    fn cursor_pos(&self) -> Option<(u16, u16)> {
        None
    }
    /// Learn the terminal size before painting. An app with a scrolling
    /// field needs the width to place its window, and `render` cannot
    /// mutate. Called every iteration, so a resize lands here too.
    fn prepare(&mut self, _term: (u16, u16)) {}
}

/// Where the hardware cursor may actually be parked. A scrolling field
/// keeps its caret inside the frame, so this is a guard rather than a
/// placement: a caret past the last column has no cell of its own, and
/// clamping it there would park it *on* the final character instead of
/// after it. Decline the park instead of lying about it.
fn park_target(cursor: Option<(u16, u16)>, cols: u16, height: u16) -> Option<(u16, u16)> {
    cursor.filter(|&(col, row)| col < cols && row < height)
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
        app.prepare((cols, rows));
        let height = app.height((cols, rows)).clamp(1, rows);
        let area = Rect::new(0, 0, cols, height);
        let mut buf = Buffer::empty(area);
        app.render(area, &mut buf);
        let cursor = park_target(app.cursor_pos(), cols, height);
        renderer
            .draw_with_cursor(&buffer_to_lines(&buf, profile), cols, cursor)
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_caret_inside_the_frame_parks_where_it_asked() {
        assert_eq!(park_target(Some((0, 0)), 20, 2), Some((0, 0)));
        assert_eq!(park_target(Some((19, 1)), 20, 2), Some((19, 1)));
    }

    #[test]
    fn a_caret_past_the_last_column_declines_rather_than_lying() {
        // Clamping to cols-1 would park the cursor ON the final
        // character; there is no honest cell, so no park.
        assert_eq!(park_target(Some((20, 0)), 20, 2), None);
        assert_eq!(park_target(Some((99, 0)), 20, 2), None);
    }

    #[test]
    fn a_caret_below_the_frame_declines_too() {
        assert_eq!(park_target(Some((0, 2)), 20, 2), None);
    }

    #[test]
    fn no_caret_stays_no_caret() {
        assert_eq!(park_target(None, 20, 2), None);
    }
}
