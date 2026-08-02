#![cfg(unix)]
mod common;

use std::time::Duration;

use common::pty::{FakeTerminal, PtySession, wait_for};

/// Path to the rat binary — mirrors `tests/pty_watch.rs`'s local
/// `rat_bin()` per that file's precedent.
fn rat_bin() -> String {
    assert_cmd::cargo::cargo_bin("rat").display().to_string()
}

fn contains(haystack: &[u8], needle: &[u8]) -> bool {
    needle.len() <= haystack.len() && haystack.windows(needle.len()).any(|w| w == needle)
}

fn find(haystack: &[u8], needle: &[u8]) -> Option<usize> {
    if needle.len() > haystack.len() {
        return None;
    }
    haystack.windows(needle.len()).position(|w| w == needle)
}

/// Accumulate everything the session writes within `total`.
fn drain_for(session: &PtySession, total: Duration) -> Vec<u8> {
    let deadline = std::time::Instant::now() + total;
    let mut out = Vec::new();
    loop {
        let now = std::time::Instant::now();
        if now >= deadline {
            return out;
        }
        out.extend(session.read_available(deadline - now));
    }
}

/// The interactive caret is the real hardware cursor: after each paint
/// the cursor climbs to the edit point and is shown — the only cursor
/// screen readers and braille displays can track.
#[test]
fn the_hardware_cursor_rests_on_the_caret() {
    let session = PtySession::spawn(
        &rat_bin(),
        &["input", "--prompt", "> ", "--value", "abc"],
        // A pinned appearance suppresses the startup OSC probe, so every
        // escape in the stream is the renderer's own.
        &[("RAT_APPEARANCE", "dark")],
    )
    .expect("spawn rat input under a pty");
    let mut terminal = FakeTerminal::dark();

    // First frame: value "abc", caret one past the end. The park bytes
    // climb one row and step to column 5 (prompt is 2 wide, cursor char
    // index 3), then show the cursor.
    assert!(
        wait_for(
            &session,
            &mut terminal,
            b"\x1b[1A\r\x1b[5C\x1b[?25h",
            Duration::from_secs(5),
        ),
        "the first frame must park a visible cursor on the caret cell"
    );

    // Left arrow moves the caret onto 'c'. With no painted caret the
    // frame bytes are identical, so the renderer takes the bare-hop
    // path: one relative move of the already-visible cursor, no
    // repaint, no re-show.
    session.write_bytes(b"\x1b[D");
    assert!(
        wait_for(
            &session,
            &mut terminal,
            b"\r\x1b[4C",
            Duration::from_secs(5),
        ),
        "moving the caret must hop the visible cursor onto it"
    );

    // Enter submits: the UI erases itself and the value reaches stdout.
    session.write_bytes(b"\r");
    let tail = drain_for(&session, Duration::from_secs(5));
    assert!(
        contains(&tail, b"abc"),
        "the submitted value must reach stdout: {:?}",
        String::from_utf8_lossy(&tail)
    );
    assert!(
        !session.kill_if_alive(Duration::from_secs(5)),
        "rat input must exit on its own after Enter"
    );
}

/// A wide prompt glyph must not push the value one cell right. ratatui
/// pads a wide glyph with a reset continuation cell, and emitting that
/// space shifted the whole line while the caret — measured in cells —
/// stayed put, landing it on the last character instead of after it.
#[test]
fn a_wide_prompt_glyph_does_not_shift_the_value() {
    let session = PtySession::spawn(
        &rat_bin(),
        &["input", "--prompt", "日", "--value", "abc"],
        &[("RAT_APPEARANCE", "dark")],
    )
    .expect("spawn rat input under a pty");
    let mut terminal = FakeTerminal::dark();

    // The value follows the prompt's reset with no padding between.
    let adjacent = "日\x1b[0mabc".as_bytes();
    let seen = wait_for_bytes(&session, &mut terminal, adjacent, Duration::from_secs(5))
        .expect("the value must sit immediately after the wide prompt");
    // 日 spans columns 0-1 and abc columns 2-4, so the caret belongs at 5.
    assert!(
        contains(&seen, b"\x1b[5C"),
        "the caret must park after the value, not on its last cell: {:?}",
        String::from_utf8_lossy(&seen)
    );

    session.write_bytes(b"\r");
    let _ = drain_for(&session, Duration::from_millis(300));
    session.kill_if_alive(Duration::from_secs(5));
}

