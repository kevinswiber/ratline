use assert_cmd::Command;

fn rat() -> Command {
    let mut cmd = Command::cargo_bin("rat").expect("rat binary builds");
    // Start from a known env; tests opt back in per-case.
    for var in [
        "NO_COLOR",
        "CLICOLOR",
        "CLICOLOR_FORCE",
        "CI",
        "COLORTERM",
        "TERM",
        "TERM_PROGRAM",
        "FOREGROUND",
        "BACKGROUND",
    ] {
        cmd.env_remove(var);
    }
    cmd
}

#[test]
fn no_color_outputs_plain_text() {
    rat()
        .env("NO_COLOR", "1")
        .args(["style", "--bold", "--foreground", "212", "x"])
        .assert()
        .success()
        .stdout("x\n");
}

#[test]
fn forced_color_survives_piped_stdout() {
    rat()
        .env("TERM", "xterm-256color")
        .args([
            "--color",
            "always",
            "style",
            "--bold",
            "--foreground",
            "212",
            "X",
        ])
        .assert()
        .success()
        .stdout("\x1b[1;38;5;212mX\x1b[0m\n");
}

#[test]
fn multiple_args_join_with_newline() {
    rat()
        .env("NO_COLOR", "1")
        .args(["style", "a", "b"])
        .assert()
        .success()
        .stdout("a\nb\n");
}

#[test]
fn trim_trims_each_line() {
    rat()
        .env("NO_COLOR", "1")
        .args(["style", "--trim", "  a  ", " b "])
        .assert()
        .success()
        .stdout("a\nb\n");
}

#[test]
fn reads_stdin_when_no_args() {
    rat()
        .env("NO_COLOR", "1")
        .arg("style")
        .write_stdin("hello\n")
        .assert()
        .success()
        .stdout("hello\n");
}

#[test]
fn empty_stdin_is_an_error() {
    rat()
        .env("NO_COLOR", "1")
        .arg("style")
        .write_stdin("")
        .assert()
        .code(1)
        .stderr(predicates::str::contains("no input provided"));
}

#[test]
fn invalid_color_fails_with_message() {
    rat()
        .args(["style", "--foreground", "notacolor", "x"])
        .assert()
        .code(1)
        .stderr(predicates::str::contains("notacolor"));
}

#[test]
fn strip_ansi_removes_input_escapes_by_default() {
    rat()
        .env("NO_COLOR", "1")
        .args(["style", "\x1b[31mred\x1b[0m"])
        .assert()
        .success()
        .stdout("red\n");
}

#[test]
fn no_strip_ansi_preserves_input_escapes() {
    rat()
        .env("NO_COLOR", "1")
        .args(["style", "--no-strip-ansi", "\x1b[31mred\x1b[0m"])
        .assert()
        .success()
        .stdout("\x1b[31mred\x1b[0m\n");
}

#[test]
fn foreground_env_var_applies() {
    rat()
        .env("TERM", "xterm-256color")
        .env("FOREGROUND", "212")
        .args(["--color", "always", "style", "X"])
        .assert()
        .success()
        .stdout("\x1b[38;5;212mX\x1b[0m\n");
}
