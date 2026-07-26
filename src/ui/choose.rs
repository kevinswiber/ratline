use crate::ui::key::Key;
use crate::ui::loop_::Outcome;

/// Pure list-picker state; the command wraps it in a UiApp.
pub struct ChooseState {
    pub items: Vec<String>,
    pub cursor: usize,
    pub selected: Vec<bool>,
    selection_order: Vec<usize>,
    pub limit: Option<usize>,
    pub offset: usize,
    pub height: u16,
}

impl ChooseState {
    pub fn new(items: Vec<String>, limit: Option<usize>, height: u16) -> Self {
        let selected = vec![false; items.len()];
        ChooseState {
            items,
            cursor: 0,
            selected,
            selection_order: Vec::new(),
            limit,
            offset: 0,
            height: height.max(1),
        }
    }

    pub fn preselect(&mut self, labels: &[String]) {
        for (idx, item) in self.items.iter().enumerate() {
            if labels.contains(item) && !self.selected[idx] {
                self.selected[idx] = true;
                self.selection_order.push(idx);
            }
        }
    }

    fn selected_count(&self) -> usize {
        self.selected.iter().filter(|&&s| s).count()
    }

    fn toggle(&mut self) {
        if self.items.is_empty() {
            return;
        }
        let idx = self.cursor;
        if self.selected[idx] {
            self.selected[idx] = false;
            self.selection_order.retain(|&i| i != idx);
            return;
        }
        match self.limit {
            // Single-select replaces the previous choice.
            Some(1) => {
                self.selected.fill(false);
                self.selection_order.clear();
                self.selected[idx] = true;
                self.selection_order.push(idx);
            }
            Some(limit) if self.selected_count() >= limit => {}
            _ => {
                self.selected[idx] = true;
                self.selection_order.push(idx);
            }
        }
    }

    fn follow_cursor(&mut self) {
        let height = usize::from(self.height);
        if self.cursor < self.offset {
            self.offset = self.cursor;
        } else if self.cursor >= self.offset + height {
            self.offset = self.cursor + 1 - height;
        }
    }

    pub fn on_key(&mut self, key: Key) -> Outcome {
        match key {
            Key::Up | Key::Char('k') => {
                self.cursor = self.cursor.saturating_sub(1);
                self.follow_cursor();
            }
            Key::Down | Key::Char('j') => {
                if !self.items.is_empty() {
                    self.cursor = (self.cursor + 1).min(self.items.len() - 1);
                }
                self.follow_cursor();
            }
            Key::Home => {
                self.cursor = 0;
                self.follow_cursor();
            }
            Key::End => {
                self.cursor = self.items.len().saturating_sub(1);
                self.follow_cursor();
            }
            Key::Space => self.toggle(),
            Key::Enter => {
                // Single-select Enter picks the cursor item when nothing was
                // explicitly toggled.
                if self.limit == Some(1) && self.selected_count() == 0 && !self.items.is_empty() {
                    self.toggle();
                }
                return Outcome::Submit;
            }
            Key::Esc => return Outcome::Abort,
            _ => {}
        }
        Outcome::Continue
    }

    /// Chosen items: list order when `ordered`, otherwise selection order.
    pub fn results(&self, ordered: bool) -> Vec<String> {
        if ordered {
            self.items
                .iter()
                .zip(&self.selected)
                .filter(|(_, sel)| **sel)
                .map(|(item, _)| item.clone())
                .collect()
        } else {
            self.selection_order
                .iter()
                .map(|&i| self.items[i].clone())
                .collect()
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ui::key::Key;
    use crate::ui::loop_::Outcome;

    fn items(n: usize) -> Vec<String> {
        (0..n).map(|i| format!("item{i}")).collect()
    }

    #[test]
    fn cursor_clamps_at_both_ends() {
        let mut st = ChooseState::new(items(3), Some(1), 10);
        assert_eq!(st.on_key(Key::Up), Outcome::Continue);
        assert_eq!(st.cursor, 0);
        st.on_key(Key::Down);
        st.on_key(Key::Down);
        st.on_key(Key::Down);
        assert_eq!(st.cursor, 2);
    }

    #[test]
    fn single_select_space_replaces() {
        let mut st = ChooseState::new(items(3), Some(1), 10);
        st.on_key(Key::Space);
        st.on_key(Key::Down);
        st.on_key(Key::Space);
        assert_eq!(st.results(true), vec!["item1"]);
    }

    #[test]
    fn limit_two_blocks_a_third_selection() {
        let mut st = ChooseState::new(items(3), Some(2), 10);
        st.on_key(Key::Space);
        st.on_key(Key::Down);
        st.on_key(Key::Space);
        st.on_key(Key::Down);
        st.on_key(Key::Space);
        assert_eq!(st.results(true), vec!["item0", "item1"]);
    }

    #[test]
    fn no_limit_accepts_all() {
        let mut st = ChooseState::new(items(4), None, 10);
        for _ in 0..4 {
            st.on_key(Key::Space);
            st.on_key(Key::Down);
        }
        assert_eq!(st.results(true).len(), 4);
    }

    #[test]
    fn space_toggles_off() {
        let mut st = ChooseState::new(items(2), None, 10);
        st.on_key(Key::Space);
        st.on_key(Key::Space);
        assert!(st.results(true).is_empty());
    }

    #[test]
    fn enter_submits_and_esc_aborts() {
        let mut st = ChooseState::new(items(2), Some(1), 10);
        assert_eq!(st.on_key(Key::Enter), Outcome::Submit);
        assert_eq!(st.on_key(Key::Esc), Outcome::Abort);
    }

    #[test]
    fn single_select_enter_picks_cursor_item_when_nothing_selected() {
        let mut st = ChooseState::new(items(3), Some(1), 10);
        st.on_key(Key::Down);
        assert_eq!(st.on_key(Key::Enter), Outcome::Submit);
        assert_eq!(st.results(true), vec!["item1"]);
    }

    #[test]
    fn offset_follows_the_cursor_both_directions() {
        let mut st = ChooseState::new(items(10), Some(1), 3);
        for _ in 0..5 {
            st.on_key(Key::Down);
        }
        assert_eq!(st.cursor, 5);
        assert!(st.offset >= 3, "offset {} must follow down", st.offset);
        for _ in 0..5 {
            st.on_key(Key::Up);
        }
        assert_eq!(st.cursor, 0);
        assert_eq!(st.offset, 0);
    }

    #[test]
    fn results_in_selection_order_when_not_ordered() {
        let mut st = ChooseState::new(items(3), None, 10);
        st.on_key(Key::Down);
        st.on_key(Key::Down);
        st.on_key(Key::Space); // item2 first
        st.on_key(Key::Up);
        st.on_key(Key::Up);
        st.on_key(Key::Space); // item0 second
        assert_eq!(st.results(false), vec!["item2", "item0"]);
        assert_eq!(st.results(true), vec!["item0", "item2"]);
    }

    #[test]
    fn empty_list_submits_empty() {
        let mut st = ChooseState::new(Vec::new(), Some(1), 10);
        assert_eq!(st.on_key(Key::Enter), Outcome::Submit);
        assert!(st.results(true).is_empty());
    }

    #[test]
    fn preselect_marks_items() {
        let mut st = ChooseState::new(items(3), None, 10);
        st.preselect(&["item1".to_string(), "item2".to_string()]);
        assert_eq!(st.results(true), vec!["item1", "item2"]);
    }
}
