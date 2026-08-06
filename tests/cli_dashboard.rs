mod common;

use assert_cmd::Command;

fn rat() -> Command {
    let mut cmd = common::rat();
    cmd.env_remove("NO_COLOR");
    cmd
}

/// Path to the rat binary, used as a portable child process: shell
/// utilities like sh, echo, and printf do not exist everywhere.
fn rat_bin() -> String {
    assert_cmd::cargo::cargo_bin("rat").display().to_string()
}

/// Write a fixture and hand back its path. Commands are interpolated so
/// every pane runs the rat binary under test. The backslash escape
/// keeps a Windows binary path a valid KDL quoted string.
fn fixture(dir: &std::path::Path, name: &str, body: &str) -> String {
    let path = dir.join(name);
    std::fs::write(&path, body).expect("write fixture");
    path.display().to_string()
}

#[test]
fn a_dashboard_renders_its_panes_once() {
    let dir = tempfile::tempdir().expect("tempdir");
    let file = fixture(
        dir.path(),
        "board.kdl",
        &format!(
            r#"
defaults {{
    height 3
    chrome #false
}}

pane "left" {{
    command "{bin}" "style" "hello"
}}

pane "right" {{
    command "{bin}" "style" "world"
}}
"#,
            bin = rat_bin().replace('\\', "\\\\")
        ),
    );
    rat()
        .env("NO_COLOR", "1")
        .args(["dashboard", &file, "--once"])
        .assert()
        .success()
        .stdout(predicates::str::contains("hello"))
        .stdout(predicates::str::contains("world"));
}

#[test]
fn a_dashboard_expands_child_tabs_before_boxing() {
    let dir = tempfile::tempdir().expect("tempdir");
    let output = dir.path().join("tabbed");
    std::fs::write(&output, "completed\tsuccess\ttest").expect("write tabbed output");
    let file = fixture(
        dir.path(),
        "tabbed.kdl",
        &format!(
            r#"
defaults height=1 chrome=#false border="none" padding="0" width="32"

pane "ci" {{
    command "{bin}" "__cat" "{output}"
}}
"#,
            bin = rat_bin().replace('\\', "\\\\"),
            output = output.display().to_string().replace('\\', "\\\\"),
        ),
    );
    rat()
        .env("NO_COLOR", "1")
        .args(["dashboard", &file, "--once"])
        .assert()
        .success()
        .stdout("completed       success test    \n");
}

#[test]
fn a_pane_child_is_told_its_inner_geometry() {
    // A Cells pane with no border and no padding has an inner width
    // equal to its declared cells, whatever the terminal is — so the
    // assertion is exact and does not depend on the harness having a
    // tty.
    let dir = tempfile::tempdir().expect("tempdir");
    let bin = rat_bin().replace('\\', "\\\\");
    let file = fixture(
        dir.path(),
        "geom.kdl",
        &format!(
            r#"
defaults height=3 chrome=#false border="none" padding="0" width="20"

pane "cols" {{
    command "{bin}" "__env" "RAT_WIDTH"
}}

pane "rows" {{
    command "{bin}" "__env" "RAT_HEIGHT"
}}

pane "whoami" {{
    command "{bin}" "__env" "RAT_PANE"
}}
"#
        ),
    );
    let assert = rat()
        .env("NO_COLOR", "1")
        .args(["dashboard", &file, "--once"])
        .assert()
        .success();
    let stdout = String::from_utf8_lossy(&assert.get_output().stdout).into_owned();
    assert!(
        stdout.contains("20"),
        "RAT_WIDTH is the pane's cells: {stdout:?}"
    );
    assert!(
        stdout.contains('3'),
        "RAT_HEIGHT is the pane's inner rows: {stdout:?}"
    );
    assert!(
        stdout.contains("whoami"),
        "RAT_PANE is the pane's name: {stdout:?}"
    );
}

#[test]
fn a_pane_taller_than_its_box_is_truncated_keep_top() {
    // `rat style` joins multiple arguments with newlines, so this child
    // prints five lines into a three-row box.
    let dir = tempfile::tempdir().expect("tempdir");
    let bin = rat_bin().replace('\\', "\\\\");
    let file = fixture(
        dir.path(),
        "tall.kdl",
        &format!(
            r#"
pane "tall" {{
    height 3
    chrome #false
    border "none"
    command "{bin}" "style" "AAA" "BBB" "CCC" "DDD" "EEE"
}}
"#
        ),
    );
    let assert = rat()
        .env("NO_COLOR", "1")
        .args(["dashboard", &file, "--once"])
        .assert()
        .success();
    let stdout = String::from_utf8_lossy(&assert.get_output().stdout).into_owned();
    assert!(
        stdout.contains("AAA"),
        "keep-top keeps the head: {stdout:?}"
    );
    assert!(
        !stdout.contains("EEE"),
        "the pin truncated nothing: {stdout:?}"
    );
}

