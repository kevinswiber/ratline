use anyhow::{anyhow, bail};

/// Fish-script-compatible compact form: ceil the minutes, carry into hours.
pub fn format_compact(seconds: i64) -> String {
    let s = seconds.max(0);
    let mut hours = s / 3600;
    let mut minutes = (s % 3600 + 59) / 60;
    if minutes == 60 {
        hours += 1;
        minutes = 0;
    }
    if hours > 0 {
        format!("{hours}h {minutes:02}m")
    } else {
        format!("{minutes}m")
    }
}

/// Exact components, zero components omitted: "1h 33m 12s".
pub fn format_long(seconds: i64) -> String {
    let s = seconds.max(0);
    let (h, m, sec) = (s / 3600, (s % 3600) / 60, s % 60);
    let mut parts = Vec::new();
    if h > 0 {
        parts.push(format!("{h}h"));
    }
    if m > 0 {
        parts.push(format!("{m}m"));
    }
    if sec > 0 || parts.is_empty() {
        parts.push(format!("{sec}s"));
    }
    parts.join(" ")
}

/// HH:MM:SS, hours unbounded.
pub fn format_clock(seconds: i64) -> String {
    let s = seconds.max(0);
    format!("{:02}:{:02}:{:02}", s / 3600, (s % 3600) / 60, s % 60)
}

/// Parse "1h33m", "90s", "1d", "2" (bare seconds); whitespace tolerated.
pub fn parse_duration(input: &str) -> anyhow::Result<i64> {
    let compact: String = input.chars().filter(|c| !c.is_whitespace()).collect();
    if compact.is_empty() {
        bail!("empty duration");
    }
    let mut total: i64 = 0;
    let mut digits = String::new();
    let mut saw_unit = false;
    for c in compact.chars() {
        if c.is_ascii_digit() {
            digits.push(c);
            continue;
        }
        if digits.is_empty() {
            bail!("invalid duration: {input:?}");
        }
        let n: i64 = digits.parse()?;
        digits.clear();
        saw_unit = true;
        total += match c {
            'd' => n * 86_400,
            'h' => n * 3_600,
            'm' => n * 60,
            's' => n,
            _ => bail!("invalid duration unit {c:?} in {input:?}"),
        };
    }
    if !digits.is_empty() {
        if saw_unit {
            bail!("trailing number without unit in {input:?}");
        }
        // A bare number is seconds.
        total = digits
            .parse()
            .map_err(|_| anyhow!("invalid duration: {input:?}"))?;
    }
    Ok(total)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn compact_matches_fish_format_duration() {
        assert_eq!(format_compact(5580), "1h 33m");
        assert_eq!(format_compact(2700), "45m");
        assert_eq!(format_compact(0), "0m");
        assert_eq!(format_compact(-5), "0m");
        assert_eq!(format_compact(59), "1m");
        // Ceil-carry: 3599s ceils 59m59s to 60m, which carries to 1h 00m.
        assert_eq!(format_compact(3599), "1h 00m");
        assert_eq!(format_compact(3600), "1h 00m");
        assert_eq!(format_compact(5548), "1h 33m");
    }

    #[test]
    fn long_omits_zero_components() {
        assert_eq!(format_long(5592), "1h 33m 12s");
        assert_eq!(format_long(2700), "45m");
        assert_eq!(format_long(90), "1m 30s");
        assert_eq!(format_long(0), "0s");
        assert_eq!(format_long(-3), "0s");
    }

    #[test]
    fn clock_is_zero_padded() {
        assert_eq!(format_clock(5592), "01:33:12");
        assert_eq!(format_clock(45), "00:00:45");
        assert_eq!(format_clock(-1), "00:00:00");
        assert_eq!(format_clock(90061), "25:01:01");
    }

    #[test]
    fn parse_duration_tokens() {
        assert_eq!(parse_duration("1h33m").unwrap(), 5580);
        assert_eq!(parse_duration("1h 33m").unwrap(), 5580);
        assert_eq!(parse_duration("90s").unwrap(), 90);
        assert_eq!(parse_duration("2h").unwrap(), 7200);
        assert_eq!(parse_duration("2").unwrap(), 2);
        assert_eq!(parse_duration("1d").unwrap(), 86400);
        assert!(parse_duration("").is_err());
        assert!(parse_duration("abc").is_err());
        assert!(parse_duration("5x").is_err());
    }

    #[test]
    fn long_round_trips_through_parse() {
        for n in [1, 59, 60, 90, 3600, 5592, 86461] {
            assert_eq!(parse_duration(&format_long(n)).unwrap(), n, "n={n}");
        }
    }
}

#[derive(Clone, Copy, PartialEq, Eq, Debug, clap::ValueEnum)]
pub enum DurationFormat {
    Compact,
    Long,
    Clock,
}
