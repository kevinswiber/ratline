use anyhow::{Context, anyhow};
use jiff::Timestamp;
use jiff::tz::TimeZone;

use crate::core::duration::format_compact;

/// Accepts "now", integer epoch seconds, or RFC3339/ISO8601.
pub fn parse_timestamp(s: &str) -> anyhow::Result<Timestamp> {
    let trimmed = s.trim();
    if trimmed.is_empty() {
        return Err(anyhow!("empty timestamp"));
    }
    if trimmed == "now" {
        return Ok(Timestamp::now());
    }
    if trimmed
        .strip_prefix('-')
        .unwrap_or(trimmed)
        .chars()
        .all(|c| c.is_ascii_digit())
    {
        let secs: i64 = trimmed.parse().context("parsing epoch seconds")?;
        return Timestamp::from_second(secs).context("epoch out of range");
    }
    trimmed
        .parse::<Timestamp>()
        .with_context(|| format!("invalid timestamp: {trimmed:?}"))
}

/// strftime formatting in UTC or the system zone.
pub fn format_timestamp(ts: Timestamp, fmt: &str, utc: bool) -> anyhow::Result<String> {
    let zone = if utc {
        TimeZone::UTC
    } else {
        TimeZone::system()
    };
    let zoned = ts.to_zoned(zone);
    jiff::fmt::strtime::format(fmt, &zoned).with_context(|| format!("invalid format: {fmt:?}"))
}

/// "5m ago" / "in 1h 33m" / "just now" (within ten seconds).
pub fn relative(from: Timestamp, to: Timestamp) -> String {
    let delta = to.as_second() - from.as_second();
    if delta.abs() < 10 {
        return "just now".to_string();
    }
    if delta < 0 {
        format!("{} ago", format_compact(-delta))
    } else {
        format!("in {}", format_compact(delta))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const EPOCH: i64 = 1785067200; // 2026-07-26T12:00:00Z

    #[test]
    fn parses_rfc3339_and_epoch_and_now() {
        assert_eq!(
            parse_timestamp("2026-07-26T12:00:00Z").unwrap().as_second(),
            EPOCH
        );
        assert_eq!(parse_timestamp("1785067200").unwrap().as_second(), EPOCH);
        let now = parse_timestamp("now").unwrap();
        assert!(now.as_second() > EPOCH - 86400 * 365 * 10);
        assert!(parse_timestamp("garbage").is_err());
        assert!(parse_timestamp("").is_err());
    }

    #[test]
    fn formats_in_utc() {
        let ts = parse_timestamp("1785067200").unwrap();
        assert_eq!(format_timestamp(ts, "%H:%M", true).unwrap(), "12:00");
        assert_eq!(format_timestamp(ts, "%s", true).unwrap(), "1785067200");
        assert_eq!(
            format_timestamp(ts, "%Y-%m-%d", true).unwrap(),
            "2026-07-26"
        );
    }

    #[test]
    fn relative_phrases() {
        let now = parse_timestamp("1785067200").unwrap();
        let past = parse_timestamp(&(EPOCH - 300).to_string()).unwrap();
        let future = parse_timestamp(&(EPOCH + 5580).to_string()).unwrap();
        assert_eq!(relative(now, past), "5m ago");
        assert_eq!(relative(now, future), "in 1h 33m");
        assert_eq!(relative(now, now), "just now");
    }
}
