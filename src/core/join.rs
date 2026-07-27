use crate::core::measure::{Align, display_width, pad_display};

/// Where a shorter block sits beside a taller one.
#[derive(Copy, Clone, PartialEq, Eq, Debug, Default, clap::ValueEnum)]
pub enum VAlign {
    #[default]
    Top,
    Middle,
    Bottom,
}

/// The `--align` value; which half is legal depends on the direction.
#[derive(Copy, Clone, PartialEq, Eq, Debug, clap::ValueEnum)]
pub enum JoinAlign {
    Top,
    Middle,
    Bottom,
    Left,
    Center,
    Right,
}

impl JoinAlign {
    /// The vertical alignment for a horizontal join.
    pub fn vertical(self) -> anyhow::Result<VAlign> {
        match self {
            JoinAlign::Top => Ok(VAlign::Top),
            JoinAlign::Middle => Ok(VAlign::Middle),
            JoinAlign::Bottom => Ok(VAlign::Bottom),
            other => Err(anyhow::anyhow!(
                "align {other:?} applies to vertical joins; use top, middle, or bottom"
            )),
        }
    }

    /// The horizontal alignment for a vertical join.
    pub fn horizontal(self) -> anyhow::Result<Align> {
        match self {
            JoinAlign::Left => Ok(Align::Left),
            JoinAlign::Center => Ok(Align::Center),
            JoinAlign::Right => Ok(Align::Right),
            other => Err(anyhow::anyhow!(
                "align {other:?} applies to horizontal joins; use left, center, or right"
            )),
        }
    }
}

/// Split a block into lines (CRLF-safe); empty input is zero lines.
pub fn parse_block(text: &str) -> Vec<String> {
    text.lines().map(str::to_string).collect()
}

/// Cells a horizontal join would occupy: block widths plus the gaps.
pub fn horizontal_width(blocks: &[Vec<String>], gap: usize) -> usize {
    let widths: usize = blocks
        .iter()
        .map(|block| block.iter().map(|l| display_width(l)).max().unwrap_or(0))
        .sum();
    widths + gap * blocks.len().saturating_sub(1)
}

/// Place blocks side by side: each block's lines pad to its widest line,
/// rows join with `gap` spaces, and nothing after the last non-empty
/// segment is emitted.
pub fn join_horizontal(blocks: &[Vec<String>], gap: usize, align: VAlign) -> Vec<String> {
    let widths: Vec<usize> = blocks
        .iter()
        .map(|block| block.iter().map(|l| display_width(l)).max().unwrap_or(0))
        .collect();
    let height = blocks.iter().map(Vec::len).max().unwrap_or(0);
    let offsets: Vec<usize> = blocks
        .iter()
        .map(|block| match align {
            VAlign::Top => 0,
            VAlign::Middle => (height - block.len()) / 2,
            VAlign::Bottom => height - block.len(),
        })
        .collect();
    let gap_text = " ".repeat(gap);
    (0..height)
        .map(|row| {
            let segments: Vec<&str> = blocks
                .iter()
                .zip(&offsets)
                .map(|(block, &offset)| {
                    row.checked_sub(offset)
                        .and_then(|i| block.get(i))
                        .map(String::as_str)
                        .unwrap_or("")
                })
                .collect();
            let Some(last) = segments.iter().rposition(|s| !s.is_empty()) else {
                return String::new();
            };
            segments[..=last]
                .iter()
                .enumerate()
                .map(|(i, segment)| {
                    if i < last {
                        pad_display(segment, widths[i], Align::Left)
                    } else {
                        segment.to_string()
                    }
                })
                .collect::<Vec<_>>()
                .join(&gap_text)
        })
        .collect()
}

