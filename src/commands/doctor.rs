use std::io::{Read, Write};
use std::time::{Duration, Instant};

use anyhow::Context;

use crate::cli::DoctorArgs;
use crate::color::{ColorProfile, SystemEnv, detect_profile};
use crate::exit::AppResult;
use crate::term::tty::UiStream;

/// Terminal support for synchronized output (mode 2026), per DECRPM.
#[derive(Copy, Clone, PartialEq, Eq, Debug)]
pub enum SyncSupport {
    Unsupported,
    Set,
    Reset,
    PermanentlySet,
    PermanentlyReset,
    NoReply,
}

impl SyncSupport {
    pub fn supported(self) -> bool {
        matches!(
            self,
            SyncSupport::Set | SyncSupport::Reset | SyncSupport::PermanentlySet
        )
    }

    fn describe(self) -> &'static str {
        match self {
            SyncSupport::Unsupported => "unsupported (mode not recognized)",
            SyncSupport::Set => "supported (currently set)",
            SyncSupport::Reset => "supported",
            SyncSupport::PermanentlySet => "supported (permanently set)",
            SyncSupport::PermanentlyReset => "unsupported (permanently reset)",
            SyncSupport::NoReply => "unknown (no DECRQM reply)",
        }
    }
}

/// Find a `CSI ? 2026 ; Ps $ y` reply anywhere in the byte stream.
pub fn parse_decrpm(bytes: &[u8]) -> SyncSupport {
    let needle = b"\x1b[?2026;";
    let Some(pos) = bytes
        .windows(needle.len())
        .position(|window| window == needle)
    else {
        return SyncSupport::NoReply;
    };
    let rest = &bytes[pos + needle.len()..];
    let Some((ps, tail)) = rest.split_first() else {
        return SyncSupport::NoReply;
    };
    if tail.len() < 2 || &tail[..2] != b"$y" {
        return SyncSupport::NoReply;
    }
    match ps {
        b'0' => SyncSupport::Unsupported,
        b'1' => SyncSupport::Set,
        b'2' => SyncSupport::Reset,
        b'3' => SyncSupport::PermanentlySet,
        b'4' => SyncSupport::PermanentlyReset,
        _ => SyncSupport::NoReply,
    }
}

/// Ask the terminal about mode 2026 and wait briefly for the reply. The
/// reader thread may outlive the wait; the process exits right after.
fn probe_sync_support(ui: &mut UiStream) -> SyncSupport {
    if !ui.is_dev_tty() || !ui.is_tty() {
        return SyncSupport::NoReply;
    }
    let Ok(reader) = std::fs::File::open("/dev/tty") else {
        return SyncSupport::NoReply;
    };
    if crossterm::terminal::enable_raw_mode().is_err() {
        return SyncSupport::NoReply;
    }
    let result = (|| {
        ui.write_all(b"\x1b[?2026$p").ok()?;
        ui.flush().ok()?;
        let (tx, rx) = std::sync::mpsc::channel::<u8>();
        std::thread::spawn(move || {
            let mut reader = reader;
            let mut byte = [0u8; 1];
            while let Ok(1) = reader.read(&mut byte) {
                if tx.send(byte[0]).is_err() {
                    break;
                }
            }
        });
        let mut collected = Vec::new();
        let deadline = Instant::now() + Duration::from_millis(250);
        loop {
            let now = Instant::now();
            if now >= deadline {
                break;
            }
            match rx.recv_timeout(deadline - now) {
                Ok(byte) => {
                    collected.push(byte);
                    if collected.ends_with(b"$y") {
                        break;
                    }
                }
                Err(_) => break,
            }
        }
        Some(parse_decrpm(&collected))
    })();
    let _ = crossterm::terminal::disable_raw_mode();
    result.unwrap_or(SyncSupport::NoReply)
}

