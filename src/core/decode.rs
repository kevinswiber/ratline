//! Child output bytes → frame lines: the one display-side decode.
//!
//! Display-side on purpose: `TickOutcome` and `Emission` payloads stay
//! bytes, the pass-through write path stays byte-exact, and the repaint
//! gate's signatures hash bytes upstream of this module — decoding here
//! changes what a frame SHOWS, never what rat captured.

/// A captured stream as frame lines: trailing newlines stripped,
/// interior blanks kept, each line decoded independently.
///
/// The order is load-bearing: stripping trailing `b'\n'` FIRST and then
/// splitting reproduces the historical
/// `from_utf8_lossy` + `trim_end_matches('\n')` + `split('\n')`
/// pipeline exactly — including the rule that an empty stream renders
/// as ONE empty line. Splitting at the byte level is safe: `0x0A` is
/// never a UTF-8 continuation byte.
pub fn stream_lines(bytes: &[u8]) -> Vec<String> {
    let mut end = bytes.len();
    while end > 0 && bytes[end - 1] == b'\n' {
        end -= 1;
    }
    bytes[..end]
        .split(|&b| b == b'\n')
        .map(decode_line)
        .collect()
}

fn decode_line(line: &[u8]) -> String {
    String::from_utf8_lossy(line).into_owned()
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
}
