//! Reassembles decoded input events from raw terminal bytes for a
//! long-running command that owns its terminal's input. The scanner never
//! retains an unrecognized run indefinitely: a complete escape sequence it
//! does not understand is dropped, and an accumulating one is bounded.

#[cfg(unix)]
use crate::core::trigger::Observation;
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

/// Bounded wait for the reader to confirm it has parked. This bounds
/// only the FAILURE path — the common case returns the moment the
/// parked flag flips (one read slice plus scheduling). Sized for a
/// starved scheduler (a loaded CI runner missed 150ms repeatedly);
/// past it the reader is treated as unresponsive and the caller must
/// not hand the terminal to a foreign reader.
#[cfg(unix)]
const PARK_ACK_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(1);

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

/// A reader thread for one fifo/fd trigger source. Its outputs are
/// exactly: the `fired` flag (rising-edge) and at most one
/// `TapChunk::Trigger` wake per rising edge — it never touches the
/// terminal, loop state, or the schedule. `ended` reports EOF or a
/// terminal error; a fifo source never ends, because the reader's own
/// dummy write end holds the pipe open by design — external writers
/// may come and go and each later write keeps working.
#[cfg(unix)]
#[derive(Debug)]
pub struct TriggerReader {
    fired: std::sync::Arc<std::sync::atomic::AtomicBool>,
    ended: std::sync::Arc<std::sync::atomic::AtomicBool>,
    shutdown: std::sync::Arc<std::sync::atomic::AtomicBool>,
    arrivals: std::sync::Arc<std::sync::Mutex<std::collections::VecDeque<Observation>>>,
    overflowed: std::sync::Arc<std::sync::atomic::AtomicBool>,
    /// High-water mark of reads taken inside one `select`. Maintained
    /// always — one store per wake is not worth a `cfg` seam through the
    /// loop's signature — and read only by the tests that pin `drainable`'s
    /// two routes apart, hence the staged allow.
    #[cfg_attr(not(test), allow(dead_code))]
    max_reads_per_select: std::sync::Arc<std::sync::atomic::AtomicUsize>,
    reader: Option<std::thread::JoinHandle<()>>,
}

/// How many un-drained arrivals a reader holds.
///
/// The reader must never grow without limit and must never block — a
/// blocked reader thread is a wedged trigger. So the queue drops the
/// **oldest** when it is full, which is the right end to lose: the loop
/// drains every iteration, so a full queue means the arrivals at the
/// front are already older than the window that would read them.
///
/// A drop is never silent. Losing an arrival can lose a window's only
/// **exogenous** observation, and the veto that observation feeds is a
/// zero test — so a silent drop would not degrade the signal, it would
/// invert it, turning "no outside writer was seen" into an accusation.
/// The overflow flag is what makes the window abstain instead.
///
/// The bound is generous on purpose: at one arrival per read and a loop
/// that drains at least every `SLICE`, reaching it means the reader is
/// seeing arrivals faster than the loop runs, which is itself the
/// condition worth reporting.
#[cfg(unix)]
pub const ARRIVAL_CAP: usize = 256;