#[test]
fn a_pane_that_has_not_run_renders_blank_at_its_declared_size() {
    // The composed frame's row count is run-constant, so however the
    // two completions interleave, every frame written is exactly the
    // declared height. A pane that has not posted yet is blank rows,
    // never a shorter frame.
    let dir = tempfile::tempdir().expect("tempdir");
    let bin = rat_bin().replace('\\', "\\\\");
    let file = fixture(
        dir.path(),
        "stack.kdl",
        &format!(
            r#"
defaults {{
    height 3
    chrome #false
    border "none"
}}

pane "a" {{
    command "{bin}" "style" "one"
}}

pane "b" {{
    command "{bin}" "style" "two"
}}
"#
        ),
    );
    let assert = rat()
        .env("NO_COLOR", "1")
        .args(["dashboard", &file, "--once"])
        .assert()
        .success();
    let stdout = String::from_utf8_lossy(&assert.get_output().stdout).into_owned();
    let rows = stdout.lines().count();
    assert!(rows >= 6, "a whole frame is 6 rows, got {rows}: {stdout:?}");
    assert_eq!(
        rows % 6,
        0,
        "every frame is exactly 6 rows; got {rows}: {stdout:?}"
    );
}

#[test]
fn an_unreadable_file_names_the_path() {
    let missing = "definitely-no-such-dashboard-xyz.kdl";
    rat()
        .args(["dashboard", missing])
        .assert()
        .code(1)
        .stderr(predicates::str::contains(missing));
}

/// A file that is not KDL fails as a parse error carrying the path —
/// there is no format selection left to point at.
#[test]
fn a_file_that_is_not_kdl_names_the_path() {
    use predicates::boolean::PredicateBooleanExt;
    // The path AND the place: the syntax error carries its line and
    // column through load's context and main's prefix. Shape only —
    // the message text between them is upstream's.
    let dir = tempfile::tempdir().expect("tempdir");
    let file = fixture(dir.path(), "board.conf", "gap = 0\n");
    rat()
        .args(["dashboard", &file])
        .assert()
        .code(1)
        .stderr(predicates::str::contains("board.conf"))
        .stderr(predicates::str::contains("line "))
        .stderr(predicates::str::contains("column "))
        // The rustc-style snippet reaches the user: the offending
        // source line is echoed on stderr, pointed at by the span —
        // uncolored, because a piped run earns the plain profile.
        .stderr(predicates::str::contains("gap = 0"))
        .stderr(predicates::str::contains("\u{1b}").not());
}

/// The failure lives in the failing pane's own box — its text, its
/// exit badge — and the dashboard around it is untouched. The height
/// pin is what makes that structural: two 5-row panes compose to
/// exactly ten rows whether they succeed or fail.
#[test]
fn a_failing_pane_shows_its_exit_code_and_the_rest_of_the_dashboard_survives() {
    let dir = tempfile::tempdir().expect("tempdir");
    let steady = dir.path().join("steady");
    std::fs::write(&steady, "steady-content").expect("seed");
    let decl = dir.path().join("dash.kdl");
    std::fs::write(
        &decl,
        format!(
            r#"
row-gap 0

defaults {{
    height 5
    border "rounded"
}}

pane "broken" {{
    command "{rat}" "__exitcode" "3" "boom-from-stderr"
}}

pane "steady" {{
    command "{rat}" "__cat" "{steady}"
}}
"#,
            rat = rat_bin().escape_default(),
            steady = steady.display().to_string().escape_default(),
        ),
    )
    .expect("write declaration");

    let assert = rat()
        .env("NO_COLOR", "1")
        .args(["dashboard", "--once", &decl.display().to_string()])
        .assert()
        .success();
    let stdout = String::from_utf8_lossy(&assert.get_output().stdout).into_owned();

    // The failing pane's own box carries its stderr and its badge.
    assert!(stdout.contains("boom-from-stderr"), "{stdout:?}");
    assert!(stdout.contains(" · exit 3"), "{stdout:?}");
    // The neighbour rendered normally: a failure never truncates it.
    assert!(stdout.contains("steady-content"), "{stdout:?}");
    // Both declared heights intact — the whole point of the pin.
    assert_eq!(
        stdout.trim_end_matches('\n').split('\n').count(),
        10,
        "declared heights must survive a failure: {stdout:?}"
    );
    // Nothing writes outside the frame engine. The failing child's
    // stderr went into its pane, not to the terminal.
    assert_eq!(
        String::from_utf8_lossy(&assert.get_output().stderr),
        "",
        "a failing pane must not leak to the terminal"
    );
}

