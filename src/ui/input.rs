use unicode_width::UnicodeWidthStr;

use crate::ui::key::Key;
use crate::ui::loop_::Outcome;

/// Readline-ish single-line editor state. The cursor is a char index.
pub struct InputState {
    pub value: String,
    pub cursor: usize,
    pub password: bool,
    pub char_limit: usize,
    /// First displayed char: the field scrolls horizontally so a value
    /// longer than the field stays editable and the caret always has a
    /// cell of its own. Maintained by `follow`, which is the only writer.
    pub offset: usize,
}

impl InputState {
    pub fn new(value: String, password: bool, char_limit: usize) -> Self {
        let cursor = value.chars().count();
        InputState {
            value,
            cursor,
            password,
            char_limit,
            offset: 0,
        }
    }

    /// Slide the window so the caret is inside a field `avail` cells
    /// wide, moving as little as possible. The caret needs a cell of its
    /// own — it can sit one past the last char — so the text may only
    /// occupy `avail - 1` cells when the caret trails it.
    pub fn follow(&mut self, avail: usize) {
        if self.value.is_empty() {
            self.offset = 0;
            return;
        }
        let room = avail.saturating_sub(1);
        if self.offset > self.cursor {
            self.offset = self.cursor;
        }
        // Widen leftward is done; now pull the window right until the
        // caret's own cell fits. Dropping one char at a time keeps wide
        // glyphs whole — a window never starts mid-glyph.
        while self.cells_between(self.offset, self.cursor) > room && self.offset < self.cursor {
            self.offset += 1;
        }
    }

    /// Display cells spanned by chars `[from, to)` of the shown text.
    fn cells_between(&self, from: usize, to: usize) -> usize {
        let shown = self.shown();
        let slice: String = shown
            .chars()
            .skip(from)
            .take(to.saturating_sub(from))
            .collect();
        slice.width()
    }

    /// The caret's column within the field.
    pub fn caret_col(&self) -> usize {
        self.cells_between(self.offset, self.cursor)
    }

    /// What the field paints: the shown text from `offset` on, or the
    /// placeholder when the value is empty.
    pub fn visible(&self, placeholder: &str) -> String {
        if self.value.is_empty() {
            return placeholder.to_string();
        }
        self.shown().chars().skip(self.offset).collect()
    }

    /// The value as it is displayed — bullets for passwords.
    fn shown(&self) -> String {
        if self.password {
            "\u{2022}".repeat(self.char_count())
        } else {
            self.value.clone()
        }
    }

    fn byte_at(&self, char_idx: usize) -> usize {
        self.value
            .char_indices()
            .nth(char_idx)
            .map_or(self.value.len(), |(b, _)| b)
    }

    fn char_count(&self) -> usize {
        self.value.chars().count()
    }

