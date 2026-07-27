use std::io::Read;

use anyhow::Context;

use crate::cli::TableArgs;
use crate::color::ColorProfile;
use crate::core::table::{Row, TableSpec, parse_columns, parse_table, render_table};
use crate::exit::AppResult;

pub fn run(args: TableArgs, profile: ColorProfile) -> AppResult {
    let mut input = String::new();
    std::io::stdin()
        .read_to_string(&mut input)
        .context("reading stdin")?;
    let mut rows = parse_table(&input, args.delimiter);
    // Strip per cell, after splitting: the stripper eats control bytes,
    // including the delimiter tabs.
    if profile == ColorProfile::Ascii {
        for row in &mut rows {
            if let Row::Cells(cells) = row {
                for cell in cells.iter_mut() {
                    let stripped = strip_ansi_escapes::strip_str(cell.as_str());
                    *cell = stripped;
                }
            }
        }
    }
    let spec = TableSpec {
        columns: parse_columns(
            args.widths.as_deref(),
            args.align.as_deref(),
            args.overflow.as_deref(),
        )?,
        separator: args.separator,
        ellipsis: args.ellipsis,
    };
    for line in render_table(&rows, &spec) {
        println!("{line}");
    }
    Ok(())
}
