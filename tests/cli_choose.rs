mod common;

use common::rat;

#[cfg(unix)]
#[test]
fn needs_a_terminal_when_interactive() {
    common::rat_detached()
        .args(["choose", "a", "b"])
        .assert()
        .code(1)
        .stderr(predicates::str::contains("terminal"));
    common::rat_detached().arg("confirm").assert().code(1);
}

#[test]
fn select_if_one_needs_no_terminal() {
    rat()
        .args(["choose", "--select-if-one", "only"])
        .assert()
        .success()
        .stdout("only\n");
}
