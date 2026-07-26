use nucleo_matcher::pattern::{CaseMatching, Normalization, Pattern};
use nucleo_matcher::{Config, Matcher, Utf32Str};

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Match {
    pub index: usize,
    pub score: u32,
    pub positions: Vec<u32>,
}

pub fn new_matcher() -> Matcher {
    Matcher::new(Config::DEFAULT)
}

/// Fuzzy-match `query` against every item; empty queries match everything.
/// Sorted output ranks by score (stable on index).
pub fn fuzzy_filter(
    matcher: &mut Matcher,
    items: &[String],
    query: &str,
    sort: bool,
) -> Vec<Match> {
    if query.is_empty() {
        return items
            .iter()
            .enumerate()
            .map(|(index, _)| Match {
                index,
                score: 0,
                positions: Vec::new(),
            })
            .collect();
    }
    let pattern = Pattern::parse(query, CaseMatching::Smart, Normalization::Smart);
    let mut buf = Vec::new();
    let mut matches: Vec<Match> = items
        .iter()
        .enumerate()
        .filter_map(|(index, item)| {
            let haystack = Utf32Str::new(item, &mut buf);
            let mut positions = Vec::new();
            pattern
                .indices(haystack, matcher, &mut positions)
                .map(|score| {
                    positions.sort_unstable();
                    positions.dedup();
                    Match {
                        index,
                        score,
                        positions,
                    }
                })
        })
        .collect();
    if sort {
        matches.sort_by(|a, b| b.score.cmp(&a.score).then(a.index.cmp(&b.index)));
    }
    matches
}

/// Case-insensitive substring matching for --no-fuzzy.
pub fn substring_filter(items: &[String], query: &str) -> Vec<Match> {
    if query.is_empty() {
        return fuzzy_filter(&mut new_matcher(), items, "", false);
    }
    let needle = query.to_lowercase();
    items
        .iter()
        .enumerate()
        .filter_map(|(index, item)| {
            let hay = item.to_lowercase();
            hay.find(&needle).map(|byte_start| {
                let char_start = hay[..byte_start].chars().count() as u32;
                let len = needle.chars().count() as u32;
                Match {
                    index,
                    score: 0,
                    positions: (char_start..char_start + len).collect(),
                }
            })
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn items(list: &[&str]) -> Vec<String> {
        list.iter().map(|s| s.to_string()).collect()
    }

    #[test]
    fn empty_query_returns_all_in_original_order() {
        let mut m = new_matcher();
        let items = items(&["b", "a", "c"]);
        let matches = fuzzy_filter(&mut m, &items, "", true);
        let indices: Vec<usize> = matches.iter().map(|m| m.index).collect();
        assert_eq!(indices, vec![0, 1, 2]);
    }

    #[test]
    fn exact_substring_outranks_scattered() {
        let mut m = new_matcher();
        let items = items(&["a-p-p-l-e sauce", "apple pie"]);
        let matches = fuzzy_filter(&mut m, &items, "apple", true);
        assert_eq!(matches[0].index, 1, "contiguous match should rank first");
    }

    #[test]
    fn non_matches_are_dropped() {
        let mut m = new_matcher();
        let items = items(&["apple", "banana"]);
        let matches = fuzzy_filter(&mut m, &items, "app", true);
        assert_eq!(matches.len(), 1);
        assert_eq!(matches[0].index, 0);
    }

    #[test]
    fn positions_point_at_matched_chars() {
        let mut m = new_matcher();
        let items = items(&["xaybzc"]);
        let matches = fuzzy_filter(&mut m, &items, "abc", true);
        assert_eq!(matches[0].positions, vec![1, 3, 5]);
    }

    #[test]
    fn unsorted_keeps_item_order() {
        let mut m = new_matcher();
        let items = items(&["zebra apple", "apple"]);
        let matches = fuzzy_filter(&mut m, &items, "apple", false);
        let indices: Vec<usize> = matches.iter().map(|m| m.index).collect();
        assert_eq!(indices, vec![0, 1]);
    }

    #[test]
    fn substring_filter_is_case_insensitive() {
        let items = items(&["Apple Pie", "banana"]);
        let matches = substring_filter(&items, "apple");
        assert_eq!(matches.len(), 1);
        assert_eq!(matches[0].index, 0);
        assert_eq!(matches[0].positions, vec![0, 1, 2, 3, 4]);
    }
}