    pub fn on_key(&mut self, key: Key) -> Outcome {
        match key {
            Key::Char(c) => {
                if self.char_count() < self.char_limit {
                    let at = self.byte_at(self.cursor);
                    self.value.insert(at, c);
                    self.cursor += 1;
                }
            }
            Key::Space => {
                if self.char_count() < self.char_limit {
                    let at = self.byte_at(self.cursor);
                    self.value.insert(at, ' ');
                    self.cursor += 1;
                }
            }
            Key::Backspace => {
                if self.cursor > 0 {
                    let at = self.byte_at(self.cursor - 1);
                    self.value.remove(at);
                    self.cursor -= 1;
                }
            }
            Key::Delete => {
                if self.cursor < self.char_count() {
                    let at = self.byte_at(self.cursor);
                    self.value.remove(at);
                }
            }
            Key::Left => self.cursor = self.cursor.saturating_sub(1),
            Key::Right => self.cursor = (self.cursor + 1).min(self.char_count()),
            Key::Home | Key::CtrlA => self.cursor = 0,
            Key::End | Key::CtrlE => self.cursor = self.char_count(),
            Key::CtrlU => {
                let at = self.byte_at(self.cursor);
                self.value.drain(..at);
                self.cursor = 0;
            }
            Key::CtrlW => {
                let end = self.byte_at(self.cursor);
                let head = &self.value[..end];
                let trimmed = head.trim_end_matches(' ');
                let word_start = trimmed.rfind(' ').map_or(0, |i| i + 1);
                let removed = self.value[word_start..end].chars().count();
                self.value.drain(word_start..end);
                self.cursor -= removed;
            }
            Key::Enter => return Outcome::Submit,
            Key::Esc => return Outcome::Abort,
            _ => {}
        }
        Outcome::Continue
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ui::key::Key;
    use crate::ui::loop_::Outcome;

    fn typed(s: &str) -> InputState {
        let mut st = InputState::new(String::new(), false, 400);
        for c in s.chars() {
            st.on_key(Key::Char(c));
        }
        st
    }

    #[test]
    fn typing_inserts_at_the_cursor() {
        let mut st = typed("abc");
        st.on_key(Key::Left);
        st.on_key(Key::Char('X'));
        assert_eq!(st.value, "abXc");
        assert_eq!(st.cursor, 3);
    }

    #[test]
    fn backspace_at_zero_is_a_noop() {
        let mut st = InputState::new(String::new(), false, 400);
        assert_eq!(st.on_key(Key::Backspace), Outcome::Continue);
        assert_eq!(st.value, "");
        let mut st = typed("ab");
        st.on_key(Key::Backspace);
        assert_eq!(st.value, "a");
    }

    #[test]
    fn ctrl_a_and_e_jump_to_the_ends() {
        let mut st = typed("hello");
        st.on_key(Key::CtrlA);
        assert_eq!(st.cursor, 0);
        st.on_key(Key::CtrlE);
        assert_eq!(st.cursor, 5);
    }

    #[test]
    fn ctrl_u_kills_before_the_cursor() {
        let mut st = typed("hello world");
        st.on_key(Key::CtrlU);
        assert_eq!(st.value, "");
        let mut st = typed("hello");
        st.on_key(Key::Left);
        st.on_key(Key::CtrlU);
        assert_eq!(st.value, "o");
        assert_eq!(st.cursor, 0);
    }

    #[test]
    fn ctrl_w_deletes_the_previous_word() {
        let mut st = typed("hello brave world");
        st.on_key(Key::CtrlW);
        assert_eq!(st.value, "hello brave ");
        st.on_key(Key::CtrlW);
        assert_eq!(st.value, "hello ");
    }

    #[test]
    fn char_limit_blocks_excess() {
        let mut st = InputState::new(String::new(), false, 3);
        for c in "abcd".chars() {
            st.on_key(Key::Char(c));
        }
        assert_eq!(st.value, "abc");
    }

    #[test]
    fn password_display_masks_by_char_count() {
        let mut st = InputState::new(String::new(), true, 400);
        for c in "héllo".chars() {
            st.on_key(Key::Char(c));
        }
        assert_eq!(st.visible("placeholder"), "•••••");
    }

    #[test]
    fn empty_value_shows_the_placeholder() {
        let st = InputState::new(String::new(), false, 400);
        assert_eq!(st.visible("Type here..."), "Type here...");
    }

    #[test]
    fn enter_submits_esc_aborts() {
        let mut st = typed("x");
        assert_eq!(st.on_key(Key::Enter), Outcome::Submit);
        assert_eq!(st.on_key(Key::Esc), Outcome::Abort);
    }

    #[test]
    fn initial_value_starts_with_cursor_at_end() {
        let st = InputState::new("abc".to_string(), false, 400);
        assert_eq!(st.cursor, 3);
    }

    #[test]
    fn a_field_wider_than_the_value_never_scrolls() {
        let mut st = typed("abc");
        st.follow(20);
        assert_eq!(st.offset, 0);
        assert_eq!(st.caret_col(), 3);
        assert_eq!(st.visible("ph"), "abc");
    }

    #[test]
    fn the_caret_stays_inside_a_full_field() {
        // The reported bug: with the value exactly filling the field the
        // caret belongs one past the last cell, which does not exist. The
        // window slides by one so the caret keeps its own cell.
        let mut st = typed("abcde");
        st.follow(5);
        assert_eq!(st.offset, 1, "the window slid to free the caret's cell");
        assert_eq!(st.visible("ph"), "bcde");
        assert_eq!(st.caret_col(), 4, "caret sits after the last shown char");
        assert!(st.caret_col() < 5, "the caret is always inside the field");
    }

    #[test]
    fn typing_on_past_the_edge_keeps_scrolling() {
        let mut st = typed("abcdefgh");
        st.follow(4);
        assert_eq!(st.visible("ph"), "fgh");
        assert_eq!(st.caret_col(), 3);
        st.on_key(Key::Char('i'));
        st.follow(4);
        assert_eq!(st.visible("ph"), "ghi");
        assert_eq!(st.caret_col(), 3);
    }

    #[test]
    fn moving_back_scrolls_the_window_minimally() {
        let mut st = typed("abcdefgh");
        st.follow(4);
        assert_eq!(st.offset, 5);
        // Left inside the window does not move it.
        st.on_key(Key::Left);
        st.follow(4);
        assert_eq!(st.offset, 5);
        assert_eq!(st.caret_col(), 2);
        // Walking off the left edge slides the window one char at a time.
        for _ in 0..3 {
            st.on_key(Key::Left);
        }
        st.follow(4);
        assert_eq!(st.offset, 4);
        assert_eq!(st.caret_col(), 0);
        // Home returns the window to the start. `visible` yields the text
        // from the offset on; the frame clips it at the field's edge.
        st.on_key(Key::Home);
        st.follow(4);
        assert_eq!(st.offset, 0);
        assert_eq!(st.caret_col(), 0);
        assert_eq!(st.visible("ph"), "abcdefgh");
    }

    #[test]
    fn wide_glyphs_scroll_by_cells_not_chars() {
        let mut st = InputState::new(String::new(), false, 400);
        for c in "日本語".chars() {
            st.on_key(Key::Char(c));
        }
        // Six cells of text in a four-cell field: only the last full glyph
        // plus the caret cell fit, and the window never splits a glyph.
        st.follow(4);
        assert_eq!(st.visible("ph"), "語");
        assert_eq!(st.caret_col(), 2);
    }

    #[test]
    fn a_password_scrolls_by_its_bullets() {
        let mut st = InputState::new(String::new(), true, 400);
        for c in "secret".chars() {
            st.on_key(Key::Char(c));
        }
        st.follow(4);
        assert_eq!(st.visible("ph"), "\u{2022}\u{2022}\u{2022}");
        assert_eq!(st.caret_col(), 3);
    }

    #[test]
    fn an_empty_value_shows_the_placeholder_unscrolled() {
        let mut st = InputState::new(String::new(), false, 400);
        st.follow(4);
        assert_eq!(st.offset, 0);
        assert_eq!(st.caret_col(), 0);
        assert_eq!(st.visible("Type here"), "Type here");
    }

    #[test]
    fn a_degenerate_field_still_answers() {
        // A one-cell field (or none) must not panic or place the caret
        // outside itself.
        let mut st = typed("abc");
        st.follow(1);
        assert_eq!(st.caret_col(), 0);
        st.follow(0);
        assert_eq!(st.caret_col(), 0);
    }
}
