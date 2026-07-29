//! Pane rendering: one source's output pinned into its declared box.
//!
//! `PaneBox::title` is the *declaration* and `PaneChrome::title` is the
//! *resolution*: only the loop knows the source's name, so the default
//! is applied there and `render_pane` uses `chrome.title` verbatim.

// The renderer lands before the engine that calls it; the dashboard
// subcommand is the first real caller and removes this.
#![allow(dead_code)]

use crate::color::ColorProfile;
use crate::core::box_model::{BoxSpec, render_box};
use crate::core::measure::{
    Align, Chunk, ELLIPSIS, chunks, display_width, pad_display, truncate_display,
};
use crate::core::registry::{LayoutNode, Overflow, PaneBox, PaneGeometry};
use crate::style_spec::StyleSpec;
use crate::term::marks::LineMark;
use crate::theme::Palette;

/// A rendered block and its marks in the block's own line coordinates.
#[derive(Clone, PartialEq, Debug, Default)]
pub struct PaneBlock {
    pub lines: Vec<String>,
    pub marks: Vec<LineMark>,
}

/// The chrome row's inputs, resolved by the loop.
pub struct PaneChrome<'a> {
    pub title: &'a str,
    /// "every 60s" | "every 60s or on trigger" | "on trigger" | "once".
    pub cadence: &'a str,
    /// "12:04:31" or "2m ago" — the pane's last-CHANGE time.
    pub stamp: &'a str,
    /// "exit 3" when the pane failed.
    pub failure: Option<&'a str>,
}

/// Pin one pane's output into its declared box: apply the overflow
/// rule, pad to the inner height, chop/pad to the inner width, append
/// the chrome row, draw the border. EXACTLY `geom.rows` lines of
/// `geom.cells` display cells. Marks arrive in output-line coordinates
/// and leave in box coordinates; marks on truncated lines are dropped.
///
/// The char shift is `border + padding.left` only because `align` is
/// `Align::Left`; a centered or right-aligned pane would need the shift
/// computed per line from the rendered row, and the horizontal mark
/// re-basing in `compose_panes` would move with it.
pub fn render_pane(
    output: &[String],
    marks: &[LineMark],
    pane: &PaneBox,
    geom: PaneGeometry,
    chrome: &PaneChrome<'_>,
    palette: &Palette,
    profile: ColorProfile,
) -> PaneBlock {
    let rows = geom.inner_rows as usize;
    let cols = geom.inner_cols as usize;

    // The overflow rule picks the surviving window and, with it, how
    // far the surviving marks moved.
    let dropped = match pane.overflow {
        Overflow::KeepTop => 0,
        Overflow::KeepBottom => output.len().saturating_sub(rows),
    };
    let mut content: Vec<String> = output.iter().skip(dropped).take(rows).cloned().collect();
    // Short output pads: the height is declared, never measured.
    content.resize(rows, String::new());
    if pane.chrome {
        content.push(chrome_row(chrome, cols, profile));
    }

    let spec = BoxSpec {
        // Some(_) is what squares the right edge — a bare spec would
        // leave ragged rows and the pin would be a lie.
        width: Some(cols),
        // Left is load-bearing: the char shift below assumes the
        // content starts at the left padding.
        align: Align::Left,
        padding: pane.padding,
        border: pane.border,
        border_style: StyleSpec {
            foreground: Some(palette.border),
            ..StyleSpec::default()
        },
        title: Some(chrome.title),
        ..BoxSpec::default()
    };
    let lines = render_box(&content, &spec, profile);

    let row_shift = pane.edge_cells() as usize + pane.padding.top;
    let char_shift = pane.edge_cells() as usize + pane.padding.left;
    let mut shifted = vec![LineMark::default(); lines.len()];
    for (i, mark) in marks.iter().enumerate().skip(dropped).take(rows) {
        if !mark.changed {
            continue;
        }
        let Some(slot) = shifted.get_mut(i - dropped + row_shift) else {
            continue;
        };
        slot.changed = true;
        slot.cells = mark
            .cells
            .iter()
            .map(|run| run.start + char_shift..run.end + char_shift)
            .collect();
    }
    PaneBlock {
        lines,
        marks: shifted,
    }
}

