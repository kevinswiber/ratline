//! Pane rendering: one source's output pinned into its declared box.
//!
//! `PaneBox::title` is the *declaration* and `PaneChrome::title` is the
//! *resolution*: only the loop knows the source's name, so the default
//! is applied there and `render_pane` uses `chrome.title` verbatim.

use crate::color::ColorProfile;
use crate::core::box_model::{BoxSpec, render_box};
use crate::core::measure::{
    Align, Chunk, ELLIPSIS, chunks, display_width, kept_chars, pad_display, truncate_display,
};
#[cfg(test)]
use crate::core::registry::Overflow;
use crate::core::registry::{LayoutNode, PaneBox, PaneGeometry, SourceId};
use crate::style_spec::StyleSpec;
use crate::term::marks::LineMark;
#[cfg(test)]
use crate::term::scroll::max_offset;
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
    /// Whether the suspicion test currently implicates this pane. A
    /// SEPARATE slot from `failure`, never a reuse of it: that one is
    /// outcome-derived and recomputed every tick, this one spans ticks,
    /// and a pane can legitimately be both failing and looping.
    pub looping: bool,
    /// "1.2k lines dropped" when this tick's output outran what the
    /// pane retains. A THIRD independent slot, for the same reason the
    /// second one exists: a pane can fail, loop and overflow at once,
    /// and each says something the others do not.
    pub truncated: Option<&'a str>,
    /// "lines 12-33 of 300" when the reader has taken this pane's window
    /// off its declared rest. A FOURTH independent slot: a pane can fail,
    /// loop, overflow and be scrolled at once. Appended last and so
    /// dropped first on a narrow pane — the recorded v1 order; the
    /// footer carries the range for a chrome-less pane (D5).
    pub scrolled: Option<&'a str>,
    /// Whether this pane holds the focus. The only visual channel a
    /// focused pane gets: the border recolors and costs no display
    /// cell, which is why a borderless pane needs the footer too.
    pub focused: bool,
    /// This pane is currently filling the frame, pre-formatted with
    /// its place in the reading order (`zoomed 2/4`) — the other panes
    /// are invisible, so Tab-cycling needs the cursor to orient by.
    /// LAST of the badges: the ones before it are conditions the pane
    /// is in, this one is what the reader did.
    pub zoomed: Option<&'a str>,
}

/// The viewport a pane's declared `Overflow` asks for: how many leading
/// lines the box drops. `KeepBottom`'s clip IS `max_offset` over the same
/// body and window — stated by CALLING it, so a per-pane offset and the
/// declared clip can never drift into two different numbers. The
/// test-side spelling of the rest state: production windows start from
/// `initial_pane_scroll` and stay clamped by `reanchor`, and the at-rest
/// equality test is what keeps the two derivations one number.
#[cfg(test)]
pub fn overflow_clip(overflow: Overflow, total: usize, rows: usize) -> usize {
    match overflow {
        Overflow::KeepTop => 0,
        Overflow::KeepBottom => max_offset(total, rows),
    }
}

/// A pane's window as a badge: `lines 12-33 of 300`, 1-based and worded
/// exactly like `scrolled_notice` so the pane row and the footer are one
/// family. `window` is the pane's `inner_rows`; how many lines it
/// actually holds is derived HERE so the badge and the footer range can
/// never be two different numbers.
pub fn scroll_badge(offset: usize, window: usize, total: usize) -> String {
    let shown = total.saturating_sub(offset).min(window);
    format!("lines {}-{} of {total}", offset + 1, offset + shown)
}

