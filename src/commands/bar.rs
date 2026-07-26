use anyhow::anyhow;

use crate::cli::BarArgs;
use crate::color::ColorProfile;
use crate::core::bar::{BarSpec, render_bar};
use crate::exit::AppResult;
use crate::style_spec::{StyleSpec, parse_color};

pub fn run(args: BarArgs, profile: ColorProfile) -> AppResult {
    let Some(value) = args.value else {
        return Err(anyhow!("--value is required").into());
    };
    if args.total < 0.0 {
        return Err(anyhow!("total must be non-negative").into());
    }

    let (preset_fill, preset_empty) = args.preset.chars();
    let spec = BarSpec {
        value,
        total: args.total,
        width: args.width,
        fill: args.fill.unwrap_or(preset_fill),
        empty: args.empty.unwrap_or(preset_empty),
        fill_style: StyleSpec {
            foreground: Some(parse_color(&args.fill_color)?),
            ..StyleSpec::default()
        },
        empty_style: StyleSpec {
            foreground: Some(parse_color(&args.empty_color)?),
            ..StyleSpec::default()
        },
        label: args.label.as_deref(),
        label_width: args.label_width,
        annotation: args.annotation,
        state: args.state.as_deref(),
    };
    println!("{}", render_bar(&spec, profile));
    Ok(())
}
