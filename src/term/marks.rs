//! Change marks for `watch`: which lines and which characters of a
//! frame differ from the previous distinct frame.
//!
//! One character-level diff over the whole ESCAPE-STRIPPED frame feeds
//! both markers. Diffing bytes that still contain escapes leaks
//! sequence fragments into the output and invents changes when only
//! the styling moved; stripping first makes styling-only changes
//! structurally invisible here.

/// One line's change marks. `changed` drives the margin gutter;
/// `cells` — maximal non-whitespace runs, as CHAR indices into the
/// line's stripped text — drive the in-content character highlights.
/// Non-empty `cells` implies `changed`; `changed` with empty `cells`
/// is a deletion seam (nothing remains in the new frame to mark).
#[derive(Clone, Default, PartialEq, Eq, Debug)]
pub struct LineMark {
    pub changed: bool,
    pub cells: Vec<std::ops::Range<usize>>,
}

/// Per-line change marks for `next` against the previous distinct
/// frame: strip escapes, run one character-level diff over the whole
/// frame, map the chunks back to lines in one linear cursor walk.
/// `None` prev means "nothing to compare" — no marks.
pub fn changed_marks(prev: Option<&[String]>, next: &[String]) -> Vec<LineMark> {
    let mut marks = vec![LineMark::default(); next.len()];
    let Some(prev) = prev else { return marks };
    let prev_text = joined_stripped(prev);
    let next_text = joined_stripped(next);
    // One cursor over `next_text`, maintained across ALL chunks:
    // `line` and `col` are the char position in the stripped frame.
    // Equal advances, Insert marks as it advances, Delete marks its
    // seam without advancing — linear in the frame, never a rescan.
    let mut line = 0usize;
    let mut col = 0usize;
    let mut run: Option<std::ops::Range<usize>> = None;
    for chunk in dissimilar::diff(&prev_text, &next_text) {
        match chunk {
            dissimilar::Chunk::Equal(t) => {
                for ch in t.chars() {
                    if ch == '\n' {
                        line += 1;
                        col = 0;
                    } else {
                        col += 1;
                    }
                }
            }
            dissimilar::Chunk::Insert(t) => {
                for ch in t.chars() {
                    if ch == '\n' {
                        flush_run(run.take(), line, &mut marks);
                        line += 1;
                        col = 0;
                    } else if ch.is_whitespace() {
                        flush_run(run.take(), line, &mut marks);
                        col += 1;
                    } else {
                        match &mut run {
                            Some(r) if r.end == col => r.end += 1,
                            _ => {
                                flush_run(run.take(), line, &mut marks);
                                run = Some(col..col + 1);
                            }
                        }
                        col += 1;
                    }
                }
                flush_run(run.take(), line, &mut marks);
            }
            dissimilar::Chunk::Delete(t) => {
                if !t.trim().is_empty() && !next.is_empty() {
                    // A delete opening with \n cut content that lived
                    // AFTER this line's end, so the seam is the next
                    // line's start — the diff may attach the separator
                    // to either edge of a line-level edit, and the
                    // seam must not depend on that factoring.
                    let seam = if t.starts_with('\n') { line + 1 } else { line };
                    if let Some(m) = marks.get_mut(seam.min(next.len() - 1)) {
                        m.changed = true;
                    }
                }
            }
        }
    }
    marks
}

/// Close an open cell run: record it and mark its line changed —
/// insertions mark lines exactly through their cells, so a
/// whitespace-only inserted line never marks at either grain.
fn flush_run(run: Option<std::ops::Range<usize>>, line: usize, marks: &mut [LineMark]) {
    if let Some(range) = run
        && let Some(m) = marks.get_mut(line)
    {
        m.cells.push(range);
        m.changed = true;
    }
}

/// Display columns the gutter occupies: the mark cell and a gap.
pub const GUTTER_COLS: usize = 2;

/// Prefix window rows with their gutter cells. `marks` indexes the
/// FULL source frame; `start` is the window's first source line.
/// `mark_cell` is the pre-rendered marked cell (styled mark + space) —
/// opaque here, the caller owns glyph, style, and profile; unmarked
/// and out-of-range rows get two spaces.
pub fn prefix_rows(
    rows: Vec<String>,
    marks: &[LineMark],
    start: usize,
    mark_cell: &str,
) -> Vec<String> {
    rows.into_iter()
        .enumerate()
        .map(|(i, row)| {
            let cell = if marks.get(start + i).is_some_and(|m| m.changed) {
                mark_cell
            } else {
                "  "
            };
            format!("{cell}{row}")
        })
        .collect()
}

