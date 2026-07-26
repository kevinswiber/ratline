use assert_cmd::Command;

fn rat() -> Command {
    Command::cargo_bin("rat").expect("rat binary builds")
}

#[test]
fn needs_a_terminal_when_interactive() {
    rat()
        .args(["choose", "a", "b"])
        .assert()
        .code(1)
        .stderr(predicates::str::contains("terminal"));
    rat().arg("confirm").assert().code(1);
}

#[test]
fn select_if_one_needs_no_terminal() {
    rat()
        .args(["choose", "--select-if-one", "only"])
        .assert()
        .success()
        .stdout("only\n");
}