/// Stream the piped dashboard's stdout through a channel so waiting for
/// a frame is bounded: a blocking read cannot swallow the deadline.
/// Duplicated from the watch suite's local helpers, never lifted — that
/// file is the byte-identity witness.
fn stdout_stream(stdout: std::process::ChildStdout) -> std::sync::mpsc::Receiver<Vec<u8>> {
    use std::io::Read;
    let (tx, rx) = std::sync::mpsc::channel();
    std::thread::spawn(move || {
        let mut stdout = stdout;
        let mut buf = [0u8; 4096];
        while let Ok(n) = stdout.read(&mut buf) {
            if n == 0 || tx.send(buf[..n].to_vec()).is_err() {
                return;
            }
        }
    });
    rx
}

/// `stdout_stream`'s stderr sibling: the once notice is a STDERR
/// surface, and the frame must stay off it.
fn stderr_stream(stderr: std::process::ChildStderr) -> std::sync::mpsc::Receiver<Vec<u8>> {
    use std::io::Read;
    let (tx, rx) = std::sync::mpsc::channel();
    std::thread::spawn(move || {
        let mut stderr = stderr;
        let mut buf = [0u8; 4096];
        while let Ok(n) = stderr.read(&mut buf) {
            if n == 0 || tx.send(buf[..n].to_vec()).is_err() {
                return;
            }
        }
    });
    rx
}

/// Drain the stream until the needle appears — a missing frame is a
/// clean failure, never a hang.
fn read_until(stream: &std::sync::mpsc::Receiver<Vec<u8>>, seen: &mut String, needle: &str) {
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(10);
    loop {
        if seen.contains(needle) {
            return;
        }
        let left = deadline.saturating_duration_since(std::time::Instant::now());
        assert!(!left.is_zero(), "never saw {needle:?} in {seen:?}");
        match stream.recv_timeout(left) {
            Ok(chunk) => seen.push_str(&String::from_utf8_lossy(&chunk)),
            Err(_) => panic!("never saw {needle:?} in {seen:?}"),
        }
    }
}

/// Reap the dashboard even when an assertion panics: an orphaned child
/// holds the harness's stdout pipe open and hangs the whole run.
struct KillOnDrop(std::process::Child);

impl Drop for KillOnDrop {
    fn drop(&mut self) {
        let _ = self.0.kill();
        let _ = self.0.wait();
    }
}

/// Per-pane triggers: a fire routes to the pane that DECLARED it. The
/// declarer is deliberately the SECOND source: a wiring that routes
/// every fire to source 0 (a shared gate, a hardcoded index) re-runs
/// alpha instead — whose bytes are unchanged, so the gated pipe writes
/// nothing and v1 never appears.
#[test]
fn a_file_trigger_refreshes_only_its_own_pane() {
    let dir = tempfile::tempdir().expect("tempdir");
    let steady = dir.path().join("steady");
    let shared = dir.path().join("shared");
    let untouched = dir.path().join("untouched");
    std::fs::write(&steady, "a0").expect("seed");
    std::fs::write(&shared, "v0").expect("seed");
    std::fs::write(&untouched, "x").expect("seed");
    let decl = dir.path().join("dash.kdl");
    std::fs::write(
        &decl,
        format!(
            r#"
row-gap 0

defaults {{
    height 1
    border "none"
    chrome #false
    interval "never"
    trigger-debounce "0ms"
}}

pane "alpha" {{
    command "{rat}" "__cat" "{steady}"
    trigger "file:{untouched}"
}}

pane "beta" {{
    command "{rat}" "__cat" "{shared}"
    trigger "file:{shared}"
}}
"#,
            rat = rat_bin().escape_default(),
            steady = steady.display().to_string().escape_default(),
            shared = shared.display().to_string().escape_default(),
            untouched = untouched.display().to_string().escape_default(),
        ),
    )
    .expect("write declaration");

    let dash = std::process::Command::new(rat_bin())
        .args(["dashboard", &decl.display().to_string()])
        .stdout(std::process::Stdio::piped())
        .spawn()
        .expect("spawn rat dashboard piped");
    let mut dash = KillOnDrop(dash);
    let stream = stdout_stream(dash.0.stdout.take().expect("piped stdout"));
    let mut seen = String::new();
    read_until(&stream, &mut seen, "v0"); // both panes' first tick

    std::fs::write(&shared, "v1").expect("mtime change");
    read_until(&stream, &mut seen, "v1"); // beta's trigger-driven frame

    // The panes stack in declaration order: in the refreshed frame,
    // alpha's retained row still precedes beta's new one.
    let last_frame = seen.rfind("a0").expect("alpha's retained row");
    assert!(
        seen[last_frame..].contains("v1"),
        "the refreshed frame keeps declaration order: {seen:?}"
    );
    // KillOnDrop reaps: kill only SENDS the signal, and an unreaped
    // child zombies (unix) and races tempdir cleanup.
}