#[cfg(unix)]
impl TriggerReader {
    /// Open a fifo/fd source and start its reader. `wake` (the tap's
    /// `sender()`) buys an immediate wake of the event wait; without it
    /// (a failed tap spawn, or a unit test) the fired flag alone
    /// signals, read once per loop slice.
    pub fn open(
        spec: &crate::core::trigger::TriggerSpec,
        wake: Option<std::sync::mpsc::Sender<TapChunk>>,
    ) -> anyhow::Result<TriggerReader> {
        use std::os::unix::fs::{FileTypeExt, OpenOptionsExt};
        use std::os::unix::io::AsRawFd;

        use anyhow::{anyhow, bail};

        use crate::core::trigger::TriggerSpec;

        // The fds the loop selects and reads; files are moved into the
        // thread so their descriptors outlive the setup.
        //
        // `drainable` is I-83's narrow half. Proving emptiness with a
        // zero-timeout `select` costs no read and is safe on every
        // descriptor; draining REPEATEDLY is not. A `fifo:` source we
        // opened `O_NONBLOCK` ourselves can only ever return `EAGAIN`, but a
        // `fd:` source keeps the caller's blocking mode and may be shared,
        // so another consumer can take the readable bytes between the probe
        // and the `read` and this thread blocks — the one place I-80 must
        // never be violated.
        let drainable = matches!(spec, crate::core::trigger::TriggerSpec::Fifo(_));
        let (fd, keep_alive) = match spec {
            TriggerSpec::Fifo(path) => {
                let read_end = std::fs::OpenOptions::new()
                    .read(true)
                    .custom_flags(libc::O_NONBLOCK)
                    .open(path)
                    .map_err(|err| match err.kind() {
                        std::io::ErrorKind::NotFound => anyhow!(
                            "trigger fifo {} does not exist; create it with: mkfifo {}",
                            path.display(),
                            path.display()
                        ),
                        _ => anyhow!("opening trigger fifo {}: {err}", path.display()),
                    })?;
                if !read_end.metadata()?.file_type().is_fifo() {
                    bail!(
                        "fifo:{} is not a named pipe; use file:{} for plain paths",
                        path.display(),
                        path.display()
                    );
                }
                // The EOF-spin fix: a fifo with no writer reports
                // readable and reads 0 forever. Holding our own
                // non-blocking write end (legal exactly because we are
                // already a reader) keeps EOF away for the whole run.
                let write_end = std::fs::OpenOptions::new()
                    .write(true)
                    .custom_flags(libc::O_NONBLOCK)
                    .open(path)?;
                (read_end.as_raw_fd(), vec![read_end, write_end])
            }
            TriggerSpec::Fd(fd) => {
                let mut stat: libc::stat = unsafe { std::mem::zeroed() };
                if unsafe { libc::fstat(*fd, &mut stat) } != 0 {
                    bail!("fd:{fd} is not an open descriptor");
                }
                if stat.st_mode & libc::S_IFMT == libc::S_IFREG {
                    bail!(
                        "fd:{fd} is a regular file, which select(2) always reports \
                         ready; use file:PATH to watch a file"
                    );
                }
                (*fd, Vec::new())
            }
            TriggerSpec::File(_) => bail!("file: triggers are polled, not read"),
        };

        let fired = std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false));
        let ended = std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false));
        let shutdown = std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false));
        let arrivals = std::sync::Arc::new(std::sync::Mutex::new(
            std::collections::VecDeque::with_capacity(ARRIVAL_CAP),
        ));
        let overflowed = std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false));
        let max_reads_per_select = std::sync::Arc::new(std::sync::atomic::AtomicUsize::new(0));
        let thread_fired = std::sync::Arc::clone(&fired);
        let thread_ended = std::sync::Arc::clone(&ended);
        let thread_shutdown = std::sync::Arc::clone(&shutdown);
        let thread_arrivals = std::sync::Arc::clone(&arrivals);
        let thread_overflowed = std::sync::Arc::clone(&overflowed);
        let thread_max_reads = std::sync::Arc::clone(&max_reads_per_select);
        let reader = std::thread::Builder::new()
            .name("rat-trigger".to_string())
            .spawn(move || {
                let _keep_alive = keep_alive;
                trigger_read_loop(
                    fd,
                    drainable,
                    &thread_fired,
                    &thread_ended,
                    &thread_shutdown,
                    &thread_arrivals,
                    &thread_overflowed,
                    &thread_max_reads,
                    wake,
                );
            })?;
        Ok(TriggerReader {
            fired,
            ended,
            shutdown,
            arrivals,
            overflowed,
            max_reads_per_select,
            reader: Some(reader),
        })
    }

    pub fn fired(&self) -> &std::sync::atomic::AtomicBool {
        &self.fired
    }

    pub fn ended(&self) -> &std::sync::atomic::AtomicBool {
        &self.ended
    }

    /// Drain the arrivals recorded since the last call.
    ///
    /// **Deliberately separate from `fired`.** The gate swaps `fired` to
    /// decide whether to respawn; this drains observations for the
    /// attribution window. One arrival is one *read*, not one write —
    /// see `trigger_read_loop` for what that does and does not
    /// distinguish. The two must never be folded into one call: a drain
    /// that consumed `fired` would lose a fire and the pane would
    /// silently stop refreshing.
    pub fn take_arrivals(&self) -> Vec<Observation> {
        let mut queue = self
            .arrivals
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        queue.drain(..).collect()
    }

    /// Hold the arrivals queue, so a test can prove `observed_at` is stamped
    /// OUTSIDE the lock: a timestamp taken inside it would be dragged
    /// forward by this contention, and the test measures exactly that.
    #[cfg(test)]
    pub fn lock_arrivals_for_test(
        &self,
    ) -> std::sync::MutexGuard<'_, std::collections::VecDeque<Observation>> {
        self.arrivals
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
    }

    /// The most reads the loop has ever taken inside one `select`. The
    /// property is about iteration structure, which no externally observable
    /// timing can pin down — a fast machine can complete three whole
    /// select/read cycles before a test looks, so counting observations
    /// cannot tell the two routes apart.
    #[cfg(test)]
    pub fn max_reads_per_select_for_test(&self) -> usize {
        self.max_reads_per_select
            .load(std::sync::atomic::Ordering::SeqCst)
    }

    /// Whether arrivals were dropped since the last call — reports and
    /// clears.
    ///
    /// Clearing is the point. The loop reads this once per iteration and
    /// makes *that* window abstain; a sticky flag would make every later
    /// window abstain too, and a badge that can never clear is the
    /// failure this whole design is built to avoid.
    pub fn overflowed(&self) -> bool {
        self.overflowed
            .swap(false, std::sync::atomic::Ordering::SeqCst)
    }
}

