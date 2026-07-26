use nucleo_matcher::Matcher;

use crate::core::fuzzy::{Match, fuzzy_filter, new_matcher, substring_filter};
use crate::ui::input::InputState;
use crate::ui::key::Key;
use crate::ui::loop_::Outcome;

/// Fuzzy-filter state: a query editor over a candidate list. Space types
/// into the query; Tab toggles selection in multi mode.
pub struct FilterState {
    pub query: InputState,
    pub items: Vec<String>,
    pub matches: Vec<Match>,
    pub cursor: usize,
    pub selected: Vec<bool>,
    pub limit: Option<usize>,
    pub offset: usize,
    pub height: u16,
    pub fuzzy: bool,
    pub sort: bool,
    matcher: Matcher,
}

impl FilterState {
    pub fn new(
        items: Vec<String>,
        limit: Option<usize>,
        height: u16,
        fuzzy: bool,
        sort: bool,
        initial_query: String,
    ) -> Self {
        let selected = vec![false; items.len()];
        let mut state = FilterState {
            query: InputState::new(initial_query, false, 1000),
            items,
            matches: Vec::new(),
            cursor: 0,
            selected,
            limit,
            offset: 0,
            height: height.max(1),
            fuzzy,
            sort,
            matcher: new_matcher(),
        };
        state.refresh();
        state
    }

    fn refresh(&mut self) {
        self.matches = if self.fuzzy {
            fuzzy_filter(&mut self.matcher, &self.items, &self.query.value, self.sort)
        } else {
            substring_filter(&self.items, &self.query.value)
        };
        self.cursor = 0;
        self.offset = 0;
    }

    fn follow_cursor(&mut self) {
        let height = usize::from(self.height);
        if self.cursor < self.offset {
            self.offset = self.cursor;
        } else if self.cursor >= self.offset + height {
            self.offset = self.cursor + 1 - height;
        }
    }

    fn selected_count(&self) -> usize {
        self.selected.iter().filter(|&&s| s).count()
    }

    pub fn on_key(&mut self, key: Key) -> Outcome {
        match key {
            Key::Up => {
                self.cursor = self.cursor.saturating_sub(1);
                self.follow_cursor();
            }
            Key::Down => {
                if !self.matches.is_empty() {
                    self.cursor = (self.cursor + 1).min(self.matches.len() - 1);
                }
                self.follow_cursor();
            }
            Key::Tab => {
                if let Some(m) = self.matches.get(self.cursor) {
                    let idx = m.index;
                    if self.selected[idx] {
                        self.selected[idx] = false;
                    } else {
                        match self.limit {
                            Some(1) => {
                                self.selected.fill(false);
                                self.selected[idx] = true;
                            }
                            Some(limit) if self.selected_count() >= limit => {}
                            _ => self.selected[idx] = true,
                        }
                    }
                }
            }
            Key::Enter => return Outcome::Submit,
            Key::Esc => return Outcome::Abort,
            other => {
                let before = self.query.value.clone();
                self.query.on_key(other);
                if self.query.value != before {
                    self.refresh();
                }
            }
        }
        Outcome::Continue
    }

    /// Selected items in list order, else the item under the cursor.
    pub fn results(&self) -> Vec<String> {
        if self.selected_count() > 0 {
            return self
                .items
                .iter()
                .zip(&self.selected)
                .filter(|(_, sel)| **sel)
                .map(|(item, _)| item.clone())
                .collect();
        }
        self.matches
            .get(self.cursor)
            .map(|m| vec![self.items[m.index].clone()])
            .unwrap_or_default()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn state(items: &[&str]) -> FilterState {
        FilterState::new(
            items.iter().map(|s| s.to_string()).collect(),
            Some(1),
            10,
            true,
            true,
            String::new(),
        )
    }

    #[test]
    fn typing_narrows_and_resets_the_cursor() {
        let mut st = state(&["apple", "banana", "apricot"]);
        st.on_key(Key::Down);
        assert_eq!(st.cursor, 1);
        for c in "ap".chars() {
            st.on_key(Key::Char(c));
        }
        assert_eq!(st.cursor, 0);
        assert_eq!(st.matches.len(), 2);
    }

    #[test]
    fn space_types_into_the_query() {
        let mut st = state(&["a b", "ab"]);
        st.on_key(Key::Char('a'));
        st.on_key(Key::Space);
        st.on_key(Key::Char('b'));
        assert_eq!(st.query.value, "a b");
    }

    #[test]
    fn tab_toggles_selection_in_multi_mode() {
        let mut st = FilterState::new(
            vec!["one".into(), "two".into()],
            None,
            10,
            true,
            true,
            String::new(),
        );
        st.on_key(Key::Tab);
        st.on_key(Key::Down);
        st.on_key(Key::Tab);
        assert_eq!(st.results(), vec!["one", "two"]);
    }

    #[test]
    fn results_fall_back_to_the_cursor_item() {
        let mut st = state(&["alpha", "beta"]);
        st.on_key(Key::Down);
        assert_eq!(st.results(), vec!["beta"]);
    }

    #[test]
    fn no_matches_yield_empty_results() {
        let mut st = state(&["alpha"]);
        for c in "zzz".chars() {
            st.on_key(Key::Char(c));
        }
        assert!(st.results().is_empty());
    }
}
