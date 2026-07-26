use std::io::Read;

use anyhow::{Context, anyhow};

use crate::cli::SparkArgs;
use crate::color::ColorProfile;
use crate::core::spark::sparkline;
use crate::exit::AppResult;
use crate::style_spec::{StyleSpec, parse_color};

pub fn run(args: SparkArgs, profile: ColorProfile) -> AppResult {
    let raw = if args.values.is_empty() {
        let mut buf = String::new();
        std::io::stdin()
            .read_to_string(&mut buf)
            .context("reading stdin")?;
        buf
    } else {
        args.values.join(" ")
    };
    let values: Vec<f64> = raw
        .split_whitespace()
        .map(|token| {
            token
                .parse::<f64>()
                .map_err(|_| anyhow!("not a number: {token:?}"))
        })
        .collect::<Result<_, _>>()?;

    let line = sparkline(&values, args.min, args.max);
    let styled = match &args.spark_color {
        Some(color) if !line.is_empty() => StyleSpec {
            foreground: Some(parse_color(color)?),
            ..StyleSpec::default()
        }
        .render(&line, profile),
        _ => line,
    };
    println!("{styled}");
    Ok(())
}
