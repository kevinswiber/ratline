mod common;

use common::rat;

#[test]
fn json_reports_the_default_appearance_under_piped_stdio() {
    // assert_cmd pipes all three stdio streams, so stderr is not a terminal
    // and the query is never emitted. No verdict reaches the palette, which
    // is what "default" records.
    rat()
        .args(["doctor", "--json"])
        .assert()
        .success()
        .stdout(predicates::str::contains("\"appearance\":\"dark\""))
        .stdout(predicates::str::contains(
            "\"appearance_source\":\"default\"",
        ));
}

#[test]
fn an_explicit_appearance_is_reported_as_explicit() {
    rat()
        .args(["--appearance", "light", "doctor", "--json"])
        .assert()
        .success()
        .stdout(predicates::str::contains("\"appearance\":\"light\""))
        .stdout(predicates::str::contains(
            "\"appearance_source\":\"explicit\"",
        ));
}

#[test]
fn a_requested_appearance_reports_the_same_way_however_it_arrived() {
    // The flag and the environment variable are one argument, so both
    // report `explicit` and cannot be distinguished.
    rat()
        .env("RAT_APPEARANCE", "dark")
        .args(["doctor", "--json"])
        .assert()
        .success()
        .stdout(predicates::str::contains("\"appearance\":\"dark\""))
        .stdout(predicates::str::contains(
            "\"appearance_source\":\"explicit\"",
        ));
}

#[test]
fn the_text_report_names_the_appearance_and_its_source() {
    rat()
        .args(["--appearance", "light", "doctor"])
        .assert()
        .success()
        .stdout(predicates::str::contains("Appearance:"))
        .stdout(predicates::str::contains("light (explicit)"));
}
