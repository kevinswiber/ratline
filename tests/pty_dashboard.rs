#![cfg(unix)]
mod common;

use std::time::Duration;

use common::pty::{
    FakeTerminal, PtySession, assert_counter_settled_at, wait_for, wait_for_counter,
};

/// Path to the rat binary — duplicated from the watch suite's local
/// helper, never lifted: `tests/pty_watch.rs` is the byte-identity
/// witness and must end this change with no modified hunks.
fn rat_bin() -> String {
    assert_cmd::cargo::cargo_bin("rat").display().to_string()
}

fn contains(haystack: &[u8], needle: &[u8]) -> bool {
    needle.len() <= haystack.len() && haystack.windows(needle.len()).any(|w| w == needle)
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

/// `counter_cmd`'s labelled sibling: two panes need two DISTINCT screen
/// needles, and `counter_cmd` prints `count-N` for every pane. Same
/// shape otherwise, including the missing trailing newline, so
/// `wait_for_counter`/`assert_counter_settled_at` read these files
/// unchanged.
fn labeled_counter_cmd(path: &std::path::Path, label: &str) -> String {
    format!(
        "echo run >> {p}; printf '{label}-%s' $(wc -l < {p})",
        p = path.display()
    )
}

/// Writes the fixture declaration and returns its path. THE ONLY
/// format-specific text in this file lives in this function and the
/// body builders below.
fn write_dashboard(dir: &std::path::Path, body: &str) -> std::path::PathBuf {
    let path = dir.join("dash.kdl");
    std::fs::write(&path, body).expect("write the dashboard declaration");
    path
}

/// Declaration body for two 1fr panes in one row, both parked at 1h.
/// `labeled_counter_cmd` emits only single-quoted sh, which a KDL
/// quoted string carries verbatim (only `"` and `\` would need
/// escapes; tempdir paths contain neither).
fn two_pane_row(left: &std::path::Path, right: &std::path::Path) -> String {
    panes_row(left, "left", "1h", right, "right", "1h")
}

/// Same row with the first pane fast, so the spawn step stays busy.
fn fast_slow_row(fast: &std::path::Path, slow: &std::path::Path) -> String {
    panes_row(fast, "fast", "500ms", slow, "slow", "1h")
}

fn panes_row(
    a: &std::path::Path,
    a_name: &str,
    a_interval: &str,
    b: &std::path::Path,
    b_name: &str,
    b_interval: &str,
) -> String {
    let a_cmd = labeled_counter_cmd(a, a_name);
    let b_cmd = labeled_counter_cmd(b, b_name);
    debug_assert!(!a_cmd.contains('"') && !b_cmd.contains('"'));
    format!(
        "gap 1\n\n\
         defaults {{\n    height 5\n    border \"rounded\"\n    width \"1fr\"\n    shell #true\n}}\n\n\
         pane \"{a_name}\" {{\n    interval \"{a_interval}\"\n    command \"{a_cmd}\"\n}}\n\n\
         pane \"{b_name}\" {{\n    interval \"{b_interval}\"\n    command \"{b_cmd}\"\n}}\n\n\
         layout {{\n    row \"{a_name}\" \"{b_name}\"\n}}\n"
    )
}

/// A resize reflows the retained outputs at the new widths straight
/// away — the wide frame still shows `left-1`/`right-1`, which no
/// re-run could produce — and the debounced respawn lands after it.
#[test]
fn a_resize_reflows_the_boxes_before_any_child_returns() {
    let dir = tempfile::tempdir().expect("tempdir");
    let left = dir.path().join("left");
    let right = dir.path().join("right");
    let decl = write_dashboard(dir.path(), &two_pane_row(&left, &right));

    let session = PtySession::spawn(&rat_bin(), &["dashboard", &decl.display().to_string()], &[])
        .expect("spawn rat dashboard under a pty");
    let mut terminal = FakeTerminal::dark();
    assert!(
        wait_for(&session, &mut terminal, b"right-1", Duration::from_secs(5)),
        "the first composition never painted"
    );
    wait_for_counter(&left, 1);
    wait_for_counter(&right, 1);

    // 80 -> 120 columns: each 1fr pane grows past 45 cells, which no
    // 80-column frame can contain.
    session.set_winsize(24, 120);
    let wide = "─".repeat(45);
    let bytes = wait_for_bytes(
        &session,
        &mut terminal,
        wide.as_bytes(),
        Duration::from_secs(5),
    )
    .expect("the boxes never reflowed");
    // The reflow came from RETAINED output: a respawn would have
    // produced -2, and the counters only move forward. Both panes are
    // named so neither can stand in for the other.
    let first_wide = bytes
        .windows(wide.len())
        .position(|w| w == wide.as_bytes())
        .expect("the wide border");
    let reflowed = &bytes[first_wide..];
    for still_at_one in [b"left-1".as_slice(), b"right-1".as_slice()] {
        assert!(
            contains(reflowed, still_at_one),
            "the wide frame must carry retained output, not a re-run: {:?}",
            String::from_utf8_lossy(reflowed)
        );
    }

    // …and then, once the window closes, exactly one respawn of EVERY
    // source: child-side evidence, value-based, never a sleep.
    wait_for_counter(&left, 2);
    wait_for_counter(&right, 2);
    assert_counter_settled_at(&left, 2);
    assert_counter_settled_at(&right, 2);
    session.write_bytes(b"q");
    assert!(
        !session.kill_if_alive(Duration::from_secs(2)),
        "the dashboard should have exited on q"
    );
}

/// The end-to-end companion to the unit race proof: with a fast pane
/// keeping the spawn step continuously busy, a resize still reflows
/// immediately and the debounced respawn-all still reaches the pane
/// that was NOT due. The slow counter advancing is the arm's proof:
/// nothing else can run a 1h pane again.
#[test]
fn a_resize_reaches_the_panes_that_were_not_due() {
    let dir = tempfile::tempdir().expect("tempdir");
    let fast = dir.path().join("fast");
    let slow = dir.path().join("slow");
    let decl = write_dashboard(dir.path(), &fast_slow_row(&fast, &slow));

    let session = PtySession::spawn(&rat_bin(), &["dashboard", &decl.display().to_string()], &[])
        .expect("spawn rat dashboard under a pty");
    let mut terminal = FakeTerminal::dark();
    assert!(
        wait_for(&session, &mut terminal, b"slow-1", Duration::from_secs(5)),
        "the first composition never painted"
    );
    wait_for_counter(&slow, 1);
    wait_for_counter(&fast, 2); // the spawn step is demonstrably busy

    session.set_winsize(24, 120);
    let wide = "─".repeat(45);
    assert!(
        wait_for_bytes(
            &session,
            &mut terminal,
            wide.as_bytes(),
            Duration::from_secs(5)
        )
        .is_some(),
        "the boxes never reflowed at the new width"
    );

    // The debounced respawn-all reached the 1h pane exactly once —
    // child-side evidence with a bounded ceiling, value-based, no
    // sleeps.
    wait_for_counter(&slow, 2);
    assert_counter_settled_at(&slow, 2);
    session.write_bytes(b"q");
    assert!(
        !session.kill_if_alive(Duration::from_secs(2)),
        "the dashboard should have exited on q"
    );
}

/// `wait_for_bytes`, but panicking the moment `forbidden` shows up in
/// the accumulated output. Duplicated from the watch suite's local
/// helper, never lifted.
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

/// The last screen row containing `needle`, carriage returns stripped.
fn row_containing<'a>(bytes: &'a [u8], needle: &[u8]) -> Option<&'a [u8]> {
    bytes
        .split(|&b| b == b'\n')
        .map(|row| row.strip_suffix(b"\r").unwrap_or(row))
        .rfind(|row| contains(row, needle))
}

