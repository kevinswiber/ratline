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
/// keeps a Windows binary path a valid TOML basic string.
fn fixture(dir: &std::path::Path, name: &str, body: &str) -> String {
    let path = dir.join(name);
    std::fs::write(&path, body).expect("write fixture");
    path.display().to_string()
}

#[test]
fn a_toml_dashboard_renders_its_panes_once() {
    let dir = tempfile::tempdir().expect("tempdir");
    let file = fixture(
        dir.path(),
        "board.toml",
        &format!(
            r#"
[defaults]
height = 3
chrome = false

[[pane]]
name = "left"
command = ["{bin}", "style", "hello"]

[[pane]]
name = "right"
command = ["{bin}", "style", "world"]
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
fn a_kdl_dashboard_renders_the_same_frame_as_its_toml_twin() {
    // Byte equality is only meaningful on a stamp-free frame: the pane
    // chrome row carries an absolute HH:MM:SS, which two runs a second
    // apart will disagree about. Both fixtures set chrome = false, so
    // what is compared is geometry, order, and content — exactly what
    // the two grammars are supposed to agree on.
    let dir = tempfile::tempdir().expect("tempdir");
    let bin = rat_bin().replace('\\', "\\\\");
    let toml = fixture(
        dir.path(),
        "board.toml",
        &format!(
            r#"
gap = 1

[defaults]
height = 4
chrome = false
border = "rounded"

[[pane]]
name = "left"
command = ["{bin}", "style", "hello"]

[[pane]]
name = "right"
command = ["{bin}", "style", "world"]

[layout]
rows = [ ["left", "right"] ]
"#
        ),
    );
    let kdl = fixture(
        dir.path(),
        "board.kdl",
        &format!(
            r#"
gap 1

defaults {{
    height 4
    chrome #false
    border "rounded"
}}

pane "left" {{
    command "{bin}" "style" "hello"
}}

pane "right" {{
    command "{bin}" "style" "world"
}}

layout {{
    row "left" "right"
}}
"#
        ),
    );

    let from_toml = rat()
        .env("NO_COLOR", "1")
        .args(["dashboard", &toml, "--once"])
        .assert()
        .success();
    let from_kdl = rat()
        .env("NO_COLOR", "1")
        .args(["dashboard", &kdl, "--once"])
        .assert()
        .success();
    assert_eq!(
        String::from_utf8_lossy(&from_toml.get_output().stdout),
        String::from_utf8_lossy(&from_kdl.get_output().stdout)
    );
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
        "geom.toml",
        &format!(
            r#"
[defaults]
height = 3
chrome = false
border = "none"
padding = "0"
width = "20"

[[pane]]
name = "cols"
command = ["{bin}", "__env", "RAT_WIDTH"]

[[pane]]
name = "rows"
command = ["{bin}", "__env", "RAT_HEIGHT"]

[[pane]]
name = "whoami"
command = ["{bin}", "__env", "RAT_PANE"]
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
        "tall.toml",
        &format!(
            r#"
[[pane]]
name = "tall"
height = 3
chrome = false
border = "none"
command = ["{bin}", "style", "AAA", "BBB", "CCC", "DDD", "EEE"]
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
        "stack.toml",
        &format!(
            r#"
[defaults]
height = 3
chrome = false
border = "none"

[[pane]]
name = "a"
command = ["{bin}", "style", "one"]

[[pane]]
name = "b"
command = ["{bin}", "style", "two"]
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
    let missing = "definitely-no-such-dashboard-xyz.toml";
    rat()
        .args(["dashboard", missing])
        .assert()
        .code(1)
        .stderr(predicates::str::contains(missing));
}

#[test]
fn an_unknown_extension_names_the_format_flag() {
    let dir = tempfile::tempdir().expect("tempdir");
    let file = fixture(dir.path(), "board.conf", "gap = 0\n");
    rat()
        .args(["dashboard", &file])
        .assert()
        .code(1)
        .stderr(predicates::str::contains("--format"));
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
    let decl = dir.path().join("dash.toml");
    std::fs::write(
        &decl,
        format!(
            r#"
row-gap = 0

[defaults]
height = 5
border = "rounded"

[[pane]]
name = "broken"
command = ["{rat}", "__exitcode", "3", "boom-from-stderr"]

[[pane]]
name = "steady"
command = ["{rat}", "__cat", "{steady}"]
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
    let decl = dir.path().join("dash.toml");
    std::fs::write(
        &decl,
        format!(
            r#"
row-gap = 0

[defaults]
height = 1
border = "none"
chrome = false
interval = "never"
trigger-debounce = "0ms"

[[pane]]
name = "alpha"
command = ["{rat}", "__cat", "{steady}"]
trigger = ["file:{untouched}"]

[[pane]]
name = "beta"
command = ["{rat}", "__cat", "{shared}"]
trigger = ["file:{shared}"]
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

/// `--once` prints ONE complete frame: a staggered pane must not make
/// the partial composition reach the pipe first.
#[test]
fn once_emits_exactly_one_complete_frame() {
    let dir = tempfile::tempdir().expect("tempdir");
    let bin = rat_bin().replace('\\', "\\\\");
    let file = fixture(
        dir.path(),
        "staggered.toml",
        &format!(
            r#"
row-gap = 0

[defaults]
height = 1
chrome = false
border = "none"

[[pane]]
name = "quick"
command = ["{bin}", "style", "one"]

[[pane]]
name = "slow"
command = ["{bin}", "__sleep", "300", "two"]
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

/// Piped mode honors the handed-down geometry: a nested one-shot
/// dashboard sizes itself to its pane instead of a hardcoded 80
/// columns.
#[test]
fn a_piped_dashboard_sizes_from_rat_width() {
    let dir = tempfile::tempdir().expect("tempdir");
    let bin = rat_bin().replace('\\', "\\\\");
    let file = fixture(
        dir.path(),
        "sized.toml",
        &format!(
            r#"
[[pane]]
name = "wide"
height = 2
chrome = false
border = "none"
command = ["{bin}", "style", "x"]
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