pub fn run(args: DoctorArgs, profile: ColorProfile) -> AppResult {
    let mut ui = UiStream::open();
    let is_tty = ui.is_tty();
    let is_dev_tty = ui.is_dev_tty();
    let (cols, rows) = ui.size();
    let detected = detect_profile(&SystemEnv, is_tty);
    let reason = profile_reason(is_tty);
    let sync = probe_sync_support(&mut ui);

    let stream = if is_dev_tty {
        "/dev/tty"
    } else {
        "stderr (fallback)"
    };
    if args.json {
        println!(
            concat!(
                "{{\"ui_stream\":\"{}\",\"is_tty\":{},\"cols\":{},\"rows\":{},",
                "\"detected_profile\":\"{:?}\",\"active_profile\":\"{:?}\",",
                "\"profile_reason\":\"{}\",\"sync_output\":\"{:?}\",",
                "\"sync_supported\":{}}}"
            ),
            stream,
            is_tty,
            cols,
            rows,
            detected,
            profile,
            reason,
            sync,
            sync.supported(),
        );
    } else {
        let mut out = std::io::stdout().lock();
        writeln!(out, "UI stream:        {stream}").context("writing")?;
        writeln!(out, "Is a tty:         {is_tty}").context("writing")?;
        writeln!(out, "Size:             {cols}x{rows}").context("writing")?;
        writeln!(out, "Detected profile: {detected:?} ({reason})").context("writing")?;
        writeln!(out, "Active profile:   {profile:?}").context("writing")?;
        writeln!(out, "Synchronized out: {}", sync.describe()).context("writing")?;
    }
    Ok(())
}

/// A short human account of what decided the profile.
fn profile_reason(is_tty: bool) -> String {
    let get = |k: &str| std::env::var(k).ok();
    if get("NO_COLOR").is_some_and(|v| !v.is_empty()) {
        return "NO_COLOR is set".into();
    }
    if get("CLICOLOR").is_some_and(|v| v == "0") {
        return "CLICOLOR=0".into();
    }
    if get("CI").is_some_and(|v| !v.is_empty()) {
        return "CI is set".into();
    }
    if !is_tty {
        return "UI stream is not a tty".into();
    }
    if let Some(ct) = get("COLORTERM") {
        return format!("COLORTERM={ct}");
    }
    match get("TERM") {
        Some(term) => format!("TERM={term}"),
        None => "TERM is unset".into(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_every_decrpm_status() {
        assert_eq!(parse_decrpm(b"\x1b[?2026;0$y"), SyncSupport::Unsupported);
        assert_eq!(parse_decrpm(b"\x1b[?2026;1$y"), SyncSupport::Set);
        assert_eq!(parse_decrpm(b"\x1b[?2026;2$y"), SyncSupport::Reset);
        assert_eq!(parse_decrpm(b"\x1b[?2026;3$y"), SyncSupport::PermanentlySet);
        assert_eq!(
            parse_decrpm(b"\x1b[?2026;4$y"),
            SyncSupport::PermanentlyReset
        );
    }

    #[test]
    fn garbage_and_truncated_replies_are_unknown() {
        assert_eq!(parse_decrpm(b""), SyncSupport::NoReply);
        assert_eq!(parse_decrpm(b"garbage"), SyncSupport::NoReply);
        assert_eq!(parse_decrpm(b"\x1b[?2026;1"), SyncSupport::NoReply);
        assert_eq!(parse_decrpm(b"\x1b[?9999;1$y"), SyncSupport::NoReply);
        assert_eq!(parse_decrpm(b"\x1b[?2026;9$y"), SyncSupport::NoReply);
    }

    #[test]
    fn reply_embedded_in_other_bytes_is_found() {
        assert_eq!(parse_decrpm(b"noise\x1b[?2026;2$ymore"), SyncSupport::Reset);
    }

    #[test]
    fn support_is_reported_for_set_and_reset() {
        assert!(SyncSupport::Set.supported());
        assert!(SyncSupport::Reset.supported());
        assert!(SyncSupport::PermanentlySet.supported());
        assert!(!SyncSupport::Unsupported.supported());
        assert!(!SyncSupport::PermanentlyReset.supported());
        assert!(!SyncSupport::NoReply.supported());
    }
}
