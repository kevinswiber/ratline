//! Reassembles decoded input events from raw terminal bytes for a
//! long-running command that owns its terminal's input. The scanner never
//! retains an unrecognized run indefinitely: a complete escape sequence it
//! does not understand is dropped, and an accumulating one is bounded.

use crate::term::theme_notify::{OscColorKind, parse_color_scheme_report, parse_osc_color_reply};
use crate::theme::Appearance;
use crate::ui::key::Key;

/// One decoded unit from the input stream.
#[derive(Clone, PartialEq, Debug)]
pub enum TapEvent {
    /// A decoded key, from `crate::ui::key`.
    Key(Key),
    /// A parsed DSR 997 push.
    ThemeNotification(Appearance),
    /// A parsed OSC 10/11 reply.
    OscColor(OscColorKind, xterm_color::Color),
}

/// Bytes accumulated for an escape sequence in progress may not grow past
/// this many without terminating; beyond it the run is discarded wholesale
/// rather than retained forever. One shared cap for both CSI and OSC runs.
const MAX_ESCAPE_LEN: usize = 128;

/// Input silence after a bare ESC before it resolves to `Key::Esc`. Real
/// terminals write a whole sequence in one write(2), so only a genuine
/// escape keypress leaves a lone ESC pending this long. The cost: Esc has
/// a ~50 ms floor, and a sequence split across a longer gap resolves as a
/// spurious Esc — benign for a resume key.
pub const ESC_HOLD: std::time::Duration = std::time::Duration::from_millis(50);

/// Reassembles `TapEvent`s from arbitrary-boundary byte chunks. A
/// complete, unrecognized escape-led run is dropped silently and never
/// retained — this is the property that keeps a long-lived reader from
/// wedging on an unknown private CSI.
pub struct TapScanner {
    buf: Vec<u8>,
    silent: std::time::Duration,
}

impl TapScanner {
    pub fn new() -> TapScanner {
        TapScanner {
            buf: Vec::new(),
            silent: std::time::Duration::ZERO,
        }
    }

    pub fn feed(&mut self, chunk: &[u8]) -> Vec<TapEvent> {
        self.silent = std::time::Duration::ZERO;
        let mut events = Vec::new();
        for &byte in chunk {
            if self.buf.is_empty() {
                if byte == 0x1b {
                    self.buf.push(byte);
                } else if let Some(key) = decode_key(byte) {
                    events.push(TapEvent::Key(key));
                }
                continue;
            }

            self.buf.push(byte);

            if self.buf.len() == 2 {
                if !matches!(self.buf[1], b'[' | b']' | b'O') {
                    // Not a recognized introducer: the leading ESC was not
                    // the start of a sequence this scanner understands.
                    // Drop it and reprocess this byte as an ordinary one.
                    self.buf.clear();
                    if let Some(key) = decode_key(byte) {
                        events.push(TapEvent::Key(key));
                    }
                }
                continue;
            }

            if self.buf.len() > MAX_ESCAPE_LEN {
                self.buf.clear();
                continue;
            }

            if self.buf[1] == b'[' {
                // A CSI run is complete at an ECMA-48 final byte; only the
                // semantic interpretation is report-specific. The report
                // parser is offered the run first — a DSR 997 report can
                // never be decoded as a key.
                if (0x40..=0x7e).contains(&byte) {
                    if let Some(appearance) = parse_color_scheme_report(&self.buf) {
                        events.push(TapEvent::ThemeNotification(appearance));
                    } else if let Some(key) = decode_csi(&self.buf) {
                        events.push(TapEvent::Key(key));
                    }
                    self.buf.clear();
                }
            } else if self.buf[1] == b'O' {
                // An SS3 run is complete at its third byte, the final.
                if let Some(key) = decode_ss3(byte) {
                    events.push(TapEvent::Key(key));
                }
                self.buf.clear();
            } else {
                // The len == 2 branch above only lets `[` or `]` continue.
                debug_assert_eq!(self.buf[1], b']');
                if self.buf.ends_with(b"\x07") || self.buf.ends_with(b"\x1b\\") {
                    if let Some((kind, color)) = parse_osc_color_reply(&self.buf) {
                        events.push(TapEvent::OscColor(kind, color));
                    }
                    self.buf.clear();
                }
            }
        }
        events
    }

