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
    /// Open /dev/tty read-write; fall back to stderr. Unlike gum (which is
    /// stderr-only), the UI survives `2>/dev/null`.
    pub fn open() -> Self {
        let inner = match OpenOptions::new().read(true).write(true).open("/dev/tty") {
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

    // Consumed by frame's state keying and doctor's report, which land next.
    #[allow(dead_code)]
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
            Inner::Tty(file) => file.write(buf),
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
