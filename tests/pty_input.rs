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

    // Left arrow moves the caret onto 'c': the repaint re-parks at
    // column 4 and the cursor is shown again at the new cell.
    session.write_bytes(b"\x1b[D");
    assert!(
        wait_for(
            &session,
            &mut terminal,
            b"\r\x1b[4C\x1b[?25h",
            Duration::from_secs(5),
        ),
        "moving the caret must re-park the visible cursor on it"
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

/// The reverse-video caret rides the same frame: one past the end of the
/// value it is a reversed space, which must reach a real terminal.
#[test]
fn the_reverse_video_caret_reaches_a_real_terminal() {
    let session = PtySession::spawn(
        &rat_bin(),
        &["input", "--prompt", "> ", "--value", "abc"],
        &[("RAT_APPEARANCE", "dark")],
    )
    .expect("spawn rat input under a pty");
    let mut terminal = FakeTerminal::dark();

    assert!(
        wait_for(
            &session,
            &mut terminal,
            b"\x1b[7m \x1b[0m",
            Duration::from_secs(5),
        ),
        "the end-of-line caret must arrive as a reversed space"
    );

    session.write_bytes(b"\r");
    let _ = drain_for(&session, Duration::from_millis(300));
    session.kill_if_alive(Duration::from_secs(5));
}
