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
            "paused · just now ·".as_bytes(),
            Duration::from_secs(2)
        ),
        "expected the paused row after the freeze key"
    );
    // Drain the freeze paint. While the age still reads "just now" the
    // per-second status repaint is byte-identical, so the differ writes
    // nothing: a parked frame is byte-silent through the plateau even
    // though the child keeps changing behind it.
    let _ = drain_for(&session, Duration::from_millis(300));
    let leaked = session.read_available(Duration::from_millis(1500));
    assert!(
        leaked.is_empty(),
        "the tail kept painting while frozen: {:?}",
        String::from_utf8_lossy(&leaked)
    );

    // At ten seconds the age starts counting: a write must arrive from
    // the wait loop — no tick ever repaints a frozen frame.
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

    // Esc resumes: a live frame arrives, recognizable by its since row —
    // the one needle a parked status repaint can never produce.
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
        "v views all · q quits".as_bytes(),
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
    assert!(
        wait_for(
            &session,
            &mut terminal,
            "paused · just now · lines 1-22 of 30 · Esc resumes".as_bytes(),
            Duration::from_secs(2)
        ),
        "expected the frozen window at the top"
    );
    session.write_bytes(b"j");
    assert!(
        wait_for(
            &session,
            &mut terminal,
            "paused · just now · lines 2-23 of 30 · Esc resumes".as_bytes(),
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
    // Let the height ring fill: stability needs eight equal ticks.
    let _ = drain_for(&session, Duration::from_millis(700));

    // A top-reaching step never enters the mode: g at the top stays Live.
    session.write_bytes(b"g");
    let noop = drain_for(&session, Duration::from_millis(400));
    assert!(
        !contains(&noop, "· live ·".as_bytes()),
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
            "lines 2-23 of 46 · live · g follows".as_bytes(),
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
fn an_unstable_frame_freezes_on_scroll() {
    // The line count alternates every tick: stability is never satisfied
    // and a scroll key falls back to the freeze.
    let dir = tempfile::tempdir().expect("tempdir");
    let flag = dir.path().join("flag").display().to_string();
    let script = format!(
        "if [ -e {flag} ]; then rm -f {flag}; echo one; echo two; else : > {flag}; echo one; fi"
    );
    let session = PtySession::spawn(
        &rat_bin(),
        &["watch", "-n", "50ms", "--shell", "--", &script],
        &[],
    )
    .expect("spawn under a pty");
    let mut terminal = FakeTerminal::dark();

    assert!(
        wait_for(&session, &mut terminal, b"one", Duration::from_secs(2)),
        "expected a first frame"
    );
    let _ = drain_for(&session, Duration::from_millis(700));

    session.write_bytes(b"j");
    assert!(
        wait_for(
            &session,
            &mut terminal,
            "paused · just now".as_bytes(),
            Duration::from_secs(2)
        ),
        "expected a jittering frame to freeze on scroll"
    );
    session.write_bytes(b"q");
    assert!(
        !session.kill_if_alive(Duration::from_secs(2)),
        "watch should have exited on q"
    );
}

#[test]
fn a_shape_change_while_scrolled_freezes() {
    // Stable until the test says otherwise: the frame grows one line the
    // moment the grow file appears, deterministically mid-scroll.
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
    let _ = drain_for(&session, Duration::from_millis(700));

    session.write_bytes(b"j");
    assert!(
        wait_for_without(
            &session,
            &mut terminal,
            "lines 2-23 of 30 · live · g follows".as_bytes(),
            b"paused",
            Duration::from_secs(2)
        ),
        "expected a stable frame to live-scroll"
    );

    // The frame changes shape under the scrolled window: auto-freeze.
    std::fs::write(&grow, b"").expect("create the grow file");
    let seen = wait_for_bytes(
        &session,
        &mut terminal,
        b"frame changed shape",
        Duration::from_secs(5),
    )
    .expect("expected the one-shot shape notice");
    assert!(
        contains(&seen, "paused ·".as_bytes()),
        "the shape change must land in a freeze"
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
            "paused · just now".as_bytes(),
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
