use std::io::Write;
use std::path::{Path, PathBuf};

use crate::core::measure::strip_escapes;

pub const SNAPSHOT_ATTEMPTS: usize = 100;

/// `rat-watch-{stamp}.txt`; collision attempts append `-2`, `-3`, …
/// (attempt 0 is bare, attempt 1 is `-2`).
pub fn snapshot_file_name(stamp: &str, attempt: usize) -> String {
    if attempt == 0 {
        format!("rat-watch-{stamp}.txt")
    } else {
        format!("rat-watch-{stamp}-{}.txt", attempt + 1)
    }
}

/// `%Y%m%d-%H%M%S` of an already-zoned instant (the caller picks the zone).
pub fn snapshot_stamp(zoned: &jiff::Zoned) -> String {
    jiff::fmt::strtime::format("%Y%m%d-%H%M%S", zoned)
        .expect("a fixed strftime spec over a zoned instant cannot fail")
}

/// One frame line per line plus a trailing newline, escapes stripped
/// unless `keep_ansi`. Tabs and carriage returns survive either way —
/// `strip_escapes` drops only escape sequences.
pub fn snapshot_body(lines: &[String], keep_ansi: bool) -> String {
    let mut body = String::new();
    for line in lines {
        if keep_ansi {
            body.push_str(line);
        } else {
            body.push_str(&strip_escapes(line));
        }
        body.push('\n');
    }
    body
}

/// Claim the first free collision-suffixed name under `dir` and write
/// `body` to it. `create_new` checks and claims in one syscall, so two
/// writers in the same second get distinct files.
pub fn write_snapshot(dir: &Path, stamp: &str, body: &str) -> std::io::Result<PathBuf> {
    for attempt in 0..SNAPSHOT_ATTEMPTS {
        let path = dir.join(snapshot_file_name(stamp, attempt));
        match std::fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&path)
        {
            Ok(mut file) => {
                file.write_all(body.as_bytes())?;
                return Ok(path);
            }
            Err(err) if err.kind() == std::io::ErrorKind::AlreadyExists => continue,
            Err(err) => return Err(err),
        }
    }
    Err(std::io::Error::other(format!(
        "no free snapshot name for {stamp} after {SNAPSHOT_ATTEMPTS} attempts"
    )))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_first_name_has_no_suffix() {
        assert_eq!(
            snapshot_file_name("20260727-134502", 0),
            "rat-watch-20260727-134502.txt"
        );
        assert_eq!(
            snapshot_file_name("20260727-134502", 1),
            "rat-watch-20260727-134502-2.txt"
        );
        assert_eq!(
            snapshot_file_name("20260727-134502", 8),
            "rat-watch-20260727-134502-9.txt"
        );
    }

    #[test]
    fn the_stamp_is_compact_local_time() {
        let zoned = jiff::civil::date(2026, 7, 27)
            .at(13, 45, 2, 0)
            .to_zoned(jiff::tz::TimeZone::fixed(jiff::tz::offset(-4)))
            .unwrap();
        assert_eq!(snapshot_stamp(&zoned), "20260727-134502");
    }

    #[test]
    fn the_body_strips_escapes_by_default() {
        let styled = vec!["\x1b[38;5;212mhi\x1b[0m".to_string()];
        assert_eq!(snapshot_body(&styled, false), "hi\n");
        assert_eq!(snapshot_body(&styled, true), "\x1b[38;5;212mhi\x1b[0m\n");
        let tabbed = vec!["a\tb".to_string()];
        assert_eq!(snapshot_body(&tabbed, false), "a\tb\n");
        assert_eq!(snapshot_body(&tabbed, true), "a\tb\n");
    }

    #[test]
    fn a_second_write_in_the_same_second_does_not_overwrite() {
        let dir = tempfile::tempdir().unwrap();
        let first = write_snapshot(dir.path(), "20260727-134502", "one\n").unwrap();
        let second = write_snapshot(dir.path(), "20260727-134502", "two\n").unwrap();
        assert_ne!(first, second);
        assert_eq!(std::fs::read_to_string(&first).unwrap(), "one\n");
        assert_eq!(std::fs::read_to_string(&second).unwrap(), "two\n");
    }

    #[test]
    fn an_exhausted_name_space_is_an_error() {
        let dir = tempfile::tempdir().unwrap();
        for attempt in 0..SNAPSHOT_ATTEMPTS {
            let name = snapshot_file_name("20260727-134502", attempt);
            std::fs::write(dir.path().join(name), "taken").unwrap();
        }
        assert!(write_snapshot(dir.path(), "20260727-134502", "body\n").is_err());
    }
}
