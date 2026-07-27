// Consumed by the table command, which lands with its wiring.
#![allow(dead_code)]

use crate::core::measure::{Align, ELLIPSIS, display_width};

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
}
