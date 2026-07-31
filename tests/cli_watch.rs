mod common;

use assert_cmd::Command;

fn rat() -> Command {
    let mut cmd = common::rat();
    cmd.env_remove("NO_COLOR");
    cmd
}

/// Path to the rat binary, used as a portable child process: shell
/// utilities like sh, echo, and printf do not exist everywhere.
fn rat_bin() -> String {
    assert_cmd::cargo::cargo_bin("rat").display().to_string()
}

#[test]
fn once_prints_child_stdout_and_exits_zero() {
    rat()
        .args(["watch", "--once", "--", &rat_bin(), "style", "hi"])
        .assert()
        .success()
        .stdout(predicates::str::contains("hi"));
}

#[test]
fn piped_output_has_no_movement_or_cursor_escapes() {
    let assert = rat()
        .env("NO_COLOR", "1")
        .args(["watch", "--once", "--", &rat_bin(), "style", "hi"])
        .assert()
        .success();
    let stdout = String::from_utf8_lossy(&assert.get_output().stdout).into_owned();
    for escape in ["\x1b[?25l", "\x1b[?2026", "\x1b[0J", "\x1b[?2031h"] {
        assert!(!stdout.contains(escape), "found {escape:?} in {stdout:?}");
    }
}

#[test]
fn child_failure_does_not_fail_the_watch() {
    rat()
        .args(["watch", "--once", "--", &rat_bin(), "__exitcode", "3"])
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
        .env("TERM", "xterm-256color")
        .args([
            "watch",
            "--once",
            "--",
            &rat_bin(),
            "--color",
            "always",
            "style",
            "--foreground",
            "212",
            "red",
        ])
        .assert()
        .success()
        .stdout(predicates::str::contains("\x1b[38;5;212mred\x1b[0m"));
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
            &rat_bin(),
            "style",
            "body",
        ])
        .assert()
        .success()
        .stdout(predicates::str::contains("My Dashboard"));
}

