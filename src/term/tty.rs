use std::fs::{File, OpenOptions};
use std::io::{Stderr, Write};

use crossterm::tty::IsTty;

/// Where UI bytes go: the controlling terminal when available, stderr
/// otherwise. Never stdout — stdout carries results only.
pub struct UiStream {
    inner: Inner,
}

enum Inner {
    Tty(File),
    Stderr(Stderr),
}

impl UiStream {
    /// Open the controlling terminal read-write (/dev/tty on unix, CONOUT$
    /// on Windows); fall back to stderr. Unlike gum (which is stderr-only),
    /// the UI survives `2>/dev/null`.
    pub fn open() -> Self {
        #[cfg(unix)]
        const CONSOLE: &str = "/dev/tty";
        #[cfg(windows)]
        const CONSOLE: &str = "CONOUT$";
        let inner = match OpenOptions::new().read(true).write(true).open(CONSOLE) {
            Ok(file) => Inner::Tty(file),
            Err(_) => Inner::Stderr(std::io::stderr()),
        };
        UiStream { inner }
    }

    pub fn is_tty(&self) -> bool {
        match &self.inner {
            Inner::Tty(file) => file.is_tty(),
            Inner::Stderr(err) => err.is_tty(),
        }
    }

    /// True when the stream is the console device rather than the stderr
    /// fallback.
    pub fn is_dev_tty(&self) -> bool {
        matches!(self.inner, Inner::Tty(_))
    }

    /// Terminal size with a conventional fallback for headless runs.
    #[allow(dead_code)]
    pub fn size(&self) -> (u16, u16) {
        crossterm::terminal::size().unwrap_or((80, 24))
    }
}

impl Write for UiStream {
    fn write(&mut self, buf: &[u8]) -> std::io::Result<usize> {
        match &mut self.inner {
            Inner::Tty(file) => {
                // Raw writes to CONOUT$ are decoded with the legacy console
                // codepage, garbling UTF-8 glyphs; go through WriteConsoleW
                // like std does for its own console handles.
                #[cfg(windows)]
                {
                    if let Some(n) = write_console_utf16(file, buf) {
                        return Ok(n);
                    }
                }
                file.write(buf)
            }
            Inner::Stderr(err) => err.write(buf),
        }
    }

    fn flush(&mut self) -> std::io::Result<()> {
        match &mut self.inner {
            Inner::Tty(file) => file.flush(),
            Inner::Stderr(err) => err.flush(),
        }
    }
}

/// Write UTF-8 bytes to a console handle as UTF-16. Returns None when the
/// handle rejects console writes (then the caller falls back to raw bytes).
#[cfg(windows)]
fn write_console_utf16(file: &File, buf: &[u8]) -> Option<usize> {
    use std::os::windows::io::AsRawHandle;

    use windows_sys::Win32::System::Console::WriteConsoleW;

    let text = String::from_utf8_lossy(buf);
    let wide: Vec<u16> = text.encode_utf16().collect();
    let mut offset = 0usize;
    while offset < wide.len() {
        let mut written: u32 = 0;
        let ok = unsafe {
            WriteConsoleW(
                file.as_raw_handle(),
                wide[offset..].as_ptr().cast(),
                (wide.len() - offset) as u32,
                &mut written,
                std::ptr::null(),
            )
        };
        if ok == 0 {
            return None;
        }
        offset += written as usize;
        if written == 0 {
            return None;
        }
    }
    Some(buf.len())
}

/// Raw mode for the guard's lifetime; restored on drop (including panic).
pub struct RawModeGuard;

impl RawModeGuard {
    pub fn enable() -> std::io::Result<Self> {
        crossterm::terminal::enable_raw_mode()?;
        Ok(RawModeGuard)
    }
}

impl Drop for RawModeGuard {
    fn drop(&mut self) {
        let _ = crossterm::terminal::disable_raw_mode();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn open_never_panics_and_size_is_positive() {
        let stream = UiStream::open();
        let (w, h) = stream.size();
        assert!(w > 0 && h > 0);
    }

    #[test]
    fn a_regular_file_is_not_a_tty() {
        let file = tempfile::tempfile().expect("tempfile");
        assert!(!file.is_tty());
    }
}
