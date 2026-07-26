use crate::ui::key::Key;
use crate::ui::loop_::Outcome;

/// Yes/no state: Yes sits on the left, No on the right.
pub struct ConfirmState {
    pub affirmative: bool,
}

impl ConfirmState {
    pub fn on_key(&mut self, key: Key) -> Outcome {
        match key {
            Key::Char('y') | Key::Char('Y') => {
                self.affirmative = true;
                Outcome::Submit
            }
            Key::Char('n') | Key::Char('N') => {
                self.affirmative = false;
                Outcome::Submit
            }
            Key::Left | Key::Char('h') => {
                self.affirmative = true;
                Outcome::Continue
            }
            Key::Right | Key::Char('l') => {
                self.affirmative = false;
                Outcome::Continue
            }
            Key::Tab => {
                self.affirmative = !self.affirmative;
                Outcome::Continue
            }
            Key::Enter => Outcome::Submit,
            Key::Esc => Outcome::Abort,
            _ => Outcome::Continue,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ui::key::Key;
    use crate::ui::loop_::Outcome;

    #[test]
    fn y_and_n_submit_immediately() {
        let mut st = ConfirmState { affirmative: true };
        assert_eq!(st.on_key(Key::Char('n')), Outcome::Submit);
        assert!(!st.affirmative);
        let mut st = ConfirmState { affirmative: false };
        assert_eq!(st.on_key(Key::Char('Y')), Outcome::Submit);
        assert!(st.affirmative);
    }

    #[test]
    fn arrows_select_and_tab_toggles_enter_submits() {
        // Yes sits on the left, No on the right.
        let mut st = ConfirmState { affirmative: false };
        st.on_key(Key::Left);
        assert!(st.affirmative);
        st.on_key(Key::Right);
        assert!(!st.affirmative);
        st.on_key(Key::Tab);
        assert!(st.affirmative);
        assert_eq!(st.on_key(Key::Enter), Outcome::Submit);
    }

    #[test]
    fn esc_aborts() {
        let mut st = ConfirmState { affirmative: true };
        assert_eq!(st.on_key(Key::Esc), Outcome::Abort);
    }
}
