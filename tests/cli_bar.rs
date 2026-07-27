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
        "RAT_APPEARANCE",
        "COLORFGBG",
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
fn explicit_label_width_pads_batch_rows() {
    // An explicit --label-width is the label column in batch mode too, so
    // bars from separate invocations align.
    let expected = format!(
        "ab{} {}{}  1/4  25.0%\n",
        " ".repeat(10),
        "█",
        "░".repeat(3)
    );
    rat()
        .env("NO_COLOR", "1")
        .args(["bar", "--label-width", "12", "--width", "4"])
        .write_stdin("ab\t1\t4\n")
        .assert()
        .success()
        .stdout(expected);
}

#[test]
fn explicit_label_width_pads_thresholded_batch_rows() {
    let expected = format!(
        "ab{} {}{}  1/4  25.0%\n",
        " ".repeat(10),
        "█",
        "░".repeat(3)
    );
    rat()
        .env("NO_COLOR", "1")
        .args([
            "bar",
            "--label-width",
            "12",
            "--width",
            "4",
            "--thresholds",
            "50:214,100:42",
        ])
        .write_stdin("ab\t1\t4\n")
        .assert()
        .success()
        .stdout(expected);
}

#[test]
fn omitted_label_width_still_autosizes_batch_rows() {
    // Without the flag, the batch label column stays the widest label.
    let expected = format!(
        "a{} {}{}  1/4  25.0%\nlonger {}{}  3/4  75.0%\n",
        " ".repeat(5),
        "█",
        "░".repeat(3),
        "█".repeat(3),
        "░",
    );
    rat()
        .env("NO_COLOR", "1")
        .args(["bar", "--width", "4"])
        .write_stdin("a\t1\t4\nlonger\t3\t4\n")
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

#[test]
fn bar_batch_renders_aligned_rows() {
    let assert = rat()
        .env("NO_COLOR", "1")
        .arg("bar")
        .write_stdin("a\t1\t4\nlonger\t3\t4\trunning\n")
        .assert()
        .success();
    let stdout = String::from_utf8_lossy(&assert.get_output().stdout).into_owned();
    let lines: Vec<&str> = stdout.lines().collect();
    assert_eq!(lines.len(), 2);
    let bar_at = |s: &str| s.find(['█', '░']).unwrap();
    assert_eq!(bar_at(lines[0]), bar_at(lines[1]));
    assert!(lines[1].contains("running"));
}

#[test]
fn bar_empty_stdin_without_value_is_an_error() {
    rat()
        .env("NO_COLOR", "1")
        .arg("bar")
        .write_stdin("")
        .assert()
        .code(1)
        .stderr(predicates::str::contains("--value"));
}

#[test]
fn bar_thresholds_color_by_band() {
    rat()
        .env("TERM", "xterm-256color")
        .args([
            "--color",
            "always",
            "bar",
            "--value",
            "10",
            "--total",
            "100",
            "--thresholds",
            "33:196,66:214,100:42",
        ])
        .assert()
        .success()
        .stdout(predicates::str::contains("\x1b[38;5;196m"));
}

#[test]
fn bar_explicit_fill_color_beats_thresholds() {
    rat()
        .env("TERM", "xterm-256color")
        .args([
            "--color",
            "always",
            "bar",
            "--value",
            "10",
            "--total",
            "100",
            "--thresholds",
            "33:196,66:214,100:42",
            "--fill-color",
            "212",
        ])
        .assert()
        .success()
        .stdout(predicates::str::contains("\x1b[38;5;212m"));
}

#[test]
fn bar_indeterminate_moves_with_tick() {
    let render = |tick: &str| {
        let assert = rat()
            .env("NO_COLOR", "1")
            .args(["bar", "--indeterminate", "--tick", tick, "--width", "16"])
            .assert()
            .success();
        String::from_utf8_lossy(&assert.get_output().stdout).into_owned()
    };
    assert_ne!(render("0"), render("1"));
    assert_eq!(render("0"), render("16"));
}

fn stdout_of(args: &[&str]) -> Vec<u8> {
    rat()
        .env("TERM", "xterm-256color")
        .args(args)
        .assert()
        .success()
        .get_output()
        .stdout
        .clone()
}

#[test]
fn theme_tokens_equal_their_dark_indices() {
    let base = [
        "--color",
        "always",
        "--appearance",
        "dark",
        "bar",
        "--value",
        "50",
    ];
    let with = |extra: [&str; 2]| -> Vec<u8> {
        let mut args = base.to_vec();
        args.extend_from_slice(&extra);
        stdout_of(&args)
    };
    assert_eq!(
        with(["--fill-color", "accent"]),
        with(["--fill-color", "212"])
    );
    assert_eq!(
        with(["--empty-color", "muted"]),
        with(["--empty-color", "240"])
    );
}

#[test]
fn threshold_tokens_equal_their_dark_indices() {
    let named = stdout_of(&[
        "--color",
        "always",
        "--appearance",
        "dark",
        "bar",
        "--value",
        "10",
        "--total",
        "100",
        "--thresholds",
        "50:warn,100:ok",
    ]);
    let indexed = stdout_of(&[
        "--color",
        "always",
        "--appearance",
        "dark",
        "bar",
        "--value",
        "10",
        "--total",
        "100",
        "--thresholds",
        "50:214,100:42",
    ]);
    assert_eq!(named, indexed);
}

// Green before and after: the pin that the default flip changes no bytes.
#[test]
fn the_empty_color_default_is_unchanged_under_dark() {
    assert_eq!(
        stdout_of(&[
            "--color",
            "always",
            "--appearance",
            "dark",
            "bar",
            "--value",
            "50"
        ]),
        stdout_of(&[
            "--color",
            "always",
            "--appearance",
            "dark",
            "bar",
            "--value",
            "50",
            "--empty-color",
            "240",
        ]),
    );
}

#[test]
fn a_colorless_profile_ignores_the_appearance() {
    let plain = rat()
        .env("NO_COLOR", "1")
        .args(["bar", "--value", "50"])
        .assert()
        .success()
        .get_output()
        .stdout
        .clone();
    let themed = rat()
        .env("NO_COLOR", "1")
        .args(["--appearance", "light", "bar", "--value", "50"])
        .assert()
        .success()
        .get_output()
        .stdout
        .clone();
    assert_eq!(plain, themed);
}