#[cfg(unix)]
impl Drop for TriggerReader {
    fn drop(&mut self) {
        use std::sync::atomic::Ordering;
        self.shutdown.store(true, Ordering::SeqCst);
        if let Some(reader) = self.reader.take() {
            // Bounded by one slice: the reader never blocks on a read
            // it has not selected for first.
            let _ = reader.join();
        }
    }
}

/// The reader's loop: select with a bounded slice, drain, raise the
/// flag on the rising edge. End discipline: EOF (`read == 0`) or any
/// terminal select/read error sets `ended` and exits — a dead source
/// must never spin silently; transient EINTR/EAGAIN are retried.
///
/// **Why this route records an instant and the polled route records a
/// bracket.** A `file:` trigger can be stat'd before and after a child,
/// so a change can be placed inside a window after the fact. A fifo
/// cannot: the bytes are drained and gone, there is no state left to
/// compare, and nothing can reconstruct when they landed. The arrival
/// instant is therefore the only attribution signal that exists on this
/// route, and the reader thread is the only place that has it.
///
/// **One arrival is one read, not one write.** A single `read` returns
/// whatever is queued, so writes that land between two selects are
/// coalesced into one arrival and the reader cannot tell them apart —
/// nothing in the pipe records how many `write` calls produced the
/// bytes. That under-counts a tight burst and never over-counts, which
/// is the safe direction: the veto asks whether an outside writer was
/// *ever* seen, and a coalesced arrival still answers that with the
/// right instant.
#[cfg(unix)]
#[allow(clippy::too_many_arguments)]
/// Is there anything to read right now?
///
/// A zero-timeout `select` proves emptiness WITHOUT a read, which is what
/// makes it safe for a descriptor whose blocking mode we do not control — a
/// speculative `read` on a shared blocking `fd:` could wedge this thread.
/// Regular files, the one kind `select` would always call ready, are
/// rejected at open.
#[cfg(unix)]
fn readable_now(fd: i32) -> bool {
    let mut set: libc::fd_set = unsafe { std::mem::zeroed() };
    unsafe {
        libc::FD_ZERO(&mut set);
        libc::FD_SET(fd, &mut set);
    }
    let mut zero = libc::timeval {
        tv_sec: 0,
        tv_usec: 0,
    };
    unsafe {
        libc::select(
            fd + 1,
            &mut set,
            std::ptr::null_mut(),
            std::ptr::null_mut(),
            &mut zero,
        ) > 0
    }
}

