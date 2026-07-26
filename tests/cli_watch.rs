use assert_cmd::Command;

fn rat() -> Command {
    let mut cmd = Command::cargo_bin("rat").expect("rat binary builds");
    cmd.env_remove("NO_COLOR");
    cmd
}

#[test]
fn once_prints_child_stdout_and_exits_zero() {
    rat()
        .args(["watch", "--once", "--", "echo", "hi"])
        .assert()
        .success()
        .stdout(predicates::str::contains("hi"));
}

#[test]
fn piped_output_has_no_movement_or_cursor_escapes() {
    let assert = rat()
        .args(["watch", "--once", "--", "echo", "hi"])
        .assert()
        .success();
    let stdout = String::from_utf8_lossy(&assert.get_output().stdout).into_owned();
    for escape in ["\x1b[?25l", "\x1b[?2026", "\x1b[0J"] {
        assert!(!stdout.contains(escape), "found {escape:?} in {stdout:?}");
    }
}

#[test]
fn child_failure_does_not_fail_the_watch() {
    rat()
        .args(["watch", "--once", "--", "sh", "-c", "exit 3"])
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
        .args(["watch", "--once", "--", "printf", "\\033[31mred\\033[0m"])
        .assert()
        .success()
        .stdout(predicates::str::contains("\x1b[31mred\x1b[0m"));
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
            "echo",
            "body",
        ])
        .assert()
        .success()
        .stdout(predicates::str::contains("My Dashboard"));
}

#[test]
fn bad_interval_is_an_error() {
    rat()
        .args(["watch", "-n", "bogus", "--once", "--", "echo", "hi"])
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

#[test]
fn shell_mode_runs_through_sh() {
    rat()
        .args(["watch", "--once", "--shell", "--", "echo $((6 * 7))"])
        .assert()
        .success()
        .stdout(predicates::str::contains("42"));
}