    /// Account an expired, empty read slice. A bare ESC pending across
    /// `ESC_HOLD` of accumulated silence resolves to `Key::Esc`; anything
    /// longer in the buffer is a reassembling sequence and is never
    /// flushed by silence.
    pub fn idle(&mut self, silence: std::time::Duration) -> Vec<TapEvent> {
        if self.buf != [0x1b] {
            return Vec::new();
        }
        self.silent += silence;
        if self.silent < ESC_HOLD {
            return Vec::new();
        }
        self.buf.clear();
        self.silent = std::time::Duration::ZERO;
        vec![TapEvent::Key(Key::Esc)]
    }
}

impl Default for TapScanner {
    fn default() -> Self {
        TapScanner::new()
    }
}

/// 0x03 → CtrlC; b'\r' | b'\n' → Enter; printable ASCII (0x20..=0x7e) →
/// Char; everything else → None. What a key *means* is the consumer's
/// business — the scanner only decodes.
pub fn decode_key(byte: u8) -> Option<Key> {
    match byte {
        0x03 => Some(Key::CtrlC),
        b'\r' | b'\n' => Some(Key::Enter),
        0x20..=0x7e => Some(Key::Char(byte as char)),
        _ => None,
    }
}

/// Exact matches only: ESC [ A/B/C/D, ESC [ H/F, ESC [ 1~/4~/7~/8~,
/// ESC [ 5~/6~. A private or parameterized run is never a key.
pub fn decode_csi(seq: &[u8]) -> Option<Key> {
    match seq {
        b"\x1b[A" => Some(Key::Up),
        b"\x1b[B" => Some(Key::Down),
        b"\x1b[C" => Some(Key::Right),
        b"\x1b[D" => Some(Key::Left),
        b"\x1b[H" | b"\x1b[1~" | b"\x1b[7~" => Some(Key::Home),
        b"\x1b[F" | b"\x1b[4~" | b"\x1b[8~" => Some(Key::End),
        b"\x1b[5~" => Some(Key::PageUp),
        b"\x1b[6~" => Some(Key::PageDown),
        _ => None,
    }
}

/// Complete SS3 run (ESC O <final>): application-cursor arrows + Home/End;
/// function-key finals are None.
pub fn decode_ss3(final_byte: u8) -> Option<Key> {
    match final_byte {
        b'A' => Some(Key::Up),
        b'B' => Some(Key::Down),
        b'C' => Some(Key::Right),
        b'D' => Some(Key::Left),
        b'H' => Some(Key::Home),
        b'F' => Some(Key::End),
        _ => None,
    }
}

/// How long the reader waits for input before re-checking its control
/// flags. Short enough that a pause or a shutdown is observed promptly,
/// long enough that an idle terminal costs nothing.
#[cfg(unix)]
const READ_SLICE: std::time::Duration = std::time::Duration::from_millis(50);

/// Bounded wait for the reader to confirm it has parked: two slices plus
/// slack. Past that the reader is unresponsive and the caller must not
/// hand the terminal to a foreign reader.
#[cfg(unix)]
const PARK_ACK_TIMEOUT: std::time::Duration = std::time::Duration::from_millis(150);

#[cfg(unix)]
#[derive(Default)]
struct TapControl {
    pause: std::sync::atomic::AtomicBool,
    parked: std::sync::atomic::AtomicBool,
    shutdown: std::sync::atomic::AtomicBool,
}

