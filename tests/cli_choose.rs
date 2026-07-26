use assert_cmd::Command;

fn rat() -> Command {
    Command::cargo_bin("rat").expect("rat binary builds")
}

/// Run rat detached from the controlling terminal, so /dev/tty cannot be
/// opened even when the test suite itself runs in an interactive session.
#[cfg(unix)]
fn rat_detached(args: &[&str]) -> Command {
    use std::os::unix::process::CommandExt;
    let mut cmd = std::process::Command::new(assert_cmd::cargo::cargo_bin("rat"));
    cmd.args(args);
    unsafe {
        cmd.pre_exec(|| {
            // The forked child is never a session leader, so this succeeds
            // and severs the controlling terminal.
            libc::setsid();
            Ok(())
        });
    }
    Command::from_std(cmd)
}

#[cfg(unix)]
#[test]
fn needs_a_terminal_when_interactive() {
    rat_detached(&["choose", "a", "b"])
        .assert()
        .code(1)
        .stderr(predicates::str::contains("terminal"));
    rat_detached(&["confirm"]).assert().code(1);
}

#[test]
fn select_if_one_needs_no_terminal() {
    rat()
        .args(["choose", "--select-if-one", "only"])
        .assert()
        .success()
        .stdout("only\n");
}
