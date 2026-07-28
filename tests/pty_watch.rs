#![cfg(unix)]
mod common;

use std::time::Duration;

use common::pty::{FakeTerminal, PtySession, wait_for};

/// Path to the rat binary, used as a portable child process — mirrors
/// `tests/cli_watch.rs`'s own local `rat_bin()` rather than adding one to
/// `tests/common`, matching that file's existing precedent.
fn rat_bin() -> String {
    assert_cmd::cargo::cargo_bin("rat").display().to_string()
}

fn contains(haystack: &[u8], needle: &[u8]) -> bool {
    needle.len() <= haystack.len() && haystack.windows(needle.len()).any(|w| w == needle)
}

/// Accumulate everything the session writes within `total` — unlike
/// `read_available`, which returns at the first chunk.
fn drain_for(session: &PtySession, total: std::time::Duration) -> Vec<u8> {
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

/// `wait_for`, returning the accumulated bytes once `needle` appears —
/// for assertions that need to inspect text near the needle.
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

/// `wait_for`, but panicking the moment `forbidden` shows up in the
/// accumulated output.
fn wait_for_without(
    session: &PtySession,
    terminal: &mut FakeTerminal,
    needle: &[u8],
    forbidden: &[u8],
    timeout: Duration,
) -> bool {
    let deadline = std::time::Instant::now() + timeout;
    let mut seen: Vec<u8> = Vec::new();
    loop {
        let now = std::time::Instant::now();
        if now >= deadline {
            return false;
        }
        let chunk = session.read_available((deadline - now).min(Duration::from_millis(50)));
        if chunk.is_empty() {
            continue;
        }
        terminal.respond(session, &chunk);
        seen.extend_from_slice(&chunk);
        assert!(
            !contains(&seen, forbidden),
            "forbidden needle {:?} appeared",
            String::from_utf8_lossy(forbidden)
        );
        if contains(&seen, needle) {
            return true;
        }
    }
}

#[test]
fn a_watch_session_under_a_pty_prints_child_output_and_quits_on_q() {
    let session = PtySession::spawn(
        &rat_bin(),
        &["watch", "-n", "50ms", "--", &rat_bin(), "style", "hi"],
        &[],
    )
    .expect("spawn rat watch under a pty");

    // Act as the terminal: answer the startup OSC 10/11 + DA1 probe so
    // the pre-dispatch resolution completes instead of blocking for its
    // full timeout. The color and polarity do not matter here — this test
    // proves the harness, not appearance classification.
    let mut terminal = FakeTerminal::dark();
    assert!(
        wait_for(&session, &mut terminal, b"hi", Duration::from_secs(2)),
        "expected a frame containing the child's output"
    );

    session.write_bytes(b"q");
    assert!(
        !session.kill_if_alive(Duration::from_secs(2)),
        "watch should have exited cleanly on q"
    );
}

#[test]
fn v_pages_the_frame_and_the_watch_keeps_running() {
    // A pager that needs no input and exits immediately, so the test does
    // not have to drive `less`. Absolute path: the child's environment is
    // built from scratch and carries no PATH.
    let session = PtySession::spawn(
        &rat_bin(),
        &["watch", "-n", "50ms", "--", &rat_bin(), "style", "hi"],
        &[("RAT_PAGER", "/bin/cat")],
    )
    .expect("spawn under a pty");
    let mut terminal = FakeTerminal::dark();

    assert!(wait_for(
        &session,
        &mut terminal,
        b"hi",
        Duration::from_secs(2)
    ));
    session.write_bytes(b"v");
    // Leaving the pager forces a repaint, so a fresh frame is proof the
    // loop resumed — and it still answers keys afterwards.
    assert!(wait_for(
        &session,
        &mut terminal,
        b"hi",
        Duration::from_secs(2)
    ));
    session.write_bytes(b"q");
    assert!(
        !session.kill_if_alive(Duration::from_secs(2)),
        "watch should have exited on q after paging"
    );
}

#[test]
fn ctrl_c_aborts_a_watch_running_under_a_terminal() {
    // Raw mode delivers 0x03 as a byte, not as SIGINT.
    let session = PtySession::spawn(
        &rat_bin(),
        &["watch", "-n", "50ms", "--", &rat_bin(), "style", "hi"],
        &[],
    )
    .expect("spawn under a pty");
    let mut terminal = FakeTerminal::dark();

    assert!(wait_for(
        &session,
        &mut terminal,
        b"hi",
        Duration::from_secs(2)
    ));
    session.write_bytes(b"\x03");
    assert!(
        !session.kill_if_alive(Duration::from_secs(2)),
        "watch should have aborted on ctrl-c"
    );
}

#[test]
fn an_unrecognized_private_csi_does_not_stop_watch_from_quitting() {
    let session = PtySession::spawn(
        &rat_bin(),
        &["watch", "-n", "50ms", "--", &rat_bin(), "style", "hi"],
        &[],
    )
    .expect("spawn under a pty");
    let mut terminal = FakeTerminal::dark();

    assert!(wait_for(
        &session,
        &mut terminal,
        b"hi",
        Duration::from_secs(2)
    ));
    // A private CSI rat has no meaning for, deliberately unrelated to
    // themes: whoever owns the input must drop it and keep reading.
    session.write_bytes(b"\x1b[?123;4x");
    session.write_bytes(b"q");

    assert!(
        !session.kill_if_alive(Duration::from_secs(2)),
        "watch stopped responding to keys after an unrecognized escape"
    );
}

#[test]
fn an_interactive_watch_subscribes_and_unsubscribes() {
    let session = PtySession::spawn(
        &rat_bin(),
        &["watch", "-n", "50ms", "--", &rat_bin(), "style", "hi"],
        &[],
    )
    .expect("spawn under a pty");
    let mut terminal = FakeTerminal::dark();

    assert!(
        wait_for(
            &session,
            &mut terminal,
            b"\x1b[?2031h",
            Duration::from_secs(2)
        ),
        "expected watch to subscribe to theme notifications"
    );

    session.write_bytes(b"q");
    assert!(
        wait_for(
            &session,
            &mut terminal,
            b"\x1b[?2031l",
            Duration::from_secs(2)
        ),
        "expected watch to unsubscribe before exiting"
    );
    assert!(
        !session.kill_if_alive(Duration::from_secs(2)),
        "watch should have exited on q"
    );
}

#[test]
fn a_terminal_theme_flip_repaints_children_in_the_new_palette() {
    let session = PtySession::spawn(
        &rat_bin(),
        &[
            "watch",
            "-n",
            "50ms",
            "--",
            // The child's stdout is captured through a pipe, so it needs an
            // explicit color decision to emit SGR at all.
            &rat_bin(),
            "--color",
            "always",
            "style",
            "--foreground",
            "accent",
            "text",
        ],
        &[],
    )
    .expect("spawn under a pty");
    let mut terminal = FakeTerminal::dark();

    assert!(
        wait_for(
            &session,
            &mut terminal,
            b"\x1b[38;5;212m",
            Duration::from_secs(2)
        ),
        "expected the dark accent before any flip"
    );

    // The terminal's colors change, and then it announces the change. The
    // announcement is realistic: one stale report followed by two corrected
    // ones, which is what a real flip produces.
    terminal.set("rgb:0000/0000/0000", "rgb:ffff/ffff/ffff");
    session.write_bytes(b"\x1b[?997;1n\x1b[?997;2n\x1b[?997;2n");

    assert!(
        wait_for(
            &session,
            &mut terminal,
            b"\x1b[38;5;129m",
            Duration::from_secs(3)
        ),
        "expected the light accent after the terminal announced a change"
    );

    session.write_bytes(b"q");
    assert!(
        !session.kill_if_alive(Duration::from_secs(2)),
        "watch should have exited on q"
    );
}

#[test]
fn a_navigation_key_freezes_the_frame_and_stops_the_tail() {
    // A child whose output changes every tick: any repaint after the
    // freeze would be new bytes on the pty.
    let session = PtySession::spawn(
        &rat_bin(),
        &[
            "watch",
            "-n",
            "50ms",
            "--",
            &rat_bin(),
            "date",
            "--format",
            "%H%M%S%f",
        ],
        &[],
    )
    .expect("spawn under a pty");
    let mut terminal = FakeTerminal::dark();

    // A first frame has painted (the synchronized-output opener).
    assert!(
        wait_for(
            &session,
            &mut terminal,
            b"\x1b[?2026h",
            Duration::from_secs(2)
        ),
        "expected a first frame"
    );

    session.write_bytes(b"j");
    assert!(
        wait_for(
            &session,
            &mut terminal,
            b"paused \xc2\xb7",
            Duration::from_secs(2)
        ),
        "expected the paused row after a navigation key"
    );
    // Drain the paint that carried the paused row, then require silence:
    // eight ticks of changing child output and not one byte painted.
    let _ = session.read_available(Duration::from_millis(150));
    let leaked = session.read_available(Duration::from_millis(400));
    assert!(
        leaked.is_empty(),
        "the tail kept painting while frozen: {:?}",
        String::from_utf8_lossy(&leaked)
    );

    // Esc resumes: a fresh repaint arrives (the opener again).
    session.write_bytes(b"\x1b");
    assert!(
        wait_for(
            &session,
            &mut terminal,
            b"\x1b[?2026h",
            Duration::from_secs(2)
        ),
        "expected the live tail to repaint after Esc"
    );
    session.write_bytes(b"q");
    assert!(
        !session.kill_if_alive(Duration::from_secs(2)),
        "watch should have exited on q after resuming"
    );
}

#[test]
fn every_live_frame_names_its_last_change() {
    // A static child: after the first frame nothing changes, so the
    // absolute stamp must hold the repaint gate shut.
    let session = PtySession::spawn(
        &rat_bin(),
        &["watch", "-n", "50ms", "--", &rat_bin(), "style", "hi"],
        &[],
    )
    .expect("spawn under a pty");
    let mut terminal = FakeTerminal::dark();

    let mut seen = wait_for_bytes(&session, &mut terminal, b"since ", Duration::from_secs(2))
        .expect("expected the live since row");
    // The stamp may straddle a chunk boundary; give it a moment to land.
    seen.extend(drain_for(&session, Duration::from_millis(150)));
    let at = seen
        .windows(6)
        .position(|w| w == b"since ")
        .expect("just matched");
    let stamp = &seen[at + 6..at + 14];
    assert!(
        stamp.iter().enumerate().all(|(i, b)| match i {
            2 | 5 => *b == b':',
            _ => b.is_ascii_digit(),
        }),
        "expected an HH:MM:SS stamp, got {:?}",
        String::from_utf8_lossy(stamp)
    );

    // Static content and an absolute stamp: not one further byte painted.
    let leaked = session.read_available(Duration::from_millis(400));
    assert!(
        leaked.is_empty(),
        "the stamp must not defeat the repaint gate: {:?}",
        String::from_utf8_lossy(&leaked)
    );

    session.write_bytes(b"q");
    assert!(
        !session.kill_if_alive(Duration::from_secs(2)),
        "watch should have exited on q"
    );
}

#[test]
fn a_frozen_frame_scrolls_and_esc_restores_the_live_view() {
    // 30 static lines on a 24-row pty: a 22-row window, 8 hidden.
    let lines: Vec<String> = (1..=30).map(|i| format!("l{i}")).collect();
    let rat = rat_bin();
    let mut args: Vec<&str> = vec!["watch", "-n", "50ms", "--", &rat, "style"];
    args.extend(lines.iter().map(String::as_str));
    let session = PtySession::spawn(&rat_bin(), &args, &[]).expect("spawn under a pty");
    let mut terminal = FakeTerminal::dark();

    // The truncation row also carries the since stamp between these two
    // segments; both arrive in the same paint.
    let seen = wait_for_bytes(
        &session,
        &mut terminal,
        "v views all · q quits".as_bytes(),
        Duration::from_secs(2),
    )
    .expect("expected the live truncation notice");
    assert!(
        contains(&seen, "… 8 more lines".as_bytes()),
        "expected the hidden-line count in the truncation row"
    );

    session.write_bytes(b"j");
    assert!(
        wait_for(
            &session,
            &mut terminal,
            "paused · lines 2-23 of 30 · Esc resumes".as_bytes(),
            Duration::from_secs(2)
        ),
        "expected the paused row one line down"
    );

    session.write_bytes(b"G");
    assert!(
        wait_for(
            &session,
            &mut terminal,
            b"lines 9-30 of 30",
            Duration::from_secs(2)
        ),
        "expected the bottom of the frame after G"
    );

    session.write_bytes(b"\x1b");
    let seen = wait_for_bytes(
        &session,
        &mut terminal,
        "v views all · q quits".as_bytes(),
        Duration::from_secs(2),
    )
    .expect("expected the live truncation notice back after Esc");
    assert!(
        contains(&seen, "… 8 more lines".as_bytes()),
        "expected the hidden-line count back after Esc"
    );
    session.write_bytes(b"q");
    assert!(
        !session.kill_if_alive(Duration::from_secs(2)),
        "watch should have exited on q"
    );
}

#[test]
fn w_and_l_change_the_view_without_pausing() {
    // One line deterministically wider than the 80-column pty: the marker
    // starts at display column 101, provably beyond the screen edge when
    // chopped, and beyond one horizontal step when shifted.
    let long_line = "x".repeat(100) + "TAILMARK";
    assert!(
        long_line.len() > 80,
        "premise: the line must overflow the pty"
    );
    let rat = rat_bin();
    let session = PtySession::spawn(
        &rat,
        &["watch", "-n", "50ms", "--", &rat, "style", &long_line],
        &[],
    )
    .expect("spawn under a pty");
    let mut terminal = FakeTerminal::dark();
    let paused_needle = "paused ·".as_bytes();

    // Wrapped by default: the marker appears on a wrapped row.
    assert!(
        wait_for(&session, &mut terminal, b"TAILMARK", Duration::from_secs(2)),
        "expected the wrapped tail of the long line"
    );
    // Flush the rest of the initial frame before watching for the chop.
    let _ = drain_for(&session, Duration::from_millis(200));

    // `w` chops: a repaint arrives without the marker and without a pause.
    session.write_bytes(b"w");
    let chopped = drain_for(&session, Duration::from_millis(500));
    assert!(
        contains(&chopped, b"\x1b[?2026h"),
        "expected a repaint after w"
    );
    assert!(
        !contains(&chopped, b"TAILMARK"),
        "the marker should be chopped off screen"
    );
    assert!(
        !contains(&chopped, paused_needle),
        "a view key must never freeze the frame"
    );

    // Four steps right (8 columns each) bring the marker into view.
    session.write_bytes(b"llll");
    assert!(
        wait_for_without(
            &session,
            &mut terminal,
            b"TAILMARK",
            paused_needle,
            Duration::from_secs(2)
        ),
        "expected the marker after shifting right"
    );

    // Back left, and `w` restores the wrapped view.
    session.write_bytes(b"hhhh");
    let _ = drain_for(&session, Duration::from_millis(300));
    session.write_bytes(b"w");
    assert!(
        wait_for_without(
            &session,
            &mut terminal,
            b"TAILMARK",
            paused_needle,
            Duration::from_secs(2)
        ),
        "expected the wrapped tail back after w"
    );

    session.write_bytes(b"q");
    assert!(
        !session.kill_if_alive(Duration::from_secs(2)),
        "watch should have exited on q"
    );
}

#[test]
fn s_writes_a_snapshot_of_the_frame() {
    // The pty harness builds the child's environment from scratch, so the
    // snapshot directory must be explicit and absolute.
    let dir = tempfile::tempdir().expect("tempdir");
    let dir_path = dir.path().display().to_string();
    let rat = rat_bin();
    // A child with real SGR in its frame, so "stripped by default" is
    // actually exercised.
    let session = PtySession::spawn(
        &rat,
        &[
            "watch",
            "-n",
            "50ms",
            "--",
            &rat,
            "--color",
            "always",
            "style",
            "--foreground",
            "212",
            "hi",
        ],
        &[("RAT_SNAPSHOT_DIR", &dir_path)],
    )
    .expect("spawn under a pty");
    let mut terminal = FakeTerminal::dark();

    assert!(
        wait_for(&session, &mut terminal, b"hi", Duration::from_secs(2)),
        "expected a first frame"
    );
    session.write_bytes(b"S");
    assert!(
        wait_for(&session, &mut terminal, b"snapshot", Duration::from_secs(2)),
        "expected the snapshot notice"
    );

    let entries: Vec<_> = std::fs::read_dir(dir.path())
        .expect("read snapshot dir")
        .map(|e| e.expect("dir entry").path())
        .collect();
    assert_eq!(
        entries.len(),
        1,
        "expected exactly one snapshot: {entries:?}"
    );
    let name = entries[0].file_name().expect("file name").to_string_lossy();
    assert!(
        name.starts_with("rat-watch-") && name.ends_with(".txt"),
        "unexpected snapshot name {name:?}"
    );
    let contents = std::fs::read_to_string(&entries[0]).expect("read snapshot");
    assert_eq!(contents, "hi\n");
    assert!(
        !contents.contains('\x1b'),
        "escapes should be stripped by default"
    );

    session.write_bytes(b"q");
    assert!(
        !session.kill_if_alive(Duration::from_secs(2)),
        "watch should have exited on q"
    );
}

#[test]
fn q_quits_from_a_frozen_frame() {
    let session = PtySession::spawn(
        &rat_bin(),
        &["watch", "-n", "50ms", "--", &rat_bin(), "style", "hi"],
        &[],
    )
    .expect("spawn under a pty");
    let mut terminal = FakeTerminal::dark();

    assert!(wait_for(
        &session,
        &mut terminal,
        b"hi",
        Duration::from_secs(2)
    ));
    session.write_bytes(b"j");
    assert!(
        wait_for(&session, &mut terminal, b"paused", Duration::from_secs(2)),
        "expected the frame to freeze"
    );
    session.write_bytes(b"q");
    // A clean exit from paused mode still unsubscribes on the way out.
    assert!(
        wait_for(
            &session,
            &mut terminal,
            b"\x1b[?2031l",
            Duration::from_secs(2)
        ),
        "expected the theme unsubscribe while quitting from paused mode"
    );
    assert!(
        !session.kill_if_alive(Duration::from_secs(2)),
        "watch should have exited on q while frozen"
    );
}

#[test]
fn v_pages_the_frozen_frame() {
    let lines: Vec<String> = (1..=30).map(|i| format!("l{i}")).collect();
    let rat = rat_bin();
    let mut args: Vec<&str> = vec!["watch", "-n", "50ms", "--", &rat, "style"];
    args.extend(lines.iter().map(String::as_str));
    let session =
        PtySession::spawn(&rat, &args, &[("RAT_PAGER", "/bin/cat")]).expect("spawn under a pty");
    let mut terminal = FakeTerminal::dark();

    assert!(wait_for(
        &session,
        &mut terminal,
        b"l1",
        Duration::from_secs(2)
    ));
    session.write_bytes(b"G");
    assert!(
        wait_for(
            &session,
            &mut terminal,
            b"lines 9-30 of 30",
            Duration::from_secs(2)
        ),
        "expected the bottom of the frozen frame"
    );
    session.write_bytes(b"v");
    // The pager streams the frozen body (no paused row), so this needle
    // can only come from the post-pager repaint — which repaints the
    // FROZEN window, proving paging did not resume the tail.
    assert!(
        wait_for(
            &session,
            &mut terminal,
            b"lines 9-30 of 30",
            Duration::from_secs(2)
        ),
        "expected the frozen window back after the pager returned"
    );
    session.write_bytes(b"q");
    assert!(
        !session.kill_if_alive(Duration::from_secs(2)),
        "watch should have exited on q"
    );
}

#[test]
fn leaving_the_pager_erases_the_stale_frame_before_repainting() {
    // The pager's alternate screen restores the pre-pager frame with the
    // cursor below it; the next frame must climb over and replace that
    // copy, not paint a duplicate underneath it.
    let session = PtySession::spawn(
        &rat_bin(),
        &["watch", "-n", "50ms", "--", &rat_bin(), "style", "hi"],
        &[("RAT_PAGER", "/bin/cat")],
    )
    .expect("spawn under a pty");
    let mut terminal = FakeTerminal::dark();

    assert!(wait_for(
        &session,
        &mut terminal,
        b"hi",
        Duration::from_secs(2)
    ));
    session.write_bytes(b"v");
    // The two-row frame (content + status row) means the post-pager
    // repaint must move up exactly two rows before its erase; without
    // that the frame lands below the restored copy.
    assert!(
        wait_for(
            &session,
            &mut terminal,
            b"\x1b[2A\r\x1b[0J",
            Duration::from_secs(2)
        ),
        "expected the post-pager repaint to climb over the restored frame"
    );
    session.write_bytes(b"q");
    assert!(
        !session.kill_if_alive(Duration::from_secs(2)),
        "watch should have exited on q"
    );
}
