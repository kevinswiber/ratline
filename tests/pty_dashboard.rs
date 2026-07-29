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
/// format-specific text in this file — the format pick's deletion
/// commit rewrites this one function (and the body builders below) if
/// the surviving format is not the one it emits.
fn write_dashboard(dir: &std::path::Path, body: &str) -> std::path::PathBuf {
    let path = dir.join("dash.toml");
    std::fs::write(&path, body).expect("write the dashboard declaration");
    path
}

/// Declaration body for two 1fr panes in one row, both parked at 1h.
/// `labeled_counter_cmd` emits only single-quoted sh, which a TOML
/// basic string carries verbatim (only `"` and `\` would need escapes;
/// tempdir paths contain neither).
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
        "gap = 1\n\n\
         [defaults]\nheight = 5\nborder = \"rounded\"\nwidth = \"1fr\"\nshell = true\n\n\
         [[pane]]\nname = \"{a_name}\"\ninterval = \"{a_interval}\"\ncommand = \"{a_cmd}\"\n\n\
         [[pane]]\nname = \"{b_name}\"\ninterval = \"{b_interval}\"\ncommand = \"{b_cmd}\"\n\n\
         [layout]\nrows = [[\"{a_name}\", \"{b_name}\"]]\n"
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
