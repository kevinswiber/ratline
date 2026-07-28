/// Columns per horizontal shift step (less's default shift).
pub const HSHIFT_STEP: usize = 8;

#[derive(Copy, Clone, PartialEq, Eq, Debug)]
pub enum ScrollStep {
    LineDown,
    LineUp,
    HalfDown,
    HalfUp,
    PageDown,
    PageUp,
    Top,
    Bottom,
}

#[derive(Copy, Clone, PartialEq, Eq, Debug, Default)]
pub struct ScrollState {
    offset: usize,
}

impl ScrollState {
    pub fn new() -> ScrollState {
        ScrollState { offset: 0 }
    }

    pub fn offset(self) -> usize {
        self.offset
    }

    /// Apply one step over a frame of `total` lines shown `window` at a
    /// time. A zero window is treated as one.
    pub fn step(self, step: ScrollStep, total: usize, window: usize) -> ScrollState {
        let window = window.max(1);
        let half = (window / 2).max(1);
        let offset = match step {
            ScrollStep::LineDown => self.offset.saturating_add(1),
            ScrollStep::LineUp => self.offset.saturating_sub(1),
            ScrollStep::HalfDown => self.offset.saturating_add(half),
            ScrollStep::HalfUp => self.offset.saturating_sub(half),
            ScrollStep::PageDown => self.offset.saturating_add(window),
            ScrollStep::PageUp => self.offset.saturating_sub(window),
            ScrollStep::Top => 0,
            ScrollStep::Bottom => max_offset(total, window),
        };
        ScrollState {
            offset: offset.min(max_offset(total, window)),
        }
    }

    /// Re-clamp after the frame or window changed (a resize). A zero window
    /// is treated as one.
    pub fn clamp(self, total: usize, window: usize) -> ScrollState {
        ScrollState {
            offset: self.offset.min(max_offset(total, window.max(1))),
        }
    }
}

/// The last offset that still fills the window: total - window, floored at 0.
pub fn max_offset(total: usize, window: usize) -> usize {
    total.saturating_sub(window)
}

/// The row that replaces the truncation notice while frozen. `shown` is how
/// many lines the paint actually kept.
pub fn paused_notice(offset: usize, shown: usize, total: usize) -> String {
    if total == 0 {
        "paused · empty frame · Esc resumes".to_string()
    } else if shown == 0 {
        format!("paused · line {} of {total} · Esc resumes", offset + 1)
    } else {
        format!(
            "paused · lines {}-{} of {total} · Esc resumes",
            offset + 1,
            offset + shown
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn stepping_down_and_up_clamps_to_the_frame() {
        assert_eq!(max_offset(30, 22), 8);
        let bottom = ScrollState { offset: 8 };
        assert_eq!(bottom.step(ScrollStep::LineDown, 30, 22).offset(), 8);
        assert_eq!(
            ScrollState::new().step(ScrollStep::LineUp, 30, 22).offset(),
            0
        );
    }

    #[test]
    fn half_and_full_steps_use_the_window() {
        let top = ScrollState::new();
        assert_eq!(top.step(ScrollStep::HalfDown, 40, 22).offset(), 11);
        assert_eq!(top.step(ScrollStep::PageDown, 30, 22).offset(), 8);
        let half = ScrollState { offset: 11 };
        assert_eq!(half.step(ScrollStep::HalfUp, 40, 22).offset(), 0);
    }

    #[test]
    fn a_half_step_is_never_zero() {
        let top = ScrollState::new();
        assert_eq!(top.step(ScrollStep::HalfDown, 30, 1).offset(), 1);
        assert_eq!(top.step(ScrollStep::HalfDown, 30, 0).offset(), 1);
    }

    #[test]
    fn top_and_bottom_are_absolute() {
        let deep = ScrollState { offset: 5 };
        assert_eq!(deep.step(ScrollStep::Top, 30, 22).offset(), 0);
        assert_eq!(deep.step(ScrollStep::Bottom, 30, 22).offset(), 8);
        assert_eq!(
            ScrollState::new().step(ScrollStep::Bottom, 30, 22).offset(),
            8
        );
    }

    #[test]
    fn a_frame_shorter_than_the_window_never_scrolls() {
        for step in [
            ScrollStep::LineDown,
            ScrollStep::LineUp,
            ScrollStep::HalfDown,
            ScrollStep::HalfUp,
            ScrollStep::PageDown,
            ScrollStep::PageUp,
            ScrollStep::Top,
            ScrollStep::Bottom,
        ] {
            assert_eq!(ScrollState::new().step(step, 3, 22).offset(), 0);
        }
    }

    #[test]
    fn clamping_after_a_shrink_pulls_the_offset_back() {
        let deep = ScrollState { offset: 8 };
        assert_eq!(deep.clamp(30, 40).offset(), 0);
        assert_eq!(deep.clamp(10, 5).offset(), 5);
    }

    #[test]
    fn the_paused_row_names_the_visible_range() {
        assert_eq!(
            paused_notice(1, 22, 30),
            "paused · lines 2-23 of 30 · Esc resumes"
        );
        assert!(paused_notice(8, 22, 30).ends_with("lines 9-30 of 30 · Esc resumes"));
        assert_eq!(paused_notice(0, 0, 0), "paused · empty frame · Esc resumes");
        assert_eq!(
            paused_notice(5, 0, 30),
            "paused · line 6 of 30 · Esc resumes"
        );
    }
}
