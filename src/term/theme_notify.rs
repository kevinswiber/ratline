//! DEC private mode 2031 (theme-change notifications), the DSR 997 report
//! it produces, and the OSC 10/11 color queries used to verify one. This
//! is the only place in the binary that writes `CSI ? 2031 h`/`l` or
//! issues a verify query. The subscription and its verify queries are
//! answered only for a terminal that already pushed a notification on its
//! own initiative, and only by the component that owns the terminal's
//! input stream in raw mode — the same component that would otherwise
//! receive the reply as a keystroke.

use std::io::Write;

use crate::color::ColorProfile;
use crate::theme::{Appearance, AppearanceSource};

const SUBSCRIBE: &[u8] = b"\x1b[?2031h"; // DECSET 2031
const UNSUBSCRIBE: &[u8] = b"\x1b[?2031l"; // DECRST 2031
// OSC 10 (foreground) then OSC 11 (background), each `?`-queried and
// ST-terminated, in one write so they land as one burst, never
// interleaved with a frame.
const REQUEST_COLORS: &[u8] = b"\x1b]10;?\x1b\\\x1b]11;?\x1b\\";

/// Owns the DEC 2031 subscription and the mid-session OSC 10/11 verify
/// query — the only component that puts either on the wire. Subscribes on
/// construction; unsubscribes on drop, best-effort — a dead terminal at
/// that point is not worth surfacing as an error. `suspend`/`resume`
/// bracket a foreign reader of the same input stream (the pager) and are
/// idempotent so the caller never has to track state itself.
pub struct ThemeNotifyGuard<W: Write> {
    out: W,
    active: bool,
}

impl<W: Write> ThemeNotifyGuard<W> {
    pub fn subscribe(mut out: W) -> std::io::Result<Self> {
        out.write_all(SUBSCRIBE)?;
        out.flush()?;
        Ok(ThemeNotifyGuard { out, active: true })
    }

    /// Idempotent: suspending an already-suspended guard writes nothing.
    pub fn suspend(&mut self) -> std::io::Result<()> {
        if !self.active {
            return Ok(());
        }
        self.out.write_all(UNSUBSCRIBE)?;
        self.out.flush()?;
        self.active = false;
        Ok(())
    }

    /// Idempotent: resuming an already-active guard writes nothing.
    pub fn resume(&mut self) -> std::io::Result<()> {
        if self.active {
            return Ok(());
        }
        self.out.write_all(SUBSCRIBE)?;
        self.out.flush()?;
        self.active = true;
        Ok(())
    }

    /// Ask for the current foreground and background in one burst; the
    /// replies land on the input stream the caller already owns.
    pub fn request_colors(&mut self) -> std::io::Result<()> {
        self.out.write_all(REQUEST_COLORS)?;
        self.out.flush()
    }
}

impl<W: Write> Drop for ThemeNotifyGuard<W> {
    fn drop(&mut self) {
        let _ = self.suspend();
    }
}

/// True when this process may subscribe to theme-change notifications.
/// Mirrors `may_detect` (`src/theme.rs`), but keys on the resolved
/// palette's provenance and adds the raw-input term `may_detect` has no
/// analogue for.
pub fn may_subscribe(
    source: AppearanceSource,
    profile: ColorProfile,
    owns_raw_input: bool,
) -> bool {
    owns_raw_input && profile != ColorProfile::Ascii && source != AppearanceSource::Explicit
}

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

/// OSC 10 (foreground) vs OSC 11 (background).
#[derive(Copy, Clone, PartialEq, Eq, Debug)]
pub enum OscColorKind {
    Foreground,
    Background,
}

/// One-shot over a complete OSC 10/11 reply. Accepts both a BEL and an
/// ST (`ESC \`) terminator.
pub fn parse_osc_color_reply(bytes: &[u8]) -> Option<(OscColorKind, xterm_color::Color)> {
    let (kind, rest) = match strip_needle(bytes, b"\x1b]10;") {
        Some(rest) => (OscColorKind::Foreground, rest),
        None => (OscColorKind::Background, strip_needle(bytes, b"\x1b]11;")?),
    };
    let payload = strip_terminator(rest)?;
    let color = xterm_color::Color::parse(payload).ok()?;
    Some((kind, color))
}

fn strip_needle<'a>(bytes: &'a [u8], needle: &[u8]) -> Option<&'a [u8]> {
    let pos = bytes
        .windows(needle.len())
        .position(|window| window == needle)?;
    Some(&bytes[pos + needle.len()..])
}

fn strip_terminator(bytes: &[u8]) -> Option<&[u8]> {
    if let Some(pos) = bytes.iter().position(|&b| b == 0x07) {
        return Some(&bytes[..pos]);
    }
    let pos = bytes.windows(2).position(|window| window == b"\x1b\\")?;
    Some(&bytes[..pos])
}