/// Compose the layout AND map every pane's marks into composed
/// coordinates in ONE walk, so the join rule can never drift between
/// the two: the horizontal offsets ARE the padding decisions, and a
/// separate mark mapper would have to re-derive them.
pub fn compose_panes(
    root: &LayoutNode,
    blocks: &[PaneBlock],
    gap: usize,
    row_gap: usize,
) -> PaneBlock {
    match root {
        LayoutNode::Pane(id) => blocks.get(id.0).cloned().unwrap_or_default(),
        LayoutNode::Column(children) => {
            let parts: Vec<PaneBlock> = children
                .iter()
                .map(|child| compose_panes(child, blocks, gap, row_gap))
                .collect();
            stack(&parts, row_gap)
        }
        LayoutNode::Row(children) => {
            let parts: Vec<PaneBlock> = children
                .iter()
                .map(|child| compose_panes(child, blocks, gap, row_gap))
                .collect();
            beside(&parts, gap)
        }
    }
}

/// Stack blocks with `row_gap` blank rows between. Mirrors
/// `join_vertical(.., Align::Left)`: lines are cloned verbatim, no
/// padding invented, so a mark's char indices are unchanged and only
/// its row moves.
fn stack(parts: &[PaneBlock], row_gap: usize) -> PaneBlock {
    let mut out = PaneBlock::default();
    for (i, part) in parts.iter().enumerate() {
        if i > 0 {
            for _ in 0..row_gap {
                out.lines.push(String::new());
                out.marks.push(LineMark::default());
            }
        }
        for (row, line) in part.lines.iter().enumerate() {
            out.lines.push(line.clone());
            // Pushed in lockstep, so lines and marks cannot fall out of
            // step even if a part arrives short.
            out.marks
                .push(part.marks.get(row).cloned().unwrap_or_default());
        }
    }
    out
}

/// Join blocks side by side and re-base their marks in the same pass.
///
/// The line rule is `join_horizontal`'s (`src/core/join.rs:93-107`),
/// restated here because the mark offsets are derived from the SAME
/// padding decisions: if `join_horizontal` changes, this changes with
/// it — which is exactly why lines and marks compose in one function,
/// and why the tests assert the composed lines equal `join_horizontal`
/// byte for byte (the restatement's standing witness).
fn beside(parts: &[PaneBlock], gap: usize) -> PaneBlock {
    let widths: Vec<usize> = parts
        .iter()
        .map(|part| {
            part.lines
                .iter()
                .map(|l| display_width(l))
                .max()
                .unwrap_or(0)
        })
        .collect();
    let height = parts.iter().map(|part| part.lines.len()).max().unwrap_or(0);
    let gap_text = " ".repeat(gap);
    let mut out = PaneBlock::default();
    for row in 0..height {
        let segments: Vec<&str> = parts
            .iter()
            .map(|part| part.lines.get(row).map(String::as_str).unwrap_or(""))
            .collect();
        // Nothing after the last non-empty segment is emitted — and a
        // row with no non-empty segment is one empty string.
        let Some(last) = segments.iter().rposition(|s| !s.is_empty()) else {
            out.lines.push(String::new());
            out.marks.push(LineMark::default());
            continue;
        };
        let mut line = String::new();
        let mut mark = LineMark::default();
        let mut chars = 0usize;
        for (i, segment) in segments[..=last].iter().enumerate() {
            if i > 0 {
                line.push_str(&gap_text);
                chars += gap;
            }
            // Everything before the last segment pads to its block's
            // width; the last one is emitted verbatim.
            let padded = if i < last {
                pad_display(segment, widths[i], Align::Left)
            } else {
                (*segment).to_string()
            };
            if let Some(source) = parts[i].marks.get(row) {
                mark.changed |= source.changed;
                // Left to right with a monotonically growing offset, so
                // the merged runs stay sorted and non-overlapping —
                // what `mark_cells` requires.
                mark.cells.extend(
                    source
                        .cells
                        .iter()
                        .map(|run| run.start + chars..run.end + chars),
                );
            }
            // CHARS, not display cells: LineMark.cells index the
            // stripped line, while the join pads by display cells.
            chars += chunks(&padded)
                .filter(|c| matches!(c, Chunk::Text(..)))
                .count();
            line.push_str(&padded);
        }
        out.lines.push(line);
        out.marks.push(mark);
    }
    out
}

