//! Child output bytes → frame lines: the one display-side decode.
//!
//! Display-side on purpose: `TickOutcome` and `Emission` payloads stay
//! bytes, the pass-through write path stays byte-exact, and the repaint
//! gate's signatures hash bytes upstream of this module — decoding here
//! changes what a frame SHOWS, never what rat captured.

const TAB_STOP: usize = 8;

/// A captured stream as frame lines: trailing newlines stripped,
/// interior blanks kept, each line decoded independently.
///
/// The order is load-bearing: stripping trailing `b'\n'` FIRST and then
/// splitting reproduces the historical
/// `from_utf8_lossy` + `trim_end_matches('\n')` + `split('\n')`
/// pipeline exactly — including the rule that an empty stream renders
/// as ONE empty line. Splitting at the byte level is safe: `0x0A` is
/// never a UTF-8 continuation byte.
#[cfg(unix)]
pub fn stream_lines(bytes: &[u8]) -> Vec<String> {
    pieces(bytes)
        .map(decode_line)
        .map(|line| crate::core::measure::expand_tabs(&line, TAB_STOP))
        .collect()
}

/// On Windows a console child writes its pipe in the CONSOLE codepage
/// (OEM 437, 850, …), not UTF-8 — so a line that is not valid UTF-8
/// decodes with the active console codepage instead of collapsing to
/// replacement characters. UTF-8 comes first and wins outright: a
/// modern child's UTF-8 output is never re-decoded. A child that emits
/// legacy bytes under a UTF-8 (`chcp 65001`) console still garbles,
/// and should — that is what the console itself would show; the
/// contract is "match the console", never "guess the encoding".
#[cfg(windows)]
pub fn stream_lines(bytes: &[u8]) -> Vec<String> {
    // Resolved once per stream, not per line: the codepage is process
    // state, and N syscalls per frame would buy nothing.
    stream_lines_at(bytes, active_codepage())
}

/// The shared strip/split: trailing `b'\n'` stripped first, then split
/// — which is what reproduces the historical trim-then-split pipeline,
/// including the empty-stream-is-one-line rule.
fn pieces(bytes: &[u8]) -> impl Iterator<Item = &[u8]> {
    let mut end = bytes.len();
    while end > 0 && bytes[end - 1] == b'\n' {
        end -= 1;
    }
    bytes[..end].split(|&b| b == b'\n')
}

#[cfg(unix)]
fn decode_line(line: &[u8]) -> String {
    String::from_utf8_lossy(line).into_owned()
}

#[cfg(windows)]
fn stream_lines_at(bytes: &[u8], codepage: u32) -> Vec<String> {
    pieces(bytes)
        .map(|line| decode_line_at(line, codepage))
        .map(|line| crate::core::measure::expand_tabs(&line, TAB_STOP))
        .collect()
}

#[cfg(windows)]
fn decode_line_at(line: &[u8], codepage: u32) -> String {
    match std::str::from_utf8(line) {
        Ok(text) => text.to_owned(),
        Err(_) => decode_with_codepage(line, codepage),
    }
}

/// The console's output codepage, or the system OEM codepage when no
/// console is attached (zero is the no-console answer; the OEM default
/// is what a console would have opened with).
#[cfg(windows)]
fn active_codepage() -> u32 {
    use windows_sys::Win32::Globalization::GetOEMCP;
    use windows_sys::Win32::System::Console::GetConsoleOutputCP;
    match unsafe { GetConsoleOutputCP() } {
        0 => unsafe { GetOEMCP() },
        cp => cp,
    }
}