/// Startup-parity classification: a direct port of
/// `terminal-colorsaurus`'s `ColorPalette::theme_mode`, so a mid-session
/// verification agrees with the startup probe on the same colors.
pub fn classify_colors(fg: Option<&xterm_color::Color>, bg: &xterm_color::Color) -> Appearance {
    let bg_l = bg.perceived_lightness();
    let Some(fg) = fg else {
        return if bg_l > 0.5 {
            Appearance::Light
        } else {
            Appearance::Dark
        };
    };
    let fg_l = fg.perceived_lightness();
    if bg_l < fg_l {
        Appearance::Dark
    } else if bg_l > fg_l || bg_l > 0.5 {
        Appearance::Light
    } else {
        Appearance::Dark
    }
}

#[cfg(test)]
mod tests {
    use rstest::rstest;

    use super::*;
    use crate::color::ColorProfile;

    #[rstest]
    // Only the row where every term permits it may subscribe.
    #[case(true, ColorProfile::TrueColor, AppearanceSource::Osc, true)]
    #[case(true, ColorProfile::TrueColor, AppearanceSource::Explicit, false)]
    #[case(true, ColorProfile::Ascii, AppearanceSource::Osc, false)]
    #[case(true, ColorProfile::Ascii, AppearanceSource::Explicit, false)]
    #[case(false, ColorProfile::TrueColor, AppearanceSource::Osc, false)]
    #[case(false, ColorProfile::TrueColor, AppearanceSource::Explicit, false)]
    #[case(false, ColorProfile::Ascii, AppearanceSource::Osc, false)]
    #[case(false, ColorProfile::Ascii, AppearanceSource::Explicit, false)]
    fn may_subscribe_matrix(
        #[case] owns_raw_input: bool,
        #[case] profile: ColorProfile,
        #[case] source: AppearanceSource,
        #[case] expected: bool,
    ) {
        assert_eq!(may_subscribe(source, profile, owns_raw_input), expected);
    }

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

    #[test]
    fn kind_10_is_foreground_and_11_is_background() {
        assert_eq!(
            parse_osc_color_reply(b"\x1b]10;rgb:aaaa/bbbb/cccc\x07").map(|(kind, _)| kind),
            Some(OscColorKind::Foreground)
        );
        assert_eq!(
            parse_osc_color_reply(b"\x1b]11;rgb:aaaa/bbbb/cccc\x07").map(|(kind, _)| kind),
            Some(OscColorKind::Background)
        );
    }

    #[test]
    fn bel_and_st_terminators_are_both_accepted() {
        let want = xterm_color::Color::rgb(0x1111, 0x2222, 0x3333);
        assert_eq!(
            parse_osc_color_reply(b"\x1b]11;rgb:1111/2222/3333\x07").map(|(_, c)| c),
            Some(want.clone())
        );
        assert_eq!(
            parse_osc_color_reply(b"\x1b]11;rgb:1111/2222/3333\x1b\\").map(|(_, c)| c),
            Some(want)
        );
    }

    #[test]
    fn short_and_four_digit_rgb_payloads_parse() {
        let (kind, color) = parse_osc_color_reply(b"\x1b]11;rgb:f/e/d\x07").unwrap();
        assert_eq!(kind, OscColorKind::Background);
        assert_eq!(color, xterm_color::Color::rgb(0xffff, 0xeeee, 0xdddd));

        let (_, color) = parse_osc_color_reply(b"\x1b]10;rgb:1e1e/1e1e/2e2e\x07").unwrap();
        assert_eq!(color, xterm_color::Color::rgb(0x1e1e, 0x1e1e, 0x2e2e));
    }

    #[test]
    fn malformed_or_unterminated_payloads_have_no_verdict() {
        assert_eq!(parse_osc_color_reply(b"\x1b]11;not-a-color\x07"), None);
        assert_eq!(parse_osc_color_reply(b"\x1b]11;rgb:1111/2222\x07"), None);
        assert_eq!(parse_osc_color_reply(b"garbage, no OSC anywhere"), None);
        assert_eq!(parse_osc_color_reply(b"\x1b]11;rgb:1111/2222/3333"), None);
    }

    #[test]
    fn classify_prefers_the_darker_background_relation() {
        let black = xterm_color::Color::rgb(0, 0, 0);
        let white = xterm_color::Color::rgb(u16::MAX, u16::MAX, u16::MAX);

        // bg < fg: light text on a dark background.
        assert_eq!(classify_colors(Some(&white), &black), Appearance::Dark);
        // bg > fg: dark text on a light background.
        assert_eq!(classify_colors(Some(&black), &white), Appearance::Light);
        // bg == fg and bg > 0.5: the `bg > 0.5` arm.
        assert_eq!(classify_colors(Some(&white), &white), Appearance::Light);
        // bg == fg and bg <= 0.5: the final `else` arm.
        assert_eq!(classify_colors(Some(&black), &black), Appearance::Dark);
    }