/// `{cadence} · {stamp}`, plus the failure badge when the pane failed:
/// faint, right-aligned, never wider than the inner box. The stamp is
/// the pane's last-CHANGE time — a produced-at stamp would repaint
/// every tick and cost byte-silence. The row truncates from the right
/// like every other line, so a pane too narrow for the badge loses the
/// badge's tail rather than the cadence — behavior, not accident.
fn chrome_row(chrome: &PaneChrome<'_>, cols: usize, profile: ColorProfile) -> String {
    let mut text = format!("{} · {}", chrome.cadence, chrome.stamp);
    if let Some(failure) = chrome.failure {
        text.push_str(" · ");
        text.push_str(failure);
    }
    let faint = StyleSpec {
        faint: true,
        ..StyleSpec::default()
    };
    let text = truncate_display(&text, cols, ELLIPSIS);
    pad_display(&faint.render(&text, profile), cols, Align::Right)
}

#[cfg(test)]
// A one-range vec like `vec![0..1]` is exactly what a single marked run
// looks like — not a mistyped `(0..1).collect()`.
#[allow(clippy::single_range_in_vec_init)]
mod tests {
    use super::*;
    // `Sides` is imported here, not at module scope: nothing in the
    // non-test half names the type, and an unused import is a warning
    // `just lint` denies.
    use crate::core::box_model::{BorderPreset, Sides};
    use crate::core::join::{VAlign, join_horizontal, join_vertical};
    use crate::core::registry::{PaneWidth, SourceId};
    use crate::theme::{Appearance, AppearanceSource};

    fn palette() -> Palette {
        Palette::builtin(Appearance::Dark, AppearanceSource::Default)
    }

    fn pane(height: u16, border: BorderPreset, padding: Sides, chrome: bool) -> PaneBox {
        PaneBox {
            height,
            width: PaneWidth::Weight(1),
            overflow: Overflow::default(),
            border,
            padding,
            title: None,
            chrome,
        }
    }

    /// The geometry the registry would resolve for this pane at `cells`
    /// wide — computed the same way, so a test never hand-waves a size.
    fn geom(pane: &PaneBox, cells: u16) -> PaneGeometry {
        PaneGeometry {
            cells,
            rows: pane.height,
            inner_cols: cells - pane.frame_cols(),
            inner_rows: pane.height - pane.frame_rows(),
        }
    }