/// A pane's marks are computed once per ITS OWN output change, so a
/// slow pane's change stays marked while a fast pane ticks under it.
/// The proof is negative on purpose — a dwelling mark writes no bytes,
/// an unmarked repaint of that row does.
#[test]
fn a_slow_panes_change_stays_marked_across_the_fast_panes_ticks() {
    let dir = tempfile::tempdir().expect("tempdir");
    let fast_count = dir.path().join("fastcount");
    let slow_value = dir.path().join("slowvalue");
    std::fs::write(&slow_value, "v0").expect("seed");
    let decl = write_dashboard(
        dir.path(),
        &format!(
            r#"
row-gap 0

defaults {{
    height 1
    border "none"
    chrome #false
    shell #true
}}

pane "fast" {{
    interval "250ms"
    command "{fast}"
}}

pane "slow" {{
    interval "300ms"
    command "printf 'slow-%s' \"$(cat {slow})\""
}}
"#,
            fast = labeled_counter_cmd(&fast_count, "fast"),
            slow = slow_value.display(),
        ),
    );

    let session = PtySession::spawn(&rat_bin(), &["dashboard", &decl.display().to_string()], &[])
        .expect("spawn rat dashboard under a pty");
    let mut terminal = FakeTerminal::dark();
    assert!(
        wait_for(&session, &mut terminal, b"slow-v0", Duration::from_secs(5)),
        "the first composition never painted"
    );
    session.write_bytes(b"D"); // gutter on, while the slow pane still reads v0
    assert!(
        wait_for(
            &session,
            &mut terminal,
            b"  slow-v0",
            Duration::from_secs(5)
        ),
        "the gutter never appeared"
    );

    std::fs::write(&slow_value, "v1").expect("the slow pane's one change");
    let bytes = wait_for_bytes(&session, &mut terminal, b"slow-v1", Duration::from_secs(5))
        .expect("the slow pane never picked up its change");
    let slow_row = row_containing(&bytes, b"slow-v1").expect("the slow row");
    assert!(
        contains(slow_row, "▌".as_bytes()),
        "the changed pane must be marked: {:?}",
        String::from_utf8_lossy(slow_row)
    );

    // Now let the FAST pane tick four more times. If the marks were
    // recomputed per composed frame, the very next fast tick would
    // repaint the slow row unmarked — which is exactly the forbidden
    // needle. Child-side evidence picks the ceiling, not a sleep.
    let n = std::fs::read_to_string(&fast_count)
        .map(|s| s.lines().count())
        .unwrap_or(0);
    assert!(
        wait_for_without(
            &session,
            &mut terminal,
            format!("fast-{}", n + 4).as_bytes(),
            b"  slow-v1",
            Duration::from_secs(5),
        ),
        "the fast pane stopped ticking"
    );
    session.write_bytes(b"q");
    assert!(
        !session.kill_if_alive(Duration::from_secs(2)),
        "the dashboard should have exited on q"
    );
}

