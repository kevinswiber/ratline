use crate::core::measure::{
    Align, ELLIPSIS, display_width, pad_display, truncate_display, wrap_display,
};

#[derive(Copy, Clone, PartialEq, Eq, Debug, Default, clap::ValueEnum)]
pub enum Overflow {
    #[default]
    Truncate,
    Wrap,
}

/// Per-column geometry. `width: None` auto-sizes to the widest cell.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct ColumnSpec {
    pub width: Option<usize>,
    pub align: Align,
    pub overflow: Overflow,
}

/// One input line: a row of cells, or a blank spacer passed through.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Row {
    Cells(Vec<String>),
    Blank,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct TableSpec {
    pub columns: Vec<ColumnSpec>,
    pub separator: String,
    pub ellipsis: String,
}

impl Default for TableSpec {
    fn default() -> Self {
        TableSpec {
            columns: Vec::new(),
            separator: "  ".to_string(),
            ellipsis: ELLIPSIS.to_string(),
        }
    }
}

pub fn parse_table(input: &str, delimiter: char) -> Vec<Row> {
    input
        .lines()
        .map(|line| {
            if line.is_empty() {
                Row::Blank
            } else {
                Row::Cells(line.split(delimiter).map(str::to_string).collect())
            }
        })
        .collect()
}

/// Parse one entry of a positional comma list; empty keeps the default.
fn parse_entry<T>(
    entry: &str,
    n: usize,
    what: &str,
    parse: impl Fn(&str) -> Option<T>,
) -> anyhow::Result<Option<T>> {
    if entry.is_empty() {
        return Ok(None);
    }
    parse(entry)
        .map(Some)
        .ok_or_else(|| anyhow::anyhow!("column {n}: invalid {what} {entry:?}"))
}

pub fn parse_widths(s: &str) -> anyhow::Result<Vec<Option<usize>>> {
    s.split(',')
        .enumerate()
        .map(|(i, entry)| parse_entry(entry.trim(), i + 1, "width", |e| e.parse().ok()))
        .collect()
}

pub fn parse_aligns(s: &str) -> anyhow::Result<Vec<Option<Align>>> {
    s.split(',')
        .enumerate()
        .map(|(i, entry)| {
            parse_entry(entry.trim(), i + 1, "alignment", |e| match e {
                "l" | "left" => Some(Align::Left),
                "r" | "right" => Some(Align::Right),
                "c" | "center" => Some(Align::Center),
                _ => None,
            })
        })
        .collect()
}

pub fn parse_overflows(s: &str) -> anyhow::Result<Vec<Option<Overflow>>> {
    s.split(',')
        .enumerate()
        .map(|(i, entry)| {
            parse_entry(entry.trim(), i + 1, "overflow", |e| match e {
                "truncate" => Some(Overflow::Truncate),
                "wrap" => Some(Overflow::Wrap),
                _ => None,
            })
        })
        .collect()
}

/// Build per-column specs from the three positional lists; short lists and
/// empty entries keep that column's default.
pub fn parse_columns(
    widths: Option<&str>,
    aligns: Option<&str>,
    overflows: Option<&str>,
) -> anyhow::Result<Vec<ColumnSpec>> {
    let widths = widths.map(parse_widths).transpose()?.unwrap_or_default();
    let aligns = aligns.map(parse_aligns).transpose()?.unwrap_or_default();
    let overflows = overflows
        .map(parse_overflows)
        .transpose()?
        .unwrap_or_default();
    let count = widths.len().max(aligns.len()).max(overflows.len());
    Ok((0..count)
        .map(|i| ColumnSpec {
            width: widths.get(i).copied().flatten(),
            align: aligns.get(i).copied().flatten().unwrap_or_default(),
            overflow: overflows.get(i).copied().flatten().unwrap_or_default(),
        })
        .collect())
}

/// Resolved column widths: pinned takes the pin, auto takes the widest cell.
pub fn resolve_widths(rows: &[Row], columns: &[ColumnSpec]) -> Vec<usize> {
    let count = rows
        .iter()
        .map(|row| match row {
            Row::Cells(cells) => cells.len(),
            Row::Blank => 0,
        })
        .max()
        .unwrap_or(0);
    (0..count)
        .map(|i| {
            columns.get(i).and_then(|c| c.width).unwrap_or_else(|| {
                rows.iter()
                    .filter_map(|row| match row {
                        Row::Cells(cells) => cells.get(i).map(|c| display_width(c)),
                        Row::Blank => None,
                    })
                    .max()
                    .unwrap_or(0)
            })
        })
        .collect()
}

/// Render rows into aligned lines. Widths resolve once for the whole table;
/// a wrapping cell turns its row into several physical lines; the final
/// non-empty column is never right-padded and trailing empty columns drop
/// their separators.
pub fn render_table(rows: &[Row], spec: &TableSpec) -> Vec<String> {
    let widths = resolve_widths(rows, &spec.columns);
    rows.iter()
        .flat_map(|row| match row {
            Row::Blank => vec![String::new()],
            Row::Cells(cells) => render_row(cells, &widths, spec),
        })
        .collect()
}

