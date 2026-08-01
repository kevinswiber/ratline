mod common;

use assert_cmd::Command;

// Detached on unix so the spinner does not paint over interactive test
// runs; plain elsewhere.
#[cfg(unix)]
fn rat() -> Command {
    common::rat_detached()
}

#[cfg(windows)]
fn rat() -> Command {
    common::rat()
}

/// Path to the rat binary, used as a portable child process.
fn rat_bin() -> String {
    assert_cmd::cargo::cargo_bin("rat").display().to_string()
}

#[test]
fn child_exit_code_is_propagated() {
    rat()
        .args(["spin", "--", &rat_bin(), "__exitcode", "3"])
        .assert()
        .code(3);
}

#[test]
fn silent_success_prints_nothing() {
    rat()
        .args(["spin", "--", &rat_bin(), "style", "hi"])
        .assert()
        .success()
        .stdout("");
}

#[test]
fn show_output_passes_child_stdout_through() {
    rat()
        .env("NO_COLOR", "1")
        .args(["spin", "--show-output", "--", &rat_bin(), "style", "hi"])
        .assert()
        .success()
        .stdout("hi\n");
}

#[test]
fn show_error_prints_output_only_on_failure() {
    rat()
        .env("NO_COLOR", "1")
        .args([
            "spin",
            "--show-error",
            "--",
            &rat_bin(),
            "__exitcode",
            "2",
            "boom",
        ])
        .assert()
        .code(2)
        .stderr(predicates::str::contains("boom"));
    rat()
        .env("NO_COLOR", "1")
        .args(["spin", "--show-error", "--", &rat_bin(), "style", "fine"])
        .assert()
        .success()
        .stdout("");
}

#[test]
fn missing_command_is_a_usage_error() {
    rat().arg("spin").assert().code(2);
}

/// The bound, and which end of the stream it keeps.
///
/// `__lines` prints the decimals `0..count`, so **which** lines survived
/// is readable from the output itself. A test that only counted rows
/// would pass just as happily on a cap that kept the wrong end.
#[test]
fn a_child_that_outruns_the_bound_keeps_its_newest_lines() {
    let assert = rat()
        .args([
            "spin",
            "--show-output",
            "--",
            &rat_bin(),
            "__lines",
            "12000",
        ])
        .assert()
        .success();
    let out = String::from_utf8(assert.get_output().stdout.clone()).expect("utf-8");
    let lines: Vec<&str> = out.lines().collect();
    assert_eq!(lines.len(), 10_000, "the bound is a line count");
    // 12000 printed and the newest 10000 kept, so 0..1999 are gone.
    assert_eq!(lines.first().copied(), Some("2000"));
    assert_eq!(lines.last().copied(), Some("11999"));
}

/// A truncation nobody is told about is the failure mode this bound
/// would otherwise introduce: output the user asked for, silently
/// incomplete. The notice goes to stderr because stdout is whatever the
/// user is piping.
#[test]
fn spin_says_how_much_it_dropped_and_from_where() {
    rat()
        .args([
            "spin",
            "--show-output",
            "--",
            &rat_bin(),
            "__lines",
            "12000",
        ])
        .assert()
        .success()
        .stderr(predicates::str::contains(
            "2.0k lines dropped from stdout — kept the newest 10000",
        ));
}

/// The stderr ROUTE, which shares the decision and not the code path.
/// Each pipe gets its own accumulator, so covering only stdout would
/// leave the other half of the shipped capture unexercised.
#[test]
fn the_childs_stderr_is_bounded_on_its_own_route() {
    let assert = rat()
        .args([
            "spin",
            "--show-output",
            "--",
            &rat_bin(),
            "__lines",
            "--stderr",
            "12000",
        ])
        .assert()
        .success();
    let err = String::from_utf8(assert.get_output().stderr.clone()).expect("utf-8");
    assert!(
        err.contains("2.0k lines dropped from stderr — kept the newest 10000"),
        "no notice for the stderr route: {}",
        &err[..err.len().min(200)]
    );
    assert!(err.contains("\n11999\n"), "the newest line did not survive");
    assert!(!err.contains("\n1999\n"), "a dropped line survived");
    assert!(
        assert.get_output().stdout.is_empty(),
        "the child printed nothing on stdout"
    );
}

/// The identity witness. A command whose output fits is replayed whole
/// and says nothing — which is every real use of `spin`, and the reason
/// the notice means something when it does appear.
#[test]
fn a_child_under_the_bound_is_replayed_whole_and_says_nothing() {
    let assert = rat()
        .args([
            "spin",
            "--show-output",
            "--",
            &rat_bin(),
            "__lines",
            "10000",
        ])
        .assert()
        .success();
    let out = String::from_utf8(assert.get_output().stdout.clone()).expect("utf-8");
    assert_eq!(
        out.lines().count(),
        10_000,
        "exactly the bound is not over it"
    );
    assert_eq!(out.lines().next(), Some("0"), "nothing was evicted");
    assert!(
        assert.get_output().stderr.is_empty(),
        "nothing was dropped, so there is nothing to say"
    );
}

/// Both pipes are drained whatever the flags say — the child must never
/// block on a full pipe — so the drop happens here too. It is not
/// reported, because a truncation of output the user did not ask to see
/// tells them nothing they can act on, and this is the default
/// invocation.
#[test]
fn a_flood_nobody_asked_to_see_is_not_reported() {
    rat()
        .args(["spin", "--", &rat_bin(), "__lines", "12000"])
        .assert()
        .success()
        .stdout("")
        .stderr("");
}

#[cfg(unix)]
#[test]
fn signal_death_becomes_exit_one() {
    rat()
        .args(["spin", "--", "sh", "-c", "kill -9 $$"])
        .assert()
        .code(1);
}

#[cfg(unix)]
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
