mod common;

use assert_cmd::Command;

const SUBCOMMANDS: [&str; 15] = [
    "style",
    "bar",
    "duration",
    "date",
    "spark",
    "log",
    "frame",
    "watch",
    "doctor",
    "choose",
    "confirm",
    "input",
    "filter",
    "spin",
    "completion",
];

fn rat() -> Command {
    Command::cargo_bin("rat").expect("rat binary builds")
}

#[test]
fn help_lists_every_subcommand() {
    let assert = rat().arg("--help").assert().success();
    let stdout = String::from_utf8_lossy(&assert.get_output().stdout).into_owned();
    for name in SUBCOMMANDS {
        assert!(stdout.contains(name), "--help is missing `{name}`");
    }
}

#[test]
fn version_exits_zero() {
    rat().arg("--version").assert().success();
}

#[test]
fn unknown_subcommand_is_usage_error() {
    rat().arg("bogus").assert().code(2);
}

#[test]
fn exitcode_harness_maps_codes_exactly() {
    for code in [0, 1, 7, 124, 130] {
        rat()
            .args(["__exitcode", &code.to_string()])
            .assert()
            .code(code);
    }
}

#[test]
fn silent_failures_write_nothing_to_stderr() {
    // 1 maps to NoSelection, 130 to Aborted, 7 to Child: all silent.
    for code in [1, 7, 130] {
        rat()
            .args(["__exitcode", &code.to_string()])
            .assert()
            .stderr(predicates::str::is_empty());
    }
}

#[test]
fn timeout_prints_timed_out() {
    rat()
        .args(["__exitcode", "124"])
        .assert()
        .code(124)
        .stderr("timed out\n");
}

#[test]
fn broken_pipe_does_not_panic() {
    let rat_path = assert_cmd::cargo::cargo_bin("rat");
    let output = std::process::Command::new("sh")
        .arg("-c")
        .arg(format!(
            "seq 1 100000 | {} spark | head -c 10 >/dev/null; true",
            rat_path.display()
        ))
        .output()
        .expect("shell runs");
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(
        !stderr.contains("panicked"),
        "rat panicked on a closed pipe: {stderr}"
    );
}

#[cfg(unix)]
#[test]
fn confirm_prompt_parses_positionally() {
    // Exit 1 proves the positional parsed (a parse error would exit 2);
    // detached so no real prompt opens in interactive test runs.
    common::rat_detached()
        .args(["confirm", "Ship it to prod?"])
        .assert()
        .code(1);
}