fn render_row(cells: &[String], widths: &[usize], spec: &TableSpec) -> Vec<String> {
    let cell_lines: Vec<Vec<String>> = cells
        .iter()
        .zip(widths)
        .enumerate()
        .map(|(i, (cell, &width))| {
            let overflow = spec.columns.get(i).map(|c| c.overflow).unwrap_or_default();
            match overflow {
                Overflow::Wrap => wrap_display(cell, width),
                Overflow::Truncate => vec![truncate_display(cell, width, &spec.ellipsis)],
            }
        })
        .collect();
    let height = cell_lines.iter().map(Vec::len).max().unwrap_or(0);
    (0..height)
        .map(|k| {
            let physical: Vec<&str> = cell_lines
                .iter()
                .map(|lines| lines.get(k).map(String::as_str).unwrap_or(""))
                .collect();
            render_physical(&physical, widths, spec)
        })
        .collect()
}

/// One physical output line from per-column cell text.
fn render_physical(rendered: &[&str], widths: &[usize], spec: &TableSpec) -> String {
    let Some(last) = rendered.iter().rposition(|cell| !cell.is_empty()) else {
        return String::new();
    };
    rendered[..=last]
        .iter()
        .enumerate()
        .map(|(i, cell)| {
            let align = spec.columns.get(i).map(|c| c.align).unwrap_or_default();
            if i < last {
                pad_display(cell, widths[i], align)
            } else {
                pad_last_cell(cell, widths[i], align)
            }
        })
        .collect::<Vec<_>>()
        .join(&spec.separator)
}