/// Accumulate everything the session writes within `total` — unlike
/// `read_available`, which returns at the first chunk. Duplicated from
/// the watch suite's local helper, never lifted.
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

/// Every `v`-prefixed six-digit counter value in the byte stream — the
/// scrub fixture prints `v%06d`, monotonic and fixed-width, so ordering
/// assertions are exact. Duplicated from the watch suite's local
/// helper, never lifted.
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

/// A stacked declaration: shared defaults plus (name, interval,
/// command) panes in declaration order. With `panes_row` above, the
/// only other place format-specific text lives — the format pick's
/// deletion commit rewrites these builders together.
fn board(defaults: &str, panes: &[(&str, &str, &str)]) -> String {
    let mut body = format!("row-gap 0\n\ndefaults {{\n{defaults}\n}}\n");
    for (name, interval, command) in panes {
        body.push_str(&format!(
            "\npane \"{name}\" {{\n    interval \"{interval}\"\n    command \"{command}\"\n}}\n"
        ));
    }
    body
}

const STACKED: &str = "    height 5\n    border \"none\"\n    chrome #false\n    shell #true";

/// Per-source schedules and the min-nap: a 50ms pane and a 2s pane run
/// at their own cadences instead of one shared clock.
#[test]
fn two_panes_tick_at_their_own_cadences() {
    let dir = tempfile::tempdir().expect("tempdir");
    let fast = dir.path().join("fast");
    let slow = dir.path().join("slow");
    let decl = write_dashboard(
        dir.path(),
        &board(
            STACKED,
            &[
                ("fast", "50ms", &labeled_counter_cmd(&fast, "fast")),
                ("slow", "2s", &labeled_counter_cmd(&slow, "slow")),
            ],
        ),
    );
    let session = PtySession::spawn(&rat_bin(), &["dashboard", &decl.display().to_string()], &[])
        .expect("spawn rat dashboard under a pty");
    let mut terminal = FakeTerminal::dark();
    // ONE accumulated wait: the slow pane's first row appears exactly
    // once (the differ never rewrites an unchanged row), so a second
    // wait that starts fresh would miss it. The fast pane's presence
    // rides the same bytes.
    let first = wait_for_bytes(&session, &mut terminal, b"slow-1", Duration::from_secs(5))
        .expect("the slow pane never painted");
    assert!(contains(&first, b"fast-"), "the fast pane never painted");
    // Child-side evidence across ~4s of real cadence — DRAINING the
    // pty while waiting: a fast pane repaints continuously, and an
    // undrained master fills its kernel buffer and blocks the loop's
    // writer, stalling every schedule. A real terminal always reads;
    // the harness must too.
    let deadline = std::time::Instant::now() + Duration::from_secs(10);
    loop {
        let _ = session.read_available(Duration::from_millis(50));
        let n = std::fs::read_to_string(&slow)
            .map(|s| s.lines().count())
            .unwrap_or(0);
        if n >= 3 {
            break;
        }
        assert!(
            std::time::Instant::now() < deadline,
            "slow pane stalled at {n}"
        );
    }
    let fast_runs = std::fs::read_to_string(&fast)
        .map(|s| s.lines().count())
        .unwrap_or(0);
    let slow_runs = std::fs::read_to_string(&slow)
        .map(|s| s.lines().count())
        .unwrap_or(0);
    // Declared ratio 40:1; a 4x floor tolerates ten-fold starvation
    // while still failing loudly if the panes share one schedule.
    assert!(slow_runs >= 3, "slow pane stalled at {slow_runs}");
    assert!(
        fast_runs >= 4 * slow_runs,
        "the cadences collapsed: fast {fast_runs}, slow {slow_runs}"
    );
    session.write_bytes(b"q");
    assert!(
        !session.kill_if_alive(Duration::from_secs(5)),
        "the dashboard should have exited on q"
    );
}