/// What one `read` in the drain loop told us to do next.
///
/// `cfg(unix)` like everything else down here: a non-cfg item this far into
/// the file lands *after* an earlier `#[cfg(test)] mod`, and on the Windows
/// leg — where the unix items vanish and it would not — clippy's
/// `items_after_test_module` fires. It compiles clean on macOS either way,
/// so only the cross-compile leg catches it.
#[cfg(unix)]
enum ReadStep {
    /// Bytes arrived; record an observation.
    Got,
    /// Nothing more to take right now — leave the drain, keep the thread.
    Idle,
    /// The source is finished or broken; the thread is done.
    Over,
}

#[cfg(unix)]
#[allow(clippy::too_many_arguments)]
fn trigger_read_loop(
    fd: i32,
    drainable: bool,
    fired: &std::sync::atomic::AtomicBool,
    ended: &std::sync::atomic::AtomicBool,
    shutdown: &std::sync::atomic::AtomicBool,
    arrivals: &std::sync::Mutex<std::collections::VecDeque<Observation>>,
    overflowed: &std::sync::atomic::AtomicBool,
    max_reads_per_select: &std::sync::atomic::AtomicUsize,
    wake: Option<std::sync::mpsc::Sender<TapChunk>>,
) {
    use std::sync::atomic::Ordering;

    let mut buf = [0u8; 256];
    // The last instant this reader PROVED the descriptor empty. `None` until
    // it has proved it once; an observation carrying `None` claims nothing.
    let mut empty_since: Option<std::time::Instant> = None;
    loop {
        if shutdown.load(Ordering::SeqCst) {
            return;
        }
        let mut read_set: libc::fd_set = unsafe { std::mem::zeroed() };
        unsafe {
            libc::FD_ZERO(&mut read_set);
            libc::FD_SET(fd, &mut read_set);
        }
        let mut timeout = libc::timeval {
            tv_sec: 0,
            tv_usec: READ_SLICE.subsec_micros() as libc::suseconds_t,
        };
        // Sampled BEFORE the probe, never after. Bytes can become readable
        // between the kernel's check inside `select` and a `now()` taken on
        // return, so a bound stamped afterwards could postdate the very
        // write it claims to precede — and the interval would then exclude
        // the moment it exists to contain. Erring early only widens the
        // interval, which is always safe.
        let candidate = std::time::Instant::now();
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
            ended.store(true, Ordering::SeqCst);
            return;
        }
        if ready == 0 {
            // Nothing became readable for a whole slice: a proof of
            // emptiness, free, and the only one an unfenced reader gets.
            empty_since = Some(candidate);
            continue;
        }
        let mut reads = 0usize;
        loop {
            let read =
                unsafe { libc::read(fd, buf.as_mut_ptr().cast::<libc::c_void>(), buf.len()) };
            let step = if read > 0 {
                ReadStep::Got
            } else if read == 0 {
                ReadStep::Over // EOF: every write end is gone (fd: sources only).
            } else {
                let err = std::io::Error::last_os_error();
                if matches!(
                    err.kind(),
                    std::io::ErrorKind::Interrupted | std::io::ErrorKind::WouldBlock
                ) {
                    ReadStep::Idle
                } else {
                    ReadStep::Over
                }
            };
            match step {
                ReadStep::Over => {
                    ended.store(true, Ordering::SeqCst);
                    return;
                }
                ReadStep::Idle => break,
                ReadStep::Got => {}
            }
            reads += 1;
            // Stamped HERE: after the read, before any lock. Contention on
            // the queue must not be able to drag this forward — that was the
            // largest of the three ways the old instant drifted.
            let observed_at = std::time::Instant::now();
            // Recorded beside the flag store, never derived from it: the
            // flag is a rising edge the gate consumes, so an arrival keyed
            // off it would be lost whenever the loop had not drained yet —
            // which is exactly the burst case the window most needs.
            {
                let mut queue = arrivals
                    .lock()
                    .unwrap_or_else(std::sync::PoisonError::into_inner);
                if queue.len() == ARRIVAL_CAP {
                    queue.pop_front();
                    overflowed.store(true, Ordering::SeqCst);
                }
                queue.push_back(Observation {
                    empty_since,
                    observed_at,
                });
            }
            if !drainable {
                break; // one read per select for a descriptor we do not own
            }
            let candidate = std::time::Instant::now();
            if !readable_now(fd) {
                empty_since = Some(candidate);
                break;
            }
        }
        max_reads_per_select.fetch_max(reads, Ordering::SeqCst);
        if reads > 0
            && !fired.swap(true, Ordering::SeqCst)
            && let Some(wake) = wake.as_ref()
        {
            let _ = wake.send(TapChunk::Trigger);
        }
    }
}