/// The final column keeps only left-side alignment padding, so a line never
/// ends in whitespace the layout invented.
fn pad_last_cell(cell: &str, width: usize, align: Align) -> String {
    let current = display_width(cell);
    if current >= width {
        return cell.to_string();
    }
    let missing = width - current;
    match align {
        Align::Left => cell.to_string(),
        Align::Right => format!("{}{cell}", " ".repeat(missing)),
        Align::Center => format!("{}{cell}", " ".repeat(missing / 2)),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::core::measure::Align;

    #[test]
    fn parse_table_splits_on_the_delimiter_and_keeps_blank_lines() {
        assert_eq!(
            parse_table("a\tb\n\nc\td\n", '\t'),
            vec![
                Row::Cells(vec!["a".into(), "b".into()]),
                Row::Blank,
                Row::Cells(vec!["c".into(), "d".into()]),
            ]
        );
    }

    #[test]
    fn parse_table_handles_crlf_input() {
        assert_eq!(
            parse_table("a\tb\r\n", '\t'),
            vec![Row::Cells(vec!["a".into(), "b".into()])]
        );
    }

    #[test]
    fn resolve_widths_auto_sizes_to_the_widest_cell() {
        assert_eq!(
            resolve_widths(&parse_table("ab\tlonger\nabcd\tx\n", '\t'), &[]),
            vec![4, 6]
        );
    }

    #[test]
    fn resolve_widths_measures_display_cells_not_bytes() {
        assert_eq!(
            resolve_widths(&parse_table("\x1b[31mab\x1b[0m\t日本\n", '\t'), &[]),
            vec![2, 4]
        );
    }

    #[test]
    fn an_explicit_width_pins_the_column() {
        let columns = vec![
            ColumnSpec {
                width: Some(9),
                ..ColumnSpec::default()
            },
            ColumnSpec::default(),
        ];
        assert_eq!(
            resolve_widths(&parse_table("ab\tlonger\n", '\t'), &columns),
            vec![9, 6]
        );
    }

    #[test]
    fn ragged_rows_use_the_widest_row_for_the_column_count() {
        assert_eq!(
            resolve_widths(&parse_table("a\nb\tc\td\n", '\t'), &[]).len(),
            3
        );
    }

    #[test]
    fn parse_columns_maps_positional_lists() {
        let columns = parse_columns(Some("27,,8"), Some("l,r"), Some("truncate,wrap")).unwrap();
        assert_eq!(columns.len(), 3);
        assert_eq!(columns[0].width, Some(27));
        assert_eq!(columns[1].width, None);
        assert_eq!(columns[1].align, Align::Right);
        assert_eq!(columns[1].overflow, Overflow::Wrap);
        assert_eq!(columns[2].width, Some(8));
        assert_eq!(columns[2].align, Align::Left);
        assert_eq!(columns[2].overflow, Overflow::Truncate);
    }

    #[test]
    fn align_accepts_letters_and_words() {
        assert_eq!(
            parse_aligns("l,right,c").unwrap(),
            vec![Some(Align::Left), Some(Align::Right), Some(Align::Center)]
        );
    }

    #[test]
    fn bad_column_specs_name_the_column() {
        let err = parse_columns(Some("27,nope"), None, None)
            .unwrap_err()
            .to_string();
        assert!(err.contains("column 2"), "got: {err}");
        assert!(parse_columns(None, Some("l,x"), None).is_err());
        assert!(parse_columns(None, None, Some("wrapp")).is_err());
    }

    fn spec(columns: Vec<ColumnSpec>) -> TableSpec {
        TableSpec {
            columns,
            ..TableSpec::default()
        }
    }

    #[test]
    fn render_pads_columns_to_a_shared_width() {
        assert_eq!(
            render_table(&parse_table("a\t1\nlonger\t22\n", '\t'), &spec(vec![])),
            vec!["a       1", "longer  22"]
        );
    }

    #[test]
    fn right_aligned_columns_pad_on_the_left() {
        let columns = vec![
            ColumnSpec::default(),
            ColumnSpec {
                align: Align::Right,
                ..ColumnSpec::default()
            },
        ];
        assert_eq!(
            render_table(&parse_table("a\t1\nb\t22\n", '\t'), &spec(columns)),
            vec!["a   1", "b  22"]
        );
    }

    #[test]
    fn pinned_columns_truncate_with_the_ellipsis() {
        let columns = vec![
            ColumnSpec {
                width: Some(4),
                ..ColumnSpec::default()
            },
            ColumnSpec::default(),
        ];
        assert_eq!(
            render_table(&parse_table("abcdefgh\tx\n", '\t'), &spec(columns)),
            vec!["abc…  x"]
        );
    }

    #[test]
    fn ansi_cells_keep_their_escapes_and_their_width() {
        assert_eq!(
            render_table(
                &parse_table("\x1b[31mab\x1b[0m\tx\nabcd\ty\n", '\t'),
                &spec(vec![])
            ),
            vec!["\x1b[31mab\x1b[0m    x", "abcd  y"]
        );
    }

    #[test]
    fn a_row_styled_end_to_end_keeps_color_across_cells() {
        // An SGR opened before the first cell and reset after the last one
        // survives the split: the open code rides cell 1, the reset rides the
        // final cell, and the padding between stays inside the colored span.
        let rows = parse_table("\x1b[38;5;42m✓\t1.1\tland the fix\x1b[0m\n", '\t');
        let out = render_table(&rows, &spec(vec![]));
        assert_eq!(out, vec!["\x1b[38;5;42m✓  1.1  land the fix\x1b[0m"]);
    }

    #[test]
    fn blank_rows_pass_through() {
        assert_eq!(
            render_table(&parse_table("a\tb\n\nc\td\n", '\t'), &spec(vec![])),
            vec!["a  b", "", "c  d"]
        );
    }

    #[test]
    fn no_line_ends_in_invented_padding() {
        for line in render_table(&parse_table("a\tb\nlonger\t\n", '\t'), &spec(vec![])) {
            assert_eq!(line, line.trim_end(), "trailing padding: {line:?}");
        }
    }

    #[test]
    fn missing_trailing_cells_drop_their_separators() {
        assert_eq!(
            render_table(&parse_table("a\tb\tc\nlonger\n", '\t'), &spec(vec![]))[1],
            "longer"
        );
    }

    #[test]
    fn a_custom_separator_sets_the_gutter() {
        let s = TableSpec {
            separator: " ".into(),
            ..TableSpec::default()
        };
        assert_eq!(
            render_table(&parse_table("ab\tx\n", '\t'), &s),
            vec!["ab x"]
        );
    }

    #[test]
    fn wrapped_cells_add_continuation_lines_in_their_column() {
        let columns = vec![
            ColumnSpec::default(),
            ColumnSpec {
                width: Some(9),
                overflow: Overflow::Wrap,
                ..ColumnSpec::default()
            },
        ];
        assert_eq!(
            render_table(
                &parse_table("id\tthe quick brown fox\n", '\t'),
                &spec(columns)
            ),
            vec!["id  the quick", "    brown fox"]
        );
    }

    #[test]
    fn wrapped_rows_keep_later_columns_aligned() {
        let columns = vec![
            ColumnSpec {
                width: Some(3),
                overflow: Overflow::Wrap,
                ..ColumnSpec::default()
            },
            ColumnSpec::default(),
        ];
        assert_eq!(
            render_table(&parse_table("a b c d\tx\n", '\t'), &spec(columns)),
            vec!["a b  x", "c d"]
        );
    }

    #[test]
    fn a_wrapped_styled_cell_reopens_its_style_per_line() {
        let columns = vec![ColumnSpec {
            width: Some(3),
            overflow: Overflow::Wrap,
            ..ColumnSpec::default()
        }];
        let out = render_table(
            &parse_table("\x1b[31ma b c d\x1b[0m\n", '\t'),
            &spec(columns),
        );
        assert_eq!(out, vec!["\x1b[31ma b\x1b[0m", "\x1b[31mc d\x1b[0m"]);
    }

    #[test]
    fn auto_width_columns_never_wrap() {
        let columns = vec![ColumnSpec {
            overflow: Overflow::Wrap,
            ..ColumnSpec::default()
        }];
        assert_eq!(
            render_table(&parse_table("a b c d\n", '\t'), &spec(columns)),
            vec!["a b c d"]
        );
    }
}
