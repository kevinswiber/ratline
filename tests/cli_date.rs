use assert_cmd::Command;

fn rat() -> Command {
    let mut cmd = Command::cargo_bin("rat").expect("rat binary builds");
    cmd.env("TZ", "UTC");
    cmd
}

#[test]
fn epoch_replaces_bsd_date_parse() {
    rat()
        .args(["date", "--epoch", "2026-07-26T12:00:00Z"])
        .assert()
        .success()
        .stdout("1785067200\n");
}

#[test]
fn format_replaces_bsd_date_r() {
    rat()
        .args(["date", "--format", "%l:%M %p", "--utc", "1785067200"])
        .assert()
        .success()
        .stdout("12:00 PM\n");
}

#[test]
fn since_computes_elapsed_seconds() {
    rat()
        .args(["date", "--since", "1785067200", "1785067260"])
        .assert()
        .success()
        .stdout("60\n");
}

#[test]
fn until_computes_remaining_seconds() {
    rat()
        .args(["date", "--until", "1785067260", "1785067200"])
        .assert()
        .success()
        .stdout("60\n");
}

#[test]
fn relative_phrases_past_times() {
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_secs();
    // 290 leaves ten seconds of slack before ceil rounding tips to "6m".
    rat()
        .args(["date", "--relative", &(now - 290).to_string()])
        .assert()
        .success()
        .stdout("5m ago\n");
}

#[test]
fn garbage_is_an_error() {
    rat().args(["date", "--epoch", "garbage"]).assert().code(1);
}
