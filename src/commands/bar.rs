use std::io::Read;

use anyhow::{Context, anyhow};
use ratatui::style::Color;

use crate::cli::BarArgs;
use crate::color::ColorProfile;
use crate::core::bar::{
    Annotation, BarSpec, auto_label_width, color_for, parse_rows, parse_thresholds, render_bar,
    render_indeterminate, render_rows,
};
use crate::exit::AppResult;
use crate::style_spec::{StyleSpec, parse_color};
use crate::theme::Palette;

const DEFAULT_FILL_COLOR: Color = Color::Indexed(212);
const DEFAULT_LABEL_WIDTH: u16 = 34;

fn fg(color: Color) -> StyleSpec {
    StyleSpec {
        foreground: Some(color),
        ..StyleSpec::default()
    }
}

pub fn run(args: BarArgs, profile: ColorProfile, _palette: Palette) -> AppResult {
    if args.total < 0.0 {
        return Err(anyhow!("total must be non-negative").into());
    }
    let explicit_fill = args.fill_color.as_deref().map(parse_color).transpose()?;
    let thresholds = args
        .thresholds
        .as_deref()
        .map(parse_thresholds)
        .transpose()?
        .unwrap_or_default();
    // Explicit --fill-color wins; otherwise thresholds pick by percent.
    let fill_color_for = |value: f64, total: f64| -> Color {
        let pct = if total > 0.0 {
            value.clamp(0.0, total) * 100.0 / total
        } else {
            0.0
        };
        explicit_fill
            .or_else(|| color_for(pct, &thresholds, None))
            .unwrap_or(DEFAULT_FILL_COLOR)
    };

    let (preset_fill, preset_empty) = args.preset.chars();
    let base = BarSpec {
        value: 0.0,
        total: args.total,
        width: args.width,
        fill: args.fill.unwrap_or(preset_fill),
        empty: args.empty.unwrap_or(preset_empty),
        fill_style: StyleSpec::default(),
        empty_style: fg(parse_color(&args.empty_color)?),
        label: args.label.as_deref(),
        label_width: args.label_width.unwrap_or(DEFAULT_LABEL_WIDTH),
        annotation: args.annotation,
        state: args.state.as_deref(),
    };

    if args.indeterminate {
        let spec = BarSpec {
            fill_style: fg(explicit_fill.unwrap_or(DEFAULT_FILL_COLOR)),
            annotation: Annotation::None,
            ..base
        };
        println!("{}", render_indeterminate(&spec, args.tick, profile));
        return Ok(());
    }

    if let Some(value) = args.value {
        let spec = BarSpec {
            value,
            fill_style: fg(fill_color_for(value, args.total)),
            ..base
        };
        println!("{}", render_bar(&spec, profile));
        return Ok(());
    }

    let mut input = String::new();
    std::io::stdin()
        .read_to_string(&mut input)
        .context("reading stdin")?;
    let rows = parse_rows(&input, args.delimiter)?;
    if rows.is_empty() {
        return Err(anyhow!("--value is required when stdin has no rows").into());
    }
    // An explicit width is the column, in batch mode too, so bars from
    // separate invocations can agree on it; only auto-size when unset.
    let label_width = args
        .label_width
        .unwrap_or_else(|| auto_label_width(&rows, DEFAULT_LABEL_WIDTH));
    if thresholds.is_empty() {
        // One shared fill style: render as an aligned block.
        let spec = BarSpec {
            fill_style: fg(explicit_fill.unwrap_or(DEFAULT_FILL_COLOR)),
            label_width,
            ..base
        };
        for line in render_rows(&rows, &spec, profile) {
            println!("{line}");
        }
    } else {
        // Threshold colors vary per row; keep the shared label column.
        for row in &rows {
            let spec = BarSpec {
                value: row.value,
                total: row.total,
                label: Some(row.label.as_str()),
                label_width,
                state: row.state.as_deref(),
                fill_style: fg(fill_color_for(row.value, row.total)),
                ..base.clone()
            };
            println!("{}", render_bar(&spec, profile));
        }
    }
    Ok(())
}
