#![cfg(unix)]
mod common;

use std::time::Duration;

use common::pty::{
    FakeTerminal, PtySession, assert_counter_settled_at, counter_cmd, first_unmatched_in_order,
    try_wait_for_in_order, wait_for, wait_for_counter, wait_for_in_order,
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

/// First index of `needle` in `haystack` — `contains`'s locating
/// sibling, for the tests that assert ORDER within a byte stream.
fn position(haystack: &[u8], needle: &[u8]) -> Option<usize> {
    if needle.len() > haystack.len() {
        return None;
    }
    haystack.windows(needle.len()).position(|w| w == needle)
}

/// When nextest KILLS one of the cycle tests: `period x terminate-after`
/// from the matching override in `.config/nextest.toml`, which is
/// `60s x 2`. Recorded because it is INVISIBLE where test authors choose
/// their timeouts — a wait longer than this is unreachable by
/// construction, since the harness kills the test before the wait can
/// expire and a terminated test prints nothing.
///
/// **Both numbers, not just the period.** An earlier version of this
/// constant held the period alone and its own doc said the kill lands at
/// `period x 2`, which made the file self-contradictory about the one
/// fact it exists to record.
const HARNESS_KILL: Duration = Duration::from_secs(120);

/// What a cycle test allows itself before giving up and REPORTING.
///
/// Deliberately far under [`HARNESS_KILL`], because those two numbers do
/// different jobs: the kill is the harness losing patience with a wedged
/// test and it prints nothing, while this is the test losing patience
/// with the product and saying so with evidence in hand. The 60s of gap
/// is what pays for the shutdown probe, the kill, and the diagnostic
/// dump — which is why there is no separate reserve to subtract.
///
/// 60s rather than a share of the suite's general 20s ceiling because
/// these two tests EARN their verdict — 50 trigger-driven respawns
/// inside the detector's 30s window — so their cost tracks how fast the
/// machine spawns processes. A `macos-latest` runner has already failed
/// the file route at the old ceiling while its fifo sibling passed at
/// 10.4s.
///
/// **Raising this alone buys nothing**: the override in
/// `.config/nextest.toml` is what actually grants the allowance, and a
/// budget past what it grants becomes unreachable again, silently.
const REPORT_BUDGET: Duration = Duration::from_secs(60);

/// The two constants above only mean something in relation to each
/// other, so the relation is asserted rather than left to a comment.
///
/// This is the guard the previous version of this file did not have:
/// a budget at or past the kill is silently unreachable — the harness
/// terminates the test before the budget can expire, and a terminated
/// test prints nothing, so the failure looks like a hang instead of a
/// timeout. Raising `REPORT_BUDGET` without raising the `.config/nextest.toml`
/// override fails HERE, loudly, instead of on a loaded runner months later.
#[test]
fn the_report_budget_leaves_the_harness_room_to_hear_it() {
    assert!(
        REPORT_BUDGET < HARNESS_KILL,
        "a budget of {REPORT_BUDGET:?} cannot be spent under a {HARNESS_KILL:?} kill"
    );
    // Not merely under it: a report needs time to be written after the
    // budget expires — the shutdown probe, the kill, the diagnostic dump.
    assert!(
        REPORT_BUDGET * 2 <= HARNESS_KILL,
        "{REPORT_BUDGET:?} leaves too little of {HARNESS_KILL:?} to report in"
    );
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
    // `interval` rides in the PROPERTY position here so the dual
    // spelling gets its coverage through a real pty, not just a parse.
    format!(
        "gap 1\n\n\
         defaults {{\n    height 5\n    border \"rounded\"\n    width \"1fr\"\n    shell #true\n}}\n\n\
         row {{\n    \
         pane \"{a_name}\" interval=\"{a_interval}\" {{\n        command \"{a_cmd}\"\n    }}\n    \
         pane \"{b_name}\" interval=\"{b_interval}\" {{\n        command \"{b_cmd}\"\n    }}\n\
         }}\n"
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
    // The reflow came from RETAINED output: a respawn would have
    // produced -2, and the counters only move forward. Both panes are
    // named so neither can stand in for the other. ONE ordered
    // capture: the retained bodies paint BELOW the wide border and can
    // legally miss the read chunk that carried it — the in-order match
    // encodes "after the border" without a slice.
    let wide = "─".repeat(45);
    wait_for_in_order(
        &session,
        &mut terminal,
        &[wide.as_bytes(), b"left-1", b"right-1"],
        Duration::from_secs(5),
    );

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

// ── Diagnostic scaffolding for the Linux wedge ─────────────────────────
//
// `a_fifo_cycle_earns_its_badge_and_its_notice_too` dies on ubuntu at the
// harness's 20 s ceiling, and a TERMINATED test prints nothing: its pty
// buffer is a local that never reaches stdout. Four CI runs have produced
// no evidence at all for that reason. Everything below exists to turn that
// silence into a report, and none of it is shipping shape.

/// `wait_for_bytes` that hands back what it saw even when the needle never
/// arrives, with the arrival time of every chunk. The shipped helper drops
/// both on timeout, which is the whole reason the failure is unexplained.
fn wait_for_bytes_verbose(
    session: &PtySession,
    terminal: &mut FakeTerminal,
    needles: &[&[u8]],
    timeout: Duration,
) -> (bool, Vec<u8>, Vec<(f64, usize)>) {
    let start = std::time::Instant::now();
    let deadline = start + timeout;
    let mut seen: Vec<u8> = Vec::new();
    let mut chunks: Vec<(f64, usize)> = Vec::new();
    loop {
        let now = std::time::Instant::now();
        if now >= deadline {
            return (false, seen, chunks);
        }
        let chunk = session.read_available((deadline - now).min(Duration::from_millis(50)));
        if chunk.is_empty() {
            continue;
        }
        terminal.respond(session, &chunk);
        seen.extend_from_slice(&chunk);
        chunks.push((start.elapsed().as_secs_f64(), chunk.len()));
        if first_unmatched_in_order(&seen, needles).is_none() {
            return (true, seen, chunks);
        }
    }
}

/// The words a pty carried, with the escape sequences taken out. Positions
/// are lost and that is fine: the questions this has to answer are whether
/// the badge, the notice and the pane bodies ever appeared at all.
fn visible(bytes: &[u8]) -> String {
    let mut out = String::new();
    let mut i = 0;
    while i < bytes.len() {
        let b = bytes[i];
        if b == 0x1b {
            i += 1;
            match bytes.get(i) {
                // CSI: parameters, then one final byte in @..~.
                Some(b'[') => {
                    i += 1;
                    while i < bytes.len() && !(0x40..=0x7e).contains(&bytes[i]) {
                        i += 1;
                    }
                    i += 1;
                }
                // OSC: runs to BEL or ST.
                Some(b']') => {
                    i += 1;
                    while i < bytes.len() && bytes[i] != 0x07 {
                        if bytes[i] == 0x1b && bytes.get(i + 1) == Some(&b'\\') {
                            i += 1;
                            break;
                        }
                        i += 1;
                    }
                    i += 1;
                }
                // Anything else two-byte.
                _ => i += 1,
            }
            out.push(' ');
            continue;
        }
        if b == b'\n' || b == b'\r' {
            out.push('\n');
        } else if (0x20..0x7f).contains(&b) {
            out.push(b as char);
        } else {
            out.push('.');
        }
        i += 1;
    }
    out
}

/// The first `n` visible characters, for "what did it actually print".
fn head(bytes: &[u8], n: usize) -> String {
    visible(bytes).chars().take(n).collect()
}

/// The last `n` visible characters — the current screen, near enough.
fn tail(bytes: &[u8], n: usize) -> String {
    let text = visible(bytes);
    let skip = text.chars().count().saturating_sub(n);
    text.chars().skip(skip).collect()
}

/// Bytes per whole second, so "output stopped at t=1.2" and "output kept
/// flowing to the deadline" are told apart at a glance.
fn per_second(chunks: &[(f64, usize)]) -> String {
    let mut buckets: Vec<usize> = Vec::new();
    for (at, len) in chunks {
        let s = *at as usize;
        if buckets.len() <= s {
            buckets.resize(s + 1, 0);
        }
        buckets[s] += len;
    }
    buckets
        .iter()
        .enumerate()
        .map(|(s, n)| format!("{s}s:{n}"))
        .collect::<Vec<_>>()
        .join(" ")
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

/// A cycle: each pane's command touches the file the other pane
/// triggers on. `interval "never"` on both, and yet they spin — every
/// run of one arms the other, forever, with no input.
fn cycle_board(sa: &std::path::Path, sb: &std::path::Path) -> String {
    let (sa, sb) = (sa.display(), sb.display());
    debug_assert!(!format!("{sa}{sb}").contains('"'));
    format!(
        "row-gap 0\n\n\
         defaults {{\n    height 3\n    border \"none\"\n    shell #true\n    interval \"never\"\n}}\n\n\
         pane \"a\" {{\n    trigger \"file:{sa}\"\n    trigger-debounce \"100ms\"\n    \
         command \"touch {sb}; echo cycle-a\"\n}}\n\n\
         pane \"b\" {{\n    trigger \"file:{sb}\"\n    trigger-debounce \"100ms\"\n    \
         command \"touch {sa}; echo cycle-b\"\n}}\n"
    )
}

/// The whole signal end to end: observation, brackets, the credit rule,
/// the verdict, and a badge reaching a real screen. Both panes print a
/// CONSTANT line, so the composition is still — which is the hazard
/// itself: without the badge this dashboard looks idle while it spawns a
/// shell several times a second.
///
/// It costs seconds by construction, because the badge is earned rather
/// than set: a pane needs 50 trigger-driven respawns inside a 30 s
/// window, and each hop of the cycle costs one debounce interval.
/// `trigger-debounce` is declared at 100 ms as the widest margin
/// available on both axes at once — the ~10 Hz it allows is several
/// times the ~1.7 Hz the threshold needs, while still bounding each
/// pane's in-flight duty far below the ceiling that makes the signal
/// abstain, which no debounce at all would not.
#[test]
fn a_cycle_of_two_panes_earns_its_badge_and_its_notice() {
    let dir = tempfile::tempdir().expect("tempdir");
    let sa = dir.path().join("sa");
    let sb = dir.path().join("sb");
    // Both trigger files exist before the baselines are taken: a path
    // that APPEARS is itself a change, and this test is about the cycle
    // rather than about a first appearance.
    for stamp in [&sa, &sb] {
        std::fs::write(stamp, b"").expect("seed the trigger file");
    }
    let decl = write_dashboard(dir.path(), &cycle_board(&sa, &sb));

    let session = PtySession::spawn(&rat_bin(), &["dashboard", &decl.display().to_string()], &[])
        .expect("spawn rat dashboard under a pty");
    let mut terminal = FakeTerminal::dark();
    let started = std::time::Instant::now();
    assert!(
        wait_for(&session, &mut terminal, b"cycle-b", Duration::from_secs(5)),
        "the first composition never painted"
    );
    // Wait on the NOTICE rather than the badge: both land in the same
    // repaint at the rising edge, and the one-shot row is painted after
    // the frame, so seeing it means the frame's bytes are already in
    // hand — which lets one 8.5 s run assert both surfaces.
    //
    // The wait is DERIVED from `REPORT_BUDGET` rather than chosen — the
    // budget this test spends before giving up and saying so, which sits
    // far under `HARNESS_KILL` so the report has room to be written. A
    // wait past the KILL would be unreachable by construction: the
    // harness terminates the test first and a terminated test prints
    // nothing, so a generous-looking number buys no tolerance and costs
    // the evidence. Subtracting what has already elapsed means a slow
    // first paint eats its own margin instead of this wait's.
    // The stop condition carries the notice's TAIL as well: "file:"
    // and "? help" follow the needle inside the same line, and a
    // capture returned at the prefix can legally end mid-notice. The
    // try-variant so the session still shuts down before judging.
    let seen = try_wait_for_in_order(
        &session,
        &mut terminal,
        &[
            "a, b: trigger loop suspected:".as_bytes(),
            b"file:",
            b"? help",
        ],
        REPORT_BUDGET
            .saturating_sub(started.elapsed())
            .max(Duration::from_secs(1)),
    );
    session.write_bytes(b"q");
    session.kill_if_alive(Duration::from_secs(5));
    let seen = seen.expect("the cycle never earned its report");
    // Badge = state, notice = event, and both surfaces carry it.
    assert!(
        contains(&seen, "· looping".as_bytes()),
        "the badge never reached a chrome row: {:?}",
        String::from_utf8_lossy(&seen)
    );
    // The evidence is named, so the claim can be argued with.
    assert!(
        contains(&seen, b"file:") && contains(&seen, b"? help"),
        "the notice must name the watched paths and where to read more"
    );
    // The needle names BOTH panes on purpose. A report that names half a
    // cycle is the failure this suite exists to catch, and it took two
    // goes to earn: crediting a change only to the bracket that observed
    // it left the true writer uncredited, so the report named whichever
    // pane happened to overlap more. Weakening this needle would let that
    // back in silently.
}

fn mkfifo(path: &std::path::Path) {
    let status = std::process::Command::new("mkfifo")
        .arg(path)
        .status()
        .expect("run mkfifo");
    assert!(status.success(), "mkfifo {} failed", path.display());
}

/// The same cycle over the reader route. Each pane's command writes a
/// byte into the fifo the other pane reads, so the loop closes through
/// pipes rather than through mtimes.
fn fifo_cycle_board(fa: &std::path::Path, fb: &std::path::Path) -> String {
    let (fa, fb) = (fa.display(), fb.display());
    debug_assert!(!format!("{fa}{fb}").contains('"'));
    format!(
        "row-gap 0\n\n\
         defaults {{\n    height 3\n    border \"none\"\n    shell #true\n    interval \"never\"\n}}\n\n\
         pane \"a\" {{\n    trigger \"fifo:{fa}\"\n    trigger-debounce \"100ms\"\n    \
         command \"printf x > {fb}; echo cycle-a\"\n}}\n\n\
         pane \"b\" {{\n    trigger \"fifo:{fb}\"\n    trigger-debounce \"100ms\"\n    \
         command \"printf x > {fa}; echo cycle-b\"\n}}\n"
    )
}

/// The reader route, end to end — and the reason it exists as a separate
/// test rather than as confidence borrowed from the `file:` one.
///
/// The two routes share the credit rule, the graph test and both report
/// surfaces, but they reach them by entirely different evidence: a
/// `file:` change is placed inside a window by stat'ing a path either
/// side of a child, while a fifo has nothing to stat and is placed by
/// the instant its bytes arrived. Everything between the reader thread
/// and the window is code the `file:` test never executes — so with the
/// drain unwired, the whole suite passed and only this fails.
///
/// The hazard is also worse here, which is why the route is worth the
/// seconds: a reader posts an early wake, so the 50 ms loop slice is not
/// a ceiling; bytes in a pipe cannot be missed the way an unmoved mtime
/// can; and there is no EOF escape, because the reader holds its own
/// write end open by design.
#[test]
fn a_fifo_cycle_earns_its_badge_and_its_notice_too() {
    let dir = tempfile::tempdir().expect("tempdir");
    let fa = dir.path().join("fa");
    let fb = dir.path().join("fb");
    // Both fifos must exist before rat opens its readers; a missing one
    // is a startup error, not a trigger that fires later.
    for f in [&fa, &fb] {
        mkfifo(f);
    }
    let decl = write_dashboard(dir.path(), &fifo_cycle_board(&fa, &fb));

    // DIAGNOSTIC: where the suspicion test writes what it decided, and why.
    //
    // ON by default, switchable off with `RAT_WEDGE_CONTROL`. It was opt-in
    // for one round, on the theory that tracing perturbed the interleaving:
    // three traced ubuntu runs came back 0/3 against a 3/4 prior. Six control
    // runs then came back 2/6 — so the RATE is about a third, and at a third
    // a 0/3 draw happens 30% of the time. The perturbation was never
    // measured, only inferred from a sample too small to say anything, and
    // main's 3/4 is the same coin landing high.
    // Normally the trace lives in the tempdir and is printed only when the
    // test fails, which is when anyone wants it. Task 2.3 needs the numbers
    // from EVERY run, passing ones included, so an operator may name a
    // directory to keep them in. Unset — every ordinary run — is unchanged.
    let trace = match std::env::var_os("RAT_WEDGE_TRACE_TO") {
        Some(keep) => {
            std::path::PathBuf::from(keep).join(format!("trace-{}.log", std::process::id()))
        }
        None => dir.path().join("trigger-trace.log"),
    };
    let trace_arg = trace.display().to_string();
    let traced = std::env::var_os("RAT_WEDGE_CONTROL").is_none();
    let envs: Vec<(&str, &str)> = if traced {
        vec![("RAT_TRIGGER_TRACE", trace_arg.as_str())]
    } else {
        Vec::new()
    };
    let session = PtySession::spawn(
        &rat_bin(),
        &["dashboard", &decl.display().to_string()],
        &envs,
    )
    .expect("spawn rat dashboard under a pty");
    let mut terminal = FakeTerminal::dark();
    let started = std::time::Instant::now();
    // The first wait KEEPS its bytes now. A control run reported a first paint
    // at 0.010 s where macOS takes 0.085 s, and the needle `cycle-b` also
    // appears in the declaration this test wrote — so a startup error quoting
    // the command would satisfy this wait without a frame ever existing. That
    // has to be answerable from the log, not from argument.
    let (painted, first_paint, _) = wait_for_bytes_verbose(
        &session,
        &mut terminal,
        &[b"cycle-b"],
        Duration::from_secs(5),
    );
    let paint_at = started.elapsed().as_secs_f64();
    // DIAGNOSTIC BOUND. A wait past `HARNESS_KILL` can only ever be KILLED —
    // and a terminated test prints nothing, which is why four failing CI runs
    // produced no evidence. Derived from what is LEFT of `REPORT_BUDGET`
    // rather than fixed, so a slow first paint eats its own margin instead of
    // the report's: every second the wait does not need is a second the wait
    // gets.
    let (found, seen, chunks) = if painted {
        // The stop condition carries the notice's TAIL: "fifo:" and
        // "? help" follow the needle inside the same line, and a
        // capture returned at the prefix can legally end mid-notice.
        wait_for_bytes_verbose(
            &session,
            &mut terminal,
            &[
                "a, b: trigger loop suspected:".as_bytes(),
                b"fifo:",
                b"? help",
            ],
            REPORT_BUDGET
                .saturating_sub(started.elapsed())
                .max(Duration::from_secs(1)),
        )
    } else {
        (false, Vec::new(), Vec::new())
    };
    // POST-HOC PROBES. Everything from here runs only once the wait has
    // already failed, so none of it can perturb the race it describes.
    //
    // Silence from a pty says nothing on its own: a loop that is turning and
    // declining to accuse looks exactly like a process that died at startup,
    // and those want opposite fixes. So ask whether it is still there, and if
    // it is, RESIZE — the one nudge the shipped code answers unconditionally,
    // by reflowing from retained output. An answer proves the loop still
    // turns and shows the current chrome; silence narrows it to a hang.
    let mut exited = false;
    let mut nudge: Vec<u8> = Vec::new();
    if !found {
        exited = session.exited();
        if !exited {
            session.set_winsize(24, 100);
            let deadline = std::time::Instant::now() + Duration::from_secs(1);
            while std::time::Instant::now() < deadline {
                let chunk = session.read_available(Duration::from_millis(50));
                terminal.respond(&session, &chunk);
                nudge.extend_from_slice(&chunk);
            }
        }
    }
    session.write_bytes(b"q");
    // `exited` already reaped, and `kill_if_alive` cannot tell a reaped child
    // from a live one — it would spin until the harness killed the test and
    // took the report with it.
    if !exited {
        // A short kill deadline on purpose: the shipped 5 s is affordable
        // only when the wait before it succeeded.
        session.kill_if_alive(Duration::from_millis(300));
    }
    if !found {
        println!("=== WEDGE REPORT ===");
        println!(
            "first paint: {painted} at {paint_at:.3}s ({} bytes)",
            first_paint.len()
        );
        println!("bytes seen after first paint: {}", seen.len());
        println!("last chunk at: {:?}", chunks.last().map(|(at, _)| *at));
        println!("bytes per second: {}", per_second(&chunks));
        println!("rat had already exited at the deadline: {exited}");
        println!("bytes answering a resize: {}", nudge.len());
        println!(
            "notice text present at all: {}",
            contains(&seen, b"trigger loop suspected")
        );
        println!(
            "badge present at all: {}",
            contains(&seen, "· looping".as_bytes())
        );
        println!(
            "--- what the first paint was ---\n{}",
            head(&first_paint, 900)
        );
        println!("--- tail of everything after it ---\n{}", tail(&seen, 900));
        println!("--- what the resize painted ---\n{}", tail(&nudge, 900));
        println!("--- trigger trace ---");
        if traced {
            match std::fs::read_to_string(&trace) {
                Ok(t) => println!("{t}"),
                Err(e) => println!("(the trace was asked for and is not there: {e})"),
            }
        } else {
            println!("(control run: RAT_WEDGE_CONTROL set, rat ran uninstrumented)");
        }
        println!("=== END WEDGE REPORT ===");
    }
    assert!(painted, "the first composition never painted");
    assert!(found, "the fifo cycle never earned its report");
    assert!(
        contains(&seen, "· looping".as_bytes()),
        "the badge never reached a chrome row: {:?}",
        String::from_utf8_lossy(&seen)
    );
    // `fifo:` and not `file:` — the evidence named must be the evidence
    // actually used, or the notice would be describing the other route.
    assert!(
        contains(&seen, b"fifo:") && contains(&seen, b"? help"),
        "the notice must name the watched fifos and where to read more"
    );
}

// ---------------------------------------------------------------------
// A live pane: a child spawned once, never expected to exit, whose
// output must reach the frame as it arrives.
// ---------------------------------------------------------------------

/// A live pane's command: record this spawn, then FOLLOW the log without
/// exiting.
///
/// `exec` so the process that outlives the shell IS the follower. A
/// shell that forks instead of execing would absorb a kill while its
/// child kept the pipes open, which is the rule the kill-slot fixtures
/// already follow.
fn following_counter_cmd(counter: &std::path::Path, log: &std::path::Path) -> String {
    format!(
        "echo run >> {c}; exec {rat} __follow {l}",
        c = counter.display(),
        rat = rat_bin(),
        l = log.display()
    )
}

/// A declaration whose panes each say whether they are long-lived.
/// Keeps the format-specific text in this file's builders, beside
/// `board` and `panes_row`. `live` rides the PROPERTY position so the
/// dual spelling gets exercised through a real file, not just a parse.
fn live_board(panes: &[(&str, &str, bool, &str)]) -> String {
    let mut body = String::from(
        "row-gap 0\n\ndefaults {\n    height 5\n    border \"rounded\"\n    shell #true\n}\n",
    );
    for (name, interval, live, command) in panes {
        let live_attr = if *live { " live=#true" } else { "" };
        body.push_str(&format!(
            "\npane \"{name}\"{live_attr} {{\n    interval \"{interval}\"\n    command \"{command}\"\n}}\n"
        ));
    }
    body
}

fn seed(path: &std::path::Path, text: &str) {
    std::fs::write(path, text).expect("seed the log");
}

fn append(path: &std::path::Path, text: &str) {
    use std::io::Write as _;
    std::fs::OpenOptions::new()
        .append(true)
        .open(path)
        .expect("reopen the log")
        .write_all(text.as_bytes())
        .expect("append to the log");
}

/// Whether any process's command line contains `needle`.
///
/// Child-side evidence that a killed child really died, which no screen
/// assertion can give: a frame says nothing about what is still running
/// behind it. POSIX `ps`, since this file is unix-only anyway.
fn any_process_matching(needle: &str) -> bool {
    let out = std::process::Command::new("ps")
        .args(["-A", "-o", "args="])
        .output()
        .expect("ps");
    String::from_utf8_lossy(&out.stdout)
        .lines()
        .any(|line| line.contains(needle))
}

/// THE INVARIANT, and the worst hazard in this feature. The loop calls
/// `schedule.completed()` for every source it drained. A live child that
/// EMITS has not completed: marking it so clears `in_flight` and
/// schedules a second child while the first is still running, whose
/// handle the slot then overwrites — orphaning the original.
///
/// Child-side evidence, never a screen assertion: the fixture appends to
/// a counter on startup, so a second spawn is a second line. The
/// interval is short on purpose — a wrongly-completed tick respawns
/// inside the settle window rather than after it.
#[test]
fn a_progress_event_does_not_complete_the_tick() {
    let dir = tempfile::tempdir().expect("tempdir");
    let (counter, log) = (dir.path().join("counter"), dir.path().join("log"));
    seed(&log, "start\n");
    let decl = write_dashboard(
        dir.path(),
        &live_board(&[(
            "follower",
            "200ms",
            true,
            &following_counter_cmd(&counter, &log),
        )]),
    );
    let session = PtySession::spawn(&rat_bin(), &["dashboard", &decl.display().to_string()], &[])
        .expect("spawn rat dashboard under a pty");
    let mut terminal = FakeTerminal::dark();
    assert!(
        wait_for(&session, &mut terminal, b"start", Duration::from_secs(5)),
        "the live pane never painted"
    );
    // Several emissions, each one a chance to be miscounted as a
    // completion. Without them the test would pass on a source that
    // simply never moved.
    for i in 0..4 {
        append(&log, &format!("line-{i}\n"));
        assert!(
            wait_for(
                &session,
                &mut terminal,
                format!("line-{i}").as_bytes(),
                Duration::from_secs(5)
            ),
            "append {i} never reached the frame"
        );
    }
    assert_counter_settled_at(&counter, 1);
}

/// The measured defect: a dashboard whose only pane is live painted 25
/// bytes, every one of them terminal capability probing. The compose
/// block was gated on a non-empty drain, and a child that never exits
/// never posts.
#[test]
fn a_dashboard_of_only_live_panes_paints_without_any_completion() {
    let dir = tempfile::tempdir().expect("tempdir");
    let (counter, log) = (dir.path().join("counter"), dir.path().join("log"));
    seed(&log, "first line\n");
    let decl = write_dashboard(
        dir.path(),
        &live_board(&[(
            "follower",
            "1h",
            true,
            &following_counter_cmd(&counter, &log),
        )]),
    );
    let session = PtySession::spawn(&rat_bin(), &["dashboard", &decl.display().to_string()], &[])
        .expect("spawn rat dashboard under a pty");
    let mut terminal = FakeTerminal::dark();
    // A 1h interval: nothing but the emission itself can produce this
    // frame, so a pass cannot come from a second tick.
    assert!(
        wait_for(
            &session,
            &mut terminal,
            b"first line",
            Duration::from_secs(5)
        ),
        "a live-only dashboard never painted"
    );
}

/// Following, not just a first paint. The order matters: appending
/// BEFORE the first paint would let one read satisfy this.
#[test]
fn appended_lines_reach_an_already_painted_frame() {
    let dir = tempfile::tempdir().expect("tempdir");
    let (counter, log) = (dir.path().join("counter"), dir.path().join("log"));
    seed(&log, "first line\n");
    let decl = write_dashboard(
        dir.path(),
        &live_board(&[(
            "follower",
            "1h",
            true,
            &following_counter_cmd(&counter, &log),
        )]),
    );
    let session = PtySession::spawn(&rat_bin(), &["dashboard", &decl.display().to_string()], &[])
        .expect("spawn rat dashboard under a pty");
    let mut terminal = FakeTerminal::dark();
    assert!(
        wait_for(
            &session,
            &mut terminal,
            b"first line",
            Duration::from_secs(5)
        ),
        "the first line never painted"
    );
    append(&log, "second line\n");
    assert!(
        wait_for(
            &session,
            &mut terminal,
            b"second line",
            Duration::from_secs(5)
        ),
        "an append never reached the painted frame"
    );
}

/// The mixed case measured on main, where the batch pane painted and the
/// live pane's box stayed empty while looking perfectly healthy.
#[test]
fn a_live_pane_beside_a_batch_pane_shows_both() {
    let dir = tempfile::tempdir().expect("tempdir");
    let (counter, log) = (dir.path().join("counter"), dir.path().join("log"));
    let batch = dir.path().join("batch");
    seed(&log, "followed-text\n");
    let decl = write_dashboard(
        dir.path(),
        &live_board(&[
            (
                "follower",
                "1h",
                true,
                &following_counter_cmd(&counter, &log),
            ),
            ("batch", "1h", false, &labeled_counter_cmd(&batch, "batch")),
        ]),
    );
    let session = PtySession::spawn(&rat_bin(), &["dashboard", &decl.display().to_string()], &[])
        .expect("spawn rat dashboard under a pty");
    let mut terminal = FakeTerminal::dark();
    // ONE accumulated wait: each needle paints exactly once at a 1h
    // cadence, so a second wait starting fresh could miss the first.
    let seen = wait_for_bytes(&session, &mut terminal, b"batch-1", Duration::from_secs(5))
        .expect("the batch pane never painted");
    if !contains(&seen, b"followed-text") {
        let more = wait_for_bytes(
            &session,
            &mut terminal,
            b"followed-text",
            Duration::from_secs(5),
        );
        assert!(more.is_some(), "the live pane never painted beside it");
    }
}

/// The repaint gate must still hold. A live pane that stops receiving
/// must stop painting: a frame that thrashes is worse than one that
/// lags, and a follower sits quiet most of the time.
#[test]
fn a_live_pane_that_stops_moving_stops_painting() {
    let dir = tempfile::tempdir().expect("tempdir");
    let (counter, log) = (dir.path().join("counter"), dir.path().join("log"));
    seed(&log, "settled\n");
    let decl = write_dashboard(
        dir.path(),
        &live_board(&[(
            "follower",
            "1h",
            true,
            &following_counter_cmd(&counter, &log),
        )]),
    );
    let session = PtySession::spawn(&rat_bin(), &["dashboard", &decl.display().to_string()], &[])
        .expect("spawn rat dashboard under a pty");
    let mut terminal = FakeTerminal::dark();
    assert!(
        wait_for(&session, &mut terminal, b"settled", Duration::from_secs(5)),
        "the live pane never painted"
    );
    // Drain the tail of the first composition, then require silence.
    // The follower keeps polling its file throughout — the claim is that
    // a poll finding nothing writes nothing. `drain_for`, not a single
    // `read_available`: the latter returns after its FIRST read, and a
    // first-composition tail larger than one read would land in the
    // silence window as a false repaint.
    let _ = drain_for(&session, Duration::from_millis(500));
    let quiet = session.read_available(Duration::from_millis(800));
    assert!(
        quiet.is_empty(),
        "a quiet live pane repainted {} bytes: {:?}",
        quiet.len(),
        String::from_utf8_lossy(&quiet)
    );
}

/// A live command that exits gets respawned, and the replacement's fresh
/// content must not render under the dead child's `exit N` — that reads
/// as a pane failing while it works. Nothing else can clear the badge:
/// the replacement never completes, so it never posts a status.
///
/// The absence is asserted against a FULL repaint, forced by a resize.
/// Asserting it against the incremental stream would pass for the wrong
/// reason: a badge that survived leaves its row byte-identical, so the
/// differ rewrites nothing and the stale text never appears either way.
#[test]
fn a_live_pane_that_exits_and_respawns_drops_the_old_exit_badge() {
    let dir = tempfile::tempdir().expect("tempdir");
    let (counter, log) = (dir.path().join("counter"), dir.path().join("log"));
    seed(&log, "second-run-content\n");
    // Run 1 exits 3 and prints nothing; run 2 follows and stays alive.
    let command = format!(
        "echo run >> {c}; if [ $(wc -l < {c}) -eq 1 ]; then exit 3; fi; exec {rat} __follow {l}",
        c = counter.display(),
        rat = rat_bin(),
        l = log.display()
    );
    let decl = write_dashboard(
        dir.path(),
        &live_board(&[("follower", "300ms", true, &command)]),
    );
    let session = PtySession::spawn(&rat_bin(), &["dashboard", &decl.display().to_string()], &[])
        .expect("spawn rat dashboard under a pty");
    let mut terminal = FakeTerminal::dark();
    // ONE ordered capture for the badge and the replacement: run 2's
    // spawn is the schedule's doing, not this test's, so its content
    // can share a read chunk with the badge frame — a second, fresh
    // wait would lose a consumed paint for good (the follower emits
    // `second-run-content` exactly once).
    wait_for_in_order(
        &session,
        &mut terminal,
        &[b"exit 3", b"second-run-content"],
        Duration::from_secs(10),
    );
    session.set_winsize(24, 120);
    // 90 cells: a needle no 80-column border can contain, so the match
    // IS the post-resize frame — at 80 columns this single pane's top
    // border already runs past 45. `╰` closes the box BELOW the chrome
    // row, so the capture provably includes the row a surviving badge
    // would occupy; stopped at the top border, the absence would be
    // asserted against bytes that never reached the badge's row.
    let wide = "─".repeat(90);
    let bytes = wait_for_in_order(
        &session,
        &mut terminal,
        &[wide.as_bytes(), "╰".as_bytes()],
        Duration::from_secs(5),
    );
    assert!(
        !contains(&bytes, b"exit 3"),
        "the replacement child inherited the dead child's exit badge"
    );
}

/// `--once` with a live pane needs no decision — the shipped loop
/// already determines it and this pins it. The first emission sets the
/// source's posted flag, which satisfies the once condition, so one
/// complete frame is emitted and the loop breaks; the held shutdown
/// guard then kills a child that would otherwise outlive the process.
#[test]
fn once_emits_one_frame_from_a_live_pane_and_leaves_no_orphan() {
    let dir = tempfile::tempdir().expect("tempdir");
    let (counter, log) = (dir.path().join("counter"), dir.path().join("log"));
    seed(&log, "once-content\n");
    let decl = write_dashboard(
        dir.path(),
        &live_board(&[(
            "follower",
            "1h",
            true,
            &following_counter_cmd(&counter, &log),
        )]),
    );
    let session = PtySession::spawn(
        &rat_bin(),
        &["dashboard", "--once", &decl.display().to_string()],
        &[],
    )
    .expect("spawn rat dashboard under a pty");
    let mut terminal = FakeTerminal::dark();
    assert!(
        wait_for(
            &session,
            &mut terminal,
            b"once-content",
            Duration::from_secs(5)
        ),
        "--once never emitted the live pane's frame"
    );
    let deadline = std::time::Instant::now() + Duration::from_secs(5);
    while !session.exited() {
        assert!(
            std::time::Instant::now() < deadline,
            "--once never exited with a live pane"
        );
        let _ = session.read_available(Duration::from_millis(50));
    }
    // The follower is spawned once and killed on the way out; a survivor
    // would hold a pipe and outlive the run that started it.
    assert_counter_settled_at(&counter, 1);
    assert!(
        !any_process_matching(&log.display().to_string()),
        "the live child outlived the --once run that spawned it"
    );
}

/// The lifecycle hazard unique to this feature: a follower has no exit
/// of its own, so if shutdown does not reach it, quitting rat leaks a
/// process that holds the log open forever. The batch q test above
/// cannot stand in — its children die of natural causes one second
/// later, so a missed kill has no symptom there.
///
/// Child-side evidence on both halves: the counter says the child was
/// spawned exactly once, and `ps` says nothing matching the log path
/// survived the quit.
#[test]
fn q_shuts_down_a_child_that_would_never_exit() {
    let dir = tempfile::tempdir().expect("tempdir");
    let (counter, log) = (dir.path().join("counter"), dir.path().join("log"));
    seed(&log, "start\n");
    let decl = write_dashboard(
        dir.path(),
        &live_board(&[(
            "follower",
            "1h",
            true,
            &following_counter_cmd(&counter, &log),
        )]),
    );
    let session = PtySession::spawn(&rat_bin(), &["dashboard", &decl.display().to_string()], &[])
        .expect("spawn rat dashboard under a pty");
    let mut terminal = FakeTerminal::dark();
    assert!(
        wait_for(&session, &mut terminal, b"start", Duration::from_secs(5)),
        "the live pane never painted"
    );
    session.write_bytes(b"q");
    assert!(
        !session.kill_if_alive(Duration::from_secs(5)),
        "the dashboard should have exited on q"
    );
    assert_counter_settled_at(&counter, 1);
    assert!(
        !any_process_matching(&log.display().to_string()),
        "the live child outlived the q that shut its dashboard down"
    );
}

/// A follower is the shape most likely to flood — that is the point of
/// it — so the bound and its marker get an end-to-end test on THIS
/// route. The worker-side cap is pinned in units; nothing before this
/// pins that a live source's drop count survives the emission, the
/// drain, and the compose to reach its own chrome row.
///
/// 1500 seeded lines against the 1000-line cap: the count is exact, so
/// a marker that merely says "some" cannot pass. The tail line proves
/// the keep direction on this route too — a live pane keeps its tail,
/// and `flood-1499` is only on screen if the HEAD is what went.
#[test]
fn a_live_pane_bounded_at_its_cap_reports_the_drop() {
    let dir = tempfile::tempdir().expect("tempdir");
    let (counter, log) = (dir.path().join("counter"), dir.path().join("log"));
    let body: String = (0..1500).map(|i| format!("flood-{i}\n")).collect();
    seed(&log, &body);
    let decl = write_dashboard(
        dir.path(),
        &live_board(&[(
            "follower",
            "1h",
            true,
            &following_counter_cmd(&counter, &log),
        )]),
    );
    let session = PtySession::spawn(&rat_bin(), &["dashboard", &decl.display().to_string()], &[])
        .expect("spawn rat dashboard under a pty");
    let mut terminal = FakeTerminal::dark();
    // ONE capture for both needles: they arrive in the same repaint,
    // and a second wait that starts after the first consumed it would
    // watch a stream where the now-unchanged chrome row never paints
    // again — the differ trap this plan has already hit once, in a new
    // costume.
    let seen = wait_for_bytes(
        &session,
        &mut terminal,
        b"500 lines dropped",
        Duration::from_secs(5),
    )
    .expect("the live pane's chrome never reported the drop");
    assert!(
        contains(&seen, b"flood-1499"),
        "the flood's tail never reached the frame"
    );
}

/// A single live pane with a `file:` trigger — the declaration whose
/// contract these tests settle.
fn live_trigger_board(interval: &str, command: &str, trigger: &std::path::Path) -> String {
    format!(
        "row-gap 0\n\ndefaults {{\n    height 5\n    border \"rounded\"\n    shell #true\n}}\n\n\
         pane \"follower\" live=#true {{\n    interval \"{interval}\"\n    trigger \"file:{t}\"\n    \
         trigger-debounce \"100ms\"\n    command \"{command}\"\n}}\n",
        t = trigger.display()
    )
}

/// The trigger contract on a live pane: a fire RESTARTS the child —
/// the revocable kill discharges the respawn request that
/// single-in-flight would otherwise hold forever. Child-side evidence
/// for both halves: the counter says a second spawn happened, and the
/// appended line proves the REPLACEMENT is the one following the log.
#[test]
fn a_file_trigger_restarts_a_live_child() {
    let dir = tempfile::tempdir().expect("tempdir");
    let (counter, log) = (dir.path().join("counter"), dir.path().join("log"));
    let poke = dir.path().join("poke");
    seed(&log, "start\n");
    seed(&poke, "seed\n");
    let decl = write_dashboard(
        dir.path(),
        &live_trigger_board("1h", &following_counter_cmd(&counter, &log), &poke),
    );
    let session = PtySession::spawn(&rat_bin(), &["dashboard", &decl.display().to_string()], &[])
        .expect("spawn rat dashboard under a pty");
    let mut terminal = FakeTerminal::dark();
    assert!(
        wait_for(&session, &mut terminal, b"start", Duration::from_secs(5)),
        "the live pane never painted"
    );
    wait_for_counter(&counter, 1);
    append(&poke, "fire\n");
    // The respawn: a second child, and exactly one — the debounce
    // collapses the single fire into a single restart.
    wait_for_counter(&counter, 2);
    append(&log, "after-restart\n");
    assert!(
        wait_for(
            &session,
            &mut terminal,
            b"after-restart",
            Duration::from_secs(5)
        ),
        "the replacement child is not following"
    );
    assert_counter_settled_at(&counter, 2);
}

/// The compounding case, by name: a trigger-only live pane whose child
/// exits. With no interval its schedule holds no deadline, so without
/// the trigger this pane is dead forever — and before this contract
/// landed, the trigger was inert on top. The fire must revive it.
#[test]
fn a_trigger_respawns_a_live_child_that_already_exited() {
    let dir = tempfile::tempdir().expect("tempdir");
    let (counter, poke) = (dir.path().join("counter"), dir.path().join("poke"));
    seed(&poke, "seed\n");
    // A live-declared command that prints its run number and exits at
    // once: run 1 paints `gone-1` and dies, leaving no child and no
    // deadline.
    let decl = write_dashboard(
        dir.path(),
        &live_trigger_board("never", &labeled_counter_cmd(&counter, "gone"), &poke),
    );
    let session = PtySession::spawn(&rat_bin(), &["dashboard", &decl.display().to_string()], &[])
        .expect("spawn rat dashboard under a pty");
    let mut terminal = FakeTerminal::dark();
    assert!(
        wait_for(&session, &mut terminal, b"gone-1", Duration::from_secs(5)),
        "the first run never painted"
    );
    append(&poke, "fire\n");
    assert!(
        wait_for(&session, &mut terminal, b"gone-2", Duration::from_secs(5)),
        "the trigger never revived the dead live pane"
    );
    assert_counter_settled_at(&counter, 2);
}

/// The supersede ladder at the trigger site: a fire delivers SIGTERM
/// first, so a child that handles it flushes a farewell — which rides
/// the completion into the frame — before the replacement spawns.
/// Under the old SIGKILL supersede the farewell can never exist.
#[test]
fn a_trigger_fire_lets_the_live_child_flush_before_the_replacement() {
    let dir = tempfile::tempdir().expect("tempdir");
    let (counter, poke) = (dir.path().join("counter"), dir.path().join("poke"));
    seed(&poke, "seed\n");
    // Run N prints ready-N and waits, trapping TERM to flush a
    // farewell. Single quotes only (a KDL double-quoted string carries
    // this verbatim); the shell IS the child (compound body, no exec)
    // so the trap lives in the signalled process; `sleep 0.1` bounds
    // trap latency; `tr -d` normalizes macOS wc's padding; every
    // witness line is echo-terminated because the live feed withholds
    // an unterminated line.
    let cmd = format!(
        "echo run >> {c}; echo ready-$(wc -l < {c} | tr -d ' '); \
         trap 'echo bye-graceful; exit 0' TERM; while :; do sleep 0.1; done",
        c = counter.display()
    );
    let decl = write_dashboard(dir.path(), &live_trigger_board("1h", &cmd, &poke));
    let session = PtySession::spawn(&rat_bin(), &["dashboard", &decl.display().to_string()], &[])
        .expect("spawn rat dashboard under a pty");
    let mut terminal = FakeTerminal::dark();
    assert!(
        wait_for(&session, &mut terminal, b"ready-1", Duration::from_secs(5)),
        "the live pane never painted"
    );
    append(&poke, "fire\n");
    // ONE capture for the whole restart sequence. The farewell is on
    // screen only between the completion's paint and the replacement's
    // (record_output replaces the body), so two sequential waits would
    // race that window; the raw byte stream keeps everything painted.
    let bytes = wait_for_bytes(&session, &mut terminal, b"ready-2", Duration::from_secs(10))
        .expect("the replacement never painted");
    let farewell = position(&bytes, b"bye-graceful")
        .expect("the farewell never reached the frame — the child was not allowed to flush");
    let replacement = position(&bytes, b"ready-2").expect("contained by the wait");
    assert!(
        farewell < replacement,
        "the farewell must paint before the replacement does"
    );
    assert_counter_settled_at(&counter, 2);
}

/// A syntax error on a real terminal paints its snippet. The load
/// error prints before any UI exists, and the colored theme is gated
/// on the profile the tty earns. `RAT_APPEARANCE` suppresses the
/// startup OSC probe, so every escape in the drained bytes belongs to
/// the snippet — the plain path emits none at all.
#[test]
fn a_syntax_error_on_a_tty_paints_its_snippet() {
    let dir = tempfile::tempdir().expect("tempdir");
    let decl = dir.path().join("bad.kdl");
    std::fs::write(
        &decl,
        "pane \"log\" interval=5s {\n    command \"date\"\n}\n",
    )
    .expect("write declaration");
    let session = PtySession::spawn(
        &rat_bin(),
        &["dashboard", &decl.display().to_string()],
        &[("RAT_APPEARANCE", "dark")],
    )
    .expect("spawn rat dashboard under a pty");
    let mut terminal = FakeTerminal::dark();
    // ONE ordered capture: the head line is the locator, but the color
    // evidence is the block that FOLLOWS it — a capture stopped at the
    // head can legally end before the escapes arrive (a read may
    // return at any byte), so wait until an escape has appeared AFTER
    // the head rather than asserting on whatever the read boundary
    // happened to deliver.
    wait_for_in_order(
        &session,
        &mut terminal,
        &[b"line 1, column", b"\x1b["],
        Duration::from_secs(5),
    );
}

/// The session owns the tab title: stack pushed and the marker title
/// set before the first frame, stack popped on quit. A plain `rat
/// watch` session in the same harness never says `]2;` at all — the
/// dashboard-only scoping, pinned from the outside.
#[test]
fn an_interactive_dashboard_owns_the_tab_title_and_restores_it() {
    let dir = tempfile::tempdir().expect("tempdir");
    let steady = dir.path().join("steady");
    std::fs::write(&steady, "steady-content").expect("seed");
    let decl = write_dashboard(
        dir.path(),
        &format!(
            "pane \"deploy\" {{\n    height 3\n    chrome #false\n    border \"none\"\n    command \"{}\" \"__cat\" \"{}\"\n}}\n",
            rat_bin().escape_default(),
            steady.display().to_string().escape_default(),
        ),
    );
    let session = PtySession::spawn(
        &rat_bin(),
        &["dashboard", &decl.display().to_string()],
        &[("RAT_APPEARANCE", "dark")],
    )
    .expect("spawn rat dashboard under a pty");
    let mut terminal = FakeTerminal::dark();
    let bytes = wait_for_bytes(
        &session,
        &mut terminal,
        b"steady-content",
        Duration::from_secs(5),
    )
    .expect("the first frame painted");
    let push = find(&bytes, b"\x1b[22;2t").expect("the title stack is pushed");
    let set = find(&bytes, b"\x1b]2;\xe2\x96\x9e ").expect("the marker title is set");
    assert!(push < set, "push before set");

    session.write_bytes(b"q");
    let mut rest = bytes;
    let deadline = std::time::Instant::now() + Duration::from_secs(5);
    while !session.exited() && std::time::Instant::now() < deadline {
        let chunk = session.read_available(Duration::from_millis(50));
        terminal.respond(&session, &chunk);
        rest.extend_from_slice(&chunk);
    }
    let chunk = session.read_available(Duration::from_millis(200));
    rest.extend_from_slice(&chunk);
    let pop = find(&rest, b"\x1b[23;2t").expect("the title stack is popped on quit");
    assert!(pop > set, "pop after set");
}

/// The scoping's other half: an interactive plain watch never touches
/// the tab title.
#[test]
fn a_plain_watch_session_never_touches_the_tab_title() {
    let dir = tempfile::tempdir().expect("tempdir");
    let steady = dir.path().join("steady");
    std::fs::write(&steady, "watch-content").expect("seed");
    let session = PtySession::spawn(
        &rat_bin(),
        &[
            "watch",
            "-n",
            "1s",
            "--",
            &rat_bin(),
            "__cat",
            &steady.display().to_string(),
        ],
        &[("RAT_APPEARANCE", "dark")],
    )
    .expect("spawn rat watch under a pty");
    let mut terminal = FakeTerminal::dark();
    let bytes = wait_for_bytes(
        &session,
        &mut terminal,
        b"watch-content",
        Duration::from_secs(5),
    )
    .expect("the first frame painted");
    assert!(
        find(&bytes, b"\x1b]2;").is_none(),
        "watch never sets a tab title"
    );
    session.write_bytes(b"q");
    session.kill_if_alive(Duration::from_secs(3));
}

fn find(haystack: &[u8], needle: &[u8]) -> Option<usize> {
    haystack
        .windows(needle.len())
        .position(|window| window == needle)
}

/// A pane-sourced title follows the pane: as the referenced pane's
/// first line changes, the tab title is re-emitted with the new role
/// text — and only then; the emitter is idempotent per text.
#[test]
fn a_pane_sourced_tab_title_follows_the_panes_output() {
    let dir = tempfile::tempdir().expect("tempdir");
    let counter = dir.path().join("count");
    let decl = write_dashboard(
        dir.path(),
        &format!(
            "title \"warming up\" ref=\"#c\"\n\npane \"c\" {{\n    height 3\n    chrome #false\n    border \"none\"\n    shell #true\n    interval \"1s\"\n    command \"{}\"\n}}\n",
            counter_cmd(&counter),
        ),
    );
    let session = PtySession::spawn(
        &rat_bin(),
        &["dashboard", &decl.display().to_string()],
        &[("RAT_APPEARANCE", "dark")],
    )
    .expect("spawn rat dashboard under a pty");
    let mut terminal = FakeTerminal::dark();
    // ONE ordered capture for both titles: the pane free-runs at 1s
    // and the emitter is idempotent per text, so a `count-2` title
    // consumed alongside the `count-1` wait would never be re-emitted
    // — the next change says count-3.
    wait_for_in_order(
        &session,
        &mut terminal,
        &[
            b"\x1b]2;\xe2\x96\x9e count-1\x07",
            b"\x1b]2;\xe2\x96\x9e count-2\x07",
        ],
        Duration::from_secs(11),
    );
    session.write_bytes(b"q");
    session.kill_if_alive(Duration::from_secs(3));
}

#[test]
fn a_hidden_tail_change_never_highlights_the_neighbour() {
    // The left pane's line is wider than its 14-cell box: chars 15..18
    // change while only chars 0..13 (+ ellipsis) are visible. With
    // highlights on, the change past the cut must not paint reverse
    // video into the gap or the right pane — pre-clip, the leaked run
    // landed exactly on the neighbour's first characters.
    let dir = tempfile::tempdir().expect("tempdir");
    let value = dir.path().join("value");
    std::fs::write(&value, "0-abcdefghijk-AAA").expect("seed");
    let decl = write_dashboard(
        dir.path(),
        &format!(
            r#"
gap 1
row-gap 0

defaults {{
    height 1
    border "none"
    chrome #false
    shell #true
}}

row {{
    pane "wide" {{
        interval "250ms"
        width "14"
        command "printf 'v%s' \"$(cat {value})\""
    }}
    pane "quiet" {{
        interval "10s"
        command "printf 'zebra-static'"
    }}
}}
"#,
            value = value.display(),
        ),
    );

    let session = PtySession::spawn(&rat_bin(), &["dashboard", &decl.display().to_string()], &[])
        .expect("spawn rat dashboard under a pty");
    let mut terminal = FakeTerminal::dark();
    assert!(
        wait_for(
            &session,
            &mut terminal,
            b"zebra-static",
            Duration::from_secs(5)
        ),
        "the first composition never painted"
    );
    session.write_bytes(b"c");
    std::fs::write(&value, "1-abcdefghijk-BBB").expect("the change");
    // The digit flip is the positive control: highlights were live on
    // the very repaint that carried the hidden-tail change. ONE
    // ordered capture through the NEIGHBOUR'S OWN CELLS: the differ
    // rewrites whole rows, and both panes share this row, so every
    // paint that carries the highlight carries `zebra-static` after it
    // — a capture stopped at the highlight could end before the
    // neighbour's columns and pass the absence vacuously. (A leak that
    // splices reverse video INTO the neighbour breaks the contiguous
    // needle and times out — still a failure, seen in the dump.)
    let seen = wait_for_in_order(
        &session,
        &mut terminal,
        &[b"\x1b[7m1\x1b[0m", b"zebra-static"],
        Duration::from_secs(5),
    );
    assert!(
        !contains(&seen, b"\x1b[7mzeb"),
        "the hidden tail's run leaked onto the neighbour: {:?}",
        String::from_utf8_lossy(&seen)
    );
    session.write_bytes(b"q");
    assert!(
        !session.kill_if_alive(Duration::from_secs(2)),
        "dashboard should have exited on q"
    );
}

#[test]
fn the_gutter_reflows_the_panes_instead_of_cutting_them() {
    // The gutter is a region of the frame, not an overlay: turning it
    // on must reserve its two columns from the allocation budget, so
    // the rightmost pane keeps its border. And the reflow must never
    // read as a resize — a view toggle that respawned every child
    // (including live ones) 250ms later would be a real cost, so the
    // counter file is the negative pin: no extra tick after the
    // debounce window has passed.
    let dir = tempfile::tempdir().expect("tempdir");
    let count = dir.path().join("count");
    let decl = write_dashboard(
        dir.path(),
        &format!(
            r#"
gap 1
row-gap 0

defaults {{
    height 3
    border "rounded"
    chrome #false
    shell #true
}}

row {{
    pane "a" {{
        interval "10s"
        command "{counter}"
    }}
    pane "b" {{
        interval "10s"
        command "printf 'zzz'"
    }}
}}
"#,
            counter = labeled_counter_cmd(&count, "a"),
        ),
    );

    let session = PtySession::spawn(&rat_bin(), &["dashboard", &decl.display().to_string()], &[])
        .expect("spawn rat dashboard under a pty");
    let mut terminal = FakeTerminal::dark();
    let first = wait_for_bytes(&session, &mut terminal, b"zzz", Duration::from_secs(5))
        .expect("the first composition never painted");
    let top = row_containing(&first, "╭".as_bytes()).expect("the first top-border row");
    let corners = |row: &[u8]| String::from_utf8_lossy(row).matches('╮').count();
    assert_eq!(corners(top), 2, "both panes start with their corners");

    session.write_bytes(b"D");
    // The gutter margin lands before the border's SGR escape, so the
    // contiguous needle is the row separator plus the two-space margin.
    let seen = wait_for_bytes(&session, &mut terminal, b"\r\n  ", Duration::from_secs(3))
        .expect("the gutter repaint never arrived");
    let top = row_containing(&seen, "╭".as_bytes()).expect("the reflowed top-border row");
    assert_eq!(
        corners(top),
        2,
        "the rightmost pane's corner must survive the gutter: {:?}",
        String::from_utf8_lossy(top)
    );

    // Past the resize debounce: a geometry drift would have respawned
    // every child and the counter would read 2.
    let _ = drain_for(&session, Duration::from_millis(700));
    let runs = std::fs::read_to_string(&count)
        .map(|s| s.lines().count())
        .unwrap_or(0);
    assert_eq!(runs, 1, "a view toggle must never restart children");

    session.write_bytes(b"q");
    assert!(
        !session.kill_if_alive(Duration::from_secs(2)),
        "dashboard should have exited on q"
    );
}

/// The KDL surface reaching real bytes: a NAMED shell in `defaults`
/// spawns the pane's script through that shell — the arithmetic only
/// computes if `sh` actually ran it.
#[test]
fn a_named_shell_in_defaults_reaches_the_frame() {
    let dir = tempfile::tempdir().expect("tempdir");
    let decl = write_dashboard(
        dir.path(),
        "defaults height=5 shell=\"sh\"\npane \"math\" {\n    command \"echo $((6 * 7))\"\n}\n",
    );

    let session = PtySession::spawn(&rat_bin(), &["dashboard", &decl.display().to_string()], &[])
        .expect("spawn rat dashboard under a pty");
    let mut terminal = FakeTerminal::dark();
    assert!(
        wait_for(&session, &mut terminal, b"42", Duration::from_secs(5)),
        "the named shell's arithmetic never painted"
    );

    session.write_bytes(b"q");
    assert!(
        !session.kill_if_alive(Duration::from_secs(2)),
        "dashboard should have exited on q"
    );
}

/// The interpreter route, discriminated: under kernel-exec `$0` IS the
/// materialized path (so the frame carries the private dir's
/// `rat-script.` prefix); under the shell fallback `$0` would be `sh`.
#[test]
fn a_shebang_script_body_paints_through_its_own_interpreter() {
    let dir = tempfile::tempdir().expect("tempdir");
    let decl = write_dashboard(
        dir.path(),
        "defaults height=5 chrome=#false\npane \"self\" {\n    script \"\"\"\n        #!/bin/sh\n        echo $0\n        \"\"\"\n}\n",
    );

    let session = PtySession::spawn(&rat_bin(), &["dashboard", &decl.display().to_string()], &[])
        .expect("spawn rat dashboard under a pty");
    let mut terminal = FakeTerminal::dark();
    assert!(
        wait_for(
            &session,
            &mut terminal,
            b"rat-script.",
            Duration::from_secs(5)
        ),
        "the script's own path never painted — the body did not run as a file"
    );

    session.write_bytes(b"q");
    assert!(
        !session.kill_if_alive(Duration::from_secs(2)),
        "dashboard should have exited on q"
    );
}

/// Written once per load, the SAME path re-executed each tick: the body
/// appends its `$0` to a side file, and every appended line must agree.
/// Per-tick materialization would show a different random directory.
#[test]
fn a_respawn_reuses_the_script_written_at_load() {
    let dir = tempfile::tempdir().expect("tempdir");
    let log = dir.path().join("paths.log");
    let decl = write_dashboard(
        dir.path(),
        &format!(
            "defaults height=5 interval=\"1s\" chrome=#false\npane \"self\" {{\n    script \"\"\"\n        #!/bin/sh\n        echo $0 >> \"{}\"\n        echo ran\n        \"\"\"\n}}\n",
            log.display()
        ),
    );

    let session = PtySession::spawn(&rat_bin(), &["dashboard", &decl.display().to_string()], &[])
        .expect("spawn rat dashboard under a pty");
    let mut terminal = FakeTerminal::dark();
    assert!(
        wait_for(&session, &mut terminal, b"ran", Duration::from_secs(5)),
        "the script never painted"
    );
    // Wait for the SECOND tick's evidence in the side file — file
    // bytes, not a drain window.
    let deadline = std::time::Instant::now() + Duration::from_secs(10);
    let lines = loop {
        let text = std::fs::read_to_string(&log).unwrap_or_default();
        let lines: Vec<String> = text.lines().map(str::to_string).collect();
        if lines.len() >= 2 {
            break lines;
        }
        assert!(
            std::time::Instant::now() < deadline,
            "second tick never ran; log so far: {text:?}"
        );
        std::thread::sleep(Duration::from_millis(100));
    };
    assert!(lines[0].contains("rat-script."), "{lines:?}");
    assert!(
        lines.iter().all(|line| line == &lines[0]),
        "ticks ran different files: {lines:?}"
    );

    session.write_bytes(b"q");
    assert!(
        !session.kill_if_alive(Duration::from_secs(2)),
        "dashboard should have exited on q"
    );
}

/// The whole lifecycle's cheapest strong witness: the private directory
/// is gone once the dashboard exits.
#[test]
fn the_script_directory_is_gone_when_the_dashboard_exits() {
    let dir = tempfile::tempdir().expect("tempdir");
    let log = dir.path().join("path.log");
    let decl = write_dashboard(
        dir.path(),
        &format!(
            "defaults height=5 chrome=#false\npane \"self\" {{\n    script \"\"\"\n        #!/bin/sh\n        echo $0 > \"{}\"\n        echo ran\n        \"\"\"\n}}\n",
            log.display()
        ),
    );

    let session = PtySession::spawn(&rat_bin(), &["dashboard", &decl.display().to_string()], &[])
        .expect("spawn rat dashboard under a pty");
    let mut terminal = FakeTerminal::dark();
    assert!(
        wait_for(&session, &mut terminal, b"ran", Duration::from_secs(5)),
        "the script never painted"
    );
    let script_path = std::path::PathBuf::from(
        std::fs::read_to_string(&log)
            .expect("the body wrote its path")
            .trim()
            .to_string(),
    );
    let script_dir = script_path.parent().expect("a parent dir").to_path_buf();
    assert!(script_dir.exists(), "{script_dir:?}");

    session.write_bytes(b"q");
    assert!(
        !session.kill_if_alive(Duration::from_secs(2)),
        "dashboard should have exited on q"
    );
    assert!(
        !script_dir.exists(),
        "the private script directory survived exit: {script_dir:?}"
    );
}

/// The whole focus indication, end to end: the border restyle and the
/// footer segment arrive in ONE repaint, borders first (the frame
/// paints top to bottom), so they are one ordered needle chain rather
/// than two waits that would race each other.
#[test]
fn tab_focuses_the_first_pane_and_esc_clears_it() {
    let dir = tempfile::tempdir().expect("tempdir");
    let count = dir.path().join("count");
    let decl = write_dashboard(
        dir.path(),
        &format!(
            r#"
gap 1
row-gap 0

defaults {{
    height 3
    border "rounded"
    chrome #false
    shell #true
    interval "10s"
}}

row {{
    pane "a" {{
        command "{counter}"
    }}
    pane "b" {{
        command "printf 'zzz'"
    }}
}}
"#,
            counter = labeled_counter_cmd(&count, "a"),
        ),
    );

    let session = PtySession::spawn(&rat_bin(), &["dashboard", &decl.display().to_string()], &[])
        .expect("spawn rat dashboard under a pty");
    let mut terminal = FakeTerminal::dark();
    assert!(
        wait_for(&session, &mut terminal, b"a-1", Duration::from_secs(5)),
        "the first composition never painted"
    );

    session.write_bytes(b"\t");
    let _ = wait_for_in_order(
        &session,
        &mut terminal,
        &[b"\x1b[1;38;5;212m", b"focus a"],
        Duration::from_secs(5),
    );

    // A bare escape resolves to Esc only after 50ms of silence, and any
    // byte inside that window cancels it — so nothing else is sent here.
    session.write_bytes(b"\x1b");
    let cleared = drain_for(&session, Duration::from_millis(700));
    let footer = row_containing(&cleared, b"? help").expect("the status row after Esc");
    assert!(
        !contains(footer, b"focus"),
        "Esc must clear the focus segment: {:?}",
        String::from_utf8_lossy(footer)
    );

    session.write_bytes(b"q");
    assert!(
        !session.kill_if_alive(Duration::from_secs(2)),
        "dashboard should have exited on q"
    );
}

/// The negative respawn pin, the gutter toggle's shape: focus moves
/// what is composed, never what is allocated, so detect_resize derives
/// the same geometry and the 250ms gate stays unarmed. A gesture that
/// restarted every child — live ones included — would be a real cost
/// paid for a border color.
#[test]
fn a_focus_gesture_never_restarts_a_child() {
    let dir = tempfile::tempdir().expect("tempdir");
    let count = dir.path().join("count");
    let decl = write_dashboard(
        dir.path(),
        &format!(
            r#"
gap 1
row-gap 0

defaults {{
    height 3
    border "rounded"
    chrome #false
    shell #true
    interval "10s"
}}

row {{
    pane "a" {{
        command "{counter}"
    }}
    pane "b" {{
        command "printf 'zzz'"
    }}
}}
"#,
            counter = labeled_counter_cmd(&count, "a"),
        ),
    );

    let session = PtySession::spawn(&rat_bin(), &["dashboard", &decl.display().to_string()], &[])
        .expect("spawn rat dashboard under a pty");
    let mut terminal = FakeTerminal::dark();
    assert!(
        wait_for(&session, &mut terminal, b"a-1", Duration::from_secs(5)),
        "the first composition never painted"
    );

    session.write_bytes(b"\t");
    let _ = wait_for_in_order(
        &session,
        &mut terminal,
        &[b"\x1b[1;38;5;212m", b"focus a"],
        Duration::from_secs(5),
    );
    // Past the resize debounce: a geometry drift would have respawned
    // every child and the counter would read 2.
    let _ = drain_for(&session, Duration::from_millis(700));
    let runs = std::fs::read_to_string(&count)
        .map(|s| s.lines().count())
        .unwrap_or(0);
    assert_eq!(runs, 1, "a focus gesture must never restart children");

    session.write_bytes(b"q");
    assert!(
        !session.kill_if_alive(Duration::from_secs(2)),
        "dashboard should have exited on q"
    );
}

/// `j` with a pane focused moves THAT pane's window over its own retained
/// body — the neighbour keeps ticking underneath, because a pane's view
/// and its body are separate objects and a scrolled pane accepts output
/// with no interaction at all.
#[test]
fn a_focused_pane_scrolls_its_own_body_while_its_neighbour_ticks() {
    let dir = tempfile::tempdir().expect("tempdir");
    let count = dir.path().join("count");
    let decl = write_dashboard(
        dir.path(),
        &format!(
            r#"
gap 1
row-gap 0

defaults {{
    height 5
    border "none"
    chrome #false
    shell #true
}}

row {{
    pane "a" {{
        interval "1h"
        command "printf 'A%s\n' 01 02 03 04 05 06 07 08 09 10"
    }}
    pane "b" {{
        interval "250ms"
        command "{counter}"
    }}
}}
"#,
            counter = labeled_counter_cmd(&count, "b"),
        ),
    );

    let session = PtySession::spawn(&rat_bin(), &["dashboard", &decl.display().to_string()], &[])
        .expect("spawn rat dashboard under a pty");
    let mut terminal = FakeTerminal::dark();
    // Pane a is 5 rows over a 10-line body: at rest it shows A01-A05, so
    // A06 is a line no unfocused frame can produce.
    assert!(
        wait_for(&session, &mut terminal, b"A05", Duration::from_secs(5)),
        "the first composition never painted"
    );
    let ticks = std::fs::read_to_string(&count)
        .map(|s| s.lines().count())
        .unwrap_or(0);

    session.write_bytes(b"\t"); // focus pane a (first in reading order)
    session.write_bytes(b"j");
    let advanced = format!("b-{}", ticks + 2);
    // ONE capture: the scrolled row and the neighbour's later ticks can
    // batch into a single read, and a consumed paint is never repainted.
    wait_for_in_order(
        &session,
        &mut terminal,
        &[b"A06", advanced.as_bytes()],
        Duration::from_secs(5),
    );

    session.write_bytes(b"q");
    assert!(
        !session.kill_if_alive(Duration::from_secs(2)),
        "dashboard should have exited on q"
    );
}

/// D4 end to end: a batch pane's body is replaced wholesale on every run,
/// and the reader's place is held through it — positional, per the
/// research's honesty note, which is why the badge names the new total.
#[test]
fn a_scrolled_pane_holds_its_place_across_a_body_replacement() {
    let dir = tempfile::tempdir().expect("tempdir");
    let count = dir.path().join("count");
    let decl = write_dashboard(
        dir.path(),
        &format!(
            r#"
row-gap 0

defaults {{
    height 5
    border "none"
    chrome #false
    shell #true
}}

pane "a" {{
    interval "1s"
    command "echo run >> {p}; n=$(($(wc -l < {p}))); printf \"r$n-%s\n\" 01 02 03 04 05 06 07 08 09 10"
}}
"#,
            p = count.display(),
        ),
    );

    let session = PtySession::spawn(&rat_bin(), &["dashboard", &decl.display().to_string()], &[])
        .expect("spawn rat dashboard under a pty");
    let mut terminal = FakeTerminal::dark();
    assert!(
        wait_for(&session, &mut terminal, b"r1-05", Duration::from_secs(5)),
        "the first run never painted"
    );
    session.write_bytes(b"\t");
    session.write_bytes(b"j");
    // r1-06 proves the step landed (offset 1 over a 5-row window); r2-06
    // proves the offset SURVIVED the replacement — at rest run two would
    // show r2-01..r2-05 and r2-06 would be unreachable.
    wait_for_in_order(
        &session,
        &mut terminal,
        &[b"r1-06", b"r2-06"],
        Duration::from_secs(6),
    );

    session.write_bytes(b"q");
    assert!(
        !session.kill_if_alive(Duration::from_secs(2)),
        "dashboard should have exited on q"
    );
}

/// The negative respawn pin for the scroll retarget, the gutter toggle's
/// shape: a pane scroll reads geometry and never derives it, so
/// detect_resize compares equal and the 250ms gate stays unarmed.
#[test]
fn a_pane_scroll_never_restarts_a_child() {
    let dir = tempfile::tempdir().expect("tempdir");
    let count_a = dir.path().join("count_a");
    let count_b = dir.path().join("count_b");
    let decl = write_dashboard(
        dir.path(),
        &format!(
            r#"
gap 1
row-gap 0

defaults {{
    height 5
    border "none"
    chrome #false
    shell #true
    interval "10s"
}}

row {{
    pane "a" {{
        command "echo run >> {pa}; printf 'A%s\n' 01 02 03 04 05 06 07 08 09 10"
    }}
    pane "b" {{
        command "{counter}"
    }}
}}
"#,
            pa = count_a.display(),
            counter = labeled_counter_cmd(&count_b, "b"),
        ),
    );

    let session = PtySession::spawn(&rat_bin(), &["dashboard", &decl.display().to_string()], &[])
        .expect("spawn rat dashboard under a pty");
    let mut terminal = FakeTerminal::dark();
    assert!(
        wait_for(&session, &mut terminal, b"A05", Duration::from_secs(5)),
        "the first composition never painted"
    );

    session.write_bytes(b"\t");
    session.write_bytes(b"j");
    let _ = wait_for_in_order(&session, &mut terminal, &[b"A06"], Duration::from_secs(5));
    // Past the resize debounce: a geometry drift would have respawned
    // every child and both counters would read 2.
    let _ = drain_for(&session, Duration::from_millis(700));
    for (name, path) in [("a", &count_a), ("b", &count_b)] {
        let runs = std::fs::read_to_string(path)
            .map(|s| s.lines().count())
            .unwrap_or(0);
        assert_eq!(
            runs, 1,
            "pane {name}: a pane scroll must never restart children"
        );
    }

    session.write_bytes(b"q");
    assert!(
        !session.kill_if_alive(Duration::from_secs(2)),
        "dashboard should have exited on q"
    );
}

/// The position badge appears when the reader takes over the pane's
/// window and is gone the moment the window is back where the
/// declaration puts it — a dashboard nobody has touched says nothing.
#[test]
fn the_scroll_badge_appears_on_a_scrolled_pane_and_leaves_at_rest() {
    let dir = tempfile::tempdir().expect("tempdir");
    let decl = write_dashboard(
        dir.path(),
        r#"
row-gap 0

defaults {
    height 5
    border "none"
    chrome #true
    shell #true
}

pane "a" {
    interval "1h"
    command "printf 'L%s\\n' 1 2 3 4 5 6"
}
"#,
    );

    let session = PtySession::spawn(&rat_bin(), &["dashboard", &decl.display().to_string()], &[])
        .expect("spawn rat dashboard under a pty");
    let mut terminal = FakeTerminal::dark();
    // 5 rows minus the chrome row is a 4-line window over 6 lines.
    let first = wait_for_bytes(&session, &mut terminal, b"L4", Duration::from_secs(5))
        .expect("the first composition never painted");
    assert!(
        !contains(&first, b"lines "),
        "a pane at rest must carry no position badge"
    );

    session.write_bytes(b"\t");
    session.write_bytes(b"j");
    session.write_bytes(b"g");
    // ONE capture: the badge, then the repaint `g` produced (its first
    // body row is L1 again), then that frame's chrome row.
    let seen = wait_for_in_order(
        &session,
        &mut terminal,
        &[b"lines 2-5 of 6", b"L1", b"every 1h"],
        Duration::from_secs(5),
    );
    let at_rest = row_containing(&seen, b"every 1h").expect("the chrome row after g");
    assert!(
        !contains(at_rest, b"lines "),
        "back at rest, the badge must be gone: {:?}",
        String::from_utf8_lossy(at_rest)
    );

    session.write_bytes(b"q");
    assert!(
        !session.kill_if_alive(Duration::from_secs(2)),
        "dashboard should have exited on q"
    );
}

/// `v` with a pane focused pages that pane's WHOLE retained body — the
/// lines its window clips are exactly what the escape hatch is for.
#[test]
fn v_pages_the_focused_panes_whole_retained_body() {
    let dir = tempfile::tempdir().expect("tempdir");
    let decl = write_dashboard(
        dir.path(),
        r#"
row-gap 0

defaults {
    border "none"
    chrome #false
    shell #true
}

pane "a" {
    height 4
    interval "1h"
    command "printf 'L%s\\n' 01 02 03 04 05 06 07 08 09 10 11 12"
}

pane "b" {
    height 1
    interval "1h"
    command "printf 'BEE'"
}
"#,
    );

    let session = PtySession::spawn(
        &rat_bin(),
        &["dashboard", &decl.display().to_string()],
        &[("RAT_PAGER", "/bin/cat")],
    )
    .expect("spawn rat dashboard under a pty");
    let mut terminal = FakeTerminal::dark();
    // Pane a's window is 4 rows over a 12-line body: L05..L12 are
    // retained and have never been painted.
    let first = wait_for_bytes(&session, &mut terminal, b"L04", Duration::from_secs(5))
        .expect("the first composition never painted");
    assert!(!contains(&first, b"L12"), "L12 must not be on screen");

    session.write_bytes(b"\t"); // focus pane a
    session.write_bytes(b"v");
    assert!(
        wait_for(&session, &mut terminal, b"L12", Duration::from_secs(5)),
        "the pager never received the pane's retained tail"
    );

    session.write_bytes(b"q");
    assert!(
        !session.kill_if_alive(Duration::from_secs(2)),
        "dashboard should have exited on q"
    );
}

/// The explicit guard: `Page` is bound in every mode, and a frozen frame
/// is a literal copy of composed strings with no pane identity in it. A
/// pane may be focused when `p` lands, and `v` must still hand the pager
/// the frozen WHOLE frame.
#[test]
fn a_paused_frame_pages_the_frozen_frame_even_with_a_pane_focused() {
    let dir = tempfile::tempdir().expect("tempdir");
    let decl = write_dashboard(
        dir.path(),
        r#"
row-gap 0

defaults {
    border "none"
    chrome #false
    shell #true
}

pane "a" {
    height 4
    interval "1h"
    command "printf 'L%s\\n' 01 02 03 04 05 06 07 08 09 10 11 12"
}

pane "b" {
    height 1
    interval "1h"
    command "printf 'BEE'"
}
"#,
    );

    let session = PtySession::spawn(
        &rat_bin(),
        &["dashboard", &decl.display().to_string()],
        &[("RAT_PAGER", "/bin/cat")],
    )
    .expect("spawn rat dashboard under a pty");
    let mut terminal = FakeTerminal::dark();
    let mut seen = wait_for_bytes(&session, &mut terminal, b"BEE", Duration::from_secs(5))
        .expect("the first composition never painted");
    session.write_bytes(b"\t"); // focus pane a
    session.write_bytes(b"p"); // then freeze it
    seen.extend(
        wait_for_bytes(&session, &mut terminal, b"paused", Duration::from_secs(5))
            .expect("the frame never froze"),
    );
    session.write_bytes(b"v");
    // The pager clobbers the screen, so the return repaints the FULL
    // frame — pane b's row is rewritten, which no other event does for a
    // 1h pane. That second `BEE` is the evidence the round trip happened.
    seen.extend(
        wait_for_bytes(&session, &mut terminal, b"BEE", Duration::from_secs(5))
            .expect("the pager never returned"),
    );
    assert!(
        !contains(&seen, b"L12"),
        "a paused v must page the frozen frame, never a pane's body"
    );

    session.write_bytes(b"q");
    assert!(
        !session.kill_if_alive(Duration::from_secs(2)),
        "dashboard should have exited on q"
    );
}
