mod common;

use assert_cmd::Command;

// Detached on unix so the spinner does not paint over interactive test
// runs; plain elsewhere.
#[cfg(unix)]
fn rat() -> Command {
    common::rat_detached()
}

#[cfg(windows)]
fn rat() -> Command {
    common::rat()
}

/// Path to the rat binary, used as a portable child process.
fn rat_bin() -> String {
    assert_cmd::cargo::cargo_bin("rat").display().to_string()
}

#[test]
fn child_exit_code_is_propagated() {
    rat()
        .args(["spin", "--", &rat_bin(), "__exitcode", "3"])
        .assert()
        .code(3);
}

#[test]
fn silent_success_prints_nothing() {
    rat()
        .args(["spin", "--", &rat_bin(), "style", "hi"])
        .assert()
        .success()
        .stdout("");
}

#[test]
fn show_output_passes_child_stdout_through() {
    rat()
        .env("NO_COLOR", "1")
        .args(["spin", "--show-output", "--", &rat_bin(), "style", "hi"])
        .assert()
        .success()
        .stdout("hi\n");
}

#[test]
fn show_error_prints_output_only_on_failure() {
    rat()
        .env("NO_COLOR", "1")
        .args([
            "spin",
            "--show-error",
            "--",
            &rat_bin(),
            "__exitcode",
            "2",
            "boom",
        ])
        .assert()
        .code(2)
        .stderr(predicates::str::contains("boom"));
    rat()
        .env("NO_COLOR", "1")
        .args(["spin", "--show-error", "--", &rat_bin(), "style", "fine"])
        .assert()
        .success()
        .stdout("");
}

#[test]
fn missing_command_is_a_usage_error() {
    rat().arg("spin").assert().code(2);
}

#[cfg(unix)]
#[test]
fn signal_death_becomes_exit_one() {
    rat()
        .args(["spin", "--", "sh", "-c", "kill -9 $$"])
        .assert()
        .code(1);
}

#[cfg(unix)]
#[test]
fn timeout_kills_the_child() {
    rat()
        .args(["spin", "--timeout", "200ms", "--", "sleep", "5"])
        .assert()
        .code(124)
        .stderr(predicates::str::contains("timed out"));
}

#[test]
fn spawn_failure_is_a_plain_error() {
    rat()
        .args(["spin", "--", "no-such-binary-abcxyz"])
        .assert()
        .code(1)
        .stderr(predicates::str::contains("no-such-binary-abcxyz"));
}
