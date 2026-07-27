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
    // The one-row frame means the post-pager repaint must move up exactly
    // one row before its erase; without that the frame lands below the
    // restored copy.
    assert!(
        wait_for(
            &session,
            &mut terminal,
            b"\x1b[1A\r\x1b[0J",
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
