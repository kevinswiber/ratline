use assert_cmd::Command;

fn rat() -> Command {
    let mut cmd = Command::cargo_bin("rat").expect("rat binary builds");
    // Both can change which palette a run picks, so tests pin neither.
    cmd.env_remove("RAT_APPEARANCE");
    cmd.env_remove("COLORFGBG");
    cmd
}

#[test]
fn fish_completions_generate() {
    rat()
        .args(["completion", "fish"])
        .assert()
        .success()
        .stdout(predicates::str::contains("complete -c rat"));
}

#[test]
fn bash_and_zsh_generate_nonempty() {
    for shell in ["bash", "zsh"] {
        let assert = rat().args(["completion", shell]).assert().success();
        assert!(!assert.get_output().stdout.is_empty(), "{shell} empty");
    }
}

#[test]
fn no_color_help_has_no_escapes() {
    let assert = rat().env("NO_COLOR", "1").arg("--help").assert().success();
    let stdout = String::from_utf8_lossy(&assert.get_output().stdout).into_owned();
    assert!(
        !stdout.contains('\x1b'),
        "help must be plain under NO_COLOR"
    );
}