/// One message on the tap's channel. Wrapping the raw bytes in an
/// envelope lets a trigger reader wake the receiver early through the
/// same channel — the wake carries no data (the fired flag is the
/// source of truth), it exists so `recv_timeout` returns now instead of
/// at the slice's end.
#[cfg(unix)]
#[derive(Clone, PartialEq, Debug)]
pub enum TapChunk {
    /// Raw bytes from the terminal device.
    Tty(Vec<u8>),
    /// A trigger reader's wake.
    // The trigger reader lands in the next commit; the allow goes with it.
    #[allow(dead_code)]
    Trigger,
}

/// A private reader for the terminal's input. Long-running commands use it
/// instead of an event library's pump so that escape sequences the terminal
/// sends on its own initiative are parsed by the component that owns the
/// input stream — and so that exactly one reader is attached to the
/// terminal at any instant.
#[cfg(unix)]
pub struct TtyTap {
    rx: std::sync::mpsc::Receiver<TapChunk>,
    /// A handle for foreign wakers (`sender()`); the terminal reader
    /// holds its own clone.
    // The trigger reader lands in the next commit; the allow goes with it.
    #[allow(dead_code)]
    tx: std::sync::mpsc::Sender<TapChunk>,
    control: std::sync::Arc<TapControl>,
    reader: Option<std::thread::JoinHandle<()>>,
}

#[cfg(unix)]
impl TtyTap {
    /// Open the terminal device and start reading. Fails when there is no
    /// controlling terminal; the caller keeps whatever input path it had.
    pub fn spawn() -> std::io::Result<TtyTap> {
        let tty = std::fs::File::open("/dev/tty")?;
        let (tx, rx) = std::sync::mpsc::channel();
        let control = std::sync::Arc::new(TapControl::default());
        let reader_control = std::sync::Arc::clone(&control);
        let reader_tx = tx.clone();
        let reader = std::thread::Builder::new()
            .name("rat-tty-tap".to_string())
            .spawn(move || read_loop(&tty, &reader_tx, &reader_control))?;
        Ok(TtyTap {
            rx,
            tx,
            control,
            reader: Some(reader),
        })
    }

    /// A sender foreign wakers may post `TapChunk::Trigger` through.
    // The trigger reader lands in the next commit; the allow goes with it.
    #[allow(dead_code)]
    pub fn sender(&self) -> std::sync::mpsc::Sender<TapChunk> {
        self.tx.clone()
    }

    /// The next chunk of input, or `None` when the slice expired.
    pub fn recv_timeout(&self, timeout: std::time::Duration) -> Option<TapChunk> {
        use std::sync::mpsc::RecvTimeoutError;
        match self.rx.recv_timeout(timeout) {
            Ok(chunk) => Some(chunk),
            Err(RecvTimeoutError::Timeout) => None,
            Err(RecvTimeoutError::Disconnected) => {
                // A reader that has exited must not turn the caller's wait
                // into a spin: sleep out the slice it asked for.
                std::thread::sleep(timeout);
                None
            }
        }
    }

    /// Stop consuming input before handing the terminal to a foreign
    /// reader. True when the handoff is established: the reader confirmed
    /// it parked, or it has already exited — either way nothing of ours is
    /// competing for the terminal. False when neither was established in
    /// time: a live-but-slow reader may still be attached, and the caller
    /// must NOT spawn a foreign reader — clear the request with `resume`
    /// and let the user retry.
    pub fn pause(&self) -> bool {
        use std::sync::atomic::Ordering;
        self.control.pause.store(true, Ordering::SeqCst);
        let deadline = std::time::Instant::now() + PARK_ACK_TIMEOUT;
        loop {
            if self.control.parked.load(Ordering::SeqCst) {
                return true;
            }
            if self
                .reader
                .as_ref()
                .is_none_or(|reader| reader.is_finished())
            {
                return true;
            }
            if std::time::Instant::now() >= deadline {
                return false;
            }
            std::thread::sleep(std::time::Duration::from_millis(2));
        }
    }

