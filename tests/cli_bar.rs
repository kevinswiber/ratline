use assert_cmd::Command;

fn rat() -> Command {
    let mut cmd = Command::cargo_bin("rat").expect("rat binary builds");
    for var in [
        "NO_COLOR",
        "CLICOLOR",
        "CLICOLOR_FORCE",
        "CI",
        "COLORTERM",
        "TERM",
    ] {
        cmd.env_remove(var);
    }
    cmd
}

#[test]
fn bar_reproduces_fish_progress_line() {
    let expected = format!(
        "{}{} {}{}  1242/1288  96.4%  running\n",
        "L100 release recovery",
        " ".repeat(13),
        "█".repeat(30),
        "░".repeat(2),
    );
    rat()
        .env("NO_COLOR", "1")
        .args([
            "bar",
            "--value",
            "1242",
            "--total",
            "1288",
            "--label",
            "L100 release recovery",
            "--state",
            "running",
        ])
        .assert()
        .success()
        .stdout(expected);
}

#[test]
fn bar_colors_under_always() {
    rat()
        .env("TERM", "xterm-256color")
        .args([
            "--color",
            "always",
            "bar",
            "--value",
            "50",
            "--fill-color",
            "212",
        ])
        .assert()
        .success()
        .stdout(predicates::str::contains("\x1b[38;5;212m"));
}

#[test]
fn bar_negative_total_is_an_error() {
    rat()
        .args(["bar", "--value", "5", "--total", "-1"])
        .assert()
        .code(1)
        .stderr(predicates::str::contains("total"));
}

#[test]
fn bar_without_value_is_an_error_for_now() {
    rat().arg("bar").write_stdin("").assert().failure();
}