/// Every change fires, not just the first. The loop now stats the watched
/// union on its own account, to learn whether a path ever moves while the
/// dashboard is idle — and that observer keeps its OWN baselines. If it ever
/// shared them with the trigger, its stat would consume the fire and the pane
/// would quietly stop refreshing, which is the failure this guards.
#[test]
fn successive_trigger_changes_each_refresh_the_pane() {
    let dir = tempfile::tempdir().expect("tempdir");
    let watched = dir.path().join("watched");
    std::fs::write(&watched, "v0").expect("seed");
    let decl = dir.path().join("dash.kdl");
    std::fs::write(
        &decl,
        format!(
            r#"
row-gap 0

defaults {{
    height 1
    border "none"
    chrome #false
    interval "never"
    trigger-debounce "0ms"
}}

pane "only" {{
    command "{rat}" "__cat" "{watched}"
    trigger "file:{watched}"
}}
"#,
            rat = rat_bin().escape_default(),
            watched = watched.display().to_string().escape_default(),
        ),
    )
    .expect("write declaration");

    let dash = std::process::Command::new(rat_bin())
        .args(["dashboard", &decl.display().to_string()])
        .stdout(std::process::Stdio::piped())
        .spawn()
        .expect("spawn rat dashboard piped");
    let mut dash = KillOnDrop(dash);
    let stream = stdout_stream(dash.0.stdout.take().expect("piped stdout"));
    let mut seen = String::new();
    read_until(&stream, &mut seen, "v0");

    // Three in a row: a shared baseline would swallow one of them.
    for value in ["v1", "v2", "v3"] {
        std::fs::write(&watched, value).expect("mtime change");
        read_until(&stream, &mut seen, value);
    }
}

/// `--once` prints ONE complete frame: a staggered pane must not make
/// the partial composition reach the pipe first.
#[test]
fn once_emits_exactly_one_complete_frame() {
    let dir = tempfile::tempdir().expect("tempdir");
    let bin = rat_bin().replace('\\', "\\\\");
    let file = fixture(
        dir.path(),
        "staggered.kdl",
        &format!(
            r#"
row-gap 0

defaults {{
    height 1
    chrome #false
    border "none"
}}

pane "quick" {{
    command "{bin}" "style" "one"
}}

pane "slow" {{
    command "{bin}" "__sleep" "300" "two"
}}
"#
        ),
    );
    let assert = rat()
        .env("NO_COLOR", "1")
        .args(["dashboard", &file, "--once"])
        .assert()
        .success();
    let stdout = String::from_utf8_lossy(&assert.get_output().stdout).into_owned();
    assert_eq!(
        stdout.trim_end_matches('\n').split('\n').count(),
        2,
        "one frame, both panes: {stdout:?}"
    );
    assert_eq!(
        stdout.matches("one").count(),
        1,
        "the quick pane printed once: {stdout:?}"
    );
    assert!(stdout.contains("two"), "the slow pane arrived: {stdout:?}");
}

/// The one deliberately slow test in this file: the notice fires only
/// after the quiet threshold (5s), and the wait IS the surface under
/// test. A test-only env knob to shrink the threshold was rejected —
/// it would ship an undocumented surface.
#[test]
fn once_says_which_pane_it_is_still_waiting_on() {
    let dir = tempfile::tempdir().expect("tempdir");
    let log = dir.path().join("empty.log");
    std::fs::write(&log, "").expect("seed");
    let decl = dir.path().join("dash.kdl");
    std::fs::write(
        &decl,
        format!(
            r#"
defaults {{
    height 3
    border "none"
    chrome #false
}}

pane "build" {{
    command "{rat}" "__lines" "1"
}}

pane "logs" {{
    command "{rat}" "__follow" "{log}"
}}
"#,
            rat = rat_bin().escape_default(),
            log = log.display().to_string().escape_default(),
        ),
    )
    .expect("write declaration");

    let dash = std::process::Command::new(rat_bin())
        .args(["dashboard", &decl.display().to_string(), "--once"])
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::piped())
        .spawn()
        .expect("spawn rat dashboard piped");
    let mut dash = KillOnDrop(dash);
    let errs = stderr_stream(dash.0.stderr.take().expect("piped stderr"));
    let outs = stdout_stream(dash.0.stdout.take().expect("piped stdout"));
    let mut seen = String::new();
    read_until(&errs, &mut seen, "\"logs\"");
    assert!(
        seen.contains("still waiting"),
        "the notice says so: {seen:?}"
    );
    assert!(
        !seen.contains("\"build\""),
        "the finished pane is never named: {seen:?}"
    );
    // No stdout byte moved: a partial wave never reaches the pipe, and
    // the diagnostic must not change that.
    assert!(
        outs.try_recv().is_err(),
        "stdout stays empty while --once waits"
    );
    // One line, once: keep draining and the notice must not repeat —
    // the loop spins in 50ms slices, so a broken one-shot would say it
    // again a dozen times over this window.
    std::thread::sleep(std::time::Duration::from_millis(600));
    while let Ok(chunk) = errs.try_recv() {
        seen.push_str(&String::from_utf8_lossy(&chunk));
    }
    assert_eq!(
        seen.matches("still waiting").count(),
        1,
        "the notice is one-shot: {seen:?}"
    );
}