/// Stack blocks with `gap` blank lines between; non-left alignment pads
/// narrower lines toward the widest block, never inventing trailing spaces.
pub fn join_vertical(blocks: &[Vec<String>], gap: usize, align: Align) -> Vec<String> {
    let width = blocks
        .iter()
        .flat_map(|block| block.iter().map(|l| display_width(l)))
        .max()
        .unwrap_or(0);
    let mut out = Vec::new();
    for (i, block) in blocks.iter().enumerate() {
        if i > 0 {
            for _ in 0..gap {
                out.push(String::new());
            }
        }
        for line in block {
            let current = display_width(line);
            let missing = width.saturating_sub(current);
            let padded = match align {
                Align::Left => line.clone(),
                Align::Center => format!("{}{line}", " ".repeat(missing / 2)),
                Align::Right => format!("{}{line}", " ".repeat(missing)),
            };
            out.push(padded);
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::core::measure::Align;

    #[test]
    fn parse_block_splits_lines_and_survives_crlf() {
        assert_eq!(parse_block("a\nb\n"), vec!["a", "b"]);
        assert_eq!(parse_block("a\nb"), vec!["a", "b"]);
        assert_eq!(parse_block("a\r\nb\r\n"), vec!["a", "b"]);
        assert_eq!(parse_block(""), Vec::<String>::new());
        assert_eq!(parse_block("\n"), vec![""]);
    }

    #[test]
    fn horizontal_join_pads_each_block_to_its_widest_line() {
        let left = parse_block("a\nlonger\n");
        let right = parse_block("1\n2\n");
        assert_eq!(
            join_horizontal(&[left, right], 1, VAlign::Top),
            vec!["a      1", "longer 2"]
        );
    }

    #[test]
    fn unequal_heights_fill_per_align() {
        let tall = parse_block("1\n2\n3\n");
        let short = parse_block("x\n");
        assert_eq!(
            join_horizontal(&[tall.clone(), short.clone()], 1, VAlign::Top),
            vec!["1 x", "2", "3"]
        );
        assert_eq!(
            join_horizontal(&[tall.clone(), short.clone()], 1, VAlign::Middle),
            vec!["1", "2 x", "3"]
        );
        assert_eq!(
            join_horizontal(&[tall, short], 1, VAlign::Bottom),
            vec!["1", "2", "3 x"]
        );
    }

    #[test]
    fn ansi_blocks_pad_by_display_width() {
        let left = parse_block("\x1b[31mab\x1b[0m\nabcd\n");
        let right = parse_block("x\ny\n");
        assert_eq!(
            join_horizontal(&[left, right], 0, VAlign::Top),
            vec!["\x1b[31mab\x1b[0m  x", "abcdy"]
        );
    }

    #[test]
    fn vertical_join_stacks_with_gap_lines() {
        assert_eq!(
            join_vertical(&[parse_block("a\n"), parse_block("b\n")], 1, Align::Left),
            vec!["a", "", "b"]
        );
    }

    #[test]
    fn vertical_join_centers_narrow_blocks_without_trailing_padding() {
        assert_eq!(
            join_vertical(
                &[parse_block("abcdef\n"), parse_block("xy\n")],
                0,
                Align::Center
            ),
            vec!["abcdef", "  xy"]
        );
    }

    #[test]
    fn join_align_rejects_a_mismatched_direction() {
        assert!(JoinAlign::Middle.horizontal().is_err());
        assert!(JoinAlign::Center.vertical().is_err());
        assert_eq!(JoinAlign::Top.vertical().unwrap(), VAlign::Top);
        assert_eq!(JoinAlign::Right.horizontal().unwrap(), Align::Right);
    }

    #[test]
    fn horizontal_width_totals_blocks_and_gaps() {
        let blocks = vec![parse_block("aaaa\n"), parse_block("bb\ncc\n")];
        assert_eq!(horizontal_width(&blocks, 2), 8);
        assert_eq!(horizontal_width(&blocks, 0), 6);
        assert_eq!(horizontal_width(&[], 2), 0);
    }

    #[test]
    fn an_empty_block_contributes_no_width() {
        assert_eq!(
            join_horizontal(&[parse_block(""), parse_block("x\n")], 1, VAlign::Top),
            vec![" x"]
        );
    }
}
