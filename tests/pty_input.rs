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
