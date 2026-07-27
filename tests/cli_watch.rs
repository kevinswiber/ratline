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

#[test]
fn once_prints_child_stdout_and_exits_zero() {
    rat()
        .args(["watch", "--once", "--", &rat_bin(), "style", "hi"])
        .assert()
        .success()
        .stdout(predicates::str::contains("hi"));
}

#[test]
fn piped_output_has_no_movement_or_cursor_escapes() {
    let assert = rat()
        .env("NO_COLOR", "1")
        .args(["watch", "--once", "--", &rat_bin(), "style", "hi"])
        .assert()
        .success();
    let stdout = String::from_utf8_lossy(&assert.get_output().stdout).into_owned();
    for escape in ["\x1b[?25l", "\x1b[?2026", "\x1b[0J", "\x1b[?2031h"] {
        assert!(!stdout.contains(escape), "found {escape:?} in {stdout:?}");
    }
}

#[test]
fn child_failure_does_not_fail_the_watch() {
    rat()
        .args(["watch", "--once", "--", &rat_bin(), "__exitcode", "3"])
        .assert()
        .success();
}

#[test]
fn missing_command_is_a_usage_error() {
    rat().args(["watch", "--once"]).assert().code(2);
}

#[test]
fn child_ansi_passes_through_verbatim() {
    rat()
        .env("TERM", "xterm-256color")
        .args([
            "watch",
            "--once",
            "--",
            &rat_bin(),
            "--color",
            "always",
            "style",
            "--foreground",
            "212",
            "red",
        ])
        .assert()
        .success()
        .stdout(predicates::str::contains("\x1b[38;5;212mred\x1b[0m"));
}

#[test]
fn title_line_is_prepended() {
    rat()
        .env("NO_COLOR", "1")
        .args([
            "watch",
            "--once",
            "--title",
            "My Dashboard",
            "--",
            &rat_bin(),
            "style",
            "body",
        ])
        .assert()
        .success()
        .stdout(predicates::str::contains("My Dashboard"));
}

#[test]
fn bad_interval_is_an_error() {
    rat()
        .args([
            "watch",
            "-n",
            "bogus",
            "--once",
            "--",
            &rat_bin(),
            "style",
            "hi",
        ])
        .assert()
        .code(1);
}

#[test]
fn spawn_failure_in_once_mode_fails_loudly() {
    rat()
        .args(["watch", "--once", "--", "definitely-no-such-binary-xyz"])
        .assert()
        .code(1)
        .stderr(predicates::str::contains("definitely-no-such-binary-xyz"));
}

// The --shell script is platform-specific by nature: sh on unix, the
// COMSPEC cmd on Windows.
#[cfg(unix)]
const SHELL_MATH: &str = "echo $((6 * 7))";
#[cfg(windows)]
const SHELL_MATH: &str = "set /a 6*7";

#[test]
fn shell_mode_runs_through_the_platform_shell() {
    rat()
        .args(["watch", "--once", "--shell", "--", SHELL_MATH])
        .assert()
        .success()
        .stdout(predicates::str::contains("42"));
}

#[test]
fn child_stderr_passes_through_when_piped() {
    rat()
        .env("NO_COLOR", "1")
        .args([
            "watch",
            "--once",
            "--",
            &rat_bin(),
            "__exitcode",
            "0",
            "err",
        ])
        .assert()
        .success()
        .stderr(predicates::str::contains("err"));
}

#[test]
fn child_stderr_stays_off_stdout_when_piped() {
    let assert = rat()
        .env("NO_COLOR", "1")
        .args([
            "watch",
            "--once",
            "--",
            &rat_bin(),
            "__exitcode",
            "0",
            "err",
        ])
        .assert()
        .success();
    let stdout = String::from_utf8_lossy(&assert.get_output().stdout).into_owned();
    assert!(
        !stdout.contains("err"),
        "stderr leaked into stdout: {stdout:?}"
    );
}

#[test]
fn clear_flag_stays_silent_when_piped() {
    let assert = rat()
        .env("NO_COLOR", "1")
        .args([
            "watch",
            "--clear",
            "--once",
            "--",
            &rat_bin(),
            "style",
            "hi",
        ])
        .assert()
        .success();
    let stdout = String::from_utf8_lossy(&assert.get_output().stdout).into_owned();
    assert!(!stdout.contains("\x1b[2J"), "piped output must stay plain");
    assert!(stdout.contains("hi"));
}

#[test]
fn children_receive_the_frame_size_env() {
    rat()
        .env("NO_COLOR", "1")
        .args(["watch", "--once", "--", &rat_bin(), "__env", "RAT_WIDTH"])
        .assert()
        .success()
        .stdout(predicates::str::is_match(r"^[0-9]+\n$").unwrap());
    rat()
        .env("NO_COLOR", "1")
        .args(["watch", "--once", "--", &rat_bin(), "__env", "RAT_HEIGHT"])
        .assert()
        .success()
        .stdout(predicates::str::is_match(r"^[0-9]+\n$").unwrap());
}

#[test]
fn children_receive_the_appearance_env() {
    rat()
        .env("NO_COLOR", "1")
        .args([
            "watch",
            "--once",
            "--",
            &rat_bin(),
            "__env",
            "RAT_APPEARANCE",
        ])
        .assert()
        .success()
        .stdout(predicates::str::is_match(r"^(light|dark)\n$").unwrap());
}

#[test]
fn the_parent_appearance_passes_through_to_children() {
    rat()
        .env("NO_COLOR", "1")
        .env("RAT_APPEARANCE", "light")
        .args([
            "watch",
            "--once",
            "--",
            &rat_bin(),
            "__env",
            "RAT_APPEARANCE",
        ])
        .assert()
        .success()
        .stdout("light\n");
}

#[test]
fn an_explicit_parent_appearance_reaches_children() {
    rat()
        .env("NO_COLOR", "1")
        .args([
            "--appearance",
            "dark",
            "watch",
            "--once",
            "--",
            &rat_bin(),
            "__env",
            "RAT_APPEARANCE",
        ])
        .assert()
        .success()
        .stdout("dark\n");
}