/// A value that reaches the right edge scrolls the field instead of
/// pinning the caret on the last character — the reported bug: once the
/// text filled the row the caret stuck to the final cell forever.
#[test]
fn a_full_field_scrolls_and_keeps_the_caret_past_the_text() {
    let session = PtySession::spawn(
        &rat_bin(),
        &["input", "--prompt", "> "],
        &[("RAT_APPEARANCE", "dark")],
    )
    .expect("spawn rat input under a pty");
    let mut terminal = FakeTerminal::dark();
    // A 20-column terminal leaves an 18-cell field after the prompt.
    session.set_winsize(10, 20);

    assert!(
        wait_for(&session, &mut terminal, b"\x1b[2C", Duration::from_secs(5)),
        "the empty field parks the caret at the prompt edge"
    );
    // Type 18 characters: one more than the field can show alongside the
    // caret, so the window must slide by one.
    session.write_bytes(b"abcdefghijklmnopqr");

    // The discriminator is the painted line, not the caret column alone:
    // an unscrolled field passes through column 19 on its way to the
    // edge, but its text always still starts at 'a'. A frame whose text
    // begins at 'b' exists only once the window has slid.
    let scrolled = b"\x1b[0mbcdefghijklmnopqr";
    let seen = wait_for_bytes(&session, &mut terminal, scrolled, Duration::from_secs(5))
        .expect("the field must scroll, dropping the leading 'a'");
    let after_frame = &seen[find(&seen, scrolled).expect("just matched") + scrolled.len()..];
    assert!(
        contains(after_frame, b"\x1b[19C"),
        "the caret must park one past the scrolled text, not on it: {:?}",
        String::from_utf8_lossy(after_frame)
    );

    session.write_bytes(b"\r");
    let _ = drain_for(&session, Duration::from_millis(300));
    session.kill_if_alive(Duration::from_secs(5));
}

/// `wait_for`, returning the accumulated bytes once `needle` appears —
/// for asserting on what surrounds the needle.
fn wait_for_bytes(
    session: &PtySession,
    terminal: &mut FakeTerminal,
    needle: &[u8],
    timeout: Duration,
) -> Option<Vec<u8>> {
    let deadline = std::time::Instant::now() + timeout;
    let mut seen: Vec<u8> = Vec::new();
    loop {
        let now = std::time::Instant::now();
        if now >= deadline {
            return None;
        }
        let chunk = session.read_available((deadline - now).min(Duration::from_millis(50)));
        if chunk.is_empty() {
            continue;
        }
        terminal.respond(session, &chunk);
        seen.extend_from_slice(&chunk);
        if contains(&seen, needle) {
            return Some(seen);
        }
    }
}

/// The frame paints no caret cell: the parked hardware cursor is the one
/// and only caret, so no reverse video reaches the terminal.
#[test]
fn the_input_frame_carries_no_reverse_video() {
    let session = PtySession::spawn(
        &rat_bin(),
        &["input", "--prompt", "> ", "--value", "abc"],
        &[("RAT_APPEARANCE", "dark")],
    )
    .expect("spawn rat input under a pty");
    let mut terminal = FakeTerminal::dark();

    // The park needle proves a full frame painted before we judge it.
    let seen = wait_for_bytes(
        &session,
        &mut terminal,
        b"\x1b[1A\r\x1b[5C\x1b[?25h",
        Duration::from_secs(5),
    )
    .expect("the first frame parks the cursor");
    assert!(
        !contains(&seen, b"\x1b[7m"),
        "the frame must not paint a caret cell: {:?}",
        String::from_utf8_lossy(&seen)
    );

    session.write_bytes(b"\r");
    let _ = drain_for(&session, Duration::from_millis(300));
    session.kill_if_alive(Duration::from_secs(5));
}
