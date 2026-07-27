use assert_cmd::Command;
use predicates::prelude::*;

fn rat() -> Command {
    let mut cmd = Command::cargo_bin("rat").expect("rat binary builds");
    for var in [
        "NO_COLOR",
        "TERM",
        "COLORTERM",
        "CLICOLOR_FORCE",
        "CI",
        "RAT_LOG_LEVEL",
        "RAT_APPEARANCE",
        "COLORFGBG",
    ] {
        cmd.env_remove(var);
    }
    cmd
}

#[test]
fn plain_message_joins_with_spaces_on_stderr() {
    rat()
        .env("NO_COLOR", "1")
        .args(["log", "hello", "world"])
        .assert()
        .success()
        .stdout("")
        .stderr("hello world\n");
}

#[test]
fn info_level_is_tagged_and_colored() {
    rat()
        .env("TERM", "xterm-256color")
        .args(["--color", "always", "log", "--level", "info", "hi"])
        .assert()
        .success()
        .stderr("\x1b[1;38;5;86mINFO\x1b[0m hi\n");
}

#[test]
fn level_tags_are_four_chars() {
    rat()
        .env("NO_COLOR", "1")
        .args(["log", "--level", "error", "boom"])
        .assert()
        .success()
        .stderr("ERRO boom\n");
    rat()
        .env("NO_COLOR", "1")
        .args(["log", "--level", "debug", "x"])
        .assert()
        .success()
        .stderr("DEBU x\n");
}

#[test]
fn min_level_suppresses_lower_levels() {
    rat()
        .env("NO_COLOR", "1")
        .args(["log", "--min-level", "warn", "--level", "info", "quiet"])
        .assert()
        .success()
        .stderr("");
    rat()
        .env("NO_COLOR", "1")
        .env("RAT_LOG_LEVEL", "error")
        .args(["log", "--level", "warn", "quiet"])
        .assert()
        .success()
        .stderr("");
}

#[test]
fn unleveled_messages_bypass_the_filter() {
    rat()
        .env("NO_COLOR", "1")
        .args(["log", "--min-level", "error", "always shown"])
        .assert()
        .success()
        .stderr("always shown\n");
}

#[test]
fn time_prefix_is_applied() {
    rat()
        .env("NO_COLOR", "1")
        .env("TZ", "UTC")
        .args(["log", "--time", "%Y", "dated"])
        .assert()
        .success()
        .stderr(predicate::str::is_match(r"^\d{4} dated\n$").unwrap());
}

#[test]
fn file_appends_plain_text() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("out.log");
    rat()
        .env("TERM", "xterm-256color")
        .args(["--color", "always", "log", "--level", "info"])
        .arg("--file")
        .arg(&path)
        .arg("to file")
        .assert()
        .success()
        .stderr("");
    let content = std::fs::read_to_string(&path).unwrap();
    assert_eq!(content, "INFO to file\n");
}

#[test]
fn info_is_the_light_hue_under_a_light_appearance() {
    rat()
        .env("TERM", "xterm-256color")
        .env("RAT_APPEARANCE", "light")
        .args(["--color", "always", "log", "--level", "info", "hi"])
        .assert()
        .success()
        .stderr("\x1b[1;38;5;30mINFO\x1b[0m hi\n");
}

#[test]
fn the_appearance_flag_beats_the_environment() {
    // The only test that exercises flag-over-environment precedence.
    rat()
        .env("TERM", "xterm-256color")
        .env("RAT_APPEARANCE", "dark")
        .args([
            "--color",
            "always",
            "--appearance",
            "light",
            "log",
            "--level",
            "info",
            "hi",
        ])
        .assert()
        .success()
        .stderr("\x1b[1;38;5;30mINFO\x1b[0m hi\n");
    // …and the mirror image, so neither side is accidentally hardcoded.
    rat()
        .env("TERM", "xterm-256color")
        .env("RAT_APPEARANCE", "light")
        .args([
            "--color",
            "always",
            "--appearance",
            "dark",
            "log",
            "--level",
            "info",
            "hi",
        ])
        .assert()
        .success()
        .stderr("\x1b[1;38;5;86mINFO\x1b[0m hi\n");
}

#[test]
fn log_warn_keeps_its_own_hue_apart_from_the_threshold_convention() {
    // The warn a log line uses is not the warn a threshold band uses.
    rat()
        .env("TERM", "xterm-256color")
        .args([
            "--color",
            "always",
            "--appearance",
            "dark",
            "log",
            "--level",
            "warn",
            "x",
        ])
        .assert()
        .success()
        .stderr("\x1b[1;38;5;192mWARN\x1b[0m x\n");
    rat()
        .env("TERM", "xterm-256color")
        .args([
            "--color",
            "always",
            "--appearance",
            "light",
            "log",
            "--level",
            "warn",
            "x",
        ])
        .assert()
        .success()
        .stderr("\x1b[1;38;5;100mWARN\x1b[0m x\n");
}
