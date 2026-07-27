//! DEC private mode 2031 (theme-change notifications), the DSR 997 report
//! it produces, and the OSC 10/11 color queries used to verify one. This
//! is the only place in the binary that writes `CSI ? 2031 h`/`l` or
//! issues a verify query. The subscription and its verify queries are
//! answered only for a terminal that already pushed a notification on its
//! own initiative, and only by the component that owns the terminal's
//! input stream in raw mode — the same component that would otherwise
//! receive the reply as a keystroke.

use crate::theme::Appearance;

/// Find a `CSI ? 997 ; Ps n` report anywhere in the byte stream. Ps=1
/// dark, Ps=2 light — the single place that mapping is encoded.
pub fn parse_color_scheme_report(bytes: &[u8]) -> Option<Appearance> {
    let needle = b"\x1b[?997;";
    let pos = bytes
        .windows(needle.len())
        .position(|window| window == needle)?;
    let rest = &bytes[pos + needle.len()..];
    let (ps, tail) = rest.split_first()?;
    if tail.first() != Some(&b'n') {
        return None;
    }
    match ps {
        b'1' => Some(Appearance::Dark),
        b'2' => Some(Appearance::Light),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_both_polarities() {
        assert_eq!(
            parse_color_scheme_report(b"\x1b[?997;1n"),
            Some(Appearance::Dark)
        );
        assert_eq!(
            parse_color_scheme_report(b"\x1b[?997;2n"),
            Some(Appearance::Light)
        );
    }

    #[test]
    fn bare_report_without_a_parameter_has_no_verdict() {
        assert_eq!(parse_color_scheme_report(b"\x1b[?997n"), None);
    }

    #[test]
    fn wrong_final_byte_and_truncation_have_no_verdict() {
        assert_eq!(parse_color_scheme_report(b"\x1b[?997;2y"), None);
        assert_eq!(parse_color_scheme_report(b"\x1b[?997;2"), None);
    }

    #[test]
    fn out_of_range_and_multi_digit_ps_have_no_verdict() {
        assert_eq!(parse_color_scheme_report(b"\x1b[?997;9n"), None);
        // The byte immediately after Ps must be `n`; a second digit means
        // this is not the single-digit form the report defines.
        assert_eq!(parse_color_scheme_report(b"\x1b[?997;12n"), None);
    }

    #[test]
    fn report_embedded_in_other_bytes_is_found() {
        assert_eq!(
            parse_color_scheme_report(b"noise\x1b[?997;2nmore"),
            Some(Appearance::Light)
        );
    }
}