    #[test]
    fn a_missing_foreground_falls_back_to_the_background_threshold() {
        let black = xterm_color::Color::rgb(0, 0, 0);
        let white = xterm_color::Color::rgb(u16::MAX, u16::MAX, u16::MAX);
        assert_eq!(classify_colors(None, &white), Appearance::Light);
        assert_eq!(classify_colors(None, &black), Appearance::Dark);
    }

    #[derive(Clone, Default)]
    struct SharedBuf(std::sync::Arc<std::sync::Mutex<Vec<u8>>>);

    impl SharedBuf {
        fn bytes(&self) -> Vec<u8> {
            self.0.lock().expect("lock poisoned").clone()
        }
    }

    impl std::io::Write for SharedBuf {
        fn write(&mut self, buf: &[u8]) -> std::io::Result<usize> {
            self.0.lock().expect("lock poisoned").extend_from_slice(buf);
            Ok(buf.len())
        }
        fn flush(&mut self) -> std::io::Result<()> {
            Ok(())
        }
    }

    #[test]
    fn subscribe_writes_the_dec_2031_set_sequence() {
        let out = SharedBuf::default();
        let _guard = ThemeNotifyGuard::subscribe(out.clone()).expect("subscribe writes cleanly");
        assert_eq!(out.bytes(), b"\x1b[?2031h".to_vec());
    }

    #[test]
    fn drop_writes_the_dec_2031_reset_sequence() {
        let out = SharedBuf::default();
        let guard = ThemeNotifyGuard::subscribe(out.clone()).expect("subscribe writes cleanly");
        drop(guard);
        assert_eq!(out.bytes(), b"\x1b[?2031h\x1b[?2031l".to_vec());
    }

    #[test]
    fn suspend_then_resume_writes_reset_then_set() {
        let out = SharedBuf::default();
        let mut guard = ThemeNotifyGuard::subscribe(out.clone()).expect("subscribe writes cleanly");
        guard.suspend().expect("suspend writes cleanly");
        guard.resume().expect("resume writes cleanly");
        assert_eq!(out.bytes(), b"\x1b[?2031h\x1b[?2031l\x1b[?2031h".to_vec());
    }

    #[test]
    fn a_second_suspend_while_already_suspended_writes_nothing_more() {
        let out = SharedBuf::default();
        let mut guard = ThemeNotifyGuard::subscribe(out.clone()).expect("subscribe writes cleanly");
        guard.suspend().expect("first suspend writes cleanly");
        guard
            .suspend()
            .expect("second suspend is a no-op, not an error");
        assert_eq!(out.bytes(), b"\x1b[?2031h\x1b[?2031l".to_vec());
    }

    #[test]
    fn a_resume_while_already_active_writes_nothing_more() {
        let out = SharedBuf::default();
        let mut guard = ThemeNotifyGuard::subscribe(out.clone()).expect("subscribe writes cleanly");
        guard
            .resume()
            .expect("resume on an already-active guard is a no-op");
        assert_eq!(out.bytes(), b"\x1b[?2031h".to_vec());
    }

    #[test]
    fn request_colors_writes_both_queries_in_one_shot() {
        let out = SharedBuf::default();
        let mut guard = ThemeNotifyGuard::subscribe(out.clone()).expect("subscribe writes cleanly");
        guard
            .request_colors()
            .expect("request_colors writes cleanly");
        assert_eq!(
            out.bytes(),
            b"\x1b[?2031h\x1b]10;?\x1b\\\x1b]11;?\x1b\\".to_vec()
        );
    }

    #[test]
    fn drop_never_panics_even_when_the_underlying_write_fails() {
        struct FailsAfterFirstWrite {
            calls: u32,
        }
        impl std::io::Write for FailsAfterFirstWrite {
            fn write(&mut self, buf: &[u8]) -> std::io::Result<usize> {
                self.calls += 1;
                if self.calls == 1 {
                    Ok(buf.len())
                } else {
                    Err(std::io::Error::other("terminal gone"))
                }
            }
            fn flush(&mut self) -> std::io::Result<()> {
                Ok(())
            }
        }

        let guard = ThemeNotifyGuard::subscribe(FailsAfterFirstWrite { calls: 0 })
            .expect("the first write (subscribe) succeeds");
        drop(guard); // must not panic even though the reset write errors
    }
}
