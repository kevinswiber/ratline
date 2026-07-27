use std::io::Read;

use anyhow::Context;

use crate::cli::TableArgs;
use crate::color::ColorProfile;
use crate::core::table::{TableSpec, parse_columns, parse_table, render_table};
use crate::exit::AppResult;
use crate::theme::Palette;

pub fn run(args: TableArgs, profile: ColorProfile, _palette: Palette) -> AppResult {
    let mut input = String::new();
    std::io::stdin()
        .read_to_string(&mut input)
        .context("reading stdin")?;
    if profile == ColorProfile::Ascii {
        input = crate::core::measure::strip_escapes(&input);
    }
    let rows = parse_table(&input, args.delimiter);
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