/// Splice reverse video onto the changed character runs of one
/// pre-rendered line: open `\x1b[7m`, close `\x1b[0m` plus a replay of
/// the child's open SGR state, and re-assert the mark after any child
/// escape inside a run — the mark PATCHES an attribute over the
/// child's own styling, never replaces its colors. `cells` are char
/// indices into the STRIPPED line, sorted and non-overlapping
/// (`changed_marks` guarantees both). Empty `cells` returns the line
/// unchanged.
// Wired into the watch loop by the highlight toggle; the pure splice
// and its tests land first.
#[cfg_attr(not(test), allow(dead_code))]
pub fn mark_cells(line: &str, cells: &[std::ops::Range<usize>]) -> String {
    if cells.is_empty() {
        return line.to_string();
    }
    let mut out = String::new();
    let mut state = crate::core::measure::SgrState::default();
    let mut inside = false;
    let mut idx = 0usize; // char index in the stripped line
    let mut next = 0usize; // cursor into cells
    for chunk in crate::core::measure::chunks(line) {
        match chunk {
            crate::core::measure::Chunk::Escape(e) => {
                state.apply(e);
                out.push_str(e);
                if inside {
                    out.push_str("\x1b[7m");
                }
            }
            crate::core::measure::Chunk::Text(t, _) => {
                while next < cells.len() && cells[next].end <= idx {
                    next += 1;
                }
                let marked = next < cells.len() && cells[next].contains(&idx);
                if marked && !inside {
                    out.push_str("\x1b[7m");
                    inside = true;
                } else if !marked && inside {
                    out.push_str("\x1b[0m");
                    out.push_str(&state.prefix());
                    inside = false;
                }
                out.push_str(t);
                idx += 1;
            }
        }
    }
    if inside {
        out.push_str("\x1b[0m");
        out.push_str(&state.prefix());
    }
    out
}

fn joined_stripped(lines: &[String]) -> String {
    lines
        .iter()
        .map(|l| crate::core::measure::strip_escapes(l))
        .collect::<Vec<_>>()
        .join("\n")
}

#[cfg(test)]
// A one-range slice like `&[2..4]` is exactly what a single marked run
// looks like — not a mistyped `(2..4).collect()`.
#[allow(clippy::single_range_in_vec_init)]
mod tests {
    use super::*;

    fn frame(items: &[&str]) -> Vec<String> {
        items.iter().map(|s| s.to_string()).collect()
    }

    /// The gutter view of a marks vector, for terse assertions.
    fn changed(marks: &[LineMark]) -> Vec<bool> {
        marks.iter().map(|m| m.changed).collect()
    }

    /// A LineMark vector from gutter bools, for the prefix tests.
    fn line_marks(changed: &[bool]) -> Vec<LineMark> {
        changed
            .iter()
            .map(|&c| LineMark {
                changed: c,
                cells: Vec::new(),
            })
            .collect()
    }

    #[test]
    fn marked_rows_get_the_mark_cell_and_unmarked_rows_get_spaces() {
        let rows = frame(&["one", "two"]);
        assert_eq!(
            prefix_rows(rows, &line_marks(&[true, false]), 0, "\x1b[1mM\x1b[0m "),
            frame(&["\x1b[1mM\x1b[0m one", "  two"])
        );
    }

    #[test]
    fn the_window_start_offsets_into_the_marks() {
        // The window shows source lines 2..4 of a five-line frame;
        // only source line 3 changed.
        let rows = frame(&["line-two", "line-three"]);
        assert_eq!(
            prefix_rows(
                rows,
                &line_marks(&[false, false, false, true, false]),
                2,
                "M "
            ),
            frame(&["  line-two", "M line-three"])
        );
    }

    #[test]
    fn rows_beyond_the_marks_get_spaces() {
        // Defensive: a marks/rows mismatch must never panic mid-paint.
        let rows = frame(&["a", "b"]);
        assert_eq!(
            prefix_rows(rows, &line_marks(&[true]), 0, "M "),
            frame(&["M a", "  b"])
        );
    }

    #[test]
    fn the_gutter_cell_occupies_its_declared_columns() {
        use crate::core::measure::display_width;
        let plain = prefix_rows(frame(&["x"]), &line_marks(&[false]), 0, "M ");
        assert_eq!(display_width(&plain[0]), GUTTER_COLS + 1);
        let marked = prefix_rows(frame(&["x"]), &line_marks(&[true]), 0, "M ");
        assert_eq!(display_width(&marked[0]), GUTTER_COLS + 1);
    }

    #[test]
    fn escape_carrying_rows_are_prefixed_untouched() {
        let rows = frame(&["\x1b[31mred\x1b[0m"]);
        assert_eq!(
            prefix_rows(rows, &line_marks(&[true]), 0, "M "),
            frame(&["M \x1b[31mred\x1b[0m"])
        );
    }