    fn chrome<'a>(failure: Option<&'a str>) -> PaneChrome<'a> {
        PaneChrome {
            title: "plan",
            cadence: "every 5s",
            stamp: "12:04:31",
            failure,
        }
    }

    fn lines(items: &[&str]) -> Vec<String> {
        items.iter().map(|s| s.to_string()).collect()
    }

    fn sides(top: usize, right: usize, bottom: usize, left: usize) -> Sides {
        Sides {
            top,
            right,
            bottom,
            left,
        }
    }

    /// A hand-built block: what `render_pane` would have produced, without
    /// paying for a box to test the join.
    fn block(lines_in: &[&str], marks_in: &[LineMark]) -> PaneBlock {
        PaneBlock {
            lines: lines(lines_in),
            marks: marks_in.to_vec(),
        }
    }

    fn marked(cells: std::ops::Range<usize>) -> LineMark {
        LineMark {
            changed: true,
            cells: vec![cells],
        }
    }

    fn row(n: usize) -> LayoutNode {
        LayoutNode::Row((0..n).map(|i| LayoutNode::Pane(SourceId(i))).collect())
    }

    fn column(n: usize) -> LayoutNode {
        LayoutNode::Column((0..n).map(|i| LayoutNode::Pane(SourceId(i))).collect())
    }

    #[test]
    fn a_column_of_rows_has_a_run_constant_height() {
        let top = block(
            &["aaa", "bbb", "ccc"],
            &[
                LineMark::default(),
                LineMark::default(),
                LineMark::default(),
            ],
        );
        let bottom = block(
            &["ddd", "eee", "fff", "ggg"],
            &[
                marked(0..3),
                LineMark::default(),
                LineMark::default(),
                LineMark::default(),
            ],
        );
        let composed = compose_panes(&column(2), &[top.clone(), bottom.clone()], 1, 1);
        assert_eq!(composed.lines.len(), 3 + 1 + 4);
        assert_eq!(composed.marks.len(), composed.lines.len());
        assert_eq!(
            composed.lines,
            join_vertical(&[top.lines.clone(), bottom.lines.clone()], 1, Align::Left)
        );
        // The gap row belongs to no pane and carries no mark.
        assert_eq!(composed.marks[3], LineMark::default());
        // The lower pane's mark moved down by the upper pane and the gap.
        assert_eq!(composed.marks[4], marked(0..3));

        // Run-constant: different content of the same declared size
        // composes to the same row count — the equal-length gate's premise.
        let top2 = block(&["zzz", "yyy", "xxx"], &vec![LineMark::default(); 3]);
        let again = compose_panes(&column(2), &[top2, bottom], 1, 1);
        assert_eq!(again.lines.len(), composed.lines.len());
    }

    #[test]
    fn a_row_takes_the_max_height_of_its_panes() {
        let short = block(&["a", "b", "c"], &vec![LineMark::default(); 3]);
        let tall = block(&["1", "2", "3", "4", "5"], &vec![LineMark::default(); 5]);
        let composed = compose_panes(&row(2), &[short.clone(), tall.clone()], 1, 0);
        assert_eq!(composed.lines.len(), 5);
        assert_eq!(composed.marks.len(), composed.lines.len());
        assert_eq!(
            composed.lines,
            join_horizontal(&[short.lines, tall.lines], 1, VAlign::Top)
        );
    }

    #[test]
    fn marks_rebase_across_a_horizontal_join() {
        let left = block(&["aaaa"], &[LineMark::default()]);
        let right = block(&["bb"], &[marked(0..2)]);
        let composed = compose_panes(&row(2), &[left.clone(), right.clone()], 1, 0);
        assert_eq!(composed.lines, vec!["aaaa bb".to_string()]);
        // Four chars of the left pane plus one gap.
        assert_eq!(composed.marks[0].cells, vec![5..7]);
        assert!(composed.marks[0].changed);
        assert_eq!(
            composed.lines,
            join_horizontal(&[left.lines, right.lines], 1, VAlign::Top)
        );
    }

    #[test]
    fn marks_rebase_by_chars_not_cells() {
        // The CJK pin: "日本語ab" is 5 CHARS and 8 DISPLAY CELLS;
        // LineMark.cells are char indices, so the right pane shifts by 5.
        // Row two pads "z" out to the block's 8 cells — 8 chars of spaces —
        // so the shift is per row, read off the PADDED segment.
        let left = block(
            &["日本語ab", "z"],
            &[LineMark::default(), LineMark::default()],
        );
        let right = block(&["xy", "xy"], &[marked(0..2), marked(0..2)]);
        assert_eq!(display_width("日本語ab"), 8);
        assert_eq!("日本語ab".chars().count(), 5);

        let composed = compose_panes(&row(2), &[left.clone(), right.clone()], 0, 0);
        assert_eq!(composed.lines, lines(&["日本語abxy", "z       xy"]));
        assert_eq!(composed.marks[0].cells, vec![5..7]);
        assert_eq!(composed.marks[1].cells, vec![8..10]);
        assert_eq!(
            composed.lines,
            join_horizontal(&[left.lines, right.lines], 0, VAlign::Top)
        );

        // The marks land on the characters they came from.
        for (line, mark) in composed.lines.iter().zip(&composed.marks) {
            let chars: Vec<char> = line.chars().collect();
            let run = &mark.cells[0];
            assert_eq!(&chars[run.start..run.end], &['x', 'y'], "in {line:?}");
        }
    }

    #[test]
    fn a_ragged_tail_row_marks_nothing() {
        // The left pane is taller; below the right pane's last row the
        // composed row is the left pane's own content, and past the left
        // pane's content it is whitespace or nothing at all. Neither can
        // mark: no pane owns a mark there, and the whitespace rule means
        // the padding a join invents never marks either.
        let tall = block(
            &["aaa", "bbb", "   ", ""],
            &[
                marked(0..3),
                LineMark::default(),
                LineMark::default(),
                LineMark::default(),
            ],
        );
        let short = block(&["zz"], &[marked(0..2)]);
        let composed = compose_panes(&row(2), &[tall.clone(), short.clone()], 1, 0);
        assert_eq!(composed.marks.len(), 4);
        assert_eq!(composed.marks[1], LineMark::default());
        assert_eq!(composed.marks[2], LineMark::default(), "whitespace tail");
        assert_eq!(composed.marks[3], LineMark::default(), "empty tail");
        // Both panes contribute to the first row: `changed` is an OR.
        assert!(composed.marks[0].changed);
        assert_eq!(composed.marks[0].cells, vec![0..3, 4..6]);
        assert_eq!(
            composed.lines,
            join_horizontal(&[tall.lines, short.lines], 1, VAlign::Top)
        );
    }

    #[test]
    fn an_empty_registry_composes_nothing() {
        assert_eq!(
            compose_panes(&LayoutNode::Column(Vec::new()), &[], 1, 1),
            PaneBlock::default()
        );
        assert_eq!(
            compose_panes(&LayoutNode::Row(Vec::new()), &[], 1, 1),
            PaneBlock::default()
        );
        // A layout the blocks do not cover composes empty rather than
        // panicking — validation is the registry's job, not a paint's.
        assert_eq!(
            compose_panes(&LayoutNode::Pane(SourceId(9)), &[], 1, 1),
            PaneBlock::default()
        );
    }

    #[test]
    fn a_pane_is_exactly_its_declared_size() {
        // Borderless: the block IS the inner box, so the whole frame is
        // assertable byte for byte.
        let bare = pane(4, BorderPreset::None, Sides::default(), true);
        let block = render_pane(
            &lines(&["ab"]),
            &[LineMark::default()],
            &bare,
            geom(&bare, 20),
            &PaneChrome {
                cadence: "once",
                ..chrome(None)
            },
            &palette(),
            ColorProfile::Ascii,
        );
        assert_eq!(
            block.lines,
            lines(&[
                "ab                  ",
                "                    ",
                "                    ",
                "     once · 12:04:31",
            ])
        );

        // Bordered and padded: 7 rows of 30 cells, every row, regardless
        // of how little the child printed.
        let boxed = pane(7, BorderPreset::Normal, sides(0, 1, 0, 1), true);
        let block = render_pane(
            &lines(&["line one", "line two"]),
            &[LineMark::default(), LineMark::default()],
            &boxed,
            geom(&boxed, 30),
            &chrome(None),
            &palette(),
            ColorProfile::Ascii,
        );
        assert_eq!(block.lines.len(), 7);
        for line in &block.lines {
            assert_eq!(display_width(line), 30, "ragged row: {line:?}");
        }
        assert_eq!(block.marks.len(), block.lines.len());
        assert!(
            block.lines[0].contains("plan"),
            "the title rides the border"
        );
    }

    #[test]
    fn overflow_keep_top_drops_the_tail() {
        let pane = pane(3, BorderPreset::None, Sides::default(), false);
        let block = render_pane(
            &lines(&["1", "2", "3", "4", "5"]),
            &vec![LineMark::default(); 5],
            &pane,
            geom(&pane, 5),
            &chrome(None),
            &palette(),
            ColorProfile::Ascii,
        );
        assert_eq!(block.lines, lines(&["1    ", "2    ", "3    "]));
    }

    #[test]
    fn overflow_keep_bottom_keeps_the_tail() {
        let pane = PaneBox {
            overflow: Overflow::KeepBottom,
            ..pane(3, BorderPreset::None, Sides::default(), false)
        };
        // The last output line changed; its mark must ride along to the
        // row the tail actually landed on.
        let mut marks = vec![LineMark::default(); 5];
        marks[4] = LineMark {
            changed: true,
            cells: vec![0..1],
        };
        let block = render_pane(
            &lines(&["1", "2", "3", "4", "5"]),
            &marks,
            &pane,
            geom(&pane, 5),
            &chrome(None),
            &palette(),
            ColorProfile::Ascii,
        );
        assert_eq!(block.lines, lines(&["3    ", "4    ", "5    "]));
        assert_eq!(block.marks[2].cells, vec![0..1]);
        assert!(!block.marks[0].changed && !block.marks[1].changed);
    }

    #[test]
    fn a_short_pane_pads_with_blank_rows() {
        let pane = pane(3, BorderPreset::None, Sides::default(), false);
        let block = render_pane(
            &lines(&["only"]),
            &[LineMark::default()],
            &pane,
            geom(&pane, 5),
            &chrome(None),
            &palette(),
            ColorProfile::Ascii,
        );
        assert_eq!(block.lines, lines(&["only ", "     ", "     "]));
        // Empty output is the same case, not a special one.
        let block = render_pane(
            &[],
            &[],
            &pane,
            geom(&pane, 5),
            &chrome(None),
            &palette(),
            ColorProfile::Ascii,
        );
        assert_eq!(block.lines, lines(&["     ", "     ", "     "]));
    }

    #[test]
    fn the_chrome_row_is_the_last_inner_row_and_right_aligned() {
        // Bottom padding proves the status row sits inside the content
        // block, not on the bottom border.
        let pane = pane(8, BorderPreset::Normal, sides(0, 1, 1, 1), true);
        let block = render_pane(
            &lines(&["x"]),
            &[LineMark::default()],
            &pane,
            geom(&pane, 30),
            &chrome(None),
            &palette(),
            ColorProfile::Ascii,
        );
        assert_eq!(block.lines.len(), 8);
        assert_eq!(block.lines[5], "│        every 5s · 12:04:31 │");
        assert_eq!(block.lines[6], format!("│{}│", " ".repeat(28)));
    }

    #[test]
    fn a_failure_badge_rides_the_chrome_row_without_changing_the_height() {
        let pane = pane(7, BorderPreset::Normal, sides(0, 1, 0, 1), true);
        let block = render_pane(
            &lines(&["boom"]),
            &[LineMark::default()],
            &pane,
            geom(&pane, 40),
            &chrome(Some("exit 3")),
            &palette(),
            ColorProfile::Ascii,
        );
        assert_eq!(block.lines.len(), 7, "a failure never changes the height");
        assert!(
            block.lines[5].contains("every 5s · 12:04:31 · exit 3"),
            "got {:?}",
            block.lines[5]
        );

        // Too narrow for the badge: the row still fits its box exactly.
        let block = render_pane(
            &lines(&["boom"]),
            &[LineMark::default()],
            &pane,
            geom(&pane, 24),
            &chrome(Some("exit 3")),
            &palette(),
            ColorProfile::Ascii,
        );
        assert_eq!(block.lines.len(), 7);
        for line in &block.lines {
            assert_eq!(display_width(line), 24, "ragged row: {line:?}");
        }
    }

    #[test]
    fn marks_shift_by_the_border_and_padding_rows() {
        let pane = pane(8, BorderPreset::Normal, sides(1, 2, 0, 2), true);
        let marks = vec![
            LineMark {
                changed: true,
                cells: vec![0..3],
            },
            LineMark::default(),
        ];
        let block = render_pane(
            &lines(&["abc", "d"]),
            &marks,
            &pane,
            geom(&pane, 30),
            &chrome(None),
            &palette(),
            ColorProfile::Ascii,
        );
        assert_eq!(block.marks.len(), block.lines.len());
        // Down by the top border (1) and the top padding (1).
        assert!(block.marks[2].changed);
        assert!(!block.marks[0].changed && !block.marks[1].changed);
        // Right by the border glyph (1 char) and the left padding (2).
        assert_eq!(block.marks[2].cells, vec![3..6]);
        // The shifted range lands on the same characters it started on.
        let stripped: Vec<char> = crate::core::measure::strip_escapes(&block.lines[2])
            .chars()
            .collect();
        assert_eq!(&stripped[3..6], &['a', 'b', 'c']);
    }
}
