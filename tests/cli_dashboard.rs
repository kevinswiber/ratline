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
