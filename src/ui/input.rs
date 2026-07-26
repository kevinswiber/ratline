use crate::ui::key::Key;
use crate::ui::loop_::Outcome;

/// Readline-ish single-line editor state. The cursor is a char index.
pub struct InputState {
    pub value: String,
    pub cursor: usize,
    pub password: bool,
    pub char_limit: usize,
}

impl InputState {
    pub fn new(value: String, password: bool, char_limit: usize) -> Self {
        let cursor = value.chars().count();
        InputState {
            value,
            cursor,
            password,
            char_limit,
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

    /// What to paint: bullets for passwords, the placeholder when empty.
    pub fn display(&self, placeholder: &str) -> String {
        if self.value.is_empty() {
            return placeholder.to_string();
        }
        if self.password {
            "\u{2022}".repeat(self.char_count())
        } else {
            self.value.clone()
        }
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
        assert_eq!(st.display("placeholder"), "•••••");
    }

    #[test]
    fn empty_value_shows_the_placeholder() {
        let st = InputState::new(String::new(), false, 400);
        assert_eq!(st.display("Type here..."), "Type here...");
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
}
