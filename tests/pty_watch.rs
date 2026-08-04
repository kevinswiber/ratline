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

/// Byte offset of `needle` in `haystack`, if present.
fn find_at(haystack: &[u8], needle: &[u8]) -> Option<usize> {
    if needle.len() > haystack.len() {
        return None;
    }
    haystack.windows(needle.len()).position(|w| w == needle)
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

    session.write_bytes(b"p");
    assert!(
        wait_for(
            &session,
            &mut terminal,
            "paused · at ".as_bytes(),
            Duration::from_secs(2)
        ),
        "expected the paused row after the freeze key"
    );
    // Drain the freeze paint. The default paused row is a static
    // wall-clock stamp, so a parked frame is byte-silent outright even
    // though the child keeps changing behind it.
    let _ = drain_for(&session, Duration::from_millis(300));
    let leaked = session.read_available(Duration::from_millis(1500));
    assert!(
        leaked.is_empty(),
        "the tail kept painting while frozen: {:?}",
        String::from_utf8_lossy(&leaked)
    );

    // t flips the row to a counting age; at ten seconds it starts
    // counting: a write must arrive from the wait loop — no tick ever
    // repaints a frozen frame.
    session.write_bytes(b"t");
    let mut seen = wait_for_bytes(&session, &mut terminal, b"s ago", Duration::from_secs(12))
        .expect("expected the counting age to repaint the status row");
    seen.extend(drain_for(&session, Duration::from_millis(1200)));
    // Only the status row breathes: the age repaint must never rewrite
    // frame content, whose timestamp carries a long digit run the status
    // row cannot produce.
    let longest_digit_run = seen
        .iter()
        .fold((0usize, 0usize), |(best, run), b| {
            if b.is_ascii_digit() {
                (best.max(run + 1), run + 1)
            } else {
                (best, 0)
            }
        })
        .0;
    assert!(
        longest_digit_run < 6,
        "frame content repainted while parked (digit run of {longest_digit_run})"
    );

    // Back to the default style, then Esc resumes: a live frame
    // arrives, recognizable by its since row — the one needle a parked
    // status repaint can never produce.
    session.write_bytes(b"t");
    session.write_bytes(b"\x1b");
    assert!(
        wait_for(&session, &mut terminal, b"since ", Duration::from_secs(2)),
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
        "· ? help".as_bytes(),
        Duration::from_secs(2),
    )
    .expect("expected the live truncation notice");
    assert!(
        contains(&seen, "… 8 more lines".as_bytes()),
        "expected the hidden-line count in the truncation row"
    );

    // p freezes (a stable frame's scroll keys live-scroll instead), then
    // j scrolls the frozen window one line down.
    session.write_bytes(b"p");
    // Segment-wise: the wall-clock stamp between the two segments is
    // nondeterministic digits.
    let seen = wait_for_bytes(
        &session,
        &mut terminal,
        "· lines 1-22 of 30 · Esc resumes".as_bytes(),
        Duration::from_secs(2),
    )
    .expect("expected the frozen window at the top");
    assert!(
        contains(&seen, "paused · at ".as_bytes()),
        "expected the paused row to stamp the freeze moment"
    );
    session.write_bytes(b"j");
    assert!(
        wait_for(
            &session,
            &mut terminal,
            "· lines 2-23 of 30 · Esc resumes".as_bytes(),
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
        "· ? help".as_bytes(),
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

/// First run of `min` or more consecutive ASCII digits — frame content
/// like a `%H%M%S%f` stamp produces one; status rows cannot.
fn first_digit_run(bytes: &[u8], min: usize) -> Option<Vec<u8>> {
    let mut run: Vec<u8> = Vec::new();
    for &b in bytes {
        if b.is_ascii_digit() {
            run.push(b);
        } else {
            if run.len() >= min {
                return Some(run);
            }
            run.clear();
        }
    }
    (run.len() >= min).then_some(run)
}

#[test]
fn a_stable_frame_scrolls_live() {
    // 46 fixed lines; line 23 carries a changing stamp that is hidden in
    // the 22-row window at the top and visible at offset 1 — proof the
    // tail keeps ticking under a scrolled window.
    let rat = rat_bin();
    let script = format!(
        "i=1; while [ $i -le 46 ]; do \
           if [ $i -eq 23 ]; then {rat} date --format %H%M%S%f; \
           else echo l$i; fi; i=$((i+1)); done"
    );
    let session = PtySession::spawn(
        &rat,
        &["watch", "-n", "50ms", "--shell", "--", &script],
        &[],
    )
    .expect("spawn under a pty");
    let mut terminal = FakeTerminal::dark();

    assert!(
        wait_for(&session, &mut terminal, b"l1", Duration::from_secs(2)),
        "expected a first frame"
    );
    let _ = drain_for(&session, Duration::from_millis(200));

    // A top-reaching step never enters the mode: g at the top stays Live.
    session.write_bytes(b"g");
    let noop = drain_for(&session, Duration::from_millis(400));
    assert!(
        !contains(&noop, "live ·".as_bytes()),
        "g at the top must not enter live-scroll"
    );
    assert!(
        !contains(&noop, "paused".as_bytes()),
        "g at the top must not freeze"
    );

    // j over a stable frame scrolls LIVE: no freeze, the window slides.
    session.write_bytes(b"j");
    assert!(
        wait_for_without(
            &session,
            &mut terminal,
            "lines 2-23 of 46 · every 50ms · ? help".as_bytes(),
            b"paused",
            Duration::from_secs(2)
        ),
        "expected the live-scrolled row, never a freeze"
    );

    // The tail never stopped: the stamp on the now-visible line keeps
    // changing across drains.
    let first = drain_for(&session, Duration::from_millis(400));
    let second = drain_for(&session, Duration::from_millis(400));
    let a = first_digit_run(&first, 6).expect("a stamp in the first drain");
    let b = first_digit_run(&second, 6).expect("a stamp in the second drain");
    assert_ne!(a, b, "the visible stamp must keep ticking while scrolled");

    // g reaches the top: the mode collapses back to Live.
    session.write_bytes(b"g");
    assert!(
        wait_for(
            &session,
            &mut terminal,
            b"24 more lines",
            Duration::from_secs(2)
        ),
        "expected the live truncation row back at the top"
    );
    session.write_bytes(b"q");
    assert!(
        !session.kill_if_alive(Duration::from_secs(2)),
        "watch should have exited on q"
    );
}

#[test]
fn the_first_scroll_after_launch_scrolls_live() {
    // No warm-up: the very first scroll key after the first frame moves
    // a live window. Freezing is explicit (p or <), never a fallback.
    let lines: Vec<String> = (1..=30).map(|i| format!("l{i}")).collect();
    let rat = rat_bin();
    let mut args: Vec<&str> = vec!["watch", "-n", "50ms", "--", &rat, "style"];
    args.extend(lines.iter().map(String::as_str));
    let session = PtySession::spawn(&rat, &args, &[]).expect("spawn under a pty");
    let mut terminal = FakeTerminal::dark();

    assert!(
        wait_for(&session, &mut terminal, b"l1", Duration::from_secs(2)),
        "expected a first frame"
    );
    session.write_bytes(b"j");
    assert!(
        wait_for_without(
            &session,
            &mut terminal,
            "lines 2-23 of 30 · every 50ms · ? help".as_bytes(),
            b"paused",
            Duration::from_secs(2)
        ),
        "expected the first scroll to live-scroll, never freeze"
    );
    session.write_bytes(b"q");
    assert!(
        !session.kill_if_alive(Duration::from_secs(2)),
        "watch should have exited on q"
    );
}

#[test]
fn a_jittering_frame_scrolls_live_without_freezing() {
    // The line count alternates every tick (30 ⇄ 31). Scrolling is a
    // live viewport even so: the window re-anchors into the current
    // shape and never captures a frozen copy on its own.
    let dir = tempfile::tempdir().expect("tempdir");
    let flag = dir.path().join("flag").display().to_string();
    let script = format!(
        "i=1; while [ $i -le 30 ]; do echo l$i; i=$((i+1)); done; \
         if [ -e {flag} ]; then rm -f {flag}; echo extra; else : > {flag}; fi"
    );
    let session = PtySession::spawn(
        &rat_bin(),
        &["watch", "-n", "50ms", "--shell", "--", &script],
        &[],
    )
    .expect("spawn under a pty");
    let mut terminal = FakeTerminal::dark();

    assert!(
        wait_for(&session, &mut terminal, b"l1", Duration::from_secs(2)),
        "expected a first frame"
    );
    session.write_bytes(b"j");
    assert!(
        wait_for_without(
            &session,
            &mut terminal,
            "live · ".as_bytes(),
            b"paused",
            Duration::from_secs(2)
        ),
        "expected a jittering frame to live-scroll, never freeze"
    );
    session.write_bytes(b"q");
    assert!(
        !session.kill_if_alive(Duration::from_secs(2)),
        "watch should have exited on q"
    );
}

#[test]
fn a_shape_change_while_scrolled_stays_live() {
    // The frame grows one line the moment the grow file appears,
    // deterministically mid-scroll. The window must stay LIVE and
    // re-anchor into the new shape: the status row names the new total,
    // and neither a freeze nor a shape notice ever appears.
    let dir = tempfile::tempdir().expect("tempdir");
    let grow = dir.path().join("grow");
    let grow_path = grow.display().to_string();
    let script = format!(
        "i=1; while [ $i -le 30 ]; do echo l$i; i=$((i+1)); done; \
         if [ -e {grow_path} ]; then echo extra; fi"
    );
    let session = PtySession::spawn(
        &rat_bin(),
        &["watch", "-n", "50ms", "--shell", "--", &script],
        &[],
    )
    .expect("spawn under a pty");
    let mut terminal = FakeTerminal::dark();

    assert!(
        wait_for(&session, &mut terminal, b"l1", Duration::from_secs(2)),
        "expected a first frame"
    );
    session.write_bytes(b"j");
    assert!(
        wait_for_without(
            &session,
            &mut terminal,
            "lines 2-23 of 30 · every 50ms · ? help".as_bytes(),
            b"paused",
            Duration::from_secs(2)
        ),
        "expected the frame to live-scroll"
    );

    std::fs::write(&grow, b"").expect("create the grow file");
    let seen = wait_for_bytes(
        &session,
        &mut terminal,
        "of 31 · every".as_bytes(),
        Duration::from_secs(8),
    )
    .expect("expected the scrolled row to pick up the new total, still live");
    assert!(
        !contains(&seen, b"paused"),
        "a shape change must not freeze a live window"
    );
    assert!(
        !contains(&seen, b"frame changed shape"),
        "the auto-freeze notice is gone with the mechanism"
    );

    session.write_bytes(b"\x1b");
    assert!(
        wait_for(&session, &mut terminal, b"since ", Duration::from_secs(2)),
        "expected Esc to return to the live view"
    );
    session.write_bytes(b"q");
    assert!(
        !session.kill_if_alive(Duration::from_secs(2)),
        "watch should have exited on q"
    );
}

#[test]
fn resuming_from_a_live_window_does_not_rerun_the_child() {
    // A slow child (2 s) on a long interval: Esc from a live-scrolled
    // window must collapse IN PLACE from the frame already on hand. A
    // forced fresh tick would block on the child and cost the whole
    // reload — the stall belongs to resume-from-Paused only, where the
    // content is genuinely stale.
    let dir = tempfile::tempdir().expect("tempdir");
    let count = dir.path().join("count");
    let count_path = count.display().to_string();
    let script = format!(
        "echo x >> {count_path}; /bin/sleep 2; i=1; while [ $i -le 30 ]; do echo l$i; i=$((i+1)); done"
    );
    let session = PtySession::spawn(
        &rat_bin(),
        &["watch", "-n", "10s", "--shell", "--", &script],
        &[],
    )
    .expect("spawn under a pty");
    let mut terminal = FakeTerminal::dark();

    assert!(
        wait_for(&session, &mut terminal, b"l1", Duration::from_secs(5)),
        "expected a first frame"
    );
    let runs = std::fs::read_to_string(&count)
        .expect("count file")
        .lines()
        .count();
    assert_eq!(runs, 1, "one child behind the first frame");
    session.write_bytes(b"j");
    assert!(
        wait_for_without(
            &session,
            &mut terminal,
            "live · ".as_bytes(),
            b"paused",
            Duration::from_secs(2)
        ),
        "expected the frame to live-scroll"
    );

    session.write_bytes(b"\x1b");
    assert!(
        wait_for(
            &session,
            &mut terminal,
            b"since ",
            Duration::from_millis(1500)
        ),
        "expected Esc to collapse in place, well inside the child's 2s runtime"
    );
    // Child-side evidence, stronger than the timing above: a forced
    // tick would have appended to the count within one loop turn.
    let _ = drain_for(&session, Duration::from_millis(400));
    let runs = std::fs::read_to_string(&count)
        .expect("count file")
        .lines()
        .count();
    assert_eq!(
        runs, 1,
        "resume from a live window must not re-run the child"
    );

    session.write_bytes(b"q");
    assert!(
        !session.kill_if_alive(Duration::from_secs(2)),
        "watch should have exited on q"
    );
}

#[test]
fn a_scroll_on_a_frame_that_fits_is_a_noop() {
    // Nothing hidden, nothing to scroll: navigation keys do nothing at
    // all — no live window, and certainly no freeze.
    let session = PtySession::spawn(
        &rat_bin(),
        &["watch", "-n", "50ms", "--", &rat_bin(), "style", "hi"],
        &[],
    )
    .expect("spawn under a pty");
    let mut terminal = FakeTerminal::dark();

    assert!(
        wait_for(&session, &mut terminal, b"hi", Duration::from_secs(2)),
        "expected a first frame"
    );
    session.write_bytes(b"j");
    let noop = drain_for(&session, Duration::from_millis(400));
    assert!(
        !contains(&noop, "live ·".as_bytes()),
        "a fitting frame has nowhere to scroll"
    );
    assert!(
        !contains(&noop, b"paused"),
        "a scroll key must never freeze a fitting frame"
    );
    session.write_bytes(b"q");
    assert!(
        !session.kill_if_alive(Duration::from_secs(2)),
        "watch should have exited on q"
    );
}

/// Every `v`-prefixed six-digit counter value in the byte stream — the
/// scrub fixtures print `v%06d`, monotonic and fixed-width, so ordering
/// assertions are exact.
fn counter_values(bytes: &[u8]) -> Vec<u64> {
    let mut vals = Vec::new();
    let mut i = 0;
    while i + 7 <= bytes.len() {
        if bytes[i] == b'v' && bytes[i + 1..i + 7].iter().all(u8::is_ascii_digit) {
            let s = std::str::from_utf8(&bytes[i + 1..i + 7]).expect("digits");
            vals.push(s.parse().expect("six digits"));
            i += 7;
        } else {
            i += 1;
        }
    }
    vals
}

#[test]
fn scrubbing_shows_an_older_frame_and_s_snapshots_it() {
    let dir = tempfile::tempdir().expect("tempdir");
    let count = dir.path().join("count").display().to_string();
    let snaps = tempfile::tempdir().expect("snapshot dir");
    let snaps_path = snaps.path().display().to_string();
    let script = format!("echo x >> {count}; n=$(wc -l < {count}); printf 'v%06d\\n' $n");
    let session = PtySession::spawn(
        &rat_bin(),
        &["watch", "-n", "50ms", "--shell", "--", &script],
        &[("RAT_SNAPSHOT_DIR", &snaps_path)],
    )
    .expect("spawn under a pty");
    let mut terminal = FakeTerminal::dark();

    // Awaited, not just drained. `<` parks on the previous DISTINCT
    // entry and is a silent no-op when none exists, and a timed drain
    // cannot promise a second frame on a loaded machine — every frame
    // costs a shell spawn. The second counter value on the wire means
    // the tick that recorded it is complete, so a scrub-back target
    // exists by construction before < is sent.
    assert!(
        wait_for(&session, &mut terminal, b"v000002", Duration::from_secs(3)),
        "expected a second distinct frame for < to step back to"
    );

    session.write_bytes(b"<");
    let seen = wait_for_bytes(
        &session,
        &mut terminal,
        "paused ·".as_bytes(),
        Duration::from_secs(2),
    )
    .expect("expected < to scrub into a pause");
    let shown = *counter_values(&seen)
        .last()
        .expect("a value on the scrubbed frame");

    session.write_bytes(b"S");
    assert!(
        wait_for(&session, &mut terminal, b"snapshot", Duration::from_secs(2)),
        "expected the snapshot notice"
    );
    let entries: Vec<_> = std::fs::read_dir(snaps.path())
        .expect("read snapshot dir")
        .map(|e| e.expect("dir entry").path())
        .collect();
    assert_eq!(entries.len(), 1, "expected one snapshot: {entries:?}");
    let contents = std::fs::read_to_string(&entries[0]).expect("read snapshot");
    let filed = *counter_values(contents.as_bytes())
        .first()
        .expect("a value in the snapshot");
    assert_eq!(filed, shown, "S must write the frame being viewed");

    session.write_bytes(b"\x1b");
    let live = wait_for_bytes(&session, &mut terminal, b"since ", Duration::from_secs(2))
        .expect("expected Esc to resume the live tail");
    let now_val = *counter_values(&live).last().expect("a live value");
    assert!(
        filed < now_val,
        "the snapshot must hold an OLDER value ({filed} vs {now_val})"
    );
    session.write_bytes(b"q");
    assert!(
        !session.kill_if_alive(Duration::from_secs(2)),
        "watch should have exited on q"
    );
}

#[test]
fn scrub_forward_past_the_newest_returns_to_live() {
    // The counter caps at 3: after three distinct frames the child goes
    // static, so history settles at exactly three entries.
    let dir = tempfile::tempdir().expect("tempdir");
    let count = dir.path().join("count").display().to_string();
    let script = format!(
        "echo x >> {count}; n=$(wc -l < {count}); \
         if [ $n -gt 3 ]; then n=3; fi; printf 'v%06d\\n' $n"
    );
    let session = PtySession::spawn(
        &rat_bin(),
        &["watch", "-n", "50ms", "--shell", "--", &script],
        &[],
    )
    .expect("spawn under a pty");
    let mut terminal = FakeTerminal::dark();

    assert!(
        wait_for(&session, &mut terminal, b"v000003", Duration::from_secs(2)),
        "expected the counter to reach its cap"
    );
    let _ = drain_for(&session, Duration::from_millis(300));

    session.write_bytes(b"<");
    assert!(
        wait_for(&session, &mut terminal, b"v000002", Duration::from_secs(2)),
        "expected < to show the previous frame"
    );
    session.write_bytes(b">");
    assert!(
        wait_for(&session, &mut terminal, b"v000003", Duration::from_secs(2)),
        "expected > to step back to the newest entry"
    );
    let _ = drain_for(&session, Duration::from_millis(200));
    // Past the newest there is nothing left to step to: the frame on
    // screen IS the live frame, so > leaves the freeze — the same walk,
    // one key, all the way back to the live view.
    session.write_bytes(b">");
    assert!(
        wait_for(&session, &mut terminal, b"since ", Duration::from_secs(2)),
        "expected > past the newest to return to the live view"
    );
    session.write_bytes(b"q");
    assert!(
        !session.kill_if_alive(Duration::from_secs(2)),
        "watch should have exited on q"
    );
}

#[test]
fn scrubbing_from_an_old_freeze_steps_backward_not_forward() {
    // The anchor contract: the tail keeps recording behind a freeze, so
    // < from an OLD freeze must step older than what is on screen —
    // an anchor-less scrub would jump forward to unseen frames.
    let dir = tempfile::tempdir().expect("tempdir");
    let count = dir.path().join("count").display().to_string();
    let script = format!("echo x >> {count}; n=$(wc -l < {count}); printf 'v%06d\\n' $n");
    let session = PtySession::spawn(
        &rat_bin(),
        &["watch", "-n", "50ms", "--shell", "--", &script],
        &[],
    )
    .expect("spawn under a pty");
    let mut terminal = FakeTerminal::dark();

    // Wait past the first few frames so the frozen value has a
    // predecessor to step back to.
    // Kept, not just awaited. The frozen copy is the last value PAINTED,
    // and neither source below is guaranteed to carry one: every frame
    // costs a shell spawn, so on a loaded machine the drain can span no
    // frame at all, and a freeze whose copy already matches the screen
    // rewrites ONLY the status row by design. Starting the accumulation
    // here means there is always at least this frame to fall back on.
    let mut all = wait_for_bytes(&session, &mut terminal, b"v000003", Duration::from_secs(3))
        .expect("expected the counter to reach three");
    all.extend(drain_for(&session, Duration::from_millis(300)));
    session.write_bytes(b"p");
    all.extend(
        wait_for_bytes(
            &session,
            &mut terminal,
            "paused ·".as_bytes(),
            Duration::from_secs(2),
        )
        .expect("expected p to freeze"),
    );
    let frozen = *counter_values(&all).last().expect("the frozen value");

    // The tail records newer frames behind the freeze.
    let _ = drain_for(&session, Duration::from_millis(500));

    session.write_bytes(b"<");
    let needle = format!("v{:06}", frozen - 1);
    let stepped = wait_for_bytes(
        &session,
        &mut terminal,
        needle.as_bytes(),
        Duration::from_secs(2),
    )
    .expect("expected < to step OLDER than the frozen frame");
    assert!(
        counter_values(&stepped).iter().all(|&v| v <= frozen),
        "a scrub-back from an old freeze must never show a newer frame"
    );

    session.write_bytes(b"\x1b");
    assert!(
        wait_for(&session, &mut terminal, b"since ", Duration::from_secs(2)),
        "expected Esc to resume the live tail"
    );
    session.write_bytes(b"q");
    assert!(
        !session.kill_if_alive(Duration::from_secs(2)),
        "watch should have exited on q"
    );
}

#[test]
fn p_freezes_and_f_resumes() {
    // A deliberate freeze needs no scroll: p parks the frame in place.
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

    assert!(
        wait_for(
            &session,
            &mut terminal,
            b"\x1b[?2026h",
            Duration::from_secs(2)
        ),
        "expected a first frame"
    );
    session.write_bytes(b"p");
    assert!(
        wait_for(
            &session,
            &mut terminal,
            "paused · at ".as_bytes(),
            Duration::from_secs(2)
        ),
        "expected p to freeze the frame"
    );
    // The tail is stopped: changing child output paints nothing.
    let _ = drain_for(&session, Duration::from_millis(300));
    let leaked = session.read_available(Duration::from_millis(400));
    assert!(
        leaked.is_empty(),
        "the tail kept painting after p: {:?}",
        String::from_utf8_lossy(&leaked)
    );

    session.write_bytes(b"F");
    assert!(
        wait_for(&session, &mut terminal, b"since ", Duration::from_secs(2)),
        "expected F to resume the live tail"
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
    session.write_bytes(b"p");
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
    // p freezes (a stable frame's G would live-scroll), then G scrolls
    // the frozen window to the bottom.
    session.write_bytes(b"p");
    assert!(
        wait_for(&session, &mut terminal, b"paused", Duration::from_secs(2)),
        "expected the frame to freeze"
    );
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

#[test]
fn a_pager_return_does_not_rerun_the_child() {
    // Leaving the pager must repaint from the frame on hand, not by
    // forcing a tick: on a slow dashboard a forced re-run would stall
    // the return by a whole child runtime. The count file is
    // child-side evidence — the child appends at the top of its
    // script, so "a child ran" is observable without any drain.
    let dir = tempfile::tempdir().expect("tempdir");
    let count = dir.path().join("count");
    let count_path = count.display().to_string();
    let script = format!("echo x >> {count_path}; n=$(wc -l < {count_path}); printf 'v%06d\n' $n");
    let session = PtySession::spawn(
        &rat_bin(),
        &["watch", "-n", "10s", "--shell", "--", &script],
        &[("RAT_PAGER", "/bin/cat")],
    )
    .expect("spawn under a pty");
    let mut terminal = FakeTerminal::dark();

    assert!(
        wait_for(&session, &mut terminal, b"v000001", Duration::from_secs(5)),
        "expected the first frame"
    );
    let runs = std::fs::read_to_string(&count)
        .expect("count file")
        .lines()
        .count();
    assert_eq!(runs, 1, "one child before paging");

    session.write_bytes(b"v");
    assert!(
        wait_for(&session, &mut terminal, b"v000001", Duration::from_secs(3)),
        "expected the frame back after the pager"
    );
    // A settle before reading child-side evidence, not a timing proof:
    // a forced tick would have spawned within one loop turn of the
    // pager's return.
    let _ = drain_for(&session, Duration::from_millis(400));
    let runs = std::fs::read_to_string(&count)
        .expect("count file")
        .lines()
        .count();
    assert_eq!(runs, 1, "the pager round-trip must not re-run the child");

    session.write_bytes(b"q");
    assert!(
        !session.kill_if_alive(Duration::from_secs(2)),
        "watch should have exited on q"
    );
}

#[test]
fn keys_answer_while_a_slow_child_runs() {
    // The finding's headline: a slow child must not deafen the loop.
    // The count file is child-side evidence a SECOND child has started
    // (and has ~3 s left) before q is pressed.
    let dir = tempfile::tempdir().expect("tempdir");
    let count = dir.path().join("count");
    let count_path = count.display().to_string();
    let script = format!(
        "echo x >> {count_path}; /bin/sleep 3; n=$(wc -l < {count_path}); printf 'v%06d\\n' $n"
    );
    let session = PtySession::spawn(
        &rat_bin(),
        &["watch", "-n", "50ms", "--shell", "--", &script],
        &[],
    )
    .expect("spawn under a pty");
    let mut terminal = FakeTerminal::dark();

    assert!(
        wait_for(&session, &mut terminal, b"v000001", Duration::from_secs(6)),
        "expected the first frame"
    );
    let deadline = std::time::Instant::now() + Duration::from_secs(3);
    loop {
        let runs = std::fs::read_to_string(&count)
            .map(|s| s.lines().count())
            .unwrap_or(0);
        if runs >= 2 {
            break;
        }
        assert!(
            std::time::Instant::now() < deadline,
            "a second child never started"
        );
        std::thread::sleep(Duration::from_millis(20));
    }
    session.write_bytes(b"q");
    // 2x headroom against the ~3 s the child still has to run: the
    // loop answered while the child was mid-flight.
    assert!(
        !session.kill_if_alive(Duration::from_millis(1500)),
        "q must exit while the child is still running"
    );
}

#[test]
fn a_key_repaints_while_a_slow_child_runs() {
    // Not just input: a real dispatch AND paint must land inside the
    // child's runtime.
    let dir = tempfile::tempdir().expect("tempdir");
    let count = dir.path().join("count");
    let count_path = count.display().to_string();
    let script = format!(
        "echo x >> {count_path}; /bin/sleep 3; i=1; while [ $i -le 30 ]; do echo l$i; i=$((i+1)); done"
    );
    let session = PtySession::spawn(
        &rat_bin(),
        &["watch", "-n", "50ms", "--shell", "--", &script],
        &[],
    )
    .expect("spawn under a pty");
    let mut terminal = FakeTerminal::dark();

    assert!(
        wait_for(&session, &mut terminal, b"l1", Duration::from_secs(6)),
        "expected the first frame"
    );
    let deadline = std::time::Instant::now() + Duration::from_secs(3);
    loop {
        let runs = std::fs::read_to_string(&count)
            .map(|s| s.lines().count())
            .unwrap_or(0);
        if runs >= 2 {
            break;
        }
        assert!(
            std::time::Instant::now() < deadline,
            "a second child never started"
        );
        std::thread::sleep(Duration::from_millis(20));
    }
    session.write_bytes(b"j");
    assert!(
        wait_for(
            &session,
            &mut terminal,
            "live · ".as_bytes(),
            Duration::from_millis(1500)
        ),
        "expected a live-scroll repaint inside the child's runtime"
    );
    session.write_bytes(b"q");
    assert!(
        !session.kill_if_alive(Duration::from_secs(2)),
        "watch should have exited on q"
    );
}

#[test]
fn quitting_kills_the_child_it_started() {
    // The child dies with the watch: today's blocking loop cannot
    // orphan one, and the async loop must not start. The interpreter
    // writing {finished} as its last act is the child-side evidence —
    // a killed script never gets there.
    let dir = tempfile::tempdir().expect("tempdir");
    let started = dir.path().join("started");
    let finished = dir.path().join("finished");
    let script = format!(
        ": > {}; /bin/sleep 1; : > {}",
        started.display(),
        finished.display()
    );
    let session = PtySession::spawn(
        &rat_bin(),
        &["watch", "-n", "10s", "--shell", "--", &script],
        &[],
    )
    .expect("spawn under a pty");
    let mut terminal = FakeTerminal::dark();

    let deadline = std::time::Instant::now() + Duration::from_secs(3);
    while !started.exists() {
        // Keep answering the startup appearance probe while polling —
        // the first tick waits behind it.
        let chunk = session.read_available(Duration::from_millis(20));
        terminal.respond(&session, &chunk);
        assert!(
            std::time::Instant::now() < deadline,
            "the child never started"
        );
    }
    // Before the first frame exists: quit must answer anyway (nothing
    // is painted yet; the frame-dependent keys are gated, q is not).
    session.write_bytes(b"q");
    assert!(
        !session.kill_if_alive(Duration::from_millis(1500)),
        "q must exit during the first child run"
    );
    // A bounded poll of child-side evidence, not a drain: the killed
    // interpreter can never write its last act.
    let deadline = std::time::Instant::now() + Duration::from_millis(2500);
    while std::time::Instant::now() < deadline {
        assert!(!finished.exists(), "the child survived the watch's exit");
        std::thread::sleep(Duration::from_millis(50));
    }
}

#[test]
fn once_paints_one_frame_and_exits() {
    // No test has ever run --once on a tty; its paint now rides the
    // same completion handler as the loop, so pin it: one frame, a
    // clean self-exit, no q needed.
    let session = PtySession::spawn(
        &rat_bin(),
        &["watch", "--once", "--", &rat_bin(), "style", "hi"],
        &[],
    )
    .expect("spawn under a pty");
    let mut terminal = FakeTerminal::dark();
    assert!(
        wait_for(&session, &mut terminal, b"hi", Duration::from_secs(3)),
        "expected the one frame"
    );
    assert!(
        !session.kill_if_alive(Duration::from_secs(2)),
        "once mode should exit on its own"
    );
}

#[test]
fn a_theme_flip_during_a_child_run_reruns_it_under_the_new_appearance() {
    // A theme adoption is an ENVIRONMENT change: the in-flight child
    // was started under the old RAT_APPEARANCE and must not satisfy
    // the repaint's tick — a fresh child has to run. The 8 s window is
    // an 18-second discriminator: a merely-satisfied request would
    // not show the light child until the natural tick at t≈22 s.
    let dir = tempfile::tempdir().expect("tempdir");
    let count = dir.path().join("count");
    let count_path = count.display().to_string();
    let script = format!("echo x >> {count_path}; /bin/sleep 2; echo appearance=$RAT_APPEARANCE");
    let session = PtySession::spawn(
        &rat_bin(),
        &["watch", "-n", "20s", "--shell", "--", &script],
        &[],
    )
    .expect("spawn under a pty");
    let mut terminal = FakeTerminal::dark();

    // The first child is RUNNING and no frame exists yet — the flip
    // lands mid-run, before the first paint.
    let deadline = std::time::Instant::now() + Duration::from_secs(3);
    while std::fs::read_to_string(&count)
        .map(|s| s.lines().count())
        .unwrap_or(0)
        < 1
    {
        let chunk = session.read_available(Duration::from_millis(20));
        terminal.respond(&session, &chunk);
        assert!(
            std::time::Instant::now() < deadline,
            "the first child never started"
        );
    }
    // The terminal's colors change, then it announces the change —
    // one stale report and two corrected ones, as a real flip sends.
    terminal.set("rgb:0000/0000/0000", "rgb:ffff/ffff/ffff");
    session.write_bytes(b"\x1b[?997;1n\x1b[?997;2n\x1b[?997;2n");

    assert!(
        wait_for(
            &session,
            &mut terminal,
            b"appearance=light",
            Duration::from_secs(8)
        ),
        "expected a fresh child under the new appearance"
    );
    session.write_bytes(b"q");
    assert!(
        !session.kill_if_alive(Duration::from_secs(2)),
        "watch should have exited on q"
    );
}

#[test]
fn resuming_from_a_pause_paints_at_once_and_still_refreshes() {
    // The idle-at-F half of the resume contract. Red provenance:
    // assertion 1 fails on any tree where resume waits for the forced
    // tick before painting (the pre-async stall, or a dropped
    // in-place repaint); assertion 2 fails if resume loses its
    // request_now (the fresh frame would wait for the natural tick at
    // t≈11 s); assertion 3 catches a spurious double-spawn.
    let dir = tempfile::tempdir().expect("tempdir");
    let count = dir.path().join("count");
    let count_path = count.display().to_string();
    let script = format!(
        "echo x >> {count_path}; /bin/sleep 1; n=$(wc -l < {count_path}); printf 'v%06d\\n' $n"
    );
    let session = PtySession::spawn(
        &rat_bin(),
        &["watch", "-n", "10s", "--shell", "--", &script],
        &[],
    )
    .expect("spawn under a pty");
    let mut terminal = FakeTerminal::dark();

    assert!(
        wait_for(&session, &mut terminal, b"v000001", Duration::from_secs(5)),
        "expected the first frame"
    );
    session.write_bytes(b"p");
    assert!(
        wait_for(
            &session,
            &mut terminal,
            "paused ·".as_bytes(),
            Duration::from_secs(2)
        ),
        "expected p to freeze"
    );
    session.write_bytes(b"F");
    assert!(
        wait_for(
            &session,
            &mut terminal,
            b"since ",
            Duration::from_millis(500)
        ),
        "expected the collapse to paint without waiting out the child"
    );
    assert!(
        wait_for(&session, &mut terminal, b"v000002", Duration::from_secs(4)),
        "expected the fresh-tick self-heal to deliver"
    );
    // A settle before reading child-side evidence: one startup child,
    // one self-heal child, nothing else.
    let _ = drain_for(&session, Duration::from_millis(1500));
    let runs = std::fs::read_to_string(&count)
        .expect("count file")
        .lines()
        .count();
    assert_eq!(runs, 2, "resume must spawn exactly one fresh child");

    session.write_bytes(b"q");
    assert!(
        !session.kill_if_alive(Duration::from_secs(2)),
        "watch should have exited on q"
    );
}

#[test]
fn resuming_while_a_child_is_in_flight_collapses_at_once_and_never_doubles() {
    // The in-flight half: F while a child runs must paint NOW, be
    // satisfied by that child's completion, and not double-spawn. Red
    // provenance: assertion 1 fails on any tree where the collapse
    // blocks on the in-flight tick; assertion 3 fails if a pending
    // request force-spawns on top of the completion that satisfied it
    // (count reads 3 well before the next honest tick at t≈13 s).
    let dir = tempfile::tempdir().expect("tempdir");
    let count = dir.path().join("count");
    let count_path = count.display().to_string();
    let script = format!(
        "echo x >> {count_path}; n=$(wc -l < {count_path}); \
         if [ $n -ge 2 ]; then /bin/sleep 3; fi; printf 'v%06d\\n' $n"
    );
    let session = PtySession::spawn(
        &rat_bin(),
        &["watch", "-n", "5s", "--shell", "--", &script],
        &[],
    )
    .expect("spawn under a pty");
    let mut terminal = FakeTerminal::dark();

    assert!(
        wait_for(&session, &mut terminal, b"v000001", Duration::from_secs(5)),
        "expected the fast first frame"
    );
    // Child-side evidence the SECOND child is in flight (its top-of-
    // script append) with ~3 s of sleep left.
    let deadline = std::time::Instant::now() + Duration::from_secs(8);
    while std::fs::read_to_string(&count)
        .map(|s| s.lines().count())
        .unwrap_or(0)
        < 2
    {
        let chunk = session.read_available(Duration::from_millis(20));
        terminal.respond(&session, &chunk);
        assert!(
            std::time::Instant::now() < deadline,
            "the second child never started"
        );
    }
    session.write_bytes(b"p");
    assert!(
        wait_for(
            &session,
            &mut terminal,
            "paused ·".as_bytes(),
            Duration::from_secs(2)
        ),
        "expected p to freeze while the child runs"
    );
    session.write_bytes(b"F");
    assert!(
        wait_for(
            &session,
            &mut terminal,
            b"since ",
            Duration::from_millis(500)
        ),
        "expected the collapse to paint while the child is still running"
    );
    assert!(
        wait_for(&session, &mut terminal, b"v000002", Duration::from_secs(6)),
        "expected the in-flight completion to deliver the fresh frame"
    );
    // Inside the quiet gap before the next natural tick: the
    // completion satisfied the request, so exactly two children ran.
    let _ = drain_for(&session, Duration::from_millis(1500));
    let runs = std::fs::read_to_string(&count)
        .expect("count file")
        .lines()
        .count();
    assert_eq!(runs, 2, "the in-flight completion must satisfy the request");

    session.write_bytes(b"q");
    assert!(
        !session.kill_if_alive(Duration::from_secs(2)),
        "watch should have exited on q"
    );
}

#[test]
fn question_mark_pages_the_key_reference() {
    // ? pages a plain-text key reference through the same pager path v
    // uses — which also means search over the bindings comes free.
    let session = PtySession::spawn(
        &rat_bin(),
        &["watch", "-n", "50ms", "--", &rat_bin(), "style", "hi"],
        &[("RAT_PAGER", "/bin/cat")],
    )
    .expect("spawn under a pty");
    let mut terminal = FakeTerminal::dark();

    assert!(
        wait_for(&session, &mut terminal, b"hi", Duration::from_secs(2)),
        "expected a first frame"
    );
    // The park handshake is ALLOWED to refuse when the reader thread
    // is starved (a loaded CI runner): the arm answers "pager
    // unavailable ... try again" by design. Honor that path the way a
    // user would — press ? again — instead of asserting a one-shot
    // success the design never promised.
    session.write_bytes(b"?");
    let mut seen: Vec<u8> = Vec::new();
    let mut presses = 1;
    let deadline = std::time::Instant::now() + Duration::from_secs(15);
    let mut next_retry = std::time::Instant::now() + Duration::from_secs(2);
    loop {
        assert!(
            std::time::Instant::now() < deadline,
            "expected the key reference in the pager (after {presses} presses)"
        );
        let chunk = session.read_available(Duration::from_millis(50));
        terminal.respond(&session, &chunk);
        seen.extend_from_slice(&chunk);
        if contains(&seen, b"freeze the frame in place") {
            break;
        }
        // A REPEATED refusal repaints an identical frame — zero bytes
        // through the differ — so the retry rides a timer, never the
        // notice text.
        if std::time::Instant::now() >= next_retry && presses < 6 {
            presses += 1;
            session.write_bytes(b"?");
            next_retry = std::time::Instant::now() + Duration::from_secs(2);
        }
    }
    assert!(
        wait_for(&session, &mut terminal, b"hi", Duration::from_secs(5)),
        "expected the frame back after the pager"
    );
    session.write_bytes(b"q");
    assert!(
        !session.kill_if_alive(Duration::from_secs(2)),
        "watch should have exited on q"
    );
}

#[test]
fn the_live_row_names_the_interval() {
    // The cadence rides every live row so the next refresh can be
    // anticipated, with the ? breadcrumb as the one remaining hint.
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
            "· every 50ms · ? help".as_bytes(),
            Duration::from_secs(2)
        ),
        "expected the interval and help breadcrumb on the live row"
    );
    session.write_bytes(b"q");
    assert!(
        !session.kill_if_alive(Duration::from_secs(2)),
        "watch should have exited on q"
    );
}

/// The last complete \r\n-delimited row containing `needle`.
fn row_containing<'a>(bytes: &'a [u8], needle: &[u8]) -> Option<&'a [u8]> {
    bytes
        .split(|&b| b == b'\n')
        .map(|row| row.strip_suffix(b"\r").unwrap_or(row))
        .rfind(|row| contains(row, needle))
}

#[test]
fn the_gutter_marks_the_changed_line_only() {
    let dir = tempfile::tempdir().expect("tempdir");
    let count = dir.path().join("count").display().to_string();
    let script = format!(
        "echo steady-anchor-line; echo x >> {count}; n=$(wc -l < {count}); printf 'v%06d\\n' $n"
    );
    let session = PtySession::spawn(
        &rat_bin(),
        &["watch", "-n", "50ms", "--shell", "--", &script],
        &[],
    )
    .expect("spawn under a pty");
    let mut terminal = FakeTerminal::dark();

    assert!(
        wait_for(&session, &mut terminal, b"v0000", Duration::from_secs(2)),
        "expected a first counter frame"
    );
    session.write_bytes(b"D");
    let bytes = wait_for_bytes(
        &session,
        &mut terminal,
        "▌".as_bytes(),
        Duration::from_secs(2),
    )
    .expect("expected a gutter mark after D");
    let counter_row = row_containing(&bytes, b"v0").expect("a counter row");
    assert!(
        contains(counter_row, "▌".as_bytes()),
        "the changed row must carry the mark: {:?}",
        String::from_utf8_lossy(counter_row)
    );
    let steady_row = row_containing(&bytes, b"steady-anchor-line").expect("the steady row");
    assert!(
        !contains(steady_row, "▌".as_bytes()),
        "an unchanged row must not be marked: {:?}",
        String::from_utf8_lossy(steady_row)
    );
    assert!(
        contains(steady_row, b"  steady-anchor-line"),
        "the unmarked two-space cell must precede the content: {:?}",
        String::from_utf8_lossy(steady_row)
    );
    session.write_bytes(b"q");
    assert!(
        !session.kill_if_alive(Duration::from_secs(2)),
        "watch should have exited on q"
    );
}

#[test]
fn the_gutter_column_survives_a_shift() {
    let dir = tempfile::tempdir().expect("tempdir");
    let count = dir.path().join("count").display().to_string();
    let script = format!(
        "echo 12345678-steady-remainder; echo x >> {count}; n=$(wc -l < {count}); \
         printf 'counter-mark v%06d\\n' $n"
    );
    let session = PtySession::spawn(
        &rat_bin(),
        &["watch", "-n", "50ms", "--shell", "--", &script],
        &[],
    )
    .expect("spawn under a pty");
    let mut terminal = FakeTerminal::dark();

    assert!(
        wait_for(&session, &mut terminal, b"v0000", Duration::from_secs(2)),
        "expected a first counter frame"
    );
    session.write_bytes(b"D");
    assert!(
        wait_for(
            &session,
            &mut terminal,
            "▌".as_bytes(),
            Duration::from_secs(2)
        ),
        "expected a gutter mark after D"
    );
    session.write_bytes(b"l");
    let bytes = wait_for_bytes(
        &session,
        &mut terminal,
        b"-steady-remainder",
        Duration::from_secs(2),
    )
    .expect("expected the shifted frame");
    let steady_row = row_containing(&bytes, b"-steady-remainder").expect("the shifted row");
    assert!(
        !contains(steady_row, b"12345678"),
        "the content region must have shifted: {:?}",
        String::from_utf8_lossy(steady_row)
    );
    let counter_row = row_containing(&bytes, b"v0").expect("a counter row");
    assert!(
        contains(counter_row, "▌".as_bytes()),
        "the margin sits outside the shift window: {:?}",
        String::from_utf8_lossy(counter_row)
    );
    session.write_bytes(b"q");
    assert!(
        !session.kill_if_alive(Duration::from_secs(2)),
        "watch should have exited on q"
    );
}

#[test]
fn toggling_the_gutter_off_restores_the_plain_frame() {
    let dir = tempfile::tempdir().expect("tempdir");
    let count = dir.path().join("count").display().to_string();
    let script = format!("echo x >> {count}; n=$(wc -l < {count}); printf 'v%06d\\n' $n");
    let session = PtySession::spawn(
        &rat_bin(),
        &["watch", "-n", "50ms", "--shell", "--", &script],
        &[],
    )
    .expect("spawn under a pty");
    let mut terminal = FakeTerminal::dark();

    assert!(
        wait_for(&session, &mut terminal, b"v0000", Duration::from_secs(2)),
        "expected a first counter frame"
    );
    session.write_bytes(b"D");
    assert!(
        wait_for(
            &session,
            &mut terminal,
            "▌".as_bytes(),
            Duration::from_secs(2)
        ),
        "expected a gutter mark after D"
    );
    session.write_bytes(b"D");
    // Swallow the transition — a marked frame may still be in flight.
    let _ = drain_for(&session, Duration::from_millis(400));
    let bytes = wait_for_bytes(&session, &mut terminal, b"v0", Duration::from_secs(2))
        .expect("expected a plain counter row after toggling off");
    assert!(
        !contains(&bytes, "▌".as_bytes()),
        "no mark cell may survive the toggle: {:?}",
        String::from_utf8_lossy(&bytes)
    );
    session.write_bytes(b"q");
    assert!(
        !session.kill_if_alive(Duration::from_secs(2)),
        "watch should have exited on q"
    );
}

#[test]
fn the_highlight_reverses_the_changed_characters() {
    let dir = tempfile::tempdir().expect("tempdir");
    let count = dir.path().join("count").display().to_string();
    let script = format!(
        "echo steady-anchor-line; echo x >> {count}; n=$(wc -l < {count}); printf 'v%06d\\n' $n"
    );
    let session = PtySession::spawn(
        &rat_bin(),
        &["watch", "-n", "50ms", "--shell", "--", &script],
        &[],
    )
    .expect("spawn under a pty");
    let mut terminal = FakeTerminal::dark();

    assert!(
        wait_for(&session, &mut terminal, b"v0000", Duration::from_secs(2)),
        "expected a first counter frame"
    );
    session.write_bytes(b"c");
    let bytes = wait_for_bytes(&session, &mut terminal, b"\x1b[7m", Duration::from_secs(2))
        .expect("expected a reverse-video mark after c");
    let marked_row = row_containing(&bytes, b"\x1b[7m").expect("a marked row");
    assert!(
        contains(marked_row, b"v0"),
        "the mark must land on the changed row: {:?}",
        String::from_utf8_lossy(marked_row)
    );
    session.write_bytes(b"q");
    assert!(
        !session.kill_if_alive(Duration::from_secs(2)),
        "watch should have exited on q"
    );
}

#[test]
fn toggling_the_highlight_off_restores_plain_rows() {
    let dir = tempfile::tempdir().expect("tempdir");
    let count = dir.path().join("count").display().to_string();
    let script = format!("echo x >> {count}; n=$(wc -l < {count}); printf 'v%06d\\n' $n");
    let session = PtySession::spawn(
        &rat_bin(),
        &["watch", "-n", "50ms", "--shell", "--", &script],
        &[],
    )
    .expect("spawn under a pty");
    let mut terminal = FakeTerminal::dark();

    assert!(
        wait_for(&session, &mut terminal, b"v0000", Duration::from_secs(2)),
        "expected a first counter frame"
    );
    session.write_bytes(b"c");
    assert!(
        wait_for(&session, &mut terminal, b"\x1b[7m", Duration::from_secs(2)),
        "expected a reverse-video mark after c"
    );
    session.write_bytes(b"c");
    // Swallow the transition — a spliced frame may still be in flight.
    let _ = drain_for(&session, Duration::from_millis(400));
    let bytes = wait_for_bytes(&session, &mut terminal, b"v0", Duration::from_secs(2))
        .expect("expected a plain counter row after toggling off");
    assert!(
        !contains(&bytes, b"\x1b[7m"),
        "no reverse-video mark may survive the toggle: {:?}",
        String::from_utf8_lossy(&bytes)
    );
    session.write_bytes(b"q");
    assert!(
        !session.kill_if_alive(Duration::from_secs(2)),
        "watch should have exited on q"
    );
}

#[test]
fn both_markers_ride_together() {
    let dir = tempfile::tempdir().expect("tempdir");
    let count = dir.path().join("count").display().to_string();
    let script = format!(
        "echo steady-anchor-line; echo x >> {count}; n=$(wc -l < {count}); printf 'v%06d\\n' $n"
    );
    let session = PtySession::spawn(
        &rat_bin(),
        &["watch", "-n", "50ms", "--shell", "--", &script],
        &[],
    )
    .expect("spawn under a pty");
    let mut terminal = FakeTerminal::dark();

    assert!(
        wait_for(&session, &mut terminal, b"v0000", Duration::from_secs(2)),
        "expected a first counter frame"
    );
    session.write_bytes(b"D");
    assert!(
        wait_for(
            &session,
            &mut terminal,
            "▌".as_bytes(),
            Duration::from_secs(2)
        ),
        "expected a gutter mark after D"
    );
    session.write_bytes(b"c");
    let bytes = wait_for_bytes(&session, &mut terminal, b"\x1b[7m", Duration::from_secs(2))
        .expect("expected a reverse-video mark after c");
    let marked_row = row_containing(&bytes, b"\x1b[7m").expect("a marked row");
    assert!(
        contains(marked_row, "▌".as_bytes()) && contains(marked_row, b"v0"),
        "margin cell and in-content mark must coexist on the changed row: {:?}",
        String::from_utf8_lossy(marked_row)
    );
    session.write_bytes(b"q");
    assert!(
        !session.kill_if_alive(Duration::from_secs(2)),
        "watch should have exited on q"
    );
}

#[test]
fn the_highlight_survives_a_shift() {
    let dir = tempfile::tempdir().expect("tempdir");
    let count = dir.path().join("count").display().to_string();
    let script = format!(
        "echo 12345678-steady-remainder; echo x >> {count}; n=$(wc -l < {count}); \
         printf 'counter-mark v%06d\\n' $n"
    );
    let session = PtySession::spawn(
        &rat_bin(),
        &["watch", "-n", "50ms", "--shell", "--", &script],
        &[],
    )
    .expect("spawn under a pty");
    let mut terminal = FakeTerminal::dark();

    assert!(
        wait_for(&session, &mut terminal, b"v0000", Duration::from_secs(2)),
        "expected a first counter frame"
    );
    session.write_bytes(b"c");
    assert!(
        wait_for(&session, &mut terminal, b"\x1b[7m", Duration::from_secs(2)),
        "expected a reverse-video mark after c"
    );
    session.write_bytes(b"l");
    let bytes = wait_for_bytes(
        &session,
        &mut terminal,
        b"-steady-remainder",
        Duration::from_secs(2),
    )
    .expect("expected the shifted frame");
    let counter_row = row_containing(&bytes, b"v0").expect("a counter row");
    assert!(
        contains(counter_row, b"\x1b[7m"),
        "the splice must ride the chopped branch too: {:?}",
        String::from_utf8_lossy(counter_row)
    );
    session.write_bytes(b"q");
    assert!(
        !session.kill_if_alive(Duration::from_secs(2)),
        "watch should have exited on q"
    );
}

#[test]
fn no_color_keeps_the_highlight_out_of_the_bytes() {
    let dir = tempfile::tempdir().expect("tempdir");
    let count = dir.path().join("count").display().to_string();
    let script = format!("echo x >> {count}; n=$(wc -l < {count}); printf 'v%06d\\n' $n");
    let session = PtySession::spawn(
        &rat_bin(),
        &["watch", "-n", "50ms", "--shell", "--", &script],
        &[("NO_COLOR", "1")],
    )
    .expect("spawn under a pty");
    let mut terminal = FakeTerminal::dark();

    assert!(
        wait_for(&session, &mut terminal, b"v0000", Duration::from_secs(2)),
        "expected a first counter frame"
    );
    session.write_bytes(b"c");
    // Swallow the transition, then capture fresh frames: a profile that
    // forbids SGR gets none from the highlighter either.
    let _ = drain_for(&session, Duration::from_millis(400));
    let bytes = wait_for_bytes(&session, &mut terminal, b"v0", Duration::from_secs(2))
        .expect("expected counter rows under NO_COLOR");
    assert!(
        !contains(&bytes, b"\x1b[7m"),
        "the highlighter must stay silent under an ascii profile: {:?}",
        String::from_utf8_lossy(&bytes)
    );
    session.write_bytes(b"q");
    assert!(
        !session.kill_if_alive(Duration::from_secs(2)),
        "watch should have exited on q"
    );
}

#[test]
fn t_flips_the_live_row_and_back() {
    let dir = tempfile::tempdir().expect("tempdir");
    let count = dir.path().join("count").display().to_string();
    let script = format!("echo x >> {count}; n=$(wc -l < {count}); printf 'v%06d\\n' $n");
    let session = PtySession::spawn(
        &rat_bin(),
        &["watch", "-n", "50ms", "--shell", "--", &script],
        &[],
    )
    .expect("spawn under a pty");
    let mut terminal = FakeTerminal::dark();

    assert!(
        wait_for(&session, &mut terminal, b"since ", Duration::from_secs(2)),
        "expected the default absolute live row"
    );
    session.write_bytes(b"t");
    // A changing child re-arms the last-change clock every tick, so the
    // flipped row correctly pins inside the just-now grace.
    assert!(
        wait_for(
            &session,
            &mut terminal,
            b"changed just now",
            Duration::from_secs(2)
        ),
        "expected the counting live row after t"
    );
    session.write_bytes(b"t");
    assert!(
        wait_for(&session, &mut terminal, b"since ", Duration::from_secs(2)),
        "expected the absolute row back after a second t"
    );
    session.write_bytes(b"q");
    assert!(
        !session.kill_if_alive(Duration::from_secs(2)),
        "watch should have exited on q"
    );
}

#[test]
fn the_paused_row_shows_a_wall_clock_and_t_makes_it_count() {
    let dir = tempfile::tempdir().expect("tempdir");
    let count = dir.path().join("count").display().to_string();
    let script = format!("echo x >> {count}; n=$(wc -l < {count}); printf 'v%06d\\n' $n");
    let session = PtySession::spawn(
        &rat_bin(),
        &["watch", "-n", "50ms", "--shell", "--", &script],
        &[],
    )
    .expect("spawn under a pty");
    let mut terminal = FakeTerminal::dark();

    assert!(
        wait_for(&session, &mut terminal, b"v0000", Duration::from_secs(2)),
        "expected a first counter frame"
    );
    session.write_bytes(b"p");
    let needle = "paused · at ".as_bytes();
    let bytes = wait_for_bytes(&session, &mut terminal, needle, Duration::from_secs(2))
        .expect("expected the default wall-clock paused row");
    let pos = bytes
        .windows(needle.len())
        .rposition(|w| w == needle)
        .expect("needle position");
    let stamp = &bytes[pos + needle.len()..pos + needle.len() + 8];
    assert!(
        stamp[2] == b':'
            && stamp[5] == b':'
            && [0, 1, 3, 4, 6, 7]
                .iter()
                .all(|&i| stamp[i].is_ascii_digit()),
        "expected an HH:MM:SS stamp, got {:?}",
        String::from_utf8_lossy(stamp)
    );
    session.write_bytes(b"t");
    assert!(
        wait_for(
            &session,
            &mut terminal,
            "paused · just now".as_bytes(),
            Duration::from_secs(2)
        ),
        "expected the counting paused row after t"
    );
    session.write_bytes(b"q");
    assert!(
        !session.kill_if_alive(Duration::from_secs(2)),
        "watch should have exited on q"
    );
}

#[test]
fn the_flipped_live_row_counts() {
    // A STATIC child: the last-change clock stays put, so the flipped
    // row genuinely counts — through the ten-second just-now grace and
    // out the other side. An event-driven needle wait, not a drain.
    let session = PtySession::spawn(
        &rat_bin(),
        &[
            "watch",
            "-n",
            "200ms",
            "--shell",
            "--",
            "echo steady-static-line",
        ],
        &[],
    )
    .expect("spawn under a pty");
    let mut terminal = FakeTerminal::dark();

    assert!(
        wait_for(
            &session,
            &mut terminal,
            b"steady-static-line",
            Duration::from_secs(2)
        ),
        "expected the first frame"
    );
    session.write_bytes(b"t");
    assert!(
        wait_for(
            &session,
            &mut terminal,
            b"changed just now",
            Duration::from_secs(2)
        ),
        "expected the counting row inside the grace"
    );
    // age_text says "just now" through 9s and "10s ago" at ten, so
    // "changed 1" first appears at the plateau's edge.
    assert!(
        wait_for(
            &session,
            &mut terminal,
            b"changed 1",
            Duration::from_secs(15)
        ),
        "expected the count to emerge from the just-now plateau"
    );
    session.write_bytes(b"q");
    assert!(
        !session.kill_if_alive(Duration::from_secs(2)),
        "watch should have exited on q"
    );
}

#[test]
fn a_fifo_write_forces_a_fresh_run_mid_heartbeat() {
    use common::pty::{counter_cmd, mkfifo_at, wait_for_counter, write_fifo};

    // -n 1h parks the interval: reaching count 2 without waiting the
    // hour is the trigger's doing (child-side counter evidence).
    let dir = tempfile::tempdir().expect("tempdir");
    let fifo = dir.path().join("t.fifo");
    mkfifo_at(&fifo);
    let counter = dir.path().join("count");
    let session = PtySession::spawn(
        &rat_bin(),
        &[
            "watch",
            "-n",
            "1h",
            "--trigger",
            &format!("fifo:{}", fifo.display()),
            "--trigger-debounce",
            "0ms",
            "--shell",
            "--",
            &counter_cmd(&counter),
        ],
        &[],
    )
    .expect("spawn rat watch under a pty");
    let mut terminal = FakeTerminal::dark();
    assert!(
        wait_for(&session, &mut terminal, b"count-1", Duration::from_secs(5)),
        "the first tick never painted"
    );
    write_fifo(&fifo, b"go\n");
    assert!(
        wait_for(&session, &mut terminal, b"count-2", Duration::from_secs(5)),
        "the fifo write never forced a run"
    );
    wait_for_counter(&counter, 2);
    session.kill_if_alive(Duration::from_secs(2));
}

#[test]
fn burst_writes_inside_one_window_coalesce_to_one_run() {
    use common::pty::{
        assert_counter_settled_at, counter_cmd, mkfifo_at, wait_for_counter, write_fifo,
    };

    let dir = tempfile::tempdir().expect("tempdir");
    let fifo = dir.path().join("t.fifo");
    mkfifo_at(&fifo);
    let counter = dir.path().join("count");
    let session = PtySession::spawn(
        &rat_bin(),
        &[
            "watch",
            "-n",
            "1h",
            "--trigger",
            &format!("fifo:{}", fifo.display()),
            "--trigger-debounce",
            "400ms",
            "--shell",
            "--",
            &counter_cmd(&counter),
        ],
        &[],
    )
    .expect("spawn rat watch under a pty");
    let mut terminal = FakeTerminal::dark();
    assert!(
        wait_for(&session, &mut terminal, b"count-1", Duration::from_secs(5)),
        "the first tick never painted"
    );
    for _ in 0..5 {
        write_fifo(&fifo, b"x\n"); // five fires, one anchored window
    }
    wait_for_counter(&counter, 2);
    // The pin: the five fires collapse to exactly one extra run —
    // value-based on the counter file, not timing.
    assert_counter_settled_at(&counter, 2);
    session.kill_if_alive(Duration::from_secs(2));
}

#[test]
fn an_fd_source_ending_shows_one_notice_and_watch_keeps_ticking() {
    use common::pty::{counter_cmd, wait_for_counter};

    // Portable across both unix CI legs — macOS libc has NO pipe2, so
    // plain pipe + FD_CLOEXEC on the WRITE end only: the child inherits
    // the read end, never the write end, so closing w in the parent IS
    // the child-side EOF.
    let mut fds = [0i32; 2];
    assert_eq!(unsafe { libc::pipe(fds.as_mut_ptr()) }, 0, "pipe");
    let (r, w) = (fds[0], fds[1]);
    assert_ne!(
        unsafe { libc::fcntl(w, libc::F_SETFD, libc::FD_CLOEXEC) },
        -1,
        "cloexec on the write end"
    );
    let dir = tempfile::tempdir().expect("tempdir");
    let counter = dir.path().join("count");
    let session = PtySession::spawn(
        &rat_bin(),
        &[
            "watch",
            "-n",
            "2s",
            "--trigger",
            &format!("fd:{r}"),
            "--trigger-debounce",
            "0ms",
            "--shell",
            "--",
            &counter_cmd(&counter),
        ],
        &[],
    )
    .expect("spawn rat watch under a pty");
    // The parent's read-end copy is the child's now.
    unsafe { libc::close(r) };
    let mut terminal = FakeTerminal::dark();
    assert!(
        wait_for(&session, &mut terminal, b"count-1", Duration::from_secs(5)),
        "the first tick never painted"
    );
    // Closing the last write end is the child-side EOF: the source ends,
    // the one-shot notice appears, and the heartbeat keeps ticking.
    unsafe { libc::close(w) };
    assert!(
        wait_for(
            &session,
            &mut terminal,
            b"trigger ended: fd:",
            Duration::from_secs(5)
        ),
        "the end-of-life notice never appeared"
    );
    let before = std::fs::read_to_string(&counter)
        .map(|s| s.lines().count())
        .unwrap_or(0);
    wait_for_counter(&counter, before + 1); // rat did not exit
    // One-shot: the frames that follow the notice must not repeat it.
    let after = drain_for(&session, Duration::from_millis(600));
    assert!(
        !contains(&after, b"trigger ended"),
        "the notice repeated after its one-shot repaint"
    );
    session.kill_if_alive(Duration::from_secs(2));
}

#[test]
fn question_mark_pages_the_key_reference_before_the_first_frame() {
    // The reference is static text, so the one window where it used to
    // be unavailable — before the first frame — is exactly the window a
    // reader most wants it in. The marker file is child-side evidence
    // that the first child is running and nothing has been painted yet.
    let dir = tempfile::tempdir().expect("tempdir");
    let started = dir.path().join("started");
    let script = format!(": > {}; /bin/sleep 5; echo hi", started.display());
    let session = PtySession::spawn(
        &rat_bin(),
        &["watch", "-n", "10s", "--shell", "--", &script],
        &[("RAT_PAGER", "/bin/cat")],
    )
    .expect("spawn under a pty");
    let mut terminal = FakeTerminal::dark();

    let deadline = std::time::Instant::now() + Duration::from_secs(3);
    while !started.exists() {
        // Keep answering the startup appearance probe while polling —
        // the first tick waits behind it.
        let chunk = session.read_available(Duration::from_millis(20));
        terminal.respond(&session, &chunk);
        assert!(
            std::time::Instant::now() < deadline,
            "the child never started"
        );
    }
    // The park handshake is ALLOWED to refuse when the reader thread is
    // starved: the arm answers "pager unavailable … try again" by
    // design. Honor that the way a user would — press ? again — on a
    // TIMER, never on the notice text: before the first frame a refusal
    // has no frame to paint the notice into, so it writes nothing at
    // all.
    session.write_bytes(b"?");
    let mut seen: Vec<u8> = Vec::new();
    let mut presses = 1;
    let deadline = std::time::Instant::now() + Duration::from_secs(15);
    let mut next_retry = std::time::Instant::now() + Duration::from_secs(2);
    loop {
        assert!(
            std::time::Instant::now() < deadline,
            "expected the key reference before any frame (after {presses} presses)"
        );
        let chunk = session.read_available(Duration::from_millis(50));
        terminal.respond(&session, &chunk);
        seen.extend_from_slice(&chunk);
        if contains(&seen, b"freeze the frame in place") {
            break;
        }
        if std::time::Instant::now() >= next_retry && presses < 6 {
            presses += 1;
            session.write_bytes(b"?");
            next_retry = std::time::Instant::now() + Duration::from_secs(2);
        }
    }
    session.write_bytes(b"q");
    assert!(
        !session.kill_if_alive(Duration::from_secs(2)),
        "watch should have exited on q"
    );
}

#[test]
fn the_status_row_never_wears_the_childs_open_color() {
    // The child opens red and never closes it. Rows join on bare CRLF,
    // so without a seal the faint status row (CSI 2m adds an attribute,
    // clears nothing) paints faint AND red. The body row must end
    // closed; the child's bytes still arrive first, verbatim.
    let session = PtySession::spawn(
        &rat_bin(),
        &[
            "watch",
            "-n",
            "100ms",
            "--",
            "/usr/bin/printf",
            r"\033[31mred",
        ],
        &[],
    )
    .expect("spawn under a pty");
    let mut terminal = FakeTerminal::dark();
    let seen = wait_for_bytes(&session, &mut terminal, b"? help", Duration::from_secs(3))
        .expect("expected a first frame with the status row");
    let text = String::from_utf8_lossy(&seen);
    assert!(
        text.contains("\x1b[31mred\x1b[0m"),
        "the body row must close what the child left open: {text:?}"
    );
    assert!(
        !text.contains("\x1b[31mred\r\n"),
        "no row may hand its open color to the next: {text:?}"
    );
    session.write_bytes(b"q");
    assert!(
        !session.kill_if_alive(Duration::from_secs(2)),
        "watch should have exited on q"
    );
}

#[test]
fn a_scrub_forward_exit_keeps_the_scroll_position() {
    // 30 static lines on a 24-row pty: a 22-row window. Freeze, scroll a
    // line down, then step past the newest: the exit must land in the
    // scrolled LIVE view at the same offset — a step keeps your place,
    // only the homing keys (Esc/F) reset to the top.
    let lines: Vec<String> = (1..=30).map(|i| format!("l{i}")).collect();
    let rat = rat_bin();
    let mut args: Vec<&str> = vec!["watch", "-n", "50ms", "--", &rat, "style"];
    args.extend(lines.iter().map(String::as_str));
    let session = PtySession::spawn(&rat_bin(), &args, &[]).expect("spawn under a pty");
    let mut terminal = FakeTerminal::dark();

    assert!(
        wait_for(
            &session,
            &mut terminal,
            "· ? help".as_bytes(),
            Duration::from_secs(2)
        ),
        "expected the live truncation notice"
    );
    session.write_bytes(b"p");
    assert!(
        wait_for(
            &session,
            &mut terminal,
            "· lines 1-22 of 30 · Esc resumes".as_bytes(),
            Duration::from_secs(2)
        ),
        "expected the frozen window at the top"
    );
    session.write_bytes(b"j");
    assert!(
        wait_for(
            &session,
            &mut terminal,
            "· lines 2-23 of 30 · Esc resumes".as_bytes(),
            Duration::from_secs(2)
        ),
        "expected the paused row one line down"
    );
    session.write_bytes(b">");
    assert!(
        wait_for_without(
            &session,
            &mut terminal,
            "lines 2-23 of 30 · every 50ms · ? help".as_bytes(),
            b"paused",
            Duration::from_secs(2)
        ),
        "expected the exit to keep the offset in the live view"
    );
    session.write_bytes(b"q");
    assert!(
        !session.kill_if_alive(Duration::from_secs(2)),
        "watch should have exited on q"
    );
}

#[test]
fn a_scrolled_counting_row_keeps_counting() {
    // The displayed-age gate regression: with t flipped, the scrolled
    // row carries the counting form and must keep advancing — a gate
    // that treats the scrolled row as time-less paints the text once
    // and stalls, wrong on a running terminal while passing any
    // single-snapshot test. The counting text leaves its "just now"
    // plateau at ten seconds; "s ago" on the wire after the scroll is
    // proof the gate repainted a scrolled frame.
    let lines: Vec<String> = (1..=30).map(|i| format!("l{i}")).collect();
    let rat = rat_bin();
    let mut args: Vec<&str> = vec!["watch", "-n", "1h", "--", &rat, "style"];
    args.extend(lines.iter().map(String::as_str));
    let session = PtySession::spawn(&rat, &args, &[]).expect("spawn under a pty");
    let mut terminal = FakeTerminal::dark();

    assert!(
        wait_for(
            &session,
            &mut terminal,
            "· ? help".as_bytes(),
            Duration::from_secs(2)
        ),
        "expected the live frame"
    );
    session.write_bytes(b"t");
    assert!(
        wait_for(&session, &mut terminal, b"changed ", Duration::from_secs(2)),
        "expected the counting live row"
    );
    session.write_bytes(b"j");
    let seen = wait_for_bytes(
        &session,
        &mut terminal,
        "live · changed ".as_bytes(),
        Duration::from_secs(2),
    )
    .expect("expected the scrolled row to carry the counting stamp");
    assert!(
        contains(&seen, b"lines 2-23 of 30"),
        "the range still names the window: {:?}",
        String::from_utf8_lossy(&seen)
    );
    assert!(
        wait_for(&session, &mut terminal, b"s ago", Duration::from_secs(15)),
        "expected the scrolled counting age to advance past the plateau"
    );
    session.write_bytes(b"q");
    assert!(
        !session.kill_if_alive(Duration::from_secs(2)),
        "watch should have exited on q"
    );
}

#[test]
fn a_bars_advance_recolors_ink_instead_of_reversing_it() {
    // Reverse video on a full block draws the cell in the background
    // color — the bar reads LESS at the moment it advances. A changed
    // block glyph must take the bold+accent ink treatment while plain
    // text keeps today's reverse.
    let dir = tempfile::tempdir().expect("tempdir");
    let value = dir.path().join("value");
    std::fs::write(&value, "██░░ 2/4").expect("seed");
    let script = format!("cat {}", value.display());
    let session = PtySession::spawn(
        &rat_bin(),
        &["watch", "-n", "100ms", "--shell", "--", &script],
        &[],
    )
    .expect("spawn under a pty");
    let mut terminal = FakeTerminal::dark();

    assert!(
        wait_for(&session, &mut terminal, b"2/4", Duration::from_secs(2)),
        "expected the first bar frame"
    );
    session.write_bytes(b"c");
    let _ = drain_for(&session, Duration::from_millis(100));
    std::fs::write(&value, "███░ 3/4").expect("the bar advances");
    // The digit's reverse is the positive control: highlights were
    // live on the exact repaint that carried the block change.
    let seen = wait_for_bytes(&session, &mut terminal, b"\x1b[7m3", Duration::from_secs(3))
        .expect("the changed digit never got its reverse highlight");
    assert!(
        contains(&seen, b"\x1b[1;38"),
        "the changed block must open the bold+accent ink: {:?}",
        String::from_utf8_lossy(&seen)
    );
    assert!(
        !contains(&seen, "\x1b[7m█".as_bytes()),
        "a block glyph must never be reversed: {:?}",
        String::from_utf8_lossy(&seen)
    );
    session.write_bytes(b"q");
    assert!(
        !session.kill_if_alive(Duration::from_secs(2)),
        "watch should have exited on q"
    );
}

fn position(haystack: &[u8], needle: &[u8]) -> Option<usize> {
    haystack.windows(needle.len()).position(|w| w == needle)
}

#[test]
fn fullscreen_takes_and_returns_the_alternate_screen() {
    // The contract: the alternate screen is entered before the first
    // frame and left on exit, so the user's screen comes back exactly
    // as it was and the frames never touch scrollback.
    let session = PtySession::spawn(
        &rat_bin(),
        &[
            "watch",
            "--fullscreen",
            "-n",
            "1s",
            "--",
            &rat_bin(),
            "style",
            "hi",
        ],
        &[],
    )
    .expect("spawn under a pty");
    let mut terminal = FakeTerminal::dark();
    let seen = wait_for_bytes(&session, &mut terminal, b"hi", Duration::from_secs(2))
        .expect("expected the first fullscreen frame");
    let enter = position(&seen, b"\x1b[?1049h").expect("the alternate screen must be entered");
    let first_frame = position(&seen, b"\x1b[?2026h").expect("the first synchronized frame");
    assert!(
        enter < first_frame,
        "the alternate screen must be entered before the first frame"
    );
    session.write_bytes(b"q");
    let tail = drain_for(&session, Duration::from_millis(600));
    assert!(
        contains(&tail, b"\x1b[?1049l"),
        "q must leave the alternate screen: {:?}",
        String::from_utf8_lossy(&tail)
    );
    assert!(
        !session.kill_if_alive(Duration::from_secs(2)),
        "watch should have exited on q"
    );
}

#[test]
fn the_fullscreen_pager_round_trip_re_enters_the_alternate_screen() {
    // `?1049` is not a nesting counter: rat must leave the alternate
    // screen before the pager (which may take it itself) and re-enter
    // afterward — and the re-entered buffer is blank, so the frame
    // must repaint from scratch rather than resuming over a copy that
    // is no longer there.
    let session = PtySession::spawn(
        &rat_bin(),
        &[
            "watch",
            "--fullscreen",
            "-n",
            "50ms",
            "--",
            &rat_bin(),
            "style",
            "hi",
        ],
        &[("RAT_PAGER", "/bin/cat")],
    )
    .expect("spawn under a pty");
    let mut terminal = FakeTerminal::dark();
    assert!(
        wait_for(&session, &mut terminal, b"hi", Duration::from_secs(2)),
        "expected the first fullscreen frame"
    );
    let _ = drain_for(&session, Duration::from_millis(100));
    session.write_bytes(b"v");
    let seen = wait_for_bytes(
        &session,
        &mut terminal,
        b"\x1b[?1049h",
        Duration::from_secs(3),
    )
    .expect("rat must re-enter the alternate screen after the pager");
    assert!(
        position(&seen, b"\x1b[?1049l").is_some(),
        "rat must leave the alternate screen before the pager: {:?}",
        String::from_utf8_lossy(&seen)
    );
    assert!(
        wait_for(&session, &mut terminal, b"hi", Duration::from_secs(2)),
        "the frame must repaint onto the blank re-entered screen"
    );
    session.write_bytes(b"q");
    assert!(
        !session.kill_if_alive(Duration::from_secs(2)),
        "watch should have exited on q"
    );
}

#[test]
fn the_wheel_scrolls_only_when_the_mouse_is_captured() {
    // Opt-in, both arms. Without --mouse the reports never reach the
    // wire (nothing asked for them), but a terminal misbehaving must
    // still not scroll: the same bytes change nothing. With --mouse,
    // one notch opens a three-line live window.
    let lines: Vec<String> = (1..=30).map(|i| format!("l{i}")).collect();
    let rat = rat_bin();
    let mut args: Vec<&str> = vec!["watch", "-n", "50ms", "--", &rat, "style"];
    args.extend(lines.iter().map(String::as_str));
    let session = PtySession::spawn(&rat, &args, &[]).expect("spawn under a pty");
    let mut terminal = FakeTerminal::dark();
    assert!(
        wait_for(
            &session,
            &mut terminal,
            "· ? help".as_bytes(),
            Duration::from_secs(2)
        ),
        "expected the live frame"
    );
    session.write_bytes(b"\x1b[<65;10;5M");
    let noop = drain_for(&session, Duration::from_millis(400));
    assert!(
        !contains(&noop, "live · ".as_bytes()),
        "an uncaptured wheel must not scroll: {:?}",
        String::from_utf8_lossy(&noop)
    );
    session.write_bytes(b"q");
    assert!(!session.kill_if_alive(Duration::from_secs(2)));

    let mut args: Vec<&str> = vec!["watch", "--mouse", "-n", "50ms", "--", &rat, "style"];
    args.extend(lines.iter().map(String::as_str));
    let session = PtySession::spawn(&rat, &args, &[]).expect("spawn under a pty");
    let mut terminal = FakeTerminal::dark();
    let seen = wait_for_bytes(
        &session,
        &mut terminal,
        "· ? help".as_bytes(),
        Duration::from_secs(2),
    )
    .expect("expected the live frame");
    assert!(
        contains(&seen, b"\x1b[?1000h") && contains(&seen, b"\x1b[?1006h"),
        "capture must be requested in the SGR encoding: {:?}",
        String::from_utf8_lossy(&seen)
    );
    session.write_bytes(b"\x1b[<65;10;5M");
    assert!(
        wait_for_without(
            &session,
            &mut terminal,
            "lines 4-25 of 30 · every 50ms · ? help".as_bytes(),
            b"paused",
            Duration::from_secs(2)
        ),
        "one notch must scroll three lines, live"
    );
    session.write_bytes(b"q");
    let tail = drain_for(&session, Duration::from_millis(600));
    assert!(
        contains(&tail, b"\x1b[?1000l"),
        "exit must release the mouse: {:?}",
        String::from_utf8_lossy(&tail)
    );
    assert!(!session.kill_if_alive(Duration::from_secs(2)));
}

#[test]
fn an_x10_click_payload_never_quits_the_session() {
    // A terminal honoring 1000 but not 1006 falls back to X10: three
    // raw payload bytes after ESC [ M. A click near column 81 puts a
    // literal `q` on the wire — the session must survive it.
    let dir = tempfile::tempdir().expect("tempdir");
    let count = dir.path().join("count").display().to_string();
    let script = format!("echo x >> {count}; n=$(wc -l < {count}); printf 'v%06d\\n' $n");
    let session = PtySession::spawn(
        &rat_bin(),
        &["watch", "--mouse", "-n", "100ms", "--shell", "--", &script],
        &[],
    )
    .expect("spawn under a pty");
    let mut terminal = FakeTerminal::dark();
    assert!(
        wait_for(&session, &mut terminal, b"v0000", Duration::from_secs(2)),
        "expected a first counter frame"
    );
    session.write_bytes(b"\x1b[M q!");
    // The counter keeps advancing: the payload's q did not quit.
    assert!(
        wait_for(&session, &mut terminal, b"v000004", Duration::from_secs(3)),
        "the session must keep running after an X10 payload"
    );
    session.write_bytes(b"q");
    assert!(!session.kill_if_alive(Duration::from_secs(2)));
}

#[test]
fn m_hands_the_mouse_back_and_takes_it_again() {
    let session = PtySession::spawn(
        &rat_bin(),
        &[
            "watch",
            "--mouse",
            "-n",
            "1s",
            "--",
            &rat_bin(),
            "style",
            "hi",
        ],
        &[],
    )
    .expect("spawn under a pty");
    let mut terminal = FakeTerminal::dark();
    assert!(
        wait_for(&session, &mut terminal, b"hi", Duration::from_secs(2)),
        "expected the first frame"
    );
    session.write_bytes(b"m");
    let seen = wait_for_bytes(
        &session,
        &mut terminal,
        b"mouse released",
        Duration::from_secs(2),
    )
    .expect("m must release the mouse and say so");
    assert!(
        contains(&seen, b"\x1b[?1000l"),
        "the release must reach the wire: {:?}",
        String::from_utf8_lossy(&seen)
    );
    session.write_bytes(b"m");
    let seen = wait_for_bytes(
        &session,
        &mut terminal,
        b"mouse captured",
        Duration::from_secs(2),
    )
    .expect("m must recapture and say so");
    assert!(
        contains(&seen, b"\x1b[?1000h"),
        "the recapture must reach the wire: {:?}",
        String::from_utf8_lossy(&seen)
    );
    session.write_bytes(b"q");
    assert!(!session.kill_if_alive(Duration::from_secs(2)));
}

#[test]
fn the_pager_round_trip_releases_and_recaptures_the_mouse() {
    let session = PtySession::spawn(
        &rat_bin(),
        &[
            "watch",
            "--mouse",
            "-n",
            "50ms",
            "--",
            &rat_bin(),
            "style",
            "hi",
        ],
        &[("RAT_PAGER", "/bin/cat")],
    )
    .expect("spawn under a pty");
    let mut terminal = FakeTerminal::dark();
    assert!(
        wait_for(&session, &mut terminal, b"hi", Duration::from_secs(2)),
        "expected the first frame"
    );
    let _ = drain_for(&session, Duration::from_millis(100));
    session.write_bytes(b"v");
    let seen = wait_for_bytes(
        &session,
        &mut terminal,
        b"\x1b[?1000h",
        Duration::from_secs(3),
    )
    .expect("the pager return must recapture the mouse");
    assert!(
        position(&seen, b"\x1b[?1000l").is_some(),
        "the pager handoff must release the mouse first: {:?}",
        String::from_utf8_lossy(&seen)
    );
    session.write_bytes(b"q");
    assert!(!session.kill_if_alive(Duration::from_secs(2)));
}

// ---------------------------------------------------------------------------
// --append: the linear stream (plan-fixed contracts; the mode appends
// distinct frames as plain rows and never rewrites one).

/// Every escape the in-place engine uses; none may reach an append
/// session's stream.
const APPEND_FORBIDDEN: [&[u8]; 6] = [
    b"\x1b[?25l",  // hide cursor
    b"\x1b[0J",    // erase below
    b"\x1b[2K",    // erase line
    b"\x1b[?2026", // synchronized output
    b"\x1b[1A",    // cursor up
    b"\x1b[?1049", // alternate screen
];

fn assert_no_forbidden_escapes(bytes: &[u8], context: &str) {
    for escape in APPEND_FORBIDDEN {
        assert!(
            !contains(bytes, escape),
            "{context}: found {:?} in {:?}",
            String::from_utf8_lossy(escape),
            String::from_utf8_lossy(bytes)
        );
    }
}

#[test]
fn append_writes_plain_rows_and_never_moves_the_cursor() {
    let rat = rat_bin();
    let session = PtySession::spawn(
        &rat,
        &["watch", "-n", "1", "--append", "--", &rat, "style", "hi"],
        &[("RAT_APPEARANCE", "dark"), ("NO_COLOR", "1")],
    )
    .expect("spawn");
    let mut terminal = FakeTerminal::dark();
    let seen = wait_for_bytes(&session, &mut terminal, b"hi", Duration::from_secs(5))
        .expect("first frame");
    assert_no_forbidden_escapes(&seen, "first frame");
    session.write_bytes(b"q");
    assert!(!session.kill_if_alive(Duration::from_secs(2)));
}

#[test]
fn the_appended_rows_end_crlf() {
    let rat = rat_bin();
    let session = PtySession::spawn(
        &rat,
        &["watch", "-n", "1", "--append", "--", &rat, "style", "hi"],
        &[("RAT_APPEARANCE", "dark"), ("NO_COLOR", "1")],
    )
    .expect("spawn");
    let mut terminal = FakeTerminal::dark();
    let seen = wait_for_bytes(&session, &mut terminal, b"hi\r\n", Duration::from_secs(5))
        .expect("raw mode clears ONLCR, so the row must carry its own CRLF");
    assert_no_forbidden_escapes(&seen, "crlf frame");
    session.write_bytes(b"q");
    assert!(!session.kill_if_alive(Duration::from_secs(2)));
}

#[test]
fn an_identical_frame_appends_nothing() {
    let dir = tempfile::tempdir().expect("tempdir");
    let count = dir.path().join("count");
    let script = format!("echo run >> {c}; echo steady", c = count.display());
    let rat = rat_bin();
    let session = PtySession::spawn(
        &rat,
        &["watch", "-n", "1", "--append", "--shell", "--", &script],
        &[("RAT_APPEARANCE", "dark"), ("NO_COLOR", "1")],
    )
    .expect("spawn");
    let mut terminal = FakeTerminal::dark();
    let first = wait_for_bytes(&session, &mut terminal, b"steady", Duration::from_secs(5))
        .expect("first frame");
    assert_no_forbidden_escapes(&first, "first frame");
    // The child keeps running (counter is the evidence) while the
    // byte-identical frames write nothing.
    common::pty::wait_for_counter(&count, 3);
    let extra = drain_for(&session, Duration::from_millis(400));
    assert!(
        extra.is_empty(),
        "an unchanged frame appended bytes: {:?}",
        String::from_utf8_lossy(&extra)
    );
    session.write_bytes(b"q");
    assert!(!session.kill_if_alive(Duration::from_secs(2)));
}

#[test]
fn a_changed_frame_appends_the_whole_frame_in_order() {
    let dir = tempfile::tempdir().expect("tempdir");
    let count = dir.path().join("count");
    let script = format!(
        "echo run >> {c}; n=$(wc -l < {c} | tr -d ' '); echo head-$n; echo alpha; echo omega",
        c = count.display()
    );
    let rat = rat_bin();
    let session = PtySession::spawn(
        &rat,
        &["watch", "-n", "1", "--append", "--shell", "--", &script],
        &[("RAT_APPEARANCE", "dark"), ("NO_COLOR", "1")],
    )
    .expect("spawn");
    let mut terminal = FakeTerminal::dark();
    wait_for(&session, &mut terminal, b"head-1", Duration::from_secs(5));
    // The SECOND burst arrives whole and ordered — whole-frame
    // appending is the settled shape. One accumulate loop: a burst can
    // land in one chunk, so sequential waits could eat their own needle.
    let deadline = std::time::Instant::now() + Duration::from_secs(6);
    let mut seen: Vec<u8> = Vec::new();
    loop {
        assert!(
            std::time::Instant::now() < deadline,
            "no ordered second burst in {:?}",
            String::from_utf8_lossy(&seen)
        );
        let chunk = session.read_available(Duration::from_millis(50));
        if chunk.is_empty() {
            continue;
        }
        terminal.respond(&session, &chunk);
        seen.extend_from_slice(&chunk);
        if let Some(head) = find_at(&seen, b"head-2")
            && let Some(alpha) = find_at(&seen[head..], b"alpha")
            && find_at(&seen[head + alpha..], b"omega").is_some()
        {
            break;
        }
    }
    session.write_bytes(b"q");
    assert!(!session.kill_if_alive(Duration::from_secs(2)));
}

#[test]
fn a_resize_appends_nothing() {
    let dir = tempfile::tempdir().expect("tempdir");
    let count = dir.path().join("count");
    let script = format!("echo run >> {c}; echo steady", c = count.display());
    let rat = rat_bin();
    let session = PtySession::spawn(
        &rat,
        &["watch", "-n", "1", "--append", "--shell", "--", &script],
        &[("RAT_APPEARANCE", "dark"), ("NO_COLOR", "1")],
    )
    .expect("spawn");
    let mut terminal = FakeTerminal::dark();
    wait_for(&session, &mut terminal, b"steady", Duration::from_secs(5));
    let before = std::fs::read_to_string(&count)
        .map(|s| s.lines().count())
        .unwrap_or(0);
    // The gate is CONTENT, not a PaintKey: a resize moves the key's
    // cols/rows and must not re-announce an unchanged frame.
    session.set_winsize(30, 100);
    common::pty::wait_for_counter(&count, before + 2);
    let extra = drain_for(&session, Duration::from_millis(400));
    assert!(
        extra.is_empty(),
        "a resize re-announced the frame: {:?}",
        String::from_utf8_lossy(&extra)
    );
    session.write_bytes(b"q");
    assert!(!session.kill_if_alive(Duration::from_secs(2)));
}

#[test]
fn an_open_color_never_leaks_past_an_appended_row() {
    let rat = rat_bin();
    let session = PtySession::spawn(
        &rat,
        &[
            "watch",
            "-n",
            "1",
            "--append",
            "--shell",
            "--",
            "printf '\\033[31mred\\n'",
        ],
        &[("RAT_APPEARANCE", "dark")],
    )
    .expect("spawn");
    let mut terminal = FakeTerminal::dark();
    let seen =
        wait_for_bytes(&session, &mut terminal, b"red", Duration::from_secs(5)).expect("frame");
    assert!(
        contains(&seen, b"red\x1b[0m"),
        "the child's open SGR must close before the row ends: {:?}",
        String::from_utf8_lossy(&seen)
    );
    assert_no_forbidden_escapes(&seen, "sealed frame");
    session.write_bytes(b"q");
    assert!(!session.kill_if_alive(Duration::from_secs(2)));
}

#[test]
fn a_flooding_child_says_so_on_its_own_row() {
    let rat = rat_bin();
    let session = PtySession::spawn(
        &rat,
        &[
            "watch", "-n", "5", "--append", "--", &rat, "__lines", "3000",
        ],
        &[("RAT_APPEARANCE", "dark"), ("NO_COLOR", "1")],
    )
    .expect("spawn");
    let mut terminal = FakeTerminal::dark();
    let seen = wait_for_bytes(
        &session,
        &mut terminal,
        b"\r\nrat watch: 2.0k lines dropped; kept the last 1000\r\n",
        Duration::from_secs(10),
    );
    assert!(
        seen.is_some(),
        "the drop marker must arrive as its own clean appended row"
    );
    session.write_bytes(b"q");
    assert!(!session.kill_if_alive(Duration::from_secs(2)));
}

#[test]
fn append_once_prints_one_frame_and_exits() {
    let rat = rat_bin();
    let session = PtySession::spawn(
        &rat,
        &["watch", "--once", "--append", "--", &rat, "__lines", "3"],
        &[("RAT_APPEARANCE", "dark"), ("NO_COLOR", "1")],
    )
    .expect("spawn");
    let mut terminal = FakeTerminal::dark();
    // ONLCR is on (no raw mode under --once), so the pty presents LF
    // rows as CRLF at the master either way.
    let seen = wait_for_bytes(&session, &mut terminal, b"2\r\n", Duration::from_secs(5))
        .expect("the once frame");
    assert!(
        contains(&seen, b"0\r\n1\r\n2\r\n"),
        "the whole frame, in order"
    );
    assert!(
        !contains(&seen, b"appending"),
        "--once prints no banner: {:?}",
        String::from_utf8_lossy(&seen)
    );
    assert_no_forbidden_escapes(&seen, "once frame");
    let deadline = std::time::Instant::now() + Duration::from_secs(5);
    while !session.exited() {
        assert!(std::time::Instant::now() < deadline, "--once must exit");
        std::thread::sleep(Duration::from_millis(20));
    }
}

#[test]
fn the_viewport_keys_append_nothing() {
    let dir = tempfile::tempdir().expect("tempdir");
    let count = dir.path().join("count");
    let script = format!("echo run >> {c}; echo steady", c = count.display());
    let rat = rat_bin();
    let session = PtySession::spawn(
        &rat,
        &["watch", "-n", "1", "--append", "--shell", "--", &script],
        &[("RAT_APPEARANCE", "dark"), ("NO_COLOR", "1")],
    )
    .expect("spawn");
    let mut terminal = FakeTerminal::dark();
    wait_for(&session, &mut terminal, b"steady", Duration::from_secs(5));
    // Every viewport key, including `t` — the counting-footer metronome
    // must be unarmable here.
    session.write_bytes(b"jGptv<");
    let before = std::fs::read_to_string(&count)
        .map(|s| s.lines().count())
        .unwrap_or(0);
    common::pty::wait_for_counter(&count, before + 2);
    let extra = drain_for(&session, Duration::from_millis(400));
    assert!(
        extra.is_empty(),
        "a viewport key appended bytes: {:?}",
        String::from_utf8_lossy(&extra)
    );
    session.write_bytes(b"q");
    assert!(!session.kill_if_alive(Duration::from_secs(2)));
}

#[test]
fn question_mark_appends_the_key_reference() {
    let rat = rat_bin();
    let session = PtySession::spawn(
        &rat,
        &["watch", "-n", "1", "--append", "--", &rat, "style", "hi"],
        &[("RAT_APPEARANCE", "dark"), ("NO_COLOR", "1")],
    )
    .expect("spawn");
    let mut terminal = FakeTerminal::dark();
    wait_for(&session, &mut terminal, b"hi", Duration::from_secs(5));
    session.write_bytes(b"?");
    let seen = wait_for_bytes(
        &session,
        &mut terminal,
        b"rat watch --append \xe2\x80\x94 keys",
        Duration::from_secs(5),
    )
    .expect("the appended reference");
    let more = wait_for_bytes(
        &session,
        &mut terminal,
        b"q                  quit",
        Duration::from_secs(2),
    );
    assert!(
        more.is_some() || contains(&seen, b"q                  quit"),
        "the reference names the way out"
    );
    assert!(
        !contains(&seen, b"\x1b[?1049"),
        "the reference APPENDS; the pager's alternate screen must not open"
    );
    session.write_bytes(b"q");
    assert!(!session.kill_if_alive(Duration::from_secs(2)));
}

#[test]
fn s_appends_the_snapshot_path() {
    let dir = tempfile::tempdir().expect("tempdir");
    let rat = rat_bin();
    let session = PtySession::spawn(
        &rat,
        &["watch", "-n", "1", "--append", "--", &rat, "style", "hi"],
        &[
            ("RAT_APPEARANCE", "dark"),
            ("NO_COLOR", "1"),
            ("RAT_SNAPSHOT_DIR", &dir.path().display().to_string()),
        ],
    )
    .expect("spawn");
    let mut terminal = FakeTerminal::dark();
    wait_for(&session, &mut terminal, b"hi", Duration::from_secs(5));
    session.write_bytes(b"S");
    let seen = wait_for_bytes(
        &session,
        &mut terminal,
        b"rat watch: snapshot \xe2\x86\x92 ",
        Duration::from_secs(5),
    );
    assert!(seen.is_some(), "the snapshot notice appends as its own row");
    let snapshots = std::fs::read_dir(dir.path()).expect("read dir").count();
    assert_eq!(snapshots, 1, "one snapshot file lands in the directory");
    session.write_bytes(b"q");
    assert!(!session.kill_if_alive(Duration::from_secs(2)));
}

#[test]
fn q_quits_and_ctrl_c_aborts_an_append_session() {
    let rat = rat_bin();
    for key in [&b"q"[..], &b"\x03"[..]] {
        let session = PtySession::spawn(
            &rat,
            &["watch", "-n", "1", "--append", "--", &rat, "style", "hi"],
            &[("RAT_APPEARANCE", "dark"), ("NO_COLOR", "1")],
        )
        .expect("spawn");
        let mut terminal = FakeTerminal::dark();
        wait_for(&session, &mut terminal, b"hi", Duration::from_secs(5));
        session.write_bytes(key);
        assert!(
            !session.kill_if_alive(Duration::from_secs(2)),
            "{key:?} must end the session"
        );
    }
}

#[test]
fn the_banner_names_the_cadence_and_the_way_out() {
    let rat = rat_bin();
    let session = PtySession::spawn(
        &rat,
        &["watch", "-n", "1", "--append", "--", &rat, "style", "hi"],
        &[("RAT_APPEARANCE", "dark"), ("NO_COLOR", "1")],
    )
    .expect("spawn");
    let mut terminal = FakeTerminal::dark();
    let seen = wait_for_bytes(&session, &mut terminal, b"hi", Duration::from_secs(5))
        .expect("first frame");
    let banner = find_at(&seen, b"rat watch: appending").expect("the banner");
    let frame = find_at(&seen, b"hi").expect("the frame");
    assert!(
        banner < frame,
        "the banner leads, once, before the first frame"
    );
    assert!(
        contains(
            &seen,
            b"rat watch: appending \xc2\xb7 every 1 \xc2\xb7 ? help"
        ),
        "the banner carries the cadence and the way out: {:?}",
        String::from_utf8_lossy(&seen)
    );
    let tail = &seen[banner + 1..];
    assert!(
        find_at(tail, b"rat watch: appending").is_none(),
        "the banner never repeats"
    );
    session.write_bytes(b"q");
    assert!(!session.kill_if_alive(Duration::from_secs(2)));
}
