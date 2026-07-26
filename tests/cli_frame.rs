use assert_cmd::Command;
use tempfile::TempDir;

fn rat_frame(dir: &TempDir, extra: &[&str], stdin: &str) -> assert_cmd::assert::Assert {
    let state = dir.path().join("state");
    let mut cmd = Command::cargo_bin("rat").expect("rat binary builds");
    cmd.arg("frame")
        .arg("--state")
        .arg(&state)
        .args(extra)
        .write_stdin(stdin.to_string());
    cmd.assert()
}

fn stdout_of(assert: assert_cmd::assert::Assert) -> String {
    String::from_utf8_lossy(&assert.get_output().stdout).into_owned()
}

#[test]
fn identical_input_repaints_once_then_skips() {
    let dir = TempDir::new().unwrap();
    let first = stdout_of(rat_frame(&dir, &["--width", "80"], "hello\n").success());
    assert!(first.contains("\x1b[?2026h"), "got: {first:?}");
    assert!(first.contains("hello"));
    let second = stdout_of(rat_frame(&dir, &["--width", "80"], "hello\n").success());
    assert_eq!(second, "", "unchanged content must write nothing");
}

#[test]
fn changed_content_moves_up_over_previous_rows() {
    let dir = TempDir::new().unwrap();
    rat_frame(&dir, &["--width", "80"], "a\nb\n").success();
    let second = stdout_of(rat_frame(&dir, &["--width", "80"], "c\n").success());
    assert!(second.contains("\x1b[2A"), "got: {second:?}");
    assert!(second.contains("\x1b[0J"));
}

#[test]
fn width_change_forces_full_repaint_without_move_up() {
    let dir = TempDir::new().unwrap();
    rat_frame(&dir, &["--width", "80"], "same\n").success();
    let second = stdout_of(rat_frame(&dir, &["--width", "100"], "same\n").success());
    assert!(!second.is_empty(), "resize must repaint despite same hash");
    assert!(
        !second.contains('A') || !second.contains("\x1b["),
        "resize must not move up over stale rows: {second:?}"
    );
}

#[test]
fn reset_deletes_state() {
    let dir = TempDir::new().unwrap();
    rat_frame(&dir, &["--width", "80"], "x\n").success();
    rat_frame(&dir, &["--reset"], "").success();
    let third = stdout_of(rat_frame(&dir, &["--width", "80"], "x\n").success());
    assert!(!third.is_empty(), "after reset the frame repaints");
}

#[test]
fn no_sync_omits_2026() {
    let dir = TempDir::new().unwrap();
    let out = stdout_of(rat_frame(&dir, &["--width", "80", "--no-sync"], "x\n").success());
    assert!(!out.contains("\x1b[?2026"), "got: {out:?}");
}

#[test]
fn hides_cursor_by_default_but_not_when_asked() {
    let dir = TempDir::new().unwrap();
    let out = stdout_of(rat_frame(&dir, &["--width", "80"], "x\n").success());
    assert!(out.contains("\x1b[?25l"));
    let dir2 = TempDir::new().unwrap();
    let out2 = stdout_of(rat_frame(&dir2, &["--width", "80", "--no-hide-cursor"], "x\n").success());
    assert!(!out2.contains("\x1b[?25l"));
}

#[test]
fn finish_shows_cursor_and_closes_frame() {
    let dir = TempDir::new().unwrap();
    rat_frame(&dir, &["--width", "80"], "x\n").success();
    let out = stdout_of(rat_frame(&dir, &["--finish"], "").success());
    assert_eq!(out, "\x1b[?2026l\x1b[?25h");
    let after = stdout_of(rat_frame(&dir, &["--width", "80"], "x\n").success());
    assert!(!after.is_empty(), "finish must also drop state");
}

#[test]
fn clear_erases_frame_and_restores_cursor() {
    let dir = TempDir::new().unwrap();
    rat_frame(&dir, &["--width", "80"], "a\nb\n").success();
    let out = stdout_of(rat_frame(&dir, &["--clear"], "").success());
    assert!(out.contains("\x1b[2A"), "got: {out:?}");
    assert!(out.contains("\x1b[0J"));
    assert!(out.ends_with("\x1b[?25h"));
}

#[test]
fn begin_and_end_are_stateless_escape_emitters() {
    let mut begin = Command::cargo_bin("rat").unwrap();
    begin
        .args(["frame", "begin"])
        .assert()
        .success()
        .stdout("\x1b[?2026h");
    let mut end = Command::cargo_bin("rat").unwrap();
    end.args(["frame", "end"])
        .assert()
        .success()
        .stdout("\x1b[?2026l");
}
