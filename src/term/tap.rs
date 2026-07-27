//! Reassembles decoded input events from raw terminal bytes for a
//! long-running command that owns its terminal's input. The scanner never
//! retains an unrecognized run indefinitely: a complete escape sequence it
//! does not understand is dropped, and an accumulating one is bounded.

use crate::term::theme_notify::{OscColorKind, parse_color_scheme_report, parse_osc_color_reply};
use crate::theme::Appearance;
use crate::ui::key::Key;

/// One decoded unit from the input stream.
#[derive(Clone, PartialEq, Debug)]
pub enum TapEvent {
    /// A decoded key, from `crate::ui::key`.
    Key(Key),
    /// A parsed DSR 997 push.
    ThemeNotification(Appearance),
    /// A parsed OSC 10/11 reply.
    OscColor(OscColorKind, xterm_color::Color),
}

/// Bytes accumulated for an escape sequence in progress may not grow past
/// this many without terminating; beyond it the run is discarded wholesale
/// rather than retained forever. One shared cap for both CSI and OSC runs.
const MAX_ESCAPE_LEN: usize = 128;

/// Reassembles `TapEvent`s from arbitrary-boundary byte chunks. A
/// complete, unrecognized escape-led run is dropped silently and never
/// retained — this is the property that keeps a long-lived reader from
/// wedging on an unknown private CSI.
pub struct TapScanner {
    buf: Vec<u8>,
}

impl TapScanner {
    pub fn new() -> TapScanner {
        TapScanner { buf: Vec::new() }
    }

    pub fn feed(&mut self, chunk: &[u8]) -> Vec<TapEvent> {
        let mut events = Vec::new();
        for &byte in chunk {
            if self.buf.is_empty() {
                if byte == 0x1b {
                    self.buf.push(byte);
                } else if let Some(key) = decode_key(byte) {
                    events.push(TapEvent::Key(key));
                }
                continue;
            }

            self.buf.push(byte);

            if self.buf.len() == 2 {
                if !matches!(self.buf[1], b'[' | b']') {
                    // Not a recognized introducer: the leading ESC was not
                    // the start of a sequence this scanner understands.
                    // Drop it and reprocess this byte as an ordinary one.
                    self.buf.clear();
                    if let Some(key) = decode_key(byte) {
                        events.push(TapEvent::Key(key));
                    }
                }
                continue;
            }

            if self.buf.len() > MAX_ESCAPE_LEN {
                self.buf.clear();
                continue;
            }

            if self.buf[1] == b'[' {
                // A CSI run is complete at an ECMA-48 final byte; only the
                // semantic interpretation is report-specific.
                if (0x40..=0x7e).contains(&byte) {
                    if let Some(appearance) = parse_color_scheme_report(&self.buf) {
                        events.push(TapEvent::ThemeNotification(appearance));
                    }
                    self.buf.clear();
                }
            } else {
                // The len == 2 branch above only lets `[` or `]` continue.
                debug_assert_eq!(self.buf[1], b']');
                if self.buf.ends_with(b"\x07") || self.buf.ends_with(b"\x1b\\") {
                    if let Some((kind, color)) = parse_osc_color_reply(&self.buf) {
                        events.push(TapEvent::OscColor(kind, color));
                    }
                    self.buf.clear();
                }
            }
        }
        events
    }
}

impl Default for TapScanner {
    fn default() -> Self {
        TapScanner::new()
    }
}

/// 0x03 → CtrlC; b'\r' | b'\n' → Enter; printable ASCII (0x20..=0x7e) →
/// Char; everything else → None. Watch acts only on CtrlC/q/v/Enter.
pub fn decode_key(byte: u8) -> Option<Key> {
    match byte {
        0x03 => Some(Key::CtrlC),
        b'\r' | b'\n' => Some(Key::Enter),
        0x20..=0x7e => Some(Key::Char(byte as char)),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_split_report_reassembles_across_feeds() {
        let mut scanner = TapScanner::new();
        assert_eq!(scanner.feed(b"\x1b[?997"), vec![]);
        assert_eq!(
            scanner.feed(b";2n"),
            vec![TapEvent::ThemeNotification(Appearance::Light)]
        );
    }

    #[test]
    fn a_report_sandwiched_between_keys_yields_all_three_in_order() {
        let mut scanner = TapScanner::new();
        let events = scanner.feed(b"a\x1b[?997;2nb");
        assert_eq!(
            events,
            vec![
                TapEvent::Key(Key::Char('a')),
                TapEvent::ThemeNotification(Appearance::Light),
                TapEvent::Key(Key::Char('b')),
            ]
        );
    }

    #[test]
    fn an_unrecognized_private_csi_is_dropped_without_wedging() {
        let mut scanner = TapScanner::new();
        assert_eq!(scanner.feed(b"\x1b[?123;4x"), vec![]);
        // The buffer must not have retained anything from the discarded run.
        assert_eq!(scanner.feed(b"z"), vec![TapEvent::Key(Key::Char('z'))]);
    }

    #[test]
    fn an_unfinished_sequence_past_the_cap_is_discarded_wholesale() {
        let mut scanner = TapScanner::new();
        // A long run that never reaches a CSI final byte. Filler is 0x00,
        // not a digit or `;`, so it can never be mistaken for a report and
        // decodes to no key either way the byte ends up being processed.
        let mut long_run = b"\x1b[".to_vec();
        long_run.resize(long_run.len() + 200, 0u8);
        assert_eq!(scanner.feed(&long_run), vec![]);
        assert_eq!(scanner.feed(b"z"), vec![TapEvent::Key(Key::Char('z'))]);
    }

    #[test]
    fn a_complete_arrow_key_is_dropped_silently() {
        // Watch parity: arrow keys do nothing today either, so the scanner
        // must not surface one as stray characters.
        let mut scanner = TapScanner::new();
        assert_eq!(scanner.feed(b"\x1b[A"), vec![]);
    }

    #[test]
    fn an_osc_color_reply_reassembles_across_feeds() {
        let mut scanner = TapScanner::new();
        assert_eq!(scanner.feed(b"\x1b]11;rgb:1e1e/1e1e/"), vec![]);
        assert_eq!(
            scanner.feed(b"2e2e\x07"),
            vec![TapEvent::OscColor(
                OscColorKind::Background,
                xterm_color::Color::rgb(0x1e1e, 0x1e1e, 0x2e2e)
            )]
        );
    }

    #[test]
    fn a_lone_escape_with_no_recognized_introducer_does_not_eat_the_next_byte() {
        let mut scanner = TapScanner::new();
        assert_eq!(scanner.feed(b"\x1b"), vec![]);
        assert_eq!(scanner.feed(b"q"), vec![TapEvent::Key(Key::Char('q'))]);
    }

    #[test]
    fn decode_key_maps_the_five_recognized_bytes() {
        assert_eq!(decode_key(0x03), Some(Key::CtrlC));
        assert_eq!(decode_key(b'\r'), Some(Key::Enter));
        assert_eq!(decode_key(b'\n'), Some(Key::Enter));
        assert_eq!(decode_key(b'q'), Some(Key::Char('q')));
        assert_eq!(decode_key(b'v'), Some(Key::Char('v')));
    }

    #[test]
    fn decode_key_has_no_verdict_for_escape_or_delete() {
        assert_eq!(decode_key(0x1b), None);
        assert_eq!(decode_key(0x7f), None);
    }
}
