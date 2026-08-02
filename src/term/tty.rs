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
            Ok(file) => {
                // Output modes are per screen buffer, which stdout shares,
                // so this also covers frames written there. Left enabled on
                // exit: a restore would race children inheriting the
                // buffer, and modern shells set the bit for themselves.
                #[cfg(windows)]
                let _ = enable_vt(&file);
                Inner::Tty(file)
            }
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
/// Ask the console to process VT sequences instead of printing them.
/// Windows Terminal always does; legacy conhost only with this bit set.
#[cfg(windows)]
fn enable_vt(file: &File) -> bool {
    use std::os::windows::io::AsRawHandle;

    use windows_sys::Win32::System::Console::{
        CONSOLE_MODE, ENABLE_VIRTUAL_TERMINAL_PROCESSING, GetConsoleMode, SetConsoleMode,
    };

    let handle = file.as_raw_handle();
    let mut mode: CONSOLE_MODE = 0;
    unsafe {
        if GetConsoleMode(handle, &mut mode) == 0 {
            return false;
        }
        if mode & ENABLE_VIRTUAL_TERMINAL_PROCESSING != 0 {
            return true;
        }
        SetConsoleMode(handle, mode | ENABLE_VIRTUAL_TERMINAL_PROCESSING) != 0
    }
}

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

/// UTF-8 console codepages (65001) for the guard's lifetime; the previous
/// codepages are restored on drop (including panic). Console codepage is
/// shared process-global state that children inherit: more.com decodes its
/// piped stdin with the console codepage, so under the default OEM codepage
/// it garbles the frame's block and spark glyphs. No-op on unix and when no
/// console is attached.
pub struct ConsoleUtf8Guard {
    #[cfg(windows)]
    saved: Option<(u32, u32)>,
}

impl ConsoleUtf8Guard {
    #[cfg(windows)]
    pub fn enable() -> Self {
        use windows_sys::Win32::System::Console::{
            GetConsoleCP, GetConsoleOutputCP, SetConsoleCP, SetConsoleOutputCP,
        };

        const CP_UTF8: u32 = 65001;
        let saved = unsafe {
            let input = GetConsoleCP();
            let output = GetConsoleOutputCP();
            // Zero means no console is attached.
            if input == 0 || output == 0 {
                None
            } else {
                SetConsoleCP(CP_UTF8);
                SetConsoleOutputCP(CP_UTF8);
                Some((input, output))
            }
        };
        ConsoleUtf8Guard { saved }
    }

    #[cfg(not(windows))]
    pub fn enable() -> Self {
        ConsoleUtf8Guard {}
    }
}

#[cfg(windows)]
impl Drop for ConsoleUtf8Guard {
    fn drop(&mut self) {
        use windows_sys::Win32::System::Console::{SetConsoleCP, SetConsoleOutputCP};

        if let Some((input, output)) = self.saved {
            unsafe {
                SetConsoleCP(input);
                SetConsoleOutputCP(output);
            }
        }
    }
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

/// Enter the alternate screen on construction, leave it on Drop —
/// beside [`RawModeGuard`], so every exit path that already restores
/// the terminal through guards restores this too (declare it AFTER the
/// raw guard: reverse drop order then leaves the alternate screen
/// before raw mode is disabled). `?1049` is write-only, so the
/// renderer's no-queries rule is untouched; the bytes go to stdout,
/// the frames' own stream. SIGKILL cannot unwind — `reset` recovers.
pub struct AltScreenGuard;

impl AltScreenGuard {
    pub fn enter() -> std::io::Result<Self> {
        use std::io::Write;
        let mut out = std::io::stdout();
        out.write_all(b"\x1b[?1049h")?;
        out.flush()?;
        Ok(AltScreenGuard)
    }
}

impl Drop for AltScreenGuard {
    fn drop(&mut self) {
        use std::io::Write;
        let mut out = std::io::stdout();
        let _ = out.write_all(b"\x1b[?1049l");
        let _ = out.flush();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[cfg(windows)]
    #[test]
    fn vt_enable_declines_a_non_console_handle() {
        // A plain file is not a console; the helper must say no without
        // panicking (this is the CI path, where no console exists at all).
        let file = tempfile::tempfile().expect("temp file opens");
        assert!(!enable_vt(&file));
    }

    #[test]
    fn open_never_panics_and_size_is_positive() {
        let stream = UiStream::open();
        let (w, h) = stream.size();
        assert!(w > 0 && h > 0);
    }

    #[test]
    fn console_utf8_guard_constructs_everywhere() {
        let _guard = ConsoleUtf8Guard::enable();
    }

    #[cfg(windows)]
    #[test]
    fn console_utf8_guard_sets_and_restores_codepages() {
        use windows_sys::Win32::System::Console::{GetConsoleCP, GetConsoleOutputCP};

        let before_in = unsafe { GetConsoleCP() };
        let before_out = unsafe { GetConsoleOutputCP() };
        {
            let _guard = ConsoleUtf8Guard::enable();
            // Without a console both codepages read 0 and the guard must
            // leave them alone.
            if before_in != 0 && before_out != 0 {
                assert_eq!(unsafe { GetConsoleCP() }, 65001);
                assert_eq!(unsafe { GetConsoleOutputCP() }, 65001);
            }
        }
        assert_eq!(unsafe { GetConsoleCP() }, before_in);
        assert_eq!(unsafe { GetConsoleOutputCP() }, before_out);
    }

    #[test]
    fn a_regular_file_is_not_a_tty() {
        let file = tempfile::tempfile().expect("tempfile");
        assert!(!file.is_tty());
    }
}
