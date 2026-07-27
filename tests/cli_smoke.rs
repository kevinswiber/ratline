mod common;

use assert_cmd::Command;

const SUBCOMMANDS: [&str; 16] = [
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
    "table",
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
    use std::io::{Read, Write};
    let mut child = std::process::Command::new(assert_cmd::cargo::cargo_bin("rat"))
        .arg("spark")
        .stdin(std::process::Stdio::piped())
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::piped())
        .spawn()
        .expect("rat spawns");
    {
        // Enough values that the sparkline output exceeds any pipe buffer.
        let mut stdin = child.stdin.take().expect("stdin piped");
        for i in 0..100_000 {
            let _ = writeln!(stdin, "{}", i % 8);
        }
    }
    // Read a taste, then close the pipe mid-write.
    let mut out = child.stdout.take().expect("stdout piped");
    let mut buf = [0u8; 10];
    let _ = out.read(&mut buf);
    drop(out);
    let _ = child.wait().expect("child exits");
    let mut err = String::new();
    child
        .stderr
        .take()
        .expect("stderr piped")
        .read_to_string(&mut err)
        .expect("stderr reads");
    assert!(
        !err.contains("panicked"),
        "rat panicked on a closed pipe: {err}"
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