#[test]
fn once_timeout_exits_124_and_writes_no_frame() {
    // An undeclared follower under a short bound: exit 124, stderr
    // names the pane, and stdout is EMPTY — a partial frame is a lie
    // in the mode whose stdout is the frame.
    let dir = tempfile::tempdir().expect("tempdir");
    let log = dir.path().join("empty.log");
    std::fs::write(&log, "").expect("seed");
    let file = fixture(
        dir.path(),
        "bounded.kdl",
        &format!(
            r#"
pane "logs" {{
    height 3
    chrome #false
    border "none"
    command "{bin}" "__follow" "{log}"
}}
"#,
            bin = rat_bin().replace('\\', "\\\\"),
            log = log.display().to_string().replace('\\', "\\\\"),
        ),
    );
    let assert = rat()
        .env("NO_COLOR", "1")
        .args(["dashboard", &file, "--once", "--once-timeout", "300ms"])
        .assert()
        .code(124)
        .stderr(predicates::str::contains("\"logs\""))
        .stderr(predicates::str::contains("gave up"));
    assert!(
        assert.get_output().stdout.is_empty(),
        "stdout must be empty on expiry"
    );
}

#[test]
fn once_timeout_needs_once() {
    use predicates::boolean::PredicateBooleanExt;
    rat()
        .args(["dashboard", "whatever.kdl", "--once-timeout", "1s"])
        .assert()
        .code(2)
        .stderr(predicates::str::contains("--once"))
        .stderr(predicates::str::contains("unexpected argument").not());
}

#[test]
fn once_timeout_does_not_fire_when_every_pane_finishes() {
    // The no-regression pin: a slow-but-finishing pane under a
    // generous bound exits 0 with its one frame, exactly as if the
    // flag were absent.
    let dir = tempfile::tempdir().expect("tempdir");
    let file = fixture(
        dir.path(),
        "finishes.kdl",
        &format!(
            r#"
pane "slow" {{
    height 3
    chrome #false
    border "none"
    command "{bin}" "__sleep" "200" "done-now"
}}
"#,
            bin = rat_bin().replace('\\', "\\\\"),
        ),
    );
    rat()
        .env("NO_COLOR", "1")
        .args(["dashboard", &file, "--once", "--once-timeout", "30s"])
        .assert()
        .success()
        .stdout(predicates::str::contains("done-now"));
}

#[test]
fn a_dashboard_title_heads_the_once_frame() {
    // The declared title is the frame's first line, piped and --once
    // alike — the same contract watch --title has always had.
    let dir = tempfile::tempdir().expect("tempdir");
    let file = fixture(
        dir.path(),
        "titled.kdl",
        &format!(
            r#"
title "Deploy status"

pane "build" {{
    height 3
    chrome #false
    border "none"
    command "{bin}" "style" "one"
}}
"#,
            bin = rat_bin().replace('\\', "\\\\")
        ),
    );
    let assert = rat()
        .env("NO_COLOR", "1")
        .args(["dashboard", &file, "--once"])
        .assert()
        .success();
    let stdout = String::from_utf8_lossy(&assert.get_output().stdout).into_owned();
    let first = stdout.lines().next().expect("a frame");
    assert!(
        first.contains("Deploy status"),
        "the title heads the frame: {stdout:?}"
    );
    assert!(stdout.contains("one"), "the pane still renders: {stdout:?}");
}

#[test]
fn a_ref_sourced_title_renders_the_pane_not_the_fallback() {
    // Role donation end to end: the referenced pane is the visible
    // title, and the fallback text never reaches the frame — it is
    // the ROLE's fallback, not a rendered line.
    let dir = tempfile::tempdir().expect("tempdir");
    let file = fixture(
        dir.path(),
        "reffed.kdl",
        &format!(
            r##"
title "Fallback" ref="#header"

pane "header" {{
    height 1
    chrome #false
    border "none"
    command "{bin}" "style" "CUSTOM-HEADER"
}}

pane "body" {{
    height 3
    chrome #false
    border "none"
    command "{bin}" "style" "body-line"
}}
"##,
            bin = rat_bin().replace('\\', "\\\\")
        ),
    );
    let assert = rat()
        .env("NO_COLOR", "1")
        .args(["dashboard", &file, "--once"])
        .assert()
        .success();
    let stdout = String::from_utf8_lossy(&assert.get_output().stdout).into_owned();
    let first = stdout.lines().next().expect("a frame");
    assert!(
        first.contains("CUSTOM-HEADER"),
        "the pane heads the frame: {stdout:?}"
    );
    assert!(
        !stdout.contains("Fallback"),
        "the fallback is role text, never a rendered line: {stdout:?}"
    );
}