    /// Read again. Bytes typed while parked are still queued in the
    /// terminal and arrive normally.
    pub fn resume(&self) {
        use std::sync::atomic::Ordering;
        self.control.parked.store(false, Ordering::SeqCst);
        self.control.pause.store(false, Ordering::SeqCst);
    }
}

#[cfg(unix)]
impl Drop for TtyTap {
    fn drop(&mut self) {
        use std::sync::atomic::Ordering;
        self.control.shutdown.store(true, Ordering::SeqCst);
        self.control.pause.store(false, Ordering::SeqCst);
        if let Some(reader) = self.reader.take() {
            // Bounded by one slice: the reader never blocks on a read it
            // has not polled for first.
            let _ = reader.join();
        }
    }
}

#[cfg(unix)]
fn read_loop(tty: &std::fs::File, tx: &std::sync::mpsc::Sender<TapChunk>, control: &TapControl) {
    use std::os::unix::io::AsRawFd;
    use std::sync::atomic::Ordering;

    let fd = tty.as_raw_fd();
    let mut buf = [0u8; 256];
    loop {
        if control.shutdown.load(Ordering::SeqCst) {
            return;
        }
        if control.pause.load(Ordering::SeqCst) {
            // Parked: the terminal belongs to someone else until resume,
            // and what they type stays queued for them.
            control.parked.store(true, Ordering::SeqCst);
            std::thread::sleep(std::time::Duration::from_millis(2));
            continue;
        }
        // select(2), not poll(2): on macOS, poll against /dev/tty reports
        // POLLNVAL without ever signaling readiness — the same quirk the
        // event library's dev-tty path works around via select.
        let mut read_set: libc::fd_set = unsafe { std::mem::zeroed() };
        unsafe {
            libc::FD_ZERO(&mut read_set);
            libc::FD_SET(fd, &mut read_set);
        }
        let mut timeout = libc::timeval {
            tv_sec: 0,
            tv_usec: READ_SLICE.subsec_micros() as libc::suseconds_t,
        };
        let ready = unsafe {
            libc::select(
                fd + 1,
                &mut read_set,
                std::ptr::null_mut(),
                std::ptr::null_mut(),
                &mut timeout,
            )
        };
        if ready < 0 {
            let err = std::io::Error::last_os_error();
            if err.kind() == std::io::ErrorKind::Interrupted {
                continue;
            }
            return;
        }
        if ready == 0 {
            continue;
        }
        // Between "readable" and "read": a pause claimed in this window
        // wins, so the byte is left for whoever comes next.
        if control.pause.load(Ordering::SeqCst) {
            continue;
        }
        let read = unsafe { libc::read(fd, buf.as_mut_ptr().cast::<libc::c_void>(), buf.len()) };
        if read <= 0 {
            return; // End of input, or the device went away.
        }
        if tx
            .send(TapChunk::Tty(buf[..read as usize].to_vec()))
            .is_err()
        {
            return; // Nobody is listening any more.
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[cfg(unix)]
    #[test]
    fn a_posted_trigger_wakes_the_receiver_early() {
        // The channel is the wake path: a Trigger posted from another
        // thread returns from recv_timeout well before the timeout.
        // Self-skipping when no terminal device exists (CI).
        let Ok(tap) = TtyTap::spawn() else { return };
        let sender = tap.sender();
        std::thread::spawn(move || {
            let _ = sender.send(TapChunk::Trigger);
        });
        let start = std::time::Instant::now();
        let got = tap.recv_timeout(std::time::Duration::from_secs(5));
        assert_eq!(got, Some(TapChunk::Trigger));
        assert!(
            start.elapsed() < std::time::Duration::from_secs(4),
            "the trigger did not wake the receiver early"
        );
    }

    #[test]
    fn a_split_report_reassembles_across_feeds() {
        let mut scanner = TapScanner::new();
        assert_eq!(scanner.feed(b"\x1b[?997"), vec![]);
        assert_eq!(
            scanner.feed(b";2n"),
            vec![TapEvent::ThemeNotification(Appearance::Light)]
        );
    }

    #[test]
    fn a_report_sandwiched_between_keys_yields_all_three_in_order() {
        let mut scanner = TapScanner::new();
        let events = scanner.feed(b"a\x1b[?997;2nb");
        assert_eq!(
            events,
            vec![
                TapEvent::Key(Key::Char('a')),
                TapEvent::ThemeNotification(Appearance::Light),
                TapEvent::Key(Key::Char('b')),
            ]
        );
    }

    #[test]
    fn an_unrecognized_private_csi_is_dropped_without_wedging() {
        let mut scanner = TapScanner::new();
        assert_eq!(scanner.feed(b"\x1b[?123;4x"), vec![]);
        // The buffer must not have retained anything from the discarded run.
        assert_eq!(scanner.feed(b"z"), vec![TapEvent::Key(Key::Char('z'))]);
    }

    #[test]
    fn an_unfinished_sequence_past_the_cap_is_discarded_wholesale() {
        let mut scanner = TapScanner::new();
        // A long run that never reaches a CSI final byte. Filler is 0x00,
        // not a digit or `;`, so it can never be mistaken for a report and
        // decodes to no key either way the byte ends up being processed.
        let mut long_run = b"\x1b[".to_vec();
        long_run.resize(long_run.len() + 200, 0u8);
        assert_eq!(scanner.feed(&long_run), vec![]);
        assert_eq!(scanner.feed(b"z"), vec![TapEvent::Key(Key::Char('z'))]);
    }

    #[test]
    fn arrow_keys_decode_through_the_scanner() {
        let mut scanner = TapScanner::new();
        assert_eq!(scanner.feed(b"\x1b[A"), vec![TapEvent::Key(Key::Up)]);
        assert_eq!(scanner.feed(b"\x1b[B"), vec![TapEvent::Key(Key::Down)]);
        assert_eq!(scanner.feed(b"\x1b[C"), vec![TapEvent::Key(Key::Right)]);
        assert_eq!(scanner.feed(b"\x1b[D"), vec![TapEvent::Key(Key::Left)]);
    }

    #[test]
    fn page_and_home_end_sequences_decode() {
        let mut scanner = TapScanner::new();
        assert_eq!(scanner.feed(b"\x1b[5~"), vec![TapEvent::Key(Key::PageUp)]);
        assert_eq!(scanner.feed(b"\x1b[6~"), vec![TapEvent::Key(Key::PageDown)]);
        assert_eq!(scanner.feed(b"\x1b[H"), vec![TapEvent::Key(Key::Home)]);
        assert_eq!(scanner.feed(b"\x1b[F"), vec![TapEvent::Key(Key::End)]);
        assert_eq!(scanner.feed(b"\x1b[1~"), vec![TapEvent::Key(Key::Home)]);
        assert_eq!(scanner.feed(b"\x1b[4~"), vec![TapEvent::Key(Key::End)]);
        assert_eq!(scanner.feed(b"\x1b[7~"), vec![TapEvent::Key(Key::Home)]);
        assert_eq!(scanner.feed(b"\x1b[8~"), vec![TapEvent::Key(Key::End)]);
    }

    #[test]
    fn a_theme_report_is_still_a_report_not_a_key() {
        // The report parser is offered a complete CSI run first; a private
        // or parameterized run is never decoded as a key.
        let mut scanner = TapScanner::new();
        assert_eq!(
            scanner.feed(b"\x1b[?997;2n"),
            vec![TapEvent::ThemeNotification(Appearance::Light)]
        );
    }

    #[test]
    fn a_split_arrow_reassembles_across_feeds() {
        let mut scanner = TapScanner::new();
        assert_eq!(scanner.feed(b"\x1b["), vec![]);
        assert_eq!(scanner.feed(b"B"), vec![TapEvent::Key(Key::Down)]);
    }

    #[test]
    fn an_application_cursor_arrow_decodes() {
        let mut scanner = TapScanner::new();
        assert_eq!(scanner.feed(b"\x1bOA"), vec![TapEvent::Key(Key::Up)]);
        // F4 on several terminals: a function-key final is not a key here —
        // without SS3 decoding it would degrade to Char('S').
        assert_eq!(scanner.feed(b"\x1bOS"), vec![]);
    }

    #[test]
    fn an_osc_color_reply_reassembles_across_feeds() {
        let mut scanner = TapScanner::new();
        assert_eq!(scanner.feed(b"\x1b]11;rgb:1e1e/1e1e/"), vec![]);
        assert_eq!(
            scanner.feed(b"2e2e\x07"),
            vec![TapEvent::OscColor(
                OscColorKind::Background,
                xterm_color::Color::rgb(0x1e1e, 0x1e1e, 0x2e2e)
            )]
        );
    }

    #[test]
    fn a_lone_escape_with_no_recognized_introducer_does_not_eat_the_next_byte() {
        let mut scanner = TapScanner::new();
        assert_eq!(scanner.feed(b"\x1b"), vec![]);
        assert_eq!(scanner.feed(b"q"), vec![TapEvent::Key(Key::Char('q'))]);
    }

    #[test]
    fn a_lone_escape_resolves_after_the_hold() {
        let mut scanner = TapScanner::new();
        assert_eq!(scanner.feed(b"\x1b"), vec![]);
        assert_eq!(scanner.idle(std::time::Duration::from_millis(20)), vec![]);
        assert_eq!(
            scanner.idle(std::time::Duration::from_millis(40)),
            vec![TapEvent::Key(Key::Esc)]
        );
        // Not sticky: the resolved escape is gone.
        assert_eq!(scanner.idle(std::time::Duration::from_millis(50)), vec![]);
    }

    #[test]
    fn bytes_cancel_a_pending_escape() {
        let mut scanner = TapScanner::new();
        assert_eq!(scanner.feed(b"\x1b"), vec![]);
        assert_eq!(scanner.idle(std::time::Duration::from_millis(30)), vec![]);
        assert_eq!(scanner.feed(b"["), vec![]);
        // A reassembling CSI is never flushed by silence.
        assert_eq!(scanner.idle(std::time::Duration::from_millis(50)), vec![]);
        assert_eq!(scanner.feed(b"A"), vec![TapEvent::Key(Key::Up)]);
    }

    #[test]
    fn an_escape_followed_by_a_plain_byte_keeps_todays_behavior() {
        let mut scanner = TapScanner::new();
        assert_eq!(scanner.feed(b"\x1bq"), vec![TapEvent::Key(Key::Char('q'))]);
    }

    #[test]
    fn idle_never_flushes_a_partial_sequence() {
        let mut scanner = TapScanner::new();
        assert_eq!(scanner.feed(b"\x1b[?997"), vec![]);
        assert_eq!(scanner.idle(std::time::Duration::from_millis(200)), vec![]);
        assert_eq!(
            scanner.feed(b";2n"),
            vec![TapEvent::ThemeNotification(Appearance::Light)]
        );
    }

    #[test]
    fn decode_key_maps_the_five_recognized_bytes() {
        assert_eq!(decode_key(0x03), Some(Key::CtrlC));
        assert_eq!(decode_key(b'\r'), Some(Key::Enter));
        assert_eq!(decode_key(b'\n'), Some(Key::Enter));
        assert_eq!(decode_key(b'q'), Some(Key::Char('q')));
        assert_eq!(decode_key(b'v'), Some(Key::Char('v')));
    }

    #[test]
    fn decode_key_has_no_verdict_for_escape_or_delete() {
        assert_eq!(decode_key(0x1b), None);
        assert_eq!(decode_key(0x7f), None);
    }
}