/// N independent slots behind one Vec of guards: q kills BOTH in-flight
/// children (neither reaches its last act) and stops further spawns.
#[test]
fn q_shuts_down_every_pane_child() {
    let dir = tempfile::tempdir().expect("tempdir");
    let count_a = dir.path().join("count-a");
    let count_b = dir.path().join("count-b");
    let fin_a = dir.path().join("fin-a");
    let fin_b = dir.path().join("fin-b");
    let child = |count: &std::path::Path, fin: &std::path::Path| {
        // The counter line lands BEFORE the sleep; the finish touch is
        // the child's last act, and a killed interpreter never reaches
        // it.
        format!(
            "echo run >> {c}; /bin/sleep 1; : > {f}",
            c = count.display(),
            f = fin.display()
        )
    };
    let decl = write_dashboard(
        dir.path(),
        &board(
            STACKED,
            &[
                ("a", "10s", &child(&count_a, &fin_a)),
                ("b", "10s", &child(&count_b, &fin_b)),
            ],
        ),
    );
    let session = PtySession::spawn(&rat_bin(), &["dashboard", &decl.display().to_string()], &[])
        .expect("spawn rat dashboard under a pty");
    let mut terminal = FakeTerminal::dark();
    // Answer the appearance probe while both children start.
    let _ = wait_for(
        &session,
        &mut terminal,
        b"never-painted",
        Duration::from_millis(200),
    );
    wait_for_counter(&count_a, 1);
    wait_for_counter(&count_b, 1);
    session.write_bytes(b"q");
    assert!(
        !session.kill_if_alive(Duration::from_secs(2)),
        "the dashboard should have exited on q"
    );
    // Neither slot leaked: no further spawns, and neither sleeping
    // child survived to its last act.
    assert_counter_settled_at(&count_a, 1);
    assert_counter_settled_at(&count_b, 1);
    assert!(!fin_a.exists(), "pane a's child outlived the shutdown");
    assert!(!fin_b.exists(), "pane b's child outlived the shutdown");
}