#[test]
fn bad_interval_is_an_error() {
    rat()
        .args([
            "watch",
            "-n",
            "bogus",
            "--once",
            "--",
            &rat_bin(),
            "style",
            "hi",
        ])
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

// The --shell script is platform-specific by nature: sh on unix, the
// COMSPEC cmd on Windows.
#[cfg(unix)]
const SHELL_MATH: &str = "echo $((6 * 7))";
#[cfg(windows)]
const SHELL_MATH: &str = "set /a 6*7";

#[test]
fn shell_mode_runs_through_the_platform_shell() {
    rat()
        .args(["watch", "--once", "--shell", "--", SHELL_MATH])
        .assert()
        .success()
        .stdout(predicates::str::contains("42"));
}

#[test]
fn child_stderr_passes_through_when_piped() {
    rat()
        .env("NO_COLOR", "1")
        .args([
            "watch",
            "--once",
            "--",
            &rat_bin(),
            "__exitcode",
            "0",
            "err",
        ])
        .assert()
        .success()
        .stderr(predicates::str::contains("err"));
}

#[test]
fn child_stderr_stays_off_stdout_when_piped() {
    let assert = rat()
        .env("NO_COLOR", "1")
        .args([
            "watch",
            "--once",
            "--",
            &rat_bin(),
            "__exitcode",
            "0",
            "err",
        ])
        .assert()
        .success();
    let stdout = String::from_utf8_lossy(&assert.get_output().stdout).into_owned();
    assert!(
        !stdout.contains("err"),
        "stderr leaked into stdout: {stdout:?}"
    );
}

#[test]
fn clear_flag_stays_silent_when_piped() {
    let assert = rat()
        .env("NO_COLOR", "1")
        .args([
            "watch",
            "--clear",
            "--once",
            "--",
            &rat_bin(),
            "style",
            "hi",
        ])
        .assert()
        .success();
    let stdout = String::from_utf8_lossy(&assert.get_output().stdout).into_owned();
    assert!(!stdout.contains("\x1b[2J"), "piped output must stay plain");
    assert!(stdout.contains("hi"));
}

#[test]
fn children_receive_the_frame_size_env() {
    rat()
        .env("NO_COLOR", "1")
        .args(["watch", "--once", "--", &rat_bin(), "__env", "RAT_WIDTH"])
        .assert()
        .success()
        .stdout(predicates::str::is_match(r"^[0-9]+\n$").unwrap());
    rat()
        .env("NO_COLOR", "1")
        .args(["watch", "--once", "--", &rat_bin(), "__env", "RAT_HEIGHT"])
        .assert()
        .success()
        .stdout(predicates::str::is_match(r"^[0-9]+\n$").unwrap());
}

#[test]
fn children_receive_the_appearance_env() {
    rat()
        .env("NO_COLOR", "1")
        .args([
            "watch",
            "--once",
            "--",
            &rat_bin(),
            "__env",
            "RAT_APPEARANCE",
        ])
        .assert()
        .success()
        .stdout(predicates::str::is_match(r"^(light|dark)\n$").unwrap());
}

#[test]
fn the_parent_appearance_passes_through_to_children() {
    rat()
        .env("NO_COLOR", "1")
        .env("RAT_APPEARANCE", "light")
        .args([
            "watch",
            "--once",
            "--",
            &rat_bin(),
            "__env",
            "RAT_APPEARANCE",
        ])
        .assert()
        .success()
        .stdout("light\n");
}

#[test]
fn an_explicit_parent_appearance_reaches_children() {
    rat()
        .env("NO_COLOR", "1")
        .args([
            "--appearance",
            "dark",
            "watch",
            "--once",
            "--",
            &rat_bin(),
            "__env",
            "RAT_APPEARANCE",
        ])
        .assert()
        .success()
        .stdout("dark\n");
}

#[test]
fn snapshot_flags_are_accepted() {
    // The flags ride along in scripted runs; --once has no input loop, so
    // no snapshot is ever taken and nothing lands in the directory.
    let dir = tempfile::tempdir().expect("tempdir");
    rat()
        .args([
            "watch",
            "--once",
            "--snapshot-dir",
            &dir.path().display().to_string(),
            "--snapshot-ansi",
            "--",
            &rat_bin(),
            "style",
            "hi",
        ])
        .assert()
        .success()
        .stdout(predicates::str::contains("hi"));
    assert_eq!(
        std::fs::read_dir(dir.path()).expect("read tempdir").count(),
        0,
        "a snapshot appeared without the snapshot key"
    );
}

#[test]
fn no_wrap_is_accepted_and_leaves_piped_output_alone() {
    // The toggle is a tty-view feature: the piped branch prints lines raw
    // and never wrapped them in the first place, so --no-wrap changes
    // nothing there.
    let long_line = "x".repeat(200);
    rat()
        .args([
            "watch",
            "--once",
            "--no-wrap",
            "--",
            &rat_bin(),
            "style",
            &long_line,
        ])
        .assert()
        .success()
        .stdout(predicates::str::contains(long_line.as_str()));
}

/// The unified loop's piped-mode payoff: a signal lands between naps
/// instead of waiting out the child. The started file is child-side
/// evidence the slow tick is in flight before the signal is sent.
#[cfg(unix)]
#[test]
fn a_signal_ends_a_piped_watch_during_a_slow_child() {
    let dir = tempfile::tempdir().expect("tempdir");
    let started = dir.path().join("started");
    let script = format!(": > {}; /bin/sleep 5", started.display());
    let mut watch = std::process::Command::new(rat_bin())
        .args(["watch", "-n", "5s", "--shell", "--", &script])
        .stdout(std::process::Stdio::piped())
        .spawn()
        .expect("spawn rat watch piped");
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(3);
    while !started.exists() {
        assert!(
            std::time::Instant::now() < deadline,
            "the child never started"
        );
        std::thread::sleep(std::time::Duration::from_millis(20));
    }
    unsafe { libc::kill(watch.id() as i32, libc::SIGTERM) };
    // Reaped well inside the child's remaining ~5 s: the loop noticed
    // the flag between naps instead of waiting the child out.
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(3);
    loop {
        if let Some(status) = watch.try_wait().expect("try_wait") {
            assert!(status.success(), "the SIGTERM path returns Ok — exit 0");
            break;
        }
        assert!(
            std::time::Instant::now() < deadline,
            "SIGTERM waited out the child"
        );
        std::thread::sleep(std::time::Duration::from_millis(20));
    }
}

#[test]
fn a_trigger_conflicts_with_once() {
    rat()
        .args([
            "watch",
            "--once",
            "--trigger",
            "file:/tmp/x",
            "--",
            &rat_bin(),
            "style",
            "hi",
        ])
        .assert()
        .failure()
        .stderr(predicates::str::contains("--once"));
}

#[test]
fn a_trigger_debounce_requires_a_trigger() {
    rat()
        .args([
            "watch",
            "--trigger-debounce",
            "1s",
            "--",
            &rat_bin(),
            "style",
            "hi",
        ])
        .assert()
        .failure()
        .stderr(predicates::str::contains("--trigger <SPEC>"));
}

#[test]
fn a_bare_path_trigger_spec_teaches_the_schemes() {
    rat()
        .args([
            "watch",
            "--trigger",
            "/tmp/x",
            "--",
            &rat_bin(),
            "style",
            "hi",
        ])
        .assert()
        .failure()
        .stderr(predicates::str::contains("file:"));
}

/// Stream the piped watch's stdout through a channel so waiting for a
/// frame is bounded: a blocking read cannot swallow the deadline.
fn stdout_stream(stdout: std::process::ChildStdout) -> std::sync::mpsc::Receiver<Vec<u8>> {
    use std::io::Read;
    let (tx, rx) = std::sync::mpsc::channel();
    std::thread::spawn(move || {
        let mut stdout = stdout;
        let mut buf = [0u8; 4096];
        while let Ok(n) = stdout.read(&mut buf) {
            if n == 0 || tx.send(buf[..n].to_vec()).is_err() {
                return;
            }
        }
    });
    rx
}

/// Drain the stream until the needle appears — a missing frame is a
/// clean failure, never a hang.
fn read_until(stream: &std::sync::mpsc::Receiver<Vec<u8>>, seen: &mut String, needle: &str) {
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(10);
    loop {
        if seen.contains(needle) {
            return;
        }
        let left = deadline.saturating_duration_since(std::time::Instant::now());
        assert!(!left.is_zero(), "never saw {needle:?} in {seen:?}");
        match stream.recv_timeout(left) {
            Ok(chunk) => seen.push_str(&String::from_utf8_lossy(&chunk)),
            Err(_) => panic!("never saw {needle:?} in {seen:?}"),
        }
    }
}

/// Reap the watch even when an assertion panics: an orphaned watch
/// holds the harness's stdout pipe open and hangs the whole run.
struct KillOnDrop(std::process::Child);

impl Drop for KillOnDrop {
    fn drop(&mut self) {
        let _ = self.0.kill();
        let _ = self.0.wait();
    }
}

/// The portable trigger proof: a file change refreshes a PIPED watch —
/// the stat-poll runs before the non-interactive sleep, so event-driven
/// append-to-a-log works without a terminal, on every platform.
#[test]
fn a_file_trigger_refreshes_a_piped_watch() {
    let dir = tempfile::tempdir().expect("tempdir");
    let watched = dir.path().join("state");
    std::fs::write(&watched, "v0").expect("seed");
    // -n 1h parks the heartbeat out of the way: any second frame is the
    // trigger's doing, not the interval's.
    let watch = std::process::Command::new(rat_bin())
        .args([
            "watch",
            "-n",
            "1h",
            "--trigger",
            &format!("file:{}", watched.display()),
            "--trigger-debounce",
            "0ms",
            "--",
            &rat_bin(),
            "__cat",
            &watched.display().to_string(),
        ])
        .stdout(std::process::Stdio::piped())
        .spawn()
        .expect("spawn rat watch piped");
    let mut watch = KillOnDrop(watch);
    let stream = stdout_stream(watch.0.stdout.take().expect("piped stdout"));
    let mut seen = String::new();
    read_until(&stream, &mut seen, "v0"); // the immediate first tick
    std::fs::write(&watched, "v1").expect("mtime change");
    read_until(&stream, &mut seen, "v1"); // the trigger-driven frame
    // The guard's Drop kills and reaps — kill only SENDS the signal, and
    // an unreaped child would zombie (unix) and race tempdir cleanup.
}

#[test]
fn a_flooding_child_is_bounded_and_says_so_on_stderr() {
    // The plain path has no chrome row to carry a badge, and this is
    // the configuration the bound was reported against — so the marker
    // has to reach somewhere, and stdout is not available: it is the
    // data a consumer parses.
    let assert = rat()
        .args(["watch", "--once", "--", &rat_bin(), "__lines", "3000"])
        .assert()
        .success();
    let stdout = String::from_utf8_lossy(&assert.get_output().stdout).into_owned();
    let stderr = String::from_utf8_lossy(&assert.get_output().stderr).into_owned();

    let body: Vec<&str> = stdout.lines().collect();
    assert_eq!(body.len(), 1000, "the retained window is the bound");
    assert_eq!(body[0], "2000", "and it is the TAIL that survived");
    assert_eq!(body[999], "2999");

    assert!(
        stderr.contains("2.0k lines dropped"),
        "the drop must be reported somewhere; stderr was {stderr:?}"
    );
    assert!(
        !stdout.contains("dropped"),
        "and never in the data: {stdout:?}"
    );
}

#[test]
fn a_child_inside_the_bound_says_nothing_at_all() {
    // The common case stays byte-for-byte what it was: no marker, no
    // stderr, nothing added to a stream someone is parsing.
    let assert = rat()
        .args(["watch", "--once", "--", &rat_bin(), "__lines", "3"])
        .assert()
        .success();
    assert_eq!(
        String::from_utf8_lossy(&assert.get_output().stdout),
        "0\n1\n2\n"
    );
    assert_eq!(String::from_utf8_lossy(&assert.get_output().stderr), "");
}
