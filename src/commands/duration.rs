use anyhow::anyhow;

use crate::cli::DurationArgs;
use crate::color::ColorProfile;
use crate::core::duration::{
    DurationFormat, format_clock, format_compact, format_long, parse_duration,
};
use crate::exit::AppResult;
use crate::theme::Palette;

pub fn run(args: DurationArgs, _profile: ColorProfile, _palette: Palette) -> AppResult {
    if args.seconds {
        println!("{}", parse_duration(&args.value)?);
        return Ok(());
    }
    let raw: f64 = args
        .value
        .parse()
        .map_err(|_| anyhow!("invalid number: {:?}", args.value))?;
    let secs = if args.ms { raw / 1000.0 } else { raw } as i64;
    let out = match args.format {
        DurationFormat::Compact => format_compact(secs),
        DurationFormat::Long => format_long(secs),
        DurationFormat::Clock => format_clock(secs),
    };
    println!("{out}");
    Ok(())
}
