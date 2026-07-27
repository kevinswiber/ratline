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