#[test]
fn a_piped_once_dashboard_never_touches_the_tab_title() {
    use predicates::boolean::PredicateBooleanExt;
    // The tab title is interactive chrome: a pipe gets frame bytes
    // and nothing else — no stack push, no OSC.
    let dir = tempfile::tempdir().expect("tempdir");
    let file = fixture(
        dir.path(),
        "titled.kdl",
        &format!(
            "title \"Deploy\"\n\npane \"a\" {{\n    height 3\n    chrome #false\n    border \"none\"\n    command \"{bin}\" \"style\" \"one\"\n}}\n",
            bin = rat_bin().replace('\\', "\\\\")
        ),
    );
    rat()
        .env("NO_COLOR", "1")
        .args(["dashboard", &file, "--once"])
        .assert()
        .success()
        .stdout(predicates::str::contains("\u{1b}]2;").not())
        .stdout(predicates::str::contains("\u{1b}[22;2t").not());
}

#[test]
fn a_pane_that_cannot_start_shows_the_reason_before_the_path() {
    // A declared width no terminal can widen truncates from the
    // right, so whatever sits last is what no reader sees. The
    // missing command is a LONG absolute path: the OS reason must
    // survive the budget, and the path's tail is what pays. The
    // reason is matched by `os error`, never an OS-specific sentence.
    let dir = tempfile::tempdir().expect("tempdir");
    let missing = format!("{}-definitely-missing", rat_bin());
    let file = fixture(
        dir.path(),
        "board.kdl",
        &format!(
            r#"
pane "plan" {{
    command "{cmd}"
    width "72"
    height 3
    chrome #false
    border "none"
    padding "0"
}}
"#,
            cmd = missing.replace('\\', "\\\\")
        ),
    );
    let assert = rat()
        .env("NO_COLOR", "1")
        .env("RAT_WIDTH", "100")
        .args(["dashboard", &file, "--once"])
        .assert()
        .success();
    let stdout = String::from_utf8_lossy(&assert.get_output().stdout).into_owned();
    assert!(
        stdout.contains("os error"),
        "the reason survives the width: {stdout:?}"
    );
    assert!(
        !stdout.contains("definitely-missing"),
        "the path's tail is what overflows: {stdout:?}"
    );
}

/// Piped mode honors the handed-down geometry: a nested one-shot
/// dashboard sizes itself to its pane instead of a hardcoded 80
/// columns.
#[test]
fn a_piped_dashboard_sizes_from_rat_width() {
    let dir = tempfile::tempdir().expect("tempdir");
    let bin = rat_bin().replace('\\', "\\\\");
    let file = fixture(
        dir.path(),
        "sized.kdl",
        &format!(
            r#"
pane "wide" {{
    height 2
    chrome #false
    border "none"
    command "{bin}" "style" "x"
}}
"#
        ),
    );
    let assert = rat()
        .env("NO_COLOR", "1")
        .env("RAT_WIDTH", "40")
        .env("RAT_HEIGHT", "20")
        .args(["dashboard", &file, "--once"])
        .assert()
        .success();
    let stdout = String::from_utf8_lossy(&assert.get_output().stdout).into_owned();
    for line in stdout.trim_end_matches('\n').split('\n') {
        assert_eq!(line.chars().count(), 40, "a 40-cell frame: {line:?}");
    }
}

/// Nested layout nodes: a row holding a column beside a pane renders
/// as a grid — the engine's tree was recursive from day one, and the
/// declaration now reaches it.
#[test]
fn a_nested_layout_renders_a_grid() {
    let dir = tempfile::tempdir().expect("tempdir");
    let bin = rat_bin().replace('\\', "\\\\");
    let file = fixture(
        dir.path(),
        "grid.kdl",
        &format!(
            r#"
defaults height=1 chrome=#false border="none"

row {{
    column {{
        pane "a" {{
            command "{bin}" "style" "one"
        }}
        pane "b" {{
            command "{bin}" "style" "two"
        }}
    }}
    pane "c" height=2 {{
        command "{bin}" "style" "three"
    }}
}}
"#
        ),
    );
    let assert = rat()
        .env("NO_COLOR", "1")
        .env("RAT_WIDTH", "40")
        .args(["dashboard", &file, "--once"])
        .assert()
        .success();
    let stdout = String::from_utf8_lossy(&assert.get_output().stdout).into_owned();
    let rows: Vec<&str> = stdout.trim_end_matches('\n').split('\n').collect();
    assert_eq!(rows.len(), 2, "a 2-row grid: {stdout:?}");
    assert!(
        rows[0].contains("one") && rows[0].contains("three"),
        "top of the column beside the tall pane: {stdout:?}"
    );
    assert!(
        rows[1].contains("two"),
        "bottom of the column on the second grid row: {stdout:?}"
    );
}

