//! Mouse reporting: DEC private modes 1000 (press/release — the
//! minimum that reports the wheel) and 1006 (the SGR encoding, whose
//! `M`/`m` finals frame correctly under the existing scanner). Written
//! by hand rather than through crossterm's `EnableMouseCapture`, which
//! also sets 1002 drag motion and the 1015 encoding nobody wants.
//! 1002 stays off (drag motion is noise here) and 1007 does not apply
//! (alternate-scroll only operates on the alternate screen).

use std::io::Write;

#[cfg(unix)]
const ENABLE: &[u8] = b"\x1b[?1000h\x1b[?1006h";
#[cfg(unix)]
const DISABLE: &[u8] = b"\x1b[?1006l\x1b[?1000l";

/// Owns mouse capture: enables on construction, disables on drop,
/// best-effort. `suspend`/`resume` bracket a foreign reader (the
/// pager) and back the `m` hand-the-mouse-back toggle; both are
/// idempotent so callers never track state themselves. On unix the
/// modes are DEC sequences on the frames' own stream; on Windows the
/// pump is crossterm's and capture is a console mode
/// (`ENABLE_MOUSE_INPUT`, which also clears quick-edit — the same
/// selection hijack by a different mechanism), so the guard goes
/// through crossterm's capture commands there.
pub struct MouseGuard<W: Write> {
    out: W,
    active: bool,
}

impl<W: Write> MouseGuard<W> {
    fn write_enable(out: &mut W) -> std::io::Result<()> {
        #[cfg(unix)]
        {
            out.write_all(ENABLE)?;
            out.flush()
        }
        #[cfg(windows)]
        {
            crossterm::execute!(out, crossterm::event::EnableMouseCapture)
        }
    }

    fn write_disable(out: &mut W) -> std::io::Result<()> {
        #[cfg(unix)]
        {
            out.write_all(DISABLE)?;
            out.flush()
        }
        #[cfg(windows)]
        {
            crossterm::execute!(out, crossterm::event::DisableMouseCapture)
        }
    }

    pub fn enable(mut out: W) -> std::io::Result<Self> {
        Self::write_enable(&mut out)?;
        Ok(MouseGuard { out, active: true })
    }

    pub fn active(&self) -> bool {
        self.active
    }

    /// Idempotent: suspending an already-suspended guard writes nothing.
    pub fn suspend(&mut self) -> std::io::Result<()> {
        if !self.active {
            return Ok(());
        }
        Self::write_disable(&mut self.out)?;
        self.active = false;
        Ok(())
    }

    /// Idempotent: resuming an already-active guard writes nothing.
    pub fn resume(&mut self) -> std::io::Result<()> {
        if self.active {
            return Ok(());
        }
        Self::write_enable(&mut self.out)?;
        self.active = true;
        Ok(())
    }
}

impl<W: Write> Drop for MouseGuard<W> {
    fn drop(&mut self) {
        let _ = self.suspend();
    }
}

// The byte-shape expectations are the unix wire form; on Windows the
// guard goes through crossterm's console-mode commands, which have no
// byte contract to pin against a plain buffer.
#[cfg(all(test, unix))]
mod tests {
    use super::*;

    #[test]
    fn enable_writes_1000_then_the_sgr_encoding() {
        let mut out = Vec::new();
        let guard = MouseGuard::enable(&mut out);
        drop(guard);
        assert_eq!(out, b"\x1b[?1000h\x1b[?1006h\x1b[?1006l\x1b[?1000l");
    }

    #[test]
    fn suspend_and_resume_are_idempotent() {
        let mut out = Vec::new();
        {
            let mut guard = MouseGuard::enable(&mut out).expect("enable");
            guard.suspend().expect("suspend");
            guard.suspend().expect("second suspend is a no-op");
            guard.resume().expect("resume");
            guard.resume().expect("second resume is a no-op");
        }
        // enable, disable, enable, and the drop's final disable.
        let joined: &[u8] =
            b"\x1b[?1000h\x1b[?1006h\x1b[?1006l\x1b[?1000l\x1b[?1000h\x1b[?1006h\x1b[?1006l\x1b[?1000l";
        assert_eq!(out, joined);
    }

    #[test]
    fn dropping_a_suspended_guard_writes_nothing_more() {
        let mut out = Vec::new();
        {
            let mut guard = MouseGuard::enable(&mut out).expect("enable");
            guard.suspend().expect("suspend");
        }
        assert_eq!(out, b"\x1b[?1000h\x1b[?1006h\x1b[?1006l\x1b[?1000l");
    }
}
