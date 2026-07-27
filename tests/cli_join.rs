use assert_cmd::Command;

fn rat() -> Command {
    let mut cmd = Command::cargo_bin("rat").expect("rat binary builds");
    for var in [
        "NO_COLOR",
        "CLICOLOR",
        "CLICOLOR_FORCE",
        "CI",
        "COLORTERM",
        "TERM",
        "RAT_WIDTH",
    ] {
        cmd.env_remove(var);
    }
    cmd
}

#[test]
fn joins_two_blocks_side_by_side() {
    rat()
        .env("NO_COLOR", "1")
        .args(["join", "--gap", "1", "a\nlonger", "1\n2"])
        .assert()
        .success()
        .stdout("a      1\nlonger 2\n");
}

#[test]
fn vertical_stacks_blocks() {
    rat()
        .env("NO_COLOR", "1")
        .args(["join", "--vertical", "--gap", "1", "a", "b"])
        .assert()
        .success()
        .stdout("a\n\nb\n");
}

#[test]
fn files_are_blocks_in_flag_order() {
    let dir = tempfile::tempdir().unwrap();
    let left = dir.path().join("left.txt");
    let right = dir.path().join("right.txt");
    std::fs::write(&left, "a\nlonger\n").unwrap();
    std::fs::write(&right, "1\n2\n").unwrap();
    rat()
        .env("NO_COLOR", "1")
        .args(["join", "--gap", "1", "--file"])
        .arg(&left)
        .arg("--file")
        .arg(&right)
        .assert()
        .success()
        .stdout("a      1\nlonger 2\n");
}

#[test]
fn a_dash_file_reads_stdin() {
    let dir = tempfile::tempdir().unwrap();
    let right = dir.path().join("right.txt");
    std::fs::write(&right, "1\n2\n").unwrap();
    rat()
        .env("NO_COLOR", "1")
        .args(["join", "--gap", "1", "--file", "-", "--file"])
        .arg(&right)
        .write_stdin("a\nlonger\n")
        .assert()
        .success()
        .stdout("a      1\nlonger 2\n");
}

#[test]
fn bordered_panels_join_cleanly() {
    // Two boxes of unequal height keep their frames intact side by side.
    rat()
        .env("NO_COLOR", "1")
        .args(["join", "--gap", "1", "┌──┐\n│hi│\n└──┘", "┌─┐\n└─┘"])
        .assert()
        .success()
        .stdout("┌──┐ ┌─┐\n│hi│ └─┘\n└──┘\n");
}

#[test]
fn mixing_blocks_and_files_is_a_usage_error() {
    rat()
        .args(["join", "a", "--file", "b.txt"])
        .assert()
        .code(2);
}

#[test]
fn no_blocks_is_a_usage_error() {
    rat().arg("join").assert().code(2);
}

#[test]
fn an_align_value_from_the_other_direction_is_an_error() {
    rat()
        .args(["join", "--align", "center", "a", "b"])
        .assert()
        .code(1)
        .stderr(predicates::str::contains("align"));
}

#[test]
fn a_missing_file_is_an_error() {
    rat()
        .args(["join", "--file", "definitely-not-here.txt"])
        .assert()
        .code(1)
        .stderr(predicates::str::contains("definitely-not-here.txt"));
}

#[test]
fn fit_stacks_when_rat_width_is_too_narrow() {
    rat()
        .env("NO_COLOR", "1")
        .env("RAT_WIDTH", "8")
        .args(["join", "--fit", "--gap", "1", "aaaa", "bbbb"])
        .assert()
        .success()
        .stdout("aaaa\n\nbbbb\n");
}

#[test]
fn fit_keeps_blocks_beside_when_they_fit() {
    rat()
        .env("NO_COLOR", "1")
        .env("RAT_WIDTH", "20")
        .args(["join", "--fit", "--gap", "1", "aaaa", "bbbb"])
        .assert()
        .success()
        .stdout("aaaa bbbb\n");
}

#[test]
fn max_width_beats_the_env_and_implies_fit() {
    rat()
        .env("NO_COLOR", "1")
        .env("RAT_WIDTH", "80")
        .args(["join", "--max-width", "5", "aaaa", "bbbb"])
        .assert()
        .success()
        .stdout("aaaa\nbbbb\n");
}

#[test]
fn a_garbage_rat_width_is_ignored() {
    rat()
        .env("NO_COLOR", "1")
        .env("RAT_WIDTH", "not-a-number")
        .args(["join", "--fit", "--max-width", "40", "aa", "bb"])
        .assert()
        .success()
        .stdout("aabb\n");
}
