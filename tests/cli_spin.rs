use assert_cmd::Command;

fn rat() -> Command {
    Command::cargo_bin("rat").expect("rat binary builds")
}

#[test]
fn child_exit_code_is_propagated() {
    rat()
        .args(["spin", "--", "sh", "-c", "exit 3"])
        .assert()
        .code(3);
}

#[test]
fn silent_success_prints_nothing() {
    rat()
        .args(["spin", "--", "echo", "hi"])
        .assert()
        .success()
        .stdout("");
}

#[test]
fn show_output_passes_child_stdout_through() {
    rat()
        .args(["spin", "--show-output", "--", "echo", "hi"])
        .assert()
        .success()
        .stdout("hi\n");
}

#[test]
fn show_error_prints_output_only_on_failure() {
    rat()
        .args([
            "spin",
            "--show-error",
            "--",
            "sh",
            "-c",
            "echo boom >&2; exit 2",
        ])
        .assert()
        .code(2)
        .stderr(predicates::str::contains("boom"));
    rat()
        .args(["spin", "--show-error", "--", "sh", "-c", "echo fine"])
        .assert()
        .success()
        .stdout("");
}

#[test]
fn missing_command_is_a_usage_error() {
    rat().arg("spin").assert().code(2);
}

#[test]
fn signal_death_becomes_exit_one() {
    rat()
        .args(["spin", "--", "sh", "-c", "kill -9 $$"])
        .assert()
        .code(1);
}

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
