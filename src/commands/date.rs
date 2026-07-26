use crate::cli::DateArgs;
use crate::color::ColorProfile;
use crate::core::datetime::{format_timestamp, parse_timestamp, relative};
use crate::exit::AppResult;

pub fn run(args: DateArgs, _profile: ColorProfile) -> AppResult {
    let value = parse_timestamp(args.value.as_deref().unwrap_or("now"))?;

    if let Some(since) = &args.since {
        let from = parse_timestamp(since)?;
        println!("{}", value.as_second() - from.as_second());
        return Ok(());
    }
    if let Some(until) = &args.until {
        let to = parse_timestamp(until)?;
        println!("{}", to.as_second() - value.as_second());
        return Ok(());
    }
    if args.epoch {
        println!("{}", value.as_second());
        return Ok(());
    }
    if args.relative {
        println!("{}", relative(parse_timestamp("now")?, value));
        return Ok(());
    }
    if let Some(fmt) = &args.format {
        println!("{}", format_timestamp(value, fmt, args.utc)?);
        return Ok(());
    }
    // Default: RFC3339 (UTC instant form).
    println!("{value}");
    Ok(())
}
