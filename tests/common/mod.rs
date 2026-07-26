#![allow(dead_code)]
use assert_cmd::Command;

pub fn rat() -> Command {
    Command::cargo_bin("rat").expect("rat binary builds")
}

/// Detached from the controlling terminal via setsid, so /dev/tty fails
/// deterministically even when the test suite runs in an interactive
/// session. Required for any test asserting no-terminal behavior; piped
/// stdio alone does not sever the controlling tty.
#[cfg(unix)]
pub fn rat_detached() -> Command {
    use std::os::unix::process::CommandExt;
    let mut cmd = std::process::Command::new(assert_cmd::cargo::cargo_bin("rat"));
    unsafe {
        cmd.pre_exec(|| {
            // The forked child is never a session leader, so setsid
            // succeeds and severs the controlling terminal.
            libc::setsid();
            Ok(())
        });
    }
    Command::from_std(cmd)
}
