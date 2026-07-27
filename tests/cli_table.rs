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
        "RAT_APPEARANCE",
        "COLORFGBG",
    ] {
        cmd.env_remove(var);
    }
    cmd
}

#[test]
fn aligns_columns_from_stdin() {
    rat()
        .env("NO_COLOR", "1")
        .arg("table")
        .write_stdin("a\t1\nlonger\t22\n")
        .assert()
        .success()
        .stdout("a       1\nlonger  22\n");
}

#[test]
fn ansi_cells_measure_by_display_width() {
    rat()
        .env("TERM", "xterm-256color")
        .args(["--color", "always", "table"])
        .write_stdin("\x1b[31mab\x1b[0m\tx\nabcd\ty\n")
        .assert()
        .success()
        .stdout("\x1b[31mab\x1b[0m    x\nabcd  y\n");
}

#[test]
fn a_pinned_first_column_lines_up_with_a_bar_label_column() {
    let bar = rat()
        .env("NO_COLOR", "1")
        .args([
            "bar",
            "--label",
            "x",
            "--label-width",
            "27",
            "--width",
            "4",
            "--value",
            "1",
            "--total",
            "4",
            "--annotation",
            "none",
        ])
        .assert()
        .success();
    let bar_line = String::from_utf8_lossy(&bar.get_output().stdout).into_owned();
    let bar_offset = bar_line.find('█').unwrap();

    let table = rat()
        .env("NO_COLOR", "1")
        .args(["table", "--widths", "27", "--separator", " "])
        .write_stdin("x\tvalue\n")
        .assert()
        .success();
    let table_line = String::from_utf8_lossy(&table.get_output().stdout).into_owned();
    assert_eq!(bar_offset, table_line.find("value").unwrap());
}

#[test]
fn per_column_align_and_overflow_apply_by_position() {
    rat()
        .env("NO_COLOR", "1")
        .args(["table", "--widths", ",4", "--align", "l,r"])
        .write_stdin("ab\t7\nlonger\t1234567\n")
        .assert()
        .success()
        .stdout("ab         7\nlonger  123…\n");
}

#[test]
fn no_color_strips_cell_escapes() {
    rat()
        .env("NO_COLOR", "1")
        .arg("table")
        .write_stdin("\x1b[31mab\x1b[0m\tx\n")
        .assert()
        .success()
        .stdout("ab  x\n");
}

#[test]
fn empty_stdin_produces_no_output() {
    rat()
        .arg("table")
        .write_stdin("")
        .assert()
        .success()
        .stdout("");
}

#[test]
fn a_bad_align_spec_names_the_column() {
    rat()
        .args(["table", "--align", "l,nope"])
        .write_stdin("a\tb\n")
        .assert()
        .code(1)
        .stderr(predicates::str::contains("column 2"));
}
