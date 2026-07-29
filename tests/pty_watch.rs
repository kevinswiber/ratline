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
        !contains(&noop, "· live".as_bytes()),
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
            "lines 2-23 of 46 · live".as_bytes(),
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
            "lines 2-23 of 30 · live".as_bytes(),
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
            "· live".as_bytes(),
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
            "lines 2-23 of 30 · live".as_bytes(),
            b"paused",
            Duration::from_secs(2)
        ),
        "expected the frame to live-scroll"
    );

    std::fs::write(&grow, b"").expect("create the grow file");
    let seen = wait_for_bytes(
        &session,
        &mut terminal,
        "of 31 · live".as_bytes(),
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
            "· live".as_bytes(),
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
        !contains(&noop, "· live".as_bytes()),
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

    assert!(
        wait_for(&session, &mut terminal, b"v0000", Duration::from_secs(2)),
        "expected a first counter frame"
    );
    // Let history accrue distinct frames.
    let _ = drain_for(&session, Duration::from_millis(400));

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
fn scrub_forward_at_the_newest_is_a_no_op() {
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
    session.write_bytes(b">");
    let leaked = session.read_available(Duration::from_millis(400));
    assert!(
        leaked.is_empty(),
        "> at the newest entry must be a no-op: {:?}",
        String::from_utf8_lossy(&leaked)
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
    assert!(
        wait_for(&session, &mut terminal, b"v000003", Duration::from_secs(3)),
        "expected the counter to reach three"
    );
    // The frozen copy is always the last value PAINTED: keep everything
    // seen up to the freeze row, since a freeze whose copy matches the
    // screen rewrites only the status row.
    let mut all = drain_for(&session, Duration::from_millis(300));
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
            "· live".as_bytes(),
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
    session.write_bytes(b"?");
    assert!(
        wait_for(
            &session,
            &mut terminal,
            b"freeze the frame in place",
            Duration::from_secs(2)
        ),
        "expected the key reference in the pager"
    );
    assert!(
        wait_for(&session, &mut terminal, b"hi", Duration::from_secs(2)),
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
