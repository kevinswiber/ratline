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
        "BORDER_FOREGROUND",
        "RAT_APPEARANCE",
        "COLORFGBG",
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

#[test]
fn a_border_draws_a_rounded_box() {
    rat()
        .env("NO_COLOR", "1")
        .args(["style", "--border", "rounded", "hi"])
        .assert()
        .success()
        .stdout("\u{256d}\u{2500}\u{2500}\u{256e}\n\u{2502}hi\u{2502}\n\u{2570}\u{2500}\u{2500}\u{256f}\n");
}

#[test]
fn no_box_flags_keep_the_previous_output_exactly() {
    rat()
        .env("NO_COLOR", "1")
        .args(["style", "--bold", "a", "b"])
        .assert()
        .success()
        .stdout("a\nb\n");
}

#[test]
fn borders_keep_their_glyphs_under_no_color() {
    rat()
        .env("NO_COLOR", "1")
        .args(["style", "--border", "rounded", "--border-color", "240", "hi"])
        .assert()
        .success()
        .stdout("\u{256d}\u{2500}\u{2500}\u{256e}\n\u{2502}hi\u{2502}\n\u{2570}\u{2500}\u{2500}\u{256f}\n");
}

#[test]
fn the_ascii_preset_is_the_dumb_terminal_border() {
    rat()
        .env("NO_COLOR", "1")
        .args(["style", "--border", "ascii", "hi"])
        .assert()
        .success()
        .stdout("+--+\n|hi|\n+--+\n");
}

#[test]
fn padding_title_and_width_compose() {
    rat()
        .env("NO_COLOR", "1")
        .args([
            "style", "--border", "normal", "--title", "run", "--padding", "0 1", "--width", "8",
            "ok",
        ])
        .assert()
        .success()
        .stdout("\u{250c}\u{2500} run \u{2500}\u{2500}\u{2500}\u{2500}\u{2510}\n\u{2502} ok       \u{2502}\n\u{2514}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2518}\n");
}

#[test]
fn the_border_color_applies_under_forced_color() {
    rat()
        .env("TERM", "xterm-256color")
        .args([
            "--color",
            "always",
            "style",
            "--border",
            "normal",
            "--border-color",
            "240",
            "x",
        ])
        .assert()
        .success()
        .stdout(predicates::str::contains("\x1b[38;5;240m\u{250c}"));
}

#[test]
fn a_background_fills_the_padding() {
    rat()
        .env("TERM", "xterm-256color")
        .args([
            "--color",
            "always",
            "style",
            "--background",
            "212",
            "--padding",
            "0 1",
            "hi",
        ])
        .assert()
        .success()
        .stdout("\x1b[48;5;212m hi \x1b[0m\n");
}

#[test]
fn a_bad_padding_shorthand_is_an_error() {
    rat()
        .args(["style", "--padding", "1 2 3 4 5", "x"])
        .assert()
        .code(1)
        .stderr(predicates::str::contains("padding"));
}

#[test]
fn tabs_survive_the_default_strip() {
    rat()
        .env("NO_COLOR", "1")
        .arg("style")
        .write_stdin("\x1b[1ma\tb\x1b[0m\n")
        .assert()
        .success()
        .stdout("a\tb\n");
}
