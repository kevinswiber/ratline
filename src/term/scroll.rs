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
    /// A state parked at `offset` — how a live-scrolled window freezes in
    /// place. Clamp separately when the frame may have shrunk.
    pub fn at(offset: usize) -> ScrollState {
        ScrollState { offset }
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

/// A window over the LIVE frame. Offset 0 is the live view itself; `G`
/// pins the window to the growing tail until any other step unpins it.
#[derive(Copy, Clone, PartialEq, Eq, Debug)]
pub struct LiveScroll {
    offset: usize,
    pinned: bool,
}

impl LiveScroll {
    /// Entering live-scroll: one step from offset 0.
    pub fn start(step: ScrollStep, total: usize, window: usize) -> LiveScroll {
        LiveScroll {
            offset: 0,
            pinned: false,
        }
        .step(step, total, window)
    }

    /// Entering live-scroll at a carried offset (the scrub-forward
    /// exit): clamped like a reanchor, and unpinned — a carried offset
    /// means HOLD the reader's place, never chase the growing tail.
    pub fn at(offset: usize, total: usize, window: usize) -> LiveScroll {
        LiveScroll {
            offset: offset.min(max_offset(total, window.max(1))),
            pinned: false,
        }
    }

    /// pinned = (step == `ScrollStep::Bottom`); any other step unpins.
    pub fn step(self, step: ScrollStep, total: usize, window: usize) -> LiveScroll {
        LiveScroll {
            offset: ScrollState {
                offset: self.offset,
            }
            .step(step, total, window)
            .offset(),
            pinned: step == ScrollStep::Bottom,
        }
    }

    /// Every tick: pinned tracks the tail; unpinned holds, clamped.
    pub fn reanchor(self, total: usize, window: usize) -> LiveScroll {
        let limit = max_offset(total, window.max(1));
        LiveScroll {
            offset: if self.pinned {
                limit
            } else {
                self.offset.min(limit)
            },
            ..self
        }
    }

    pub fn offset(self) -> usize {
        self.offset
    }

    /// Offset 0 means the window is the live view: collapse the mode.
    pub fn at_top(self) -> bool {
        self.offset == 0
    }
}

/// The live-scrolled status row: the same time segment and tail the
/// live row carries, with the window's range spliced between them —
/// the three modes read as one family. `time_seg` is pre-formatted
/// (`since 12:07:45` or its counting form), and `live_tail` starts
/// with ` · ` by construction, the same trick the live row relies on.
/// Like the paused row, no retention marker — that stays live-only.
pub fn scrolled_notice(
    time_seg: &str,
    live_tail: &str,
    offset: usize,
    shown: usize,
    total: usize,
) -> String {
    format!(
        "live · {time_seg} · lines {}-{} of {total}{live_tail}",
        offset + 1,
        offset + shown
    )
}

/// The row that replaces the truncation notice while frozen. `age` is the
/// pre-formatted time since the viewed frame was current ("just now",
/// "14s ago"); `shown` is how many lines the paint actually kept.
pub fn paused_notice(age: &str, offset: usize, shown: usize, total: usize) -> String {
    if total == 0 {
        format!("paused · {age} · empty frame · Esc resumes")
    } else if shown == 0 {
        format!(
            "paused · {age} · line {} of {total} · Esc resumes",
            offset + 1
        )
    } else {
        format!(
            "paused · {age} · lines {}-{} of {total} · Esc resumes",
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
            ScrollState::default()
                .step(ScrollStep::LineUp, 30, 22)
                .offset(),
            0
        );
    }

    #[test]
    fn half_and_full_steps_use_the_window() {
        let top = ScrollState::default();
        assert_eq!(top.step(ScrollStep::HalfDown, 40, 22).offset(), 11);
        assert_eq!(top.step(ScrollStep::PageDown, 30, 22).offset(), 8);
        let half = ScrollState { offset: 11 };
        assert_eq!(half.step(ScrollStep::HalfUp, 40, 22).offset(), 0);
    }

    #[test]
    fn a_half_step_is_never_zero() {
        let top = ScrollState::default();
        assert_eq!(top.step(ScrollStep::HalfDown, 30, 1).offset(), 1);
        assert_eq!(top.step(ScrollStep::HalfDown, 30, 0).offset(), 1);
    }

    #[test]
    fn top_and_bottom_are_absolute() {
        let deep = ScrollState { offset: 5 };
        assert_eq!(deep.step(ScrollStep::Top, 30, 22).offset(), 0);
        assert_eq!(deep.step(ScrollStep::Bottom, 30, 22).offset(), 8);
        assert_eq!(
            ScrollState::default()
                .step(ScrollStep::Bottom, 30, 22)
                .offset(),
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
            assert_eq!(ScrollState::default().step(step, 3, 22).offset(), 0);
        }
    }

    #[test]
    fn clamping_after_a_shrink_pulls_the_offset_back() {
        let deep = ScrollState { offset: 8 };
        assert_eq!(deep.clamp(30, 40).offset(), 0);
        assert_eq!(deep.clamp(10, 5).offset(), 5);
        assert_eq!(ScrollState::at(8).offset(), 8);
    }

    #[test]
    fn starting_a_live_scroll_steps_from_the_top() {
        let one = LiveScroll::start(ScrollStep::LineDown, 46, 22);
        assert_eq!(one.offset(), 1);
        assert!(!one.at_top());
        let bottom = LiveScroll::start(ScrollStep::Bottom, 46, 22);
        assert_eq!(bottom.offset(), max_offset(46, 22));
    }

    #[test]
    fn entering_at_a_carried_offset_clamps_and_holds() {
        // A scrub-forward exit carries the pause's offset: clamped like
        // a reanchor, and unpinned — carried means HOLD, never chase.
        let deep = LiveScroll::at(100, 30, 10);
        assert_eq!(deep.offset(), max_offset(30, 10));
        let carried = LiveScroll::at(9, 46, 22);
        assert_eq!(carried.offset(), 9);
        assert_eq!(
            carried.reanchor(60, 22).offset(),
            9,
            "the carried window holds while the tail grows"
        );
        assert!(
            LiveScroll::at(5, 8, 22).at_top(),
            "a frame that fits the window lands in the plain live view"
        );
    }

    #[test]
    fn a_pinned_window_tracks_a_growing_tail() {
        let pinned = LiveScroll::start(ScrollStep::Bottom, 46, 22);
        assert_eq!(pinned.reanchor(50, 22).offset(), 28);
    }

    #[test]
    fn an_unpinned_window_holds_and_clamps() {
        let mut s = LiveScroll::start(ScrollStep::LineDown, 46, 22);
        for _ in 0..8 {
            s = s.step(ScrollStep::LineDown, 46, 22);
        }
        assert_eq!(s.offset(), 9);
        assert_eq!(s.reanchor(46, 22).offset(), 9, "an unpinned window holds");
        assert_eq!(
            s.reanchor(10, 22).offset(),
            0,
            "a shrunk frame pulls it back"
        );
    }

    #[test]
    fn any_step_but_bottom_unpins() {
        let pinned = LiveScroll::start(ScrollStep::Bottom, 46, 22);
        let unpinned = pinned.step(ScrollStep::LineUp, 46, 22);
        assert_eq!(unpinned.offset(), 23);
        assert_eq!(
            unpinned.reanchor(50, 22).offset(),
            23,
            "an unpinned window no longer tracks the tail"
        );
        let repinned = unpinned.step(ScrollStep::Bottom, 46, 22);
        assert_eq!(repinned.reanchor(50, 22).offset(), 28, "G re-pins");
    }

    #[test]
    fn reaching_the_top_reports_it() {
        let deep = LiveScroll::start(ScrollStep::PageDown, 46, 22);
        assert!(deep.step(ScrollStep::Top, 46, 22).at_top());
        let one = LiveScroll::start(ScrollStep::LineDown, 46, 22);
        assert!(one.step(ScrollStep::LineUp, 46, 22).at_top());
    }

    #[test]
    fn the_scrolled_row_names_the_range_between_time_and_cadence() {
        assert_eq!(
            scrolled_notice(
                "since 12:07:45",
                " · every 60s or on trigger · ? help",
                29,
                29,
                59
            ),
            "live · since 12:07:45 · lines 30-58 of 59 · every 60s or on trigger · ? help"
        );
        assert_eq!(
            scrolled_notice("changed 14s ago", " · every 2s · ? help", 8, 22, 46),
            "live · changed 14s ago · lines 9-30 of 46 · every 2s · ? help"
        );
        assert_eq!(
            scrolled_notice("since 12:07:45", " · 2 sources · ? help", 0, 10, 30),
            "live · since 12:07:45 · lines 1-10 of 30 · 2 sources · ? help"
        );
    }

    #[test]
    fn the_paused_row_names_the_visible_range() {
        assert_eq!(
            paused_notice("just now", 1, 22, 30),
            "paused · just now · lines 2-23 of 30 · Esc resumes"
        );
        let aged = paused_notice("14s ago", 8, 22, 30);
        assert!(aged.starts_with("paused · 14s ago · "), "got: {aged}");
        assert!(
            aged.ends_with("lines 9-30 of 30 · Esc resumes"),
            "got: {aged}"
        );
        assert_eq!(
            paused_notice("just now", 0, 0, 0),
            "paused · just now · empty frame · Esc resumes"
        );
        assert_eq!(
            paused_notice("14s ago", 5, 0, 30),
            "paused · 14s ago · line 6 of 30 · Esc resumes"
        );
    }
}