#[cfg(all(test, unix))]
mod trigger_reader_tests {
    use std::io::Write;
    use std::os::unix::io::AsRawFd;
    use std::sync::atomic::Ordering;
    use std::time::Duration;

    use super::*;
    use crate::core::trigger::TriggerSpec;

    fn mkfifo(path: &std::path::Path) {
        let cpath = std::ffi::CString::new(path.as_os_str().as_encoded_bytes().to_vec()).unwrap();
        assert_eq!(unsafe { libc::mkfifo(cpath.as_ptr(), 0o600) }, 0, "mkfifo");
    }

    fn wait_until(mut cond: impl FnMut() -> bool) -> bool {
        let deadline = std::time::Instant::now() + Duration::from_secs(3);
        while std::time::Instant::now() < deadline {
            if cond() {
                return true;
            }
            std::thread::sleep(Duration::from_millis(10));
        }
        false
    }

    #[test]
    fn a_fifo_write_raises_the_fired_flag() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("t.fifo");
        mkfifo(&path);
        let reader = TriggerReader::open(&TriggerSpec::Fifo(path.clone()), None).unwrap();
        let mut writer = std::fs::OpenOptions::new().write(true).open(&path).unwrap();
        writer.write_all(b"x").unwrap();
        assert!(
            wait_until(|| reader.fired().swap(false, Ordering::SeqCst)),
            "the write never raised the flag"
        );
    }

    #[test]
    fn a_writerless_fifo_does_not_spin_or_end() {
        // The dummy write end keeps EOF away while no writer exists.
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("t.fifo");
        mkfifo(&path);
        let reader = TriggerReader::open(&TriggerSpec::Fifo(path), None).unwrap();
        std::thread::sleep(Duration::from_millis(200));
        assert!(!reader.ended().load(Ordering::SeqCst));
        assert!(!reader.fired().load(Ordering::SeqCst));
    }

    #[test]
    fn a_regular_file_fd_is_rejected_at_open_with_the_teaching_error() {
        let dir = tempfile::tempdir().unwrap();
        let f = dir.path().join("reg");
        std::fs::write(&f, b"x").unwrap();
        let file = std::fs::File::open(&f).unwrap();
        let err = TriggerReader::open(&TriggerSpec::Fd(file.as_raw_fd()), None)
            .unwrap_err()
            .to_string();
        assert!(err.contains("file:"), "{err}"); // S_ISREG teaches file:
    }

    /// Await one fire with a tight poll. The overflow tests do hundreds of
    /// round trips, and `wait_until`'s 10 ms sleep turns that into a
    /// multi-second nap that timed out on a loaded CI runner.
    fn poke(reader: &TriggerReader, writer: &mut std::fs::File) {
        reader.fired().store(false, Ordering::SeqCst);
        writer.write_all(b"x").unwrap();
        let deadline = std::time::Instant::now() + Duration::from_secs(5);
        while std::time::Instant::now() < deadline {
            if reader.fired().load(Ordering::SeqCst) {
                return;
            }
            std::thread::sleep(Duration::from_micros(200));
        }
        panic!("the write never reached the reader");
    }

    /// Open a fifo reader and a writer onto it.
    fn fifo_pair(dir: &std::path::Path) -> (TriggerReader, std::fs::File) {
        let path = dir.join("t.fifo");
        mkfifo(&path);
        let reader = TriggerReader::open(&TriggerSpec::Fifo(path.clone()), None).unwrap();
        let writer = std::fs::OpenOptions::new().write(true).open(&path).unwrap();
        (reader, writer)
    }

    #[test]
    fn each_fire_records_one_arrival_instant() {
        let dir = tempfile::tempdir().unwrap();
        let (reader, mut writer) = fifo_pair(dir.path());
        writer.write_all(b"x").unwrap();
        assert!(
            wait_until(|| reader.fired().load(Ordering::SeqCst)),
            "the write never raised the flag"
        );
        assert_eq!(reader.take_arrivals().len(), 1);
    }

    #[test]
    fn the_instant_is_taken_at_arrival_not_when_the_loop_drains() {
        // The accuracy claim, and the whole reason this route is cheaper
        // than the polled one: the reader already knows the exact
        // instant, where a slice poll is quantised. Delaying the drain
        // must not move the recorded time — if the instant were taken
        // here, it would land after the sleep and this fails.
        let dir = tempfile::tempdir().unwrap();
        let (reader, mut writer) = fifo_pair(dir.path());
        let before = std::time::Instant::now();
        writer.write_all(b"x").unwrap();
        assert!(wait_until(|| reader.fired().load(Ordering::SeqCst)));
        std::thread::sleep(Duration::from_millis(300));
        let arrivals = reader.take_arrivals();
        assert_eq!(arrivals.len(), 1);
        let at = arrivals[0].observed_at;
        assert!(
            at.duration_since(before) < Duration::from_millis(250),
            "recorded {:?} after the write — taken at the drain, not at arrival",
            at.duration_since(before)
        );
        assert!(
            at.elapsed() >= Duration::from_millis(250),
            "only {:?} before the drain; the sleep did not separate them",
            at.elapsed()
        );
    }

    #[test]
    fn taking_arrivals_does_not_disturb_the_fired_flag() {
        // The loop swaps `fired` to drive the gate. If taking arrivals
        // consumed it, a fire would be lost and the pane would stop
        // refreshing — the same failure mode the observer's separate
        // baselines exist to prevent, arriving by a different door.
        let dir = tempfile::tempdir().unwrap();
        let (reader, mut writer) = fifo_pair(dir.path());
        writer.write_all(b"x").unwrap();
        assert!(wait_until(|| reader.fired().load(Ordering::SeqCst)));
        let _ = reader.take_arrivals();
        assert!(
            reader.fired().load(Ordering::SeqCst),
            "taking arrivals cleared the gate's flag"
        );
    }

    #[test]
    fn every_separately_observed_write_records_its_own_arrival() {
        // The veto counts OBSERVATIONS, not respawns: the debounce
        // collapses a burst into one respawn, but each arrival is a
        // distinct datum for the credit rule. Each write is awaited, so
        // each is a separate read — see the tight-burst test below for
        // what the reader can and cannot distinguish.
        let dir = tempfile::tempdir().unwrap();
        let (reader, mut writer) = fifo_pair(dir.path());
        for _ in 0..5 {
            poke(&reader, &mut writer);
        }
        assert_eq!(reader.take_arrivals().len(), 5);
    }

    #[test]
    fn a_tight_burst_coalesces_and_that_is_the_safe_direction() {
        // MEASURED, not assumed: 20 writes with no wait between them
        // produced exactly ONE arrival. A single `read` returns whatever
        // is queued, and nothing in a pipe records how many `write`
        // calls produced the bytes — so this route cannot count writes
        // and must not claim to.
        //
        // Pinned because the limit is load-bearing in one direction
        // only. Coalescing UNDER-counts and can never over-count, which
        // is the safe way round: the veto asks whether an outside writer
        // was ever seen, and a coalesced arrival still answers that with
        // a correct instant. Over-counting would be the dangerous error
        // — it would manufacture exogenous observations that never
        // happened and clear a veto that should have held.
        let dir = tempfile::tempdir().unwrap();
        let (reader, mut writer) = fifo_pair(dir.path());
        for _ in 0..20 {
            writer.write_all(b"x").unwrap();
        }
        assert!(wait_until(|| reader.fired().load(Ordering::SeqCst)));
        std::thread::sleep(Duration::from_millis(200));
        let n = reader.take_arrivals().len();
        assert!(
            (1..=20).contains(&n),
            "{n} arrivals from 20 writes — more than 20 is impossible, \
             and zero would mean the burst was lost entirely"
        );
    }

    #[test]
    fn the_queue_is_bounded_and_a_drop_is_reported_not_silent() {
        // A silent drop would corrupt a zero test: losing an arrival can
        // lose the window's only EXOGENOUS observation, flipping the
        // veto into a false positive. Blocking the reader is not an
        // option either, so it drops the oldest and SAYS SO.
        let dir = tempfile::tempdir().unwrap();
        let (reader, mut writer) = fifo_pair(dir.path());
        for _ in 0..(ARRIVAL_CAP * 2) {
            poke(&reader, &mut writer);
        }
        assert!(reader.overflowed(), "a drop must be observable");
        assert!(
            reader.take_arrivals().len() <= ARRIVAL_CAP,
            "the queue grew past its bound"
        );
    }

    #[test]
    fn overflowed_reports_and_clears() {
        // The loop reads it once per iteration and the window abstains
        // for THAT window; a sticky flag would make every later window
        // abstain too, and the badge could never clear.
        let dir = tempfile::tempdir().unwrap();
        let (reader, mut writer) = fifo_pair(dir.path());
        for _ in 0..(ARRIVAL_CAP + 1) {
            poke(&reader, &mut writer);
        }
        assert!(reader.overflowed());
        assert!(!reader.overflowed(), "the flag did not clear on read");
    }

    #[test]
    fn fd_eof_sets_ended_and_the_thread_exits() {
        // An fd: source whose write side closes ends cleanly — this is
        // fd:-only territory; a fifo never reaches it past the dummy
        // writer.
        let mut fds = [0i32; 2];
        assert_eq!(unsafe { libc::pipe(fds.as_mut_ptr()) }, 0, "pipe");
        let (r, w) = (fds[0], fds[1]);
        let reader = TriggerReader::open(&TriggerSpec::Fd(r), None).unwrap();
        unsafe { libc::close(w) };
        assert!(
            wait_until(|| reader.ended().load(Ordering::SeqCst)),
            "EOF never set ended"
        );
    }

    // ── The empty frontier (task 2.1) ───────────────────────────────────
    //
    // The reader stops reporting an instant and starts reporting an
    // interval: bytes appeared between the last moment it PROVED the
    // descriptor empty and the moment its read returned.

    /// Drain until `n` observations have been recorded, or fail. The reader
    /// is a thread, so every assertion about what it recorded needs a
    /// settle.
    fn wait_for_observations(reader: &TriggerReader, n: usize) -> Vec<Observation> {
        let mut out = Vec::new();
        let deadline = std::time::Instant::now() + Duration::from_secs(3);
        while std::time::Instant::now() < deadline {
            out.extend(reader.take_arrivals());
            if out.len() >= n {
                return out;
            }
            std::thread::sleep(Duration::from_micros(200));
        }
        panic!("wanted {n} observations, saw {}", out.len());
    }

    /// A blocking, caller-owned pipe — a faithful stand-in for a real `fd:`
    /// source, whose flags this process does not control and must not
    /// change.
    fn os_pipe_pair() -> (std::fs::File, std::fs::File) {
        use std::os::unix::io::FromRawFd;
        let mut fds = [0i32; 2];
        assert_eq!(unsafe { libc::pipe(fds.as_mut_ptr()) }, 0, "pipe");
        unsafe {
            (
                std::fs::File::from_raw_fd(fds[0]),
                std::fs::File::from_raw_fd(fds[1]),
            )
        }
    }

    #[test]
    fn an_observation_is_bounded_below_by_a_proof_of_emptiness() {
        let dir = tempfile::tempdir().unwrap();
        let (reader, mut writer) = fifo_pair(dir.path());
        // Let the reader select at least once with nothing to read: that is
        // a proof of emptiness, and it must land BEFORE the write.
        std::thread::sleep(READ_SLICE * 2);
        let before = std::time::Instant::now();
        writer.write_all(b"x").unwrap();

        let observations = wait_for_observations(&reader, 1);
        let o = observations[0];
        assert!(
            o.empty_since.is_some(),
            "the reader must report a lower bound"
        );
        assert!(
            o.empty_since.unwrap() <= before,
            "the proof of emptiness must precede the write it bounds"
        );
        assert!(o.observed_at >= before, "and the read must follow it");
    }

    #[test]
    fn the_stamp_is_taken_before_the_queue_lock() {
        // Hold the arrivals lock across a write, so any timestamp taken
        // INSIDE the lock would be dragged forward by the contention. The
        // reported instant must not move.
        let dir = tempfile::tempdir().unwrap();
        let (reader, mut writer) = fifo_pair(dir.path());
        let held = reader.lock_arrivals_for_test();
        let at_write = std::time::Instant::now();
        writer.write_all(b"x").unwrap();
        std::thread::sleep(Duration::from_millis(120));
        drop(held);

        let o = wait_for_observations(&reader, 1)[0];
        assert!(
            o.observed_at < at_write + Duration::from_millis(100),
            "observed_at was taken after the lock, not after the read: {:?}",
            o.observed_at.duration_since(at_write)
        );
    }

    #[test]
    fn a_burst_drains_within_one_slice_and_shares_one_lower_bound() {
        // Several writes queued before the reader wakes are read in one
        // pass. Each chunk gets its own observed_at; they share the one
        // proof of emptiness that preceded them, because that is all that
        // is known.
        let dir = tempfile::tempdir().unwrap();
        let (reader, mut writer) = fifo_pair(dir.path());
        std::thread::sleep(READ_SLICE * 2);
        for _ in 0..3 {
            writer.write_all(&[b'x'; 300]).unwrap(); // > 256, so several reads
        }
        let observations = wait_for_observations(&reader, 2);
        let first = observations[0].empty_since;
        assert!(first.is_some());
        assert!(
            observations.iter().all(|o| o.empty_since == first),
            "one proof of emptiness bounds every chunk drained after it"
        );
    }

    #[test]
    fn an_fd_source_is_never_read_twice_in_one_select() {
        // I-83's narrow half. A `fd:` descriptor keeps the caller's
        // blocking mode and may be shared, so a speculative second read can
        // block the reader thread — the one place I-80 must not be
        // violated.
        //
        // Asserted DIRECTLY, on a counter the loop maintains, rather than
        // inferred from how many observations happened to be queued when
        // the test looked: with 700 bytes waiting the reader can
        // legitimately complete three whole select/read iterations before
        // any drain, so an observation count cannot tell one-read-per-select
        // from three-reads-in-one-select. It would pass for the wrong reason
        // on a fast machine.
        let (rx, mut tx) = os_pipe_pair();
        let reader = TriggerReader::open(&TriggerSpec::Fd(rx.as_raw_fd()), None).unwrap();
        tx.write_all(&[b'x'; 700]).unwrap(); // > 256: needs three reads to drain
        wait_for_observations(&reader, 3);

        assert_eq!(
            reader.max_reads_per_select_for_test(),
            1,
            "a fd: source must take exactly one read per select"
        );
        drop(tx);
    }

    #[test]
    fn a_fifo_source_does_drain_within_one_select() {
        // The other side of the same flag, so `drainable` cannot be quietly
        // false everywhere and still pass its own test suite.
        let dir = tempfile::tempdir().unwrap();
        let (reader, mut writer) = fifo_pair(dir.path());
        std::thread::sleep(Duration::from_millis(120));
        writer.write_all(&[b'x'; 700]).unwrap();
        wait_for_observations(&reader, 3);

        assert!(
            reader.max_reads_per_select_for_test() > 1,
            "an owned non-blocking fifo drains while readable"
        );
    }
}