    #[test]
    fn empty_cells_return_the_line_unchanged() {
        assert_eq!(mark_cells("abc", &[]), "abc");
        assert_eq!(mark_cells("\x1b[31mabc\x1b[0m", &[]), "\x1b[31mabc\x1b[0m");
    }

    #[test]
    fn a_run_is_wrapped_in_reverse_video() {
        assert_eq!(mark_cells("abcdef", &[2..4]), "ab\x1b[7mcd\x1b[0mef");
    }

    #[test]
    fn closing_a_run_replays_the_childs_open_state() {
        // The child's red must survive past the mark (patch, don't
        // replace): the close is reset + replay, so `ef` is red again,
        // exactly as the child painted it.
        assert_eq!(
            mark_cells("\x1b[31mabcdef\x1b[0m", &[2..4]),
            "\x1b[31mab\x1b[7mcd\x1b[0m\x1b[31mef\x1b[0m"
        );
    }

    #[test]
    fn a_child_reset_inside_a_run_cannot_strip_the_mark() {
        assert_eq!(
            mark_cells("ab\x1b[0mcd", &[1..3]),
            "a\x1b[7mb\x1b[0m\x1b[7mc\x1b[0md"
        );
    }

    #[test]
    fn a_run_reaching_the_line_end_reopens_the_childs_state() {
        // The child left red open at line end; after the mark closes,
        // the line must end in the same open state the raw line did.
        assert_eq!(
            mark_cells("\x1b[31mabc", &[2..3]),
            "\x1b[31mab\x1b[7mc\x1b[0m\x1b[31m"
        );
    }

    #[test]
    fn cell_indices_are_chars_not_bytes_or_columns() {
        // '日' is char index 1 — the same coordinate system
        // changed_marks produces.
        assert_eq!(mark_cells("a日b", &[1..2]), "a\x1b[7m日\x1b[0mb");
    }

    #[test]
    fn a_shift_cut_through_a_marked_run_keeps_the_mark() {
        // Composition with the chopped renderer (splice BEFORE chop):
        // shift_chop's replay reopens the mark when the cut lands
        // inside a run.
        use crate::core::measure::shift_chop;
        let spliced = mark_cells("abcdef", &[3..5]);
        assert_eq!(spliced, "abc\x1b[7mde\x1b[0mf");
        assert_eq!(shift_chop(&spliced, 4, 10), "\x1b[7me\x1b[0mf");
    }

    #[test]
    fn splices_never_change_the_row_math() {
        // Zero-width escapes leave rendered_rows untouched, so
        // wrapped-mode highlights cannot shift row accounting.
        use crate::term::inline::rendered_rows;
        let long = "word ".repeat(40); // wraps at 80 cols
        let spliced = mark_cells(&long, &[6..10, 96..104]);
        assert_eq!(rendered_rows(&[spliced], 80), rendered_rows(&[long], 80));
    }

    #[test]
    fn two_runs_are_marked_independently() {
        assert_eq!(
            mark_cells("abcd", &[1..2, 3..4]),
            "a\x1b[7mb\x1b[0mc\x1b[7md\x1b[0m"
        );
    }

    #[test]
    fn an_inserted_line_marks_only_itself() {
        // The cascade pin: a positional diff marks all five lines here;
        // the whole-frame char diff must mark exactly the insertion.
        let prev = frame(&["alpha   aaa", "bravo   bbb", "charlie ccc", "delta   ddd"]);
        let next = frame(&[
            "EXTRA HEADER LINE",
            "alpha   aaa",
            "bravo   bbb",
            "charlie ccc",
            "delta   ddd",
        ]);
        let marks = changed_marks(Some(&prev), &next);
        assert_eq!(changed(&marks), vec![true, false, false, false, false]);
        // The inserted line's cells are its non-whitespace words.
        assert_eq!(marks[0].cells, vec![0..5, 6..12, 13..17]);
    }

    #[test]
    fn a_changed_value_marks_its_line_and_cells() {
        let prev = frame(&["header", "count: 41", "footer"]);
        let next = frame(&["header", "count: 42", "footer"]);
        let marks = changed_marks(Some(&prev), &next);
        assert_eq!(changed(&marks), vec![false, true, false]);
        // "count: 41" → "count: 42": only the final char is an insert.
        assert_eq!(marks[1].cells, vec![8..9]);
        assert!(marks[0].cells.is_empty() && marks[2].cells.is_empty());
    }

    #[test]
    fn a_multi_line_insert_marks_every_inserted_line() {
        let prev = frame(&["a", "z"]);
        let next = frame(&["a", "new one", "new two", "z"]);
        let marks = changed_marks(Some(&prev), &next);
        assert_eq!(changed(&marks), vec![false, true, true, false]);
        assert!(!marks[1].cells.is_empty() && !marks[2].cells.is_empty());
    }

