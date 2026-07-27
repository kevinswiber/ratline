use assert_cmd::Command;

fn rat() -> Command {
    let mut cmd = Command::cargo_bin("rat").expect("rat binary builds");
    for var in [
        "NO_COLOR",
        "TERM",
        "COLORTERM",
        "CLICOLOR_FORCE",
        "CI",
        "RAT_APPEARANCE",
        "COLORFGBG",
    ] {
        cmd.env_remove(var);
    }
    cmd
}

#[test]
fn args_render_a_sparkline() {
    rat()
        .args(["spark", "0", "1", "2", "3", "4", "5", "6", "7"])
        .assert()
        .success()
        .stdout("▁▂▃▄▅▆▇█\n");
}

#[test]
fn stdin_values_work() {
    rat()
        .arg("spark")
        .write_stdin("0 7\n3.5\n")
        .assert()
        .success()
        .stdout("▁█▅\n");
}

#[test]
fn color_wraps_the_line() {
    rat()
        .env("TERM", "xterm-256color")
        .args([
            "--color",
            "always",
            "spark",
            "--spark-color",
            "212",
            "1",
            "2",
        ])
        .assert()
        .success()
        .stdout("\x1b[38;5;212m▁█\x1b[0m\n");
}

#[test]
fn non_numeric_input_is_an_error() {
    rat().args(["spark", "abc"]).assert().code(1);
}