/// Pin one pane's output into its declared box: drop the viewport's
/// leading lines, pad to the inner height, chop/pad to the inner width,
/// append the chrome row, draw the border. EXACTLY `geom.rows` lines of
/// `geom.cells` display cells. Marks arrive in output-line coordinates
/// and leave in box coordinates; marks on truncated lines are dropped.
///
/// The char shift is `border + padding.left` only because `align` is
/// `Align::Left`; a centered or right-aligned pane would need the shift
/// computed per line from the rendered row, and the horizontal mark
/// re-basing in `compose_panes` would move with it.
///
/// `dropped` is the pane's viewport: how many leading output lines the
/// box drops. It is ALSO the mark re-base term and the index the
/// truncation clip reads the untruncated source at — one number, three
/// uses, which is why a moving viewport carries its highlights for free.
/// Callers derive it from `overflow_clip` (at rest) or a pane's
/// `LiveScroll` (scrolled); `render_pane` no longer knows `Overflow`.
#[allow(clippy::too_many_arguments)]
pub fn render_pane(
    output: &[String],
    marks: &[LineMark],
    pane: &PaneBox,
    geom: PaneGeometry,
    dropped: usize,
    chrome: &PaneChrome<'_>,
    palette: &Palette,
    profile: ColorProfile,
) -> PaneBlock {
    let rows = geom.inner_rows as usize;
    let cols = geom.inner_cols as usize;

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
            foreground: Some(if chrome.focused {
                palette.accent
            } else {
                palette.border
            }),
            bold: chrome.focused,
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
        // Marks are computed against the pane's own untruncated output;
        // the box cut them to `cols` cells. Clip each run to the chars
        // that survived — via the same rule the cut used, never
        // re-derived — or a run past the cut addresses the border, the
        // gap, and the neighbour once the panes are joined. The clip
        // limit includes the ellipsis, so a run reaching the cut marks
        // `…` ("continues past the edge" — a decision, pinned by test).
        // `changed` stays true even when every run clips away: the
        // line DID change, and the gutter must still say so.
        let limit = output
            .get(i)
            .map_or(0, |line| kept_chars(line, cols, ELLIPSIS));
        slot.cells = mark
            .cells
            .iter()
            .map(|run| run.start.min(limit)..run.end.min(limit))
            .filter(|run| !run.is_empty())
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

/// Where one pane's block lands in composed-frame coordinates, and how
/// big it is. Rows and columns, never cells-vs-chars: these coordinates
/// are what a directional move compares, and a mark's char indices are
/// a different space entirely.
#[derive(Copy, Clone, PartialEq, Eq, Debug, Default)]
pub struct PaneRect {
    pub row: usize,
    pub col: usize,
    pub rows: usize,
    pub cols: usize,
}

/// Walk the layout the way `compose_panes` joins it and record where
/// every pane's block lands, returning the walked node's own
/// (rows, cols). `sizes[id.0]` is the block's own size — the caller
/// decides what a collapsed pane measures, which is why this never
/// reads a `PaneGeometry`. `origin` is the composition's own top-left,
/// so a dashboard title row is one row of offset rather than a term
/// inside the walk.
pub fn pane_rects(
    root: &LayoutNode,
    sizes: &[(usize, usize)],
    gap: usize,
    row_gap: usize,
    origin: (usize, usize),
    out: &mut [PaneRect],
) -> (usize, usize) {
    match root {
        LayoutNode::Pane(id) => {
            let (rows, cols) = sizes.get(id.0).copied().unwrap_or((0, 0));
            if let Some(rect) = out.get_mut(id.0) {
                *rect = PaneRect {
                    row: origin.0,
                    col: origin.1,
                    rows,
                    cols,
                };
            }
            (rows, cols)
        }
        // `stack`: a part starts `row_gap` rows below the previous
        // part's end, and the block is as wide as its widest part.
        LayoutNode::Column(children) => {
            let (mut rows, mut cols) = (0usize, 0usize);
            for (i, child) in children.iter().enumerate() {
                if i > 0 {
                    rows += row_gap;
                }
                let (h, w) =
                    pane_rects(child, sizes, gap, row_gap, (origin.0 + rows, origin.1), out);
                rows += h;
                cols = cols.max(w);
            }
            (rows, cols)
        }
        // `beside`: a part starts `gap` cells past the previous part's
        // width, and the block is as tall as its tallest part.
        LayoutNode::Row(children) => {
            let (mut rows, mut cols) = (0usize, 0usize);
            for (i, child) in children.iter().enumerate() {
                if i > 0 {
                    cols += gap;
                }
                let (h, w) =
                    pane_rects(child, sizes, gap, row_gap, (origin.0, origin.1 + cols), out);
                cols += w;
                rows = rows.max(h);
            }
            (rows, cols)
        }
    }
}

/// The panes in reading order: rows left to right, columns top to
/// bottom — the order the declaration placed them.
pub fn pane_order(root: &LayoutNode) -> Vec<SourceId> {
    match root {
        LayoutNode::Pane(id) => vec![*id],
        LayoutNode::Row(children) | LayoutNode::Column(children) => {
            children.iter().flat_map(pane_order).collect()
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

/// `{cadence} · {stamp}`, plus each badge the pane currently carries:
/// faint, right-aligned, never wider than the inner box. The stamp is
/// the pane's last-CHANGE time — a produced-at stamp would repaint
/// every tick and cost byte-silence. The row truncates from the right
/// like every other line, so a pane too narrow for the badges loses
/// their tail rather than the cadence — behavior, not accident.
///
/// The word is `looping`: nine cells with its separator, the word a
/// user would type into a search box, and confident in the way
/// `CrashLoopBackOff` is confident over a mechanism that only counts.
/// The hedging belongs where there is room for it — a notice row and
/// the key reference — never in a nine-cell badge.
/// A collapsed pane: EXACTLY one line of `geom.cells` display cells —
/// the pane's name on the left, the chrome row's facts on the right.
/// No body is composed, because collapse hides output rather than
/// shrinking it: nothing here reads `output`, `marks`, or `overflow`,
/// and the pane's `PaneGeometry` arrives unchanged and leaves unused
/// past its width.
///
/// The name is unconditional. A bordered pane's title rides a border
/// this row has no room for, so this is the one place a collapsed pane
/// says what it is — and a row the user cannot name is a row they
/// cannot act on. The facts follow the pane's own `chrome`
/// declaration: `chrome = #false` asked never to see the cadence row,
/// and a collapsed row is not the place to overrule it.
pub fn render_pane_collapsed(
    pane: &PaneBox,
    geom: PaneGeometry,
    chrome: &PaneChrome<'_>,
    palette: &Palette,
    profile: ColorProfile,
) -> PaneBlock {
    let cols = geom.cells as usize;
    let name_style = if chrome.focused {
        StyleSpec {
            bold: true,
            foreground: Some(palette.accent),
            ..StyleSpec::default()
        }
    } else {
        StyleSpec::default()
    };
    let name = truncate_display(chrome.title, cols, ELLIPSIS);
    // The name takes the row first and keeps one separating cell;
    // whatever is left is the facts' budget, so a narrow row loses the
    // facts' tail — the rule chrome_row already truncates by.
    let budget = cols.saturating_sub(display_width(&name) + 1);
    let facts = if pane.chrome {
        truncate_display(&chrome_facts(chrome), budget, ELLIPSIS)
    } else {
        String::new()
    };
    let faint = StyleSpec {
        faint: true,
        ..StyleSpec::default()
    };
    // Measured before styling, padded after: escapes are zero cells, so
    // the pad lands on the same column either way — the idiom
    // `chrome_row` already uses.
    let left = pad_display(
        &name_style.render(&name, profile),
        cols - display_width(&facts),
        Align::Left,
    );
    PaneBlock {
        lines: vec![format!("{left}{}", faint.render(&facts, profile))],
        // One line, one mark. `stack`/`beside` push lines and marks in
        // lockstep and a block that arrives short slides every mark
        // below it.
        marks: vec![LineMark::default()],
    }
}

/// `{cadence} · {stamp}` plus the badges the pane currently carries —
/// the shared core of the chrome row and the collapsed row. The scroll
/// and `zoomed` badges stay at `chrome_row`'s own site: a collapsed row
/// must not advertise a viewport it is not showing.
fn chrome_facts(chrome: &PaneChrome<'_>) -> String {
    let mut text = format!("{} · {}", chrome.cadence, chrome.stamp);
    push_badge(&mut text, chrome.failure);
    push_badge(&mut text, chrome.looping.then_some("looping"));
    push_badge(&mut text, chrome.truncated);
    text
}

fn chrome_row(chrome: &PaneChrome<'_>, cols: usize, profile: ColorProfile) -> String {
    let mut text = chrome_facts(chrome);
    push_badge(&mut text, chrome.scrolled);
    push_badge(&mut text, chrome.zoomed);
    let faint = StyleSpec {
        faint: true,
        ..StyleSpec::default()
    };
    let text = truncate_display(&text, cols, ELLIPSIS);
    pad_display(&faint.render(&text, profile), cols, Align::Right)
}

/// One badge in the chrome row's separator form. Two of them ride this
/// row independently, so the join is written once rather than nested.
fn push_badge(text: &mut String, badge: Option<&str>) {
    if let Some(badge) = badge {
        text.push_str(" · ");
        text.push_str(badge);
    }
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
    use crate::core::registry::PaneWidth;
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
            looping: false,
            truncated: None,
            scrolled: None,
            focused: false,
            zoomed: None,
        }
    }

    fn lines(items: &[&str]) -> Vec<String> {
        items.iter().map(|s| s.to_string()).collect()
    }

    /// `render_pane` at its declared viewport — what the shipped
    /// `Overflow` match produced before the clip became a parameter.
    /// Every test that is not ABOUT the offset renders at rest, and the
    /// derivation is written down once, here.
    #[allow(clippy::too_many_arguments)]
    fn render_at_rest(
        output: &[String],
        marks: &[LineMark],
        pane: &PaneBox,
        geom: PaneGeometry,
        chrome: &PaneChrome<'_>,
        palette: &Palette,
        profile: ColorProfile,
    ) -> PaneBlock {
        let dropped = overflow_clip(pane.overflow, output.len(), geom.inner_rows as usize);
        render_pane(output, marks, pane, geom, dropped, chrome, palette, profile)
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
        let block = render_at_rest(
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
        let block = render_at_rest(
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
        let block = render_at_rest(
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
        let block = render_at_rest(
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
        let block = render_at_rest(
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
        let block = render_at_rest(
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
        let block = render_at_rest(
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
        let block = render_at_rest(
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
        let block = render_at_rest(
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
        let block = render_at_rest(
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

    #[test]
    fn marks_past_the_truncation_cut_never_leave_the_pane() {
        // Normal border, no padding, inner 20 cells: a 30-char line
        // keeps 19 chars + the ellipsis. A run wholly past the cut
        // vanishes (the gutter bit still says the line changed); a
        // straddling run clips at the cut and marks the ellipsis —
        // "continues past the edge", a decision, not an accident.
        let pane = pane(4, BorderPreset::Normal, sides(0, 0, 0, 0), false);
        let source = "abcdefghijklmnopqrstuvwxyz0123";
        let hidden = vec![marked(20..25), LineMark::default()];
        let block = render_at_rest(
            &lines(&[source, "x"]),
            &hidden,
            &pane,
            geom(&pane, 22),
            &chrome(None),
            &palette(),
            ColorProfile::Ascii,
        );
        assert!(
            block.marks[1].changed,
            "the line DID change — the gutter must still say so"
        );
        assert!(
            block.marks[1].cells.is_empty(),
            "a change hidden in the tail paints no highlight: {:?}",
            block.marks[1].cells
        );

        let straddling = vec![marked(15..25), LineMark::default()];
        let block = render_at_rest(
            &lines(&[source, "x"]),
            &straddling,
            &pane,
            geom(&pane, 22),
            &chrome(None),
            &palette(),
            ColorProfile::Ascii,
        );
        assert_eq!(block.marks[1].cells, vec![16..21]);
        let stripped: Vec<char> = crate::core::measure::strip_escapes(&block.lines[1])
            .chars()
            .collect();
        assert_eq!(
            stripped[20], '…',
            "a run reaching the cut marks the ellipsis"
        );
    }

    #[test]
    fn a_wide_rune_cut_clips_marks_in_chars_not_cells() {
        // "日本語abcd" is 10 cells over 7 chars; an inner width of 6
        // cells keeps 日本 (4 cells, the straddling 語 drops whole)
        // plus the ellipsis: 3 CHARS survive. The clip must count
        // chars, not cells, or wide runes shift every boundary.
        let pane = pane(3, BorderPreset::Normal, sides(0, 0, 0, 0), false);
        let source = "日本語abcd";
        let marks = vec![marked(1..5)];
        let block = render_at_rest(
            &lines(&[source]),
            &marks,
            &pane,
            geom(&pane, 8),
            &chrome(None),
            &palette(),
            ColorProfile::Ascii,
        );
        // char_shift 1 (border); clip at 3 kept chars → 2..4, inside
        // the box, ending on the ellipsis.
        assert_eq!(block.marks[1].cells, vec![2..4]);
    }

    #[test]
    fn a_truncated_panes_highlight_never_reaches_its_neighbour() {
        // The composed regression: a change past the left pane's cut
        // must not mark the gap or the right pane — composed marks
        // never index at or past the left block's own chars.
        let pane_box = pane(3, BorderPreset::Normal, sides(0, 0, 0, 0), false);
        let long = "abcdefghijklmnopqrstuvwxyz0123";
        let left = render_at_rest(
            &lines(&[long]),
            &[marked(20..25)],
            &pane_box,
            geom(&pane_box, 22),
            &chrome(None),
            &palette(),
            ColorProfile::Ascii,
        );
        let right = render_at_rest(
            &lines(&["quiet"]),
            &[LineMark::default()],
            &pane_box,
            geom(&pane_box, 22),
            &chrome(None),
            &palette(),
            ColorProfile::Ascii,
        );
        let left_chars = crate::core::measure::strip_escapes(&left.lines[1])
            .chars()
            .count();
        let composed = compose_panes(&row(2), &[left, right], 1, 0);
        for (i, mark) in composed.marks.iter().enumerate() {
            for run in &mark.cells {
                assert!(
                    run.end <= left_chars,
                    "row {i}: run {run:?} leaks past the left pane ({left_chars} chars)"
                );
            }
        }
    }

    #[test]
    fn a_looping_pane_shows_the_badge() {
        let row = chrome_row(
            &PaneChrome {
                looping: true,
                ..chrome(None)
            },
            40,
            ColorProfile::Ascii,
        );
        assert!(row.contains("· looping"), "got {row:?}");
    }

    #[test]
    fn a_pane_can_be_both_failing_and_looping() {
        // Two independent slots: one is outcome-derived and recomputed
        // every tick, the other spans ticks. Neither replaces the other.
        let row = chrome_row(
            &PaneChrome {
                looping: true,
                ..chrome(Some("exit 3"))
            },
            60,
            ColorProfile::Ascii,
        );
        assert!(
            row.contains("every 5s · 12:04:31 · exit 3 · looping"),
            "got {row:?}"
        );
    }

    #[test]
    fn a_truncated_pane_shows_the_marker_after_the_other_badges() {
        // A third independent slot: a pane can fail, loop and truncate
        // at once, and the newest badge appends rather than displacing.
        let row = chrome_row(
            &PaneChrome {
                looping: true,
                truncated: Some("1.2k lines dropped"),
                ..chrome(Some("exit 3"))
            },
            80,
            ColorProfile::Ascii,
        );
        assert!(
            row.contains("every 5s · 12:04:31 · exit 3 · looping · 1.2k lines dropped"),
            "got {row:?}"
        );
    }

    #[test]
    fn a_narrow_pane_loses_the_marker_before_it_loses_the_cadence() {
        // The same append-and-truncate-tail rule the other badges obey.
        // `on trigger · 12:04:31 · 90 lines dropped` is 40 cells, which
        // makes 40 the exact width at which the marker survives whole.
        let truncated = PaneChrome {
            cadence: "on trigger",
            truncated: Some("90 lines dropped"),
            ..chrome(None)
        };
        assert_eq!(
            chrome_row(&truncated, 40, ColorProfile::Ascii),
            "on trigger · 12:04:31 · 90 lines dropped"
        );
        let narrow = chrome_row(&truncated, 39, ColorProfile::Ascii);
        assert!(
            narrow.starts_with("on trigger · 12:04:31"),
            "got {narrow:?}"
        );
        assert!(!narrow.contains("dropped"), "got {narrow:?}");
    }

    #[test]
    fn a_narrow_pane_loses_the_badge_rather_than_the_cadence() {
        // Shipped behaviour asserted rather than assumed: the row
        // truncates from the RIGHT, so the badge's tail goes before the
        // cadence does. `on trigger · 12:04:31 · looping` is 31 cells,
        // which makes 31 the exact width at which the badge survives.
        let looping = PaneChrome {
            cadence: "on trigger",
            looping: true,
            ..chrome(None)
        };
        assert_eq!(
            chrome_row(&looping, 31, ColorProfile::Ascii),
            "on trigger · 12:04:31 · looping"
        );
        let narrow = chrome_row(&looping, 30, ColorProfile::Ascii);
        assert!(
            narrow.starts_with("on trigger · 12:04:31"),
            "the cadence and the stamp survive: {narrow:?}"
        );
        assert!(!narrow.contains("looping"), "got {narrow:?}");
        assert_eq!(display_width(&narrow), 30, "and the row still fits");
    }

    #[test]
    fn a_pane_with_no_chrome_row_renders_no_badge_and_does_not_panic() {
        // `chrome = #false` is legal and shipped, and `· exit 3` is
        // ALREADY invisible on such a pane. The badge inherits that
        // hole rather than opening a new one — which is exactly why a
        // notice row that names the panes has to exist too.
        let bare = pane(4, BorderPreset::None, Sides::default(), false);
        let block = render_at_rest(
            &lines(&["ab"]),
            &[LineMark::default()],
            &bare,
            geom(&bare, 20),
            &PaneChrome {
                looping: true,
                ..chrome(Some("exit 3"))
            },
            &palette(),
            ColorProfile::Ascii,
        );
        assert_eq!(block.lines.len(), 4, "a chrome-less pane is all content");
        assert!(!block.lines.iter().any(|l| l.contains("looping")));
        assert!(!block.lines.iter().any(|l| l.contains("exit 3")));
    }

    #[test]
    fn the_rect_walk_mirrors_the_stack_and_beside_offsets() {
        // Depth two: a column of two rows, the second row taller than
        // the first and narrower.
        let root = LayoutNode::Column(vec![
            LayoutNode::Row(vec![
                LayoutNode::Pane(SourceId(0)),
                LayoutNode::Pane(SourceId(1)),
            ]),
            LayoutNode::Row(vec![
                LayoutNode::Pane(SourceId(2)),
                LayoutNode::Pane(SourceId(3)),
            ]),
        ]);
        let sizes = [(4, 10), (4, 20), (6, 12), (6, 8)];
        let mut rects = vec![PaneRect::default(); 4];
        let (rows, cols) = pane_rects(&root, &sizes, 2, 1, (0, 0), &mut rects);
        assert_eq!(
            rects[0],
            PaneRect {
                row: 0,
                col: 0,
                rows: 4,
                cols: 10
            }
        );
        // Past the first pane's cells plus the gap.
        assert_eq!(
            rects[1],
            PaneRect {
                row: 0,
                col: 12,
                rows: 4,
                cols: 20
            }
        );
        // One blank row below the first row's height.
        assert_eq!(
            rects[2],
            PaneRect {
                row: 5,
                col: 0,
                rows: 6,
                cols: 12
            }
        );
        assert_eq!(
            rects[3],
            PaneRect {
                row: 5,
                col: 14,
                rows: 6,
                cols: 8
            }
        );
        // A column is as wide as its widest row and as tall as the sum
        // of its rows plus the gaps between them.
        assert_eq!((rows, cols), (11, 32));
    }

    #[test]
    fn a_collapsed_size_shortens_the_column_it_sits_in() {
        // The caller decides what a collapsed pane measures; the walk
        // only places what it is handed, which is what keeps this
        // function out of PaneGeometry.
        let root = LayoutNode::Column(vec![
            LayoutNode::Pane(SourceId(0)),
            LayoutNode::Pane(SourceId(1)),
        ]);
        let mut rects = vec![PaneRect::default(); 2];
        assert_eq!(
            pane_rects(&root, &[(5, 30), (5, 30)], 0, 0, (0, 0), &mut rects),
            (10, 30)
        );
        assert_eq!(rects[1].row, 5);
        assert_eq!(
            pane_rects(&root, &[(1, 30), (5, 30)], 0, 0, (0, 0), &mut rects),
            (6, 30)
        );
        assert_eq!(rects[1].row, 1);
    }

    #[test]
    fn the_walk_reports_the_dimensions_compose_panes_produces() {
        // The walk restates stack/beside's offset rules, so the
        // composition itself is the standing witness for the restatement.
        let tall = pane(6, BorderPreset::None, Sides::default(), false);
        let short = pane(4, BorderPreset::Normal, Sides::default(), false);
        let render = |p: &PaneBox, cells: u16| {
            render_at_rest(
                &lines(&["x"]),
                &[LineMark::default()],
                p,
                geom(p, cells),
                &chrome(None),
                &palette(),
                ColorProfile::Ascii,
            )
        };
        let blocks = vec![render(&tall, 20), render(&short, 12)];
        let sizes: Vec<(usize, usize)> = blocks
            .iter()
            .map(|b| {
                (
                    b.lines.len(),
                    b.lines.iter().map(|l| display_width(l)).max().unwrap_or(0),
                )
            })
            .collect();
        let root = LayoutNode::Row(vec![
            LayoutNode::Pane(SourceId(0)),
            LayoutNode::Pane(SourceId(1)),
        ]);
        let mut rects = vec![PaneRect::default(); 2];
        let (rows, cols) = pane_rects(&root, &sizes, 1, 0, (0, 0), &mut rects);
        let composed = compose_panes(&root, &blocks, 1, 0);
        assert_eq!(rows, composed.lines.len());
        assert_eq!(
            cols,
            composed
                .lines
                .iter()
                .map(|l| display_width(l))
                .max()
                .unwrap_or(0)
        );
        assert_eq!(rects[1].col, 21, "past 20 cells and the gap");

        // The origin is what a dashboard title row costs: one line is
        // inserted above the composition, so every rect moves down one.
        let mut shifted = vec![PaneRect::default(); 2];
        pane_rects(&root, &sizes, 1, 0, (1, 0), &mut shifted);
        assert_eq!(shifted[0].row, 1);
        assert_eq!(shifted[1].row, 1);
    }

    #[test]
    fn pane_order_flattens_in_declaration_order() {
        let root = LayoutNode::Column(vec![
            LayoutNode::Row(vec![
                LayoutNode::Pane(SourceId(2)),
                LayoutNode::Pane(SourceId(0)),
            ]),
            LayoutNode::Pane(SourceId(1)),
        ]);
        // The tree's order, never the id's: rows left to right, columns
        // top to bottom.
        assert_eq!(
            pane_order(&root),
            vec![SourceId(2), SourceId(0), SourceId(1)]
        );
        assert!(pane_order(&LayoutNode::Row(Vec::new())).is_empty());
    }

    #[test]
    fn a_focused_pane_wears_the_accent_border_and_no_extra_cell() {
        let boxed = pane(4, BorderPreset::Normal, Sides::default(), false);
        let draw = |focused: bool| {
            render_at_rest(
                &lines(&["x"]),
                &[LineMark::default()],
                &boxed,
                geom(&boxed, 20),
                &PaneChrome {
                    focused,
                    ..chrome(None)
                },
                &palette(),
                ColorProfile::Ansi256,
            )
        };
        let plain = draw(false);
        let focused = draw(true);
        // The dark accent, bolded: the border's own style, nothing else.
        assert!(
            focused.lines[0].contains("\x1b[1;38;5;212m"),
            "got {:?}",
            focused.lines[0]
        );
        assert!(!plain.lines[0].contains("\x1b[1;38;5;212m"));
        // Indication costs no display cell and no row: the pane is
        // exactly its declared size either way.
        assert_eq!(focused.lines.len(), plain.lines.len());
        for (a, b) in focused.lines.iter().zip(&plain.lines) {
            assert_eq!(display_width(a), display_width(b), "{a:?} vs {b:?}");
        }
    }

    #[test]
    fn an_unfocused_pane_renders_exactly_what_it_always_did() {
        // Phase 1's byte-inertness pin at pane scope: with the flag
        // off, not one byte of the box moves.
        let boxed = pane(4, BorderPreset::Normal, Sides::default(), true);
        let block = render_at_rest(
            &lines(&["x"]),
            &[LineMark::default()],
            &boxed,
            geom(&boxed, 20),
            &chrome(None),
            &palette(),
            ColorProfile::Ansi256,
        );
        assert!(block.lines[0].contains("\x1b[38;5;240m"));
        assert!(!block.lines[0].contains("\x1b[1;"));
    }

    #[test]
    fn an_out_of_range_pane_places_nothing_and_panics_at_nothing() {
        // Validation is the registry's job: a layout the sizes do not
        // cover walks to zero, the way compose_panes composes empty.
        let mut rects = vec![PaneRect::default(); 1];
        assert_eq!(
            pane_rects(
                &LayoutNode::Pane(SourceId(9)),
                &[(4, 10)],
                1,
                1,
                (0, 0),
                &mut rects
            ),
            (0, 0)
        );
        assert_eq!(rects[0], PaneRect::default());
    }

    #[test]
    fn the_overflow_clip_is_the_declared_viewport() {
        // KeepTop's clip is the head; KeepBottom's IS max_offset — the
        // identity per-pane scrolling rests on, so it is structural
        // (`overflow_clip` calls `max_offset`) and asserted, not restated.
        assert_eq!(overflow_clip(Overflow::KeepTop, 100, 3), 0);
        assert_eq!(overflow_clip(Overflow::KeepBottom, 100, 3), 97);
        assert_eq!(
            overflow_clip(Overflow::KeepBottom, 100, 3),
            crate::term::scroll::max_offset(100, 3)
        );
        // A body shorter than the window never clips, either way.
        assert_eq!(overflow_clip(Overflow::KeepBottom, 2, 3), 0);
        assert_eq!(overflow_clip(Overflow::KeepTop, 0, 3), 0);
    }

    #[test]
    fn an_offset_mid_body_renders_that_window_with_its_marks() {
        // The parameter is the whole feature: a KeepTop pane rendered at
        // offset 2 shows lines 3-5, and the mark on output line 4 lands on
        // the row that line actually occupies — the re-base term IS the
        // offset, so nothing else had to move.
        let pane = pane(3, BorderPreset::None, Sides::default(), false);
        let body = lines(&["L1", "L2", "L3", "L4", "L5", "L6"]);
        let mut marks = vec![LineMark::default(); 6];
        marks[3] = marked(0..2);
        let block = render_pane(
            &body,
            &marks,
            &pane,
            geom(&pane, 5),
            2,
            &chrome(None),
            &palette(),
            ColorProfile::Ascii,
        );
        assert_eq!(block.lines, lines(&["L3   ", "L4   ", "L5   "]));
        assert!(block.marks[1].changed, "L4's mark rode the offset");
        assert_eq!(block.marks[1].cells, vec![0..2]);
        assert!(!block.marks[0].changed && !block.marks[2].changed);

        // An offset with fewer than `rows` lines left still pads: the
        // height is declared, never measured.
        let block = render_pane(
            &body,
            &marks,
            &pane,
            geom(&pane, 5),
            5,
            &chrome(None),
            &palette(),
            ColorProfile::Ascii,
        );
        assert_eq!(block.lines, lines(&["L6   ", "     ", "     "]));
    }

    #[test]
    fn the_scroll_badge_is_the_scrolled_rows_phrase() {
        // The footer already says `lines 30-58 of 59` while the whole
        // frame is scrolled. A pane saying the same thing differently
        // would read as a different fact, so the badge is asserted
        // against the shipped row rather than against a literal alone.
        assert_eq!(scroll_badge(11, 22, 300), "lines 12-33 of 300");
        assert!(
            crate::term::scroll::scrolled_notice("since 12:07:45", "", 11, 22, 300)
                .contains(&scroll_badge(11, 22, 300)),
            "the pane badge and the footer row must be the same phrase"
        );
        // A window past the body's tail reports what it actually holds.
        assert_eq!(scroll_badge(8, 22, 10), "lines 9-10 of 10");
    }

    #[test]
    fn a_scrolled_pane_shows_its_position_after_the_other_badges() {
        // A fourth independent slot: a pane can fail, loop, truncate and
        // be scrolled at once. 76 cells of text, so 80 shows it whole.
        let row = chrome_row(
            &PaneChrome {
                looping: true,
                truncated: Some("1.2k lines dropped"),
                scrolled: Some("lines 2-5 of 6"),
                ..chrome(Some("exit 3"))
            },
            80,
            ColorProfile::Ascii,
        );
        assert!(
            row.contains(
                "every 5s · 12:04:31 · exit 3 · looping · 1.2k lines dropped · lines 2-5 of 6"
            ),
            "got {row:?}"
        );
    }

    #[test]
    fn a_pane_at_rest_shows_no_position_at_all() {
        // The byte-inertness pin for the chrome row: `scrolled` is None
        // for every pane nobody has scrolled, so an untouched dashboard
        // composes exactly the bytes it composes on main.
        let row = chrome_row(&chrome(None), 40, ColorProfile::Ascii);
        assert!(row.ends_with("every 5s · 12:04:31"), "got {row:?}");
        assert!(!row.contains("lines"), "got {row:?}");
        assert_eq!(display_width(&row), 40);
    }

    #[test]
    fn a_narrow_pane_loses_the_position_before_the_dropped_marker() {
        // The recorded v1 order: appended last, dropped first — the same
        // truncate-from-the-right rule every other badge obeys.
        // `on trigger · 12:04:31 · lines 2-5 of 6` is 38 cells.
        let scrolled = PaneChrome {
            cadence: "on trigger",
            scrolled: Some("lines 2-5 of 6"),
            ..chrome(None)
        };
        assert_eq!(
            chrome_row(&scrolled, 38, ColorProfile::Ascii),
            "on trigger · 12:04:31 · lines 2-5 of 6"
        );
        let narrow = chrome_row(&scrolled, 37, ColorProfile::Ascii);
        assert!(
            narrow.starts_with("on trigger · 12:04:31"),
            "the cadence and the stamp survive: {narrow:?}"
        );
        assert_eq!(display_width(&narrow), 37, "and the row still fits");
    }

    #[test]
    fn a_zoomed_pane_says_so_last_in_its_chrome_row() {
        let pane = pane(5, BorderPreset::Rounded, sides(0, 0, 0, 0), true);
        let mut chrome = chrome(Some("exit 3"));
        chrome.looping = true;
        chrome.truncated = Some("2 lines dropped");
        chrome.zoomed = Some("zoomed 2/4");
        // Wide enough that no badge is truncated from the right.
        let block = render_pane(
            &lines(&["body"]),
            &[],
            &pane,
            geom(&pane, 90),
            0,
            &chrome,
            &palette(),
            ColorProfile::Ascii,
        );
        let row = block
            .lines
            .iter()
            .find(|l| l.contains("zoomed"))
            .expect("the chrome row carries the badge");
        let at = row.find("zoomed").expect("badge");
        assert!(
            row.contains("zoomed 2/4"),
            "the badge carries the cycle cursor: {row:?}"
        );
        for earlier in ["every 5s", "exit 3", "looping", "2 lines dropped"] {
            assert!(
                row.find(earlier).expect(earlier) < at,
                "`zoomed` is the last badge: {row:?}"
            );
        }
        chrome.zoomed = None;
        let plain = render_pane(
            &lines(&["body"]),
            &[],
            &pane,
            geom(&pane, 90),
            0,
            &chrome,
            &palette(),
            ColorProfile::Ascii,
        );
        assert!(
            !plain.lines.iter().any(|l| l.contains("zoomed")),
            "no badge when the pane is not zoomed"
        );
    }

    #[test]
    fn a_collapsed_pane_is_exactly_one_row_of_its_declared_width() {
        // Seven declared rows, one rendered: the box is not drawn at all —
        // a 1-row bordered box is inexpressible, so the row is composed
        // directly.
        let boxed = pane(7, BorderPreset::Normal, sides(0, 1, 0, 1), true);
        let block = render_pane_collapsed(
            &boxed,
            geom(&boxed, 30),
            &chrome(None),
            &palette(),
            ColorProfile::Ascii,
        );
        assert_eq!(block.lines.len(), 1);
        assert_eq!(display_width(&block.lines[0]), 30, "{:?}", block.lines[0]);
        // Lines and marks in lockstep: a block that arrives short slides
        // every mark below it in the composed frame.
        assert_eq!(block.marks, vec![LineMark::default()]);
        // The name rides the left edge, the facts the right.
        assert!(
            block.lines[0].starts_with("plan"),
            "got {:?}",
            block.lines[0]
        );
        assert!(
            block.lines[0].ends_with("every 5s · 12:04:31"),
            "got {:?}",
            block.lines[0]
        );
    }

    #[test]
    fn a_collapsed_row_carries_every_badge_the_chrome_row_would_have() {
        let bare = pane(4, BorderPreset::None, Sides::default(), true);
        let block = render_pane_collapsed(
            &bare,
            geom(&bare, 70),
            &PaneChrome {
                looping: true,
                truncated: Some("90 lines dropped"),
                ..chrome(Some("exit 3"))
            },
            &palette(),
            ColorProfile::Ascii,
        );
        assert!(
            block.lines[0].contains("every 5s · 12:04:31 · exit 3 · looping · 90 lines dropped"),
            "got {:?}",
            block.lines[0]
        );
    }

    #[test]
    fn a_narrow_collapsed_row_loses_the_facts_before_the_name() {
        // The same append-and-truncate-tail rule chrome_row obeys, applied
        // to a row that also has to hold a name: the name is what the next
        // keystroke acts on, so it is the last thing to go.
        let bare = pane(4, BorderPreset::None, Sides::default(), true);
        for cells in [30u16, 12, 6, 4, 2, 1] {
            let block = render_pane_collapsed(
                &bare,
                geom(&bare, cells),
                &chrome(None),
                &palette(),
                ColorProfile::Ascii,
            );
            assert_eq!(block.lines.len(), 1, "at {cells} cells");
            assert_eq!(
                display_width(&block.lines[0]),
                cells as usize,
                "ragged row at {cells} cells: {:?}",
                block.lines[0]
            );
        }
        let narrow = render_pane_collapsed(
            &bare,
            geom(&bare, 12),
            &chrome(None),
            &palette(),
            ColorProfile::Ascii,
        );
        assert_eq!(narrow.lines[0], "plan every …");
        let tiny = render_pane_collapsed(
            &bare,
            geom(&bare, 4),
            &chrome(None),
            &palette(),
            ColorProfile::Ascii,
        );
        assert_eq!(tiny.lines[0], "plan", "no room for facts, still a name");
    }

    #[test]
    fn a_chrome_less_pane_collapses_to_its_name_alone() {
        // `chrome = #false` asked never to see the cadence row; collapse
        // must not smuggle it in. The badges inherit the hole they already
        // have on an expanded chrome-less pane
        // (`a_pane_with_no_chrome_row_renders_no_badge_and_does_not_panic`),
        // which is why D5 puts a chrome-less pane's position on the FOOTER.
        let bare = pane(4, BorderPreset::None, Sides::default(), false);
        let block = render_pane_collapsed(
            &bare,
            geom(&bare, 20),
            &PaneChrome {
                looping: true,
                ..chrome(Some("exit 3"))
            },
            &palette(),
            ColorProfile::Ascii,
        );
        assert_eq!(block.lines[0], format!("plan{}", " ".repeat(16)));
        assert!(
            !block.lines[0].contains("every"),
            "no cadence on a chrome-less pane"
        );
    }

    #[test]
    fn a_collapsed_pane_shortens_the_column_by_the_rows_it_hides() {
        // Column collapse shortens the composed frame; nothing is
        // redistributed (INV-8).
        let bare = pane(5, BorderPreset::None, Sides::default(), false);
        let body = lines(&["r1", "r2", "r3", "r4", "r5"]);
        let expanded = render_at_rest(
            &body,
            &vec![LineMark::default(); 5],
            &bare,
            geom(&bare, 10),
            &chrome(None),
            &palette(),
            ColorProfile::Ascii,
        );
        let collapsed = render_pane_collapsed(
            &bare,
            geom(&bare, 10),
            &chrome(None),
            &palette(),
            ColorProfile::Ascii,
        );
        let both = compose_panes(&column(2), &[expanded.clone(), expanded.clone()], 1, 1);
        let one = compose_panes(&column(2), &[collapsed, expanded], 1, 1);
        assert_eq!(
            both.lines.len() - one.lines.len(),
            4,
            "5 declared rows, 1 rendered"
        );
        assert_eq!(one.marks.len(), one.lines.len());
    }

    #[test]
    fn a_collapsed_pane_in_a_row_frees_nothing_and_leaves_blanks() {
        // Row collapse frees nothing: `beside` takes the row's max height
        // and pads the short part's missing rows to blanks (grounding §8).
        let bare = pane(5, BorderPreset::None, Sides::default(), false);
        let expanded = render_at_rest(
            &lines(&["r1", "r2", "r3", "r4", "r5"]),
            &vec![LineMark::default(); 5],
            &bare,
            geom(&bare, 10),
            &chrome(None),
            &palette(),
            ColorProfile::Ascii,
        );
        let collapsed = render_pane_collapsed(
            &bare,
            geom(&bare, 10),
            &chrome(None),
            &palette(),
            ColorProfile::Ascii,
        );
        let composed = compose_panes(&row(2), &[collapsed, expanded], 1, 0);
        assert_eq!(
            composed.lines.len(),
            5,
            "the row keeps its tallest pane's height"
        );
        assert!(
            composed.lines[1].starts_with(&" ".repeat(11)),
            "the collapsed pane's missing rows are blanks: {:?}",
            composed.lines[1]
        );
        assert!(composed.lines[1].contains("r2"), "the sibling is untouched");
    }

    #[test]
    fn a_focused_collapsed_row_wears_the_accent_without_spending_a_cell() {
        // The focused restyle reaches the collapsed row too — the border it
        // would otherwise have restyled is not drawn here (the BoxSpec
        // restyle lives in `render_pane`; this row has no box).
        let bare = pane(4, BorderPreset::None, Sides::default(), true);
        let accent = StyleSpec {
            bold: true,
            foreground: Some(palette().accent),
            ..StyleSpec::default()
        };
        let expected = accent.render("plan", ColorProfile::TrueColor);
        let focused = render_pane_collapsed(
            &bare,
            geom(&bare, 30),
            &PaneChrome {
                focused: true,
                ..chrome(None)
            },
            &palette(),
            ColorProfile::TrueColor,
        );
        let plain = render_pane_collapsed(
            &bare,
            geom(&bare, 30),
            &chrome(None),
            &palette(),
            ColorProfile::TrueColor,
        );
        assert!(
            focused.lines[0].contains(&expected),
            "got {:?}",
            focused.lines[0]
        );
        assert!(!plain.lines[0].contains(&expected));
        // Escapes are zero display cells: focus never costs the row a cell.
        assert_eq!(display_width(&focused.lines[0]), 30);
        assert_eq!(display_width(&plain.lines[0]), 30);
    }
}