    #[test]
    fn whitespace_splits_a_cell_run() {
        let prev = frame(&["x", "zz"]);
        let next = frame(&["x", "ab cd"]);
        let marks = changed_marks(Some(&prev), &next);
        // Two words, two runs — never one run spanning the space.
        assert_eq!(marks[1].cells, vec![0..2, 3..5]);
    }

    #[test]
    fn cell_indices_count_chars_not_bytes() {
        let prev = frame(&["ab"]);
        let next = frame(&["a日"]);
        let marks = changed_marks(Some(&prev), &next);
        // '日' is char index 1 (byte index would be 1..4).
        assert_eq!(marks[0].cells, vec![1..2]);
    }

    #[test]
    fn an_sgr_only_change_marks_nothing() {
        // The diff never sees escape bytes, so a line that merely
        // changed color cannot mark.
        let prev = frame(&["\x1b[32mSTATUS ok\x1b[0m", "tail"]);
        let next = frame(&["\x1b[31mSTATUS ok\x1b[0m", "tail"]);
        assert_eq!(
            changed_marks(Some(&prev), &next),
            vec![LineMark::default(), LineMark::default()]
        );
    }

    #[test]
    fn a_whitespace_only_change_marks_nothing() {
        // Column realignment noise: pure whitespace insertion is not a
        // change worth a mark.
        let prev = frame(&["name  val", "x"]);
        let next = frame(&["name   val", "x"]);
        assert_eq!(
            changed(&changed_marks(Some(&prev), &next)),
            vec![false, false]
        );
    }

    #[test]
    fn a_real_change_beside_whitespace_still_marks() {
        // A digit growing (9 → 10) shifts spacing too; the non-space
        // part of the change must still mark the line.
        let prev = frame(&["val  9 end", "x"]);
        let next = frame(&["val 10 end", "x"]);
        let marks = changed_marks(Some(&prev), &next);
        assert_eq!(changed(&marks), vec![true, false]);
        assert!(!marks[0].cells.is_empty());
    }

    #[test]
    fn a_deleted_line_marks_the_seam_with_no_cells() {
        let prev = frame(&["head", "gone", "tail"]);
        let next = frame(&["head", "tail"]);
        let marks = changed_marks(Some(&prev), &next);
        assert_eq!(changed(&marks), vec![false, true]);
        // Nothing exists in the new frame to highlight: gutter-only.
        assert!(marks[1].cells.is_empty());
    }

    #[test]
    fn a_deletion_at_the_end_clamps_to_the_last_line() {
        let prev = frame(&["head", "gone"]);
        let next = frame(&["head"]);
        let marks = changed_marks(Some(&prev), &next);
        assert_eq!(changed(&marks), vec![true]);
        assert!(marks[0].cells.is_empty());
    }

    #[test]
    fn no_predecessor_means_no_marks() {
        let next = frame(&["a", "b"]);
        assert_eq!(
            changed_marks(None, &next),
            vec![LineMark::default(), LineMark::default()]
        );
    }

    #[test]
    fn identical_frames_mark_nothing() {
        let f = frame(&["a", "b"]);
        assert_eq!(changed(&changed_marks(Some(&f), &f)), vec![false, false]);
    }

    #[test]
    fn an_empty_next_frame_yields_no_marks() {
        let prev = frame(&["a"]);
        assert_eq!(changed_marks(Some(&prev), &[]), Vec::<LineMark>::new());
    }

    #[test]
    fn a_large_single_line_insert_maps_in_linear_time() {
        // The linear-walk pin (one cursor, no prefix rescans): a
        // quadratic mapper visibly hangs on this input; the linear one
        // finishes in milliseconds. The fixture is insert-shaped (a
        // common prefix and suffix frame the new line) so the diff
        // itself stays trivial and the mapping is what's measured.
        let big = "x".repeat(100_000);
        let prev = frame(&["header", "footer"]);
        let next = frame(&["header", &big, "footer"]);
        let marks = changed_marks(Some(&prev), &next);
        assert_eq!(changed(&marks), vec![false, true, false]);
        assert_eq!(marks[1].cells, vec![0..100_000]);
    }

    #[test]
    fn an_empty_previous_frame_marks_the_insertions() {
        // Empty prev is a real (distinct) frame, not "no predecessor".
        let next = frame(&["a", "b"]);
        let marks = changed_marks(Some(&[] as &[String]), &next);
        assert_eq!(changed(&marks), vec![true, true]);
        assert_eq!(marks[0].cells, vec![0..1]);
        assert_eq!(marks[1].cells, vec![0..1]);
    }
}