#[test]
fn a_flooding_pane_wears_the_marker_on_its_own_chrome_row() {
    // The pane route's own end-to-end proof. The unit tests show the
    // badge renders and joins the signature; only this shows the count
    // reaching it from a real child, through the reader, the outcome
    // and the drain. A shared decision covered on one route only is
    // covered on neither.
    let dir = tempfile::tempdir().expect("tempdir");
    let bin = rat_bin().replace('\\', "\\\\");
    let file = fixture(
        dir.path(),
        "flood.kdl",
        &format!(
            r#"
defaults height=6 width="60" border="none" padding="0"

pane "flood" {{
    command "{bin}" "__lines" "1500"
}}

pane "quiet" {{
    command "{bin}" "style" "calm"
}}
"#
        ),
    );
    let assert = rat()
        .env("NO_COLOR", "1")
        .args(["dashboard", &file, "--once"])
        .assert()
        .success();
    let stdout = String::from_utf8_lossy(&assert.get_output().stdout).into_owned();

    assert!(
        stdout.contains("500 lines dropped"),
        "the flooding pane must say so; got {stdout:?}"
    );
    // The default overflow keeps the head, so the pane retained lines
    // 0..999 and paints the first of them. The last line the child
    // printed is gone — the direction working, not a fault.
    assert!(
        stdout.starts_with('0'),
        "a keep-top pane shows its head: {stdout:?}"
    );
    assert!(
        !stdout.contains("1499"),
        "and its tail is what went: {stdout:?}"
    );
    // One pane overflowed, not both: the marker is per-pane state and
    // the quiet pane's chrome row must stay clean.
    assert_eq!(
        stdout.matches("lines dropped").count(),
        1,
        "only the flooding pane wears it: {stdout:?}"
    );
}

/// The piped route for a live pane under `--once`: a source whose child
/// never exits still produces exactly one frame and a clean exit,
/// because its first emission is what satisfies the once condition.
///
/// Its own test rather than a corollary of the pty one: a piped run and
/// a terminal run take different branches through the loop, and the
/// shared half is the half already covered.
#[test]
#[cfg(unix)]
fn once_with_a_live_pane_emits_one_frame_and_exits() {
    let dir = tempfile::tempdir().expect("tempdir");
    let log = dir.path().join("log");
    std::fs::write(&log, "piped-live-content\n").expect("seed the log");
    let file = fixture(
        dir.path(),
        "live.kdl",
        &format!(
            r#"
defaults {{
    height 3
    chrome #false
}}

pane "follower" live=#true {{
    command "{bin}" "__follow" "{log}"
}}
"#,
            bin = rat_bin().replace('\\', "\\\\"),
            log = log.display()
        ),
    );
    let out = rat()
        .args(["dashboard", "--once", &file])
        .env("RAT_WIDTH", "40")
        .env("RAT_HEIGHT", "10")
        .assert()
        .success();
    let stdout = String::from_utf8_lossy(&out.get_output().stdout).into_owned();
    assert!(
        stdout.contains("piped-live-content"),
        "the live pane's body never reached the frame: {stdout:?}"
    );
    assert_eq!(
        stdout.matches("piped-live-content").count(),
        1,
        "--once must emit exactly one frame: {stdout:?}"
    );
}

/// The no-shebang fallback, through a NAMED shell — and the tmpdir
/// stays untouched: a body with no `#!` is never materialized.
#[cfg(unix)]
#[test]
fn a_script_body_with_no_shebang_falls_back_to_the_shell() {
    let dir = tempfile::tempdir().expect("tempdir");
    let tmp = tempfile::tempdir().expect("isolated TMPDIR");
    let file = fixture(
        dir.path(),
        "board.kdl",
        "defaults height=3 chrome=#false\npane \"fall\" {\n    shell \"sh\"\n    script \"echo fell-back\"\n}\n",
    );
    rat()
        .env("NO_COLOR", "1")
        .env("TMPDIR", tmp.path())
        .args(["dashboard", &file, "--once"])
        .assert()
        .success()
        .stdout(predicates::str::contains("fell-back"));
    let leftovers: Vec<_> = std::fs::read_dir(tmp.path())
        .expect("read the isolated tmpdir")
        .filter_map(|entry| entry.ok())
        .filter(|entry| {
            entry
                .file_name()
                .to_string_lossy()
                .starts_with("rat-script.")
        })
        .collect();
    assert!(leftovers.is_empty(), "{leftovers:?}");
}

