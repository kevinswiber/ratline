use assert_cmd::Command;

fn rat() -> Command {
    Command::cargo_bin("rat").expect("rat binary builds")
}

#[test]
fn formats_seconds_compact_by_default() {
    rat()
        .args(["duration", "5548"])
        .assert()
        .success()
        .stdout("1h 33m\n");
}

#[test]
fn format_long_and_clock() {
    rat()
        .args(["duration", "--format", "long", "5592"])
        .assert()
        .success()
        .stdout("1h 33m 12s\n");
    rat()
        .args(["duration", "--format", "clock", "5592"])
        .assert()
        .success()
        .stdout("01:33:12\n");
}

#[test]
fn seconds_flag_parses_duration_strings() {
    rat()
        .args(["duration", "--seconds", "1h33m"])
        .assert()
        .success()
        .stdout("5580\n");
}

#[test]
fn ms_flag_takes_milliseconds() {
    rat()
        .args(["duration", "--ms", "5548000"])
        .assert()
        .success()
        .stdout("1h 33m\n");
}

#[test]
fn garbage_is_an_error() {
    rat().args(["duration", "abc"]).assert().code(1);
}
