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
use crate::core::measure::{Align, ELLIPSIS, pad_display, truncate_display};
use crate::core::registry::{Overflow, PaneBox, PaneGeometry};
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
    use crate::core::measure::display_width;
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