/// The no-shebang fallback with NO shell key anywhere: absence promotes
/// to the platform's shell — the same thing `shell #true` means — so a
/// plain body just works on every platform. The body invokes the rat
/// binary, this file's portable-child convention.
#[test]
fn a_script_body_with_no_shell_key_falls_back_to_the_platform_shell() {
    let dir = tempfile::tempdir().expect("tempdir");
    // cmd wants backslash separators for a program path. The body is
    // unquoted — the quoted spelling has its own windows arm
    // (`a_quoted_program_in_a_script_body_runs`).
    let bin = if cfg!(windows) {
        rat_bin().replace('/', "\\")
    } else {
        rat_bin()
    };
    let file = fixture(
        dir.path(),
        "board.kdl",
        &format!(
            "defaults height=3 chrome=#false\npane \"fall\" {{\n    script \"{bin} style promoted-shell\"\n}}\n",
            bin = bin.replace('\\', "\\\\")
        ),
    );
    rat()
        .env("NO_COLOR", "1")
        .args(["dashboard", &file, "--once"])
        .assert()
        .success()
        .stdout(predicates::str::contains("promoted-shell"));
}

/// A script body that QUOTES its program path, routed through the
/// promoted platform shell — the spelling an author with a spaced path
/// must write. The body has to reach `cmd /C` byte-for-byte: cmd does
/// not parse the `\"` escapes MSVCRT-style argument quoting writes.
#[cfg(windows)]
#[test]
fn a_quoted_program_in_a_script_body_runs() {
    let dir = tempfile::tempdir().expect("tempdir");
    let body = format!(
        "\"{}\" style quoted-script-ran",
        rat_bin().replace('/', "\\")
    );
    let file = fixture(
        dir.path(),
        "board.kdl",
        &format!(
            "defaults height=3 chrome=#false\npane \"quoted\" {{\n    script \"{body}\"\n}}\n",
            body = body.replace('\\', "\\\\").replace('"', "\\\"")
        ),
    );
    rat()
        .env("NO_COLOR", "1")
        .args(["dashboard", &file, "--once"])
        .assert()
        .success()
        .stdout(predicates::str::contains("quoted-script-ran"));
}

/// The three winvm-measured mechanisms at once: any one missing fails
/// this — an unstripped `#!` (cmd chokes on the line), a missing `.cmd`
/// extension ("is not recognized"), or a missing `cmd /C` (the file is
/// not a PE).
#[cfg(windows)]
#[test]
fn a_cmd_body_runs_as_a_batch_file() {
    let dir = tempfile::tempdir().expect("tempdir");
    let file = fixture(
        dir.path(),
        "board.kdl",
        "defaults height=4 chrome=#false\npane \"batch\" {\n    script \"#!cmd\\n@echo off\\nset /a x=6*7\\necho answer-%x%\"\n}\n",
    );
    rat()
        .env("NO_COLOR", "1")
        .args(["dashboard", &file, "--once"])
        .assert()
        .success()
        .stdout(predicates::str::contains("answer-42"));
}

/// `.ps1` (pwsh refuses anything else), `env` substitution, and
/// `-NoProfile` WITHOUT `-Command` (with `-Command` the path would be
/// evaluated as an expression, not run as a file) — measured working
/// form: `pwsh -NoProfile <file.ps1>`.
#[cfg(windows)]
#[test]
fn a_pwsh_body_runs_as_a_ps1_file() {
    let dir = tempfile::tempdir().expect("tempdir");
    let file = fixture(
        dir.path(),
        "board.kdl",
        "defaults height=4 chrome=#false\npane \"ps\" {\n    script \"#!/usr/bin/env pwsh\\nWrite-Output (6 * 7)\"\n}\n",
    );
    rat()
        .env("NO_COLOR", "1")
        .args(["dashboard", &file, "--once"])
        .assert()
        .success()
        .stdout(predicates::str::contains("42"));
}

/// The standard-spawn-does-the-search contract: `env powershell`
/// resolves the name on PATH (Rust Command's walk with `.exe`
/// inference; no PATHEXT), and Windows PowerShell answers.
#[cfg(windows)]
#[test]
fn an_env_shebang_finds_its_interpreter_on_the_path() {
    let dir = tempfile::tempdir().expect("tempdir");
    let file = fixture(
        dir.path(),
        "board.kdl",
        "defaults height=4 chrome=#false\npane \"onpath\" {\n    script \"#!/usr/bin/env powershell\\nWrite-Output found-on-path\"\n}\n",
    );
    rat()
        .env("NO_COLOR", "1")
        .args(["dashboard", &file, "--once"])
        .assert()
        .success()
        .stdout(predicates::str::contains("found-on-path"));
}