/// `MultiByteToWideChar` without `MB_ERR_INVALID_CHARS`: an
/// undecodable byte becomes the default character rather than an
/// error — the lossy tier, same spirit as `from_utf8_lossy`. A refused
/// codepage falls back to replacement characters; garble beats a
/// panic in a frame body.
#[cfg(windows)]
fn decode_with_codepage(bytes: &[u8], codepage: u32) -> String {
    use windows_sys::Win32::Globalization::MultiByteToWideChar;
    if bytes.is_empty() {
        return String::new();
    }
    // SAFETY: the pointer/length pair names `bytes` exactly; a null
    // output buffer with zero length is the documented sizing call.
    let needed = unsafe {
        MultiByteToWideChar(
            codepage,
            0,
            bytes.as_ptr(),
            bytes.len() as i32,
            std::ptr::null_mut(),
            0,
        )
    };
    if needed <= 0 {
        return String::from_utf8_lossy(bytes).into_owned();
    }
    let mut wide = vec![0u16; needed as usize];
    // SAFETY: the output buffer was sized by the call above and its
    // length is passed alongside; the input pair is unchanged.
    let written = unsafe {
        MultiByteToWideChar(
            codepage,
            0,
            bytes.as_ptr(),
            bytes.len() as i32,
            wide.as_mut_ptr(),
            needed,
        )
    };
    if written <= 0 {
        return String::from_utf8_lossy(bytes).into_owned();
    }
    String::from_utf16_lossy(&wide[..written as usize])
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn an_empty_stream_is_one_empty_line() {
        // The shipped rule (watch.rs pins it at the seam level): EMPTY
        // renders as ONE empty line, because split of an empty
        // remainder yields one empty piece.
        assert_eq!(stream_lines(b""), vec![String::new()]);
    }

    #[test]
    fn trailing_newlines_are_stripped_and_interior_blanks_survive() {
        assert_eq!(
            stream_lines(b"a\n\nb\n\n\n"),
            vec!["a".to_string(), String::new(), "b".to_string()]
        );
    }

    #[cfg(unix)]
    #[test]
    fn invalid_utf8_on_unix_still_becomes_replacement_chars() {
        // Unchanged unix contract — lossy per line.
        assert_eq!(
            stream_lines(b"gr\x81\xE1e"),
            vec!["gr\u{FFFD}\u{FFFD}e".to_string()]
        );
    }

    #[test]
    fn utf8_input_round_trips_exactly() {
        assert_eq!(
            stream_lines("grüße\n".as_bytes()),
            vec!["grüße".to_string()]
        );
    }

    #[test]
    fn tabs_become_geometry_safe_spaces() {
        assert_eq!(
            stream_lines(b"completed\tsuccess\ttest\n"),
            vec!["completed       success test".to_string()]
        );
    }

    #[cfg(windows)]
    #[test]
    fn cp437_bytes_decode_through_the_codepage() {
        // The measured fixture bytes: grüße in CP437. The codepage is
        // passed EXPLICITLY so this is deterministic on any Windows box
        // regardless of its console or locale.
        assert_eq!(
            stream_lines_at(b"gr\x81\xE1e\n", 437),
            vec!["grüße".to_string()]
        );
    }

    #[cfg(windows)]
    #[test]
    fn valid_utf8_is_never_re_decoded() {
        // UTF-8 FIRST: a modern child's UTF-8 must pass through even
        // when the console CP would read those bytes differently.
        // grüße as UTF-8 re-decoded via 437 would come out as
        // box-drawing mojibake.
        assert_eq!(
            stream_lines_at("grüße\n".as_bytes(), 437),
            vec!["grüße".to_string()]
        );
    }

    #[cfg(windows)]
    #[test]
    fn decode_granularity_is_the_line() {
        // A mixed stream — one UTF-8 tool and one legacy tool
        // interleaved — must not let either line poison the other.
        let mut bytes = Vec::from("utf8-grüße\n".as_bytes());
        bytes.extend_from_slice(b"oem-gr\x81\xE1e\n");
        assert_eq!(
            stream_lines_at(&bytes, 437),
            vec!["utf8-grüße".to_string(), "oem-grüße".to_string()]
        );
    }
}