/// A freeze is whole-dashboard: the frozen frame is byte-silent while
/// BOTH children keep ticking behind it.
#[test]
fn p_freezes_the_whole_dashboard_while_children_keep_ticking() {
    let dir = tempfile::tempdir().expect("tempdir");
    let a = dir.path().join("a");
    let b = dir.path().join("b");
    let decl = write_dashboard(
        dir.path(),
        &board(
            STACKED,
            &[
                ("fast", "50ms", &labeled_counter_cmd(&a, "fast")),
                ("slow", "120ms", &labeled_counter_cmd(&b, "slow")),
            ],
        ),
    );
    let session = PtySession::spawn(&rat_bin(), &["dashboard", &decl.display().to_string()], &[])
        .expect("spawn rat dashboard under a pty");
    let mut terminal = FakeTerminal::dark();
    // ONE accumulated wait — see two_panes_tick_at_their_own_cadences.
    let first = wait_for_bytes(&session, &mut terminal, b"slow-1", Duration::from_secs(5))
        .expect("the slow pane never painted");
    assert!(contains(&first, b"fast-"), "the fast pane never painted");
    session.write_bytes(b"p");
    assert!(
        wait_for_bytes(
            &session,
            &mut terminal,
            "paused ·".as_bytes(),
            Duration::from_secs(5)
        )
        .is_some(),
        "p never froze the frame"
    );
    // Flush the freeze paint, then the pin: the frozen frame must be
    // byte-silent — the default absolute stamps keep the chrome rows
    // from counting, so a repaint here means a pane painted through
    // the freeze.
    let _ = drain_for(&session, Duration::from_millis(300));
    let before_a = std::fs::read_to_string(&a)
        .map(|s| s.lines().count())
        .unwrap_or(0);
    let before_b = std::fs::read_to_string(&b)
        .map(|s| s.lines().count())
        .unwrap_or(0);
    let leaked = session.read_available(Duration::from_millis(400));
    assert!(
        leaked.is_empty(),
        "a pane painted through the freeze: {:?}",
        String::from_utf8_lossy(&leaked)
    );
    // …while both children kept running behind it.
    wait_for_counter(&a, before_a + 2);
    wait_for_counter(&b, before_b + 2);
    session.write_bytes(b"q");
    assert!(
        !session.kill_if_alive(Duration::from_secs(5)),
        "the dashboard should have exited on q"
    );
}

/// Esc resumes the WHOLE dashboard: both parked panes re-run exactly
/// once. Only provable against parked panes — a fast pane's counter
/// advances whether or not the resume requested anything.
#[test]
fn esc_resumes_and_reruns_every_pane_at_once() {
    let dir = tempfile::tempdir().expect("tempdir");
    let a = dir.path().join("a");
    let b = dir.path().join("b");
    let decl = write_dashboard(
        dir.path(),
        &board(
            STACKED,
            &[
                ("a", "1h", &labeled_counter_cmd(&a, "a")),
                ("b", "1h", &labeled_counter_cmd(&b, "b")),
            ],
        ),
    );
    let session = PtySession::spawn(&rat_bin(), &["dashboard", &decl.display().to_string()], &[])
        .expect("spawn rat dashboard under a pty");
    let mut terminal = FakeTerminal::dark();
    assert!(
        wait_for(&session, &mut terminal, b"b-1", Duration::from_secs(5)),
        "the first composition never painted"
    );
    wait_for_counter(&a, 1);
    wait_for_counter(&b, 1);
    session.write_bytes(b"p");
    assert!(
        wait_for_bytes(
            &session,
            &mut terminal,
            "paused ·".as_bytes(),
            Duration::from_secs(5)
        )
        .is_some(),
        "p never froze the frame"
    );
    session.write_bytes(b"\x1b");
    // Both 1h panes can only run again if the resume requested every
    // source; a resume that requested only the first hangs here.
    wait_for_counter(&a, 2);
    wait_for_counter(&b, 2);
    // …and exactly once: no double-fire.
    assert_counter_settled_at(&a, 2);
    assert_counter_settled_at(&b, 2);
    session.write_bytes(b"q");
    assert!(
        !session.kill_if_alive(Duration::from_secs(5)),
        "the dashboard should have exited on q"
    );
}

