#![allow(dead_code)]
#[cfg(unix)]
pub mod pty;

use assert_cmd::Command;

pub fn rat() -> Command {
    let mut cmd = Command::cargo_bin("rat").expect("rat binary builds");
    // Both can change which palette a run picks, so tests pin neither.
    cmd.env_remove("RAT_APPEARANCE");
    cmd.env_remove("COLORFGBG");
    // A developer's shell must not redirect where test snapshots land.
    cmd.env_remove("RAT_SNAPSHOT_DIR");
    cmd
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
    cmd.env_remove("RAT_APPEARANCE");
    cmd.env_remove("COLORFGBG");
    cmd.env_remove("RAT_SNAPSHOT_DIR");
    Command::from_std(cmd)
}