/// S writes the DISPLAYED mixed-age composition: the parked pane's
/// stale row beside the fast pane's fresh one, at the declared row
/// total, escapes stripped.
#[test]
fn s_snapshots_the_mixed_age_composition() {
    let dir = tempfile::tempdir().expect("tempdir");
    let snaps = dir.path().join("snaps");
    std::fs::create_dir(&snaps).expect("snapshot dir");
    let fast = dir.path().join("fast");
    let slow = dir.path().join("slow");
    let decl = write_dashboard(
        dir.path(),
        &board(
            STACKED,
            &[
                ("fast", "50ms", &labeled_counter_cmd(&fast, "fast")),
                ("slow", "1h", &labeled_counter_cmd(&slow, "slow")),
            ],
        ),
    );
    let session = PtySession::spawn(
        &rat_bin(),
        &["dashboard", &decl.display().to_string()],
        &[("RAT_SNAPSHOT_DIR", &snaps.display().to_string())],
    )
    .expect("spawn rat dashboard under a pty");
    let mut terminal = FakeTerminal::dark();
    // ONE accumulated wait — see two_panes_tick_at_their_own_cadences.
    let first = wait_for_bytes(&session, &mut terminal, b"slow-1", Duration::from_secs(5))
        .expect("the parked pane never painted");
    assert!(contains(&first, b"fast-"), "the fast pane never painted");
    session.write_bytes(b"S");
    assert!(
        wait_for(&session, &mut terminal, b"snapshot", Duration::from_secs(5)),
        "expected the snapshot notice"
    );
    let entries: Vec<_> = std::fs::read_dir(&snaps)
        .expect("read the snapshot dir")
        .map(|e| e.expect("entry").path())
        .collect();
    assert_eq!(entries.len(), 1, "exactly one snapshot: {entries:?}");
    let name = entries[0]
        .file_name()
        .unwrap()
        .to_string_lossy()
        .into_owned();
    // The shipped prefix, deliberately pinned: a dashboard session
    // files a rat-watch-*.txt today — the alarm if that ever changes.
    assert!(name.starts_with("rat-watch-"), "{name}");
    assert!(name.ends_with(".txt"), "{name}");
    let contents = std::fs::read_to_string(&entries[0]).expect("snapshot body");
    assert!(
        contents.contains("slow-1"),
        "the stale pane composed: {contents:?}"
    );
    assert!(
        contents.contains("fast-"),
        "the fresh pane composed: {contents:?}"
    );
    assert_eq!(
        contents.trim_end_matches('\n').split('\n').count(),
        10,
        "the declared row total survives: {contents:?}"
    );
    assert!(!contents.contains('\x1b'), "escapes stripped by default");
    session.write_bytes(b"q");
    assert!(
        !session.kill_if_alive(Duration::from_secs(5)),
        "the dashboard should have exited on q"
    );
}

/// A scrub replays a DISPLAYED composition verbatim: a strictly older
/// counter value, with the parked pane's row still in it.
#[test]
fn scrub_steps_distinct_composed_frames() {
    let dir = tempfile::tempdir().expect("tempdir");
    let count = dir.path().join("count");
    let slow = dir.path().join("slow");
    let ticker = format!(
        "echo x >> {c}; n=$(wc -l < {c}); printf 'v%06d' $n",
        c = count.display()
    );
    let decl = write_dashboard(
        dir.path(),
        &board(
            "    height 1\n    border \"none\"\n    chrome #false\n    shell #true",
            &[
                ("ticker", "50ms", &ticker),
                ("slow", "1h", &labeled_counter_cmd(&slow, "slow")),
            ],
        ),
    );
    let session = PtySession::spawn(&rat_bin(), &["dashboard", &decl.display().to_string()], &[])
        .expect("spawn rat dashboard under a pty");
    let mut terminal = FakeTerminal::dark();
    assert!(
        wait_for(&session, &mut terminal, b"v000001", Duration::from_secs(5)),
        "the ticker never painted"
    );
    // Let history accrue distinct compositions.
    let _ = drain_for(&session, Duration::from_millis(400));
    session.write_bytes(b"<");
    let scrub = wait_for_bytes(
        &session,
        &mut terminal,
        "paused ·".as_bytes(),
        Duration::from_secs(5),
    )
    .expect("the scrub never parked");
    let scrubbed = *counter_values(&scrub).last().expect("a scrubbed value");
    assert!(
        contains(&scrub, b"slow-1"),
        "the replay carries the WHOLE composition: {:?}",
        String::from_utf8_lossy(&scrub)
    );
    session.write_bytes(b"\x1b");
    let live = drain_for(&session, Duration::from_millis(600));
    let newest = counter_values(&live)
        .into_iter()
        .max()
        .expect("a live value");
    assert!(
        scrubbed < newest,
        "the scrubbed frame must be strictly older: {scrubbed} vs {newest}"
    );
    session.write_bytes(b"q");
    assert!(
        !session.kill_if_alive(Duration::from_secs(5)),
        "the dashboard should have exited on q"
    );
}
