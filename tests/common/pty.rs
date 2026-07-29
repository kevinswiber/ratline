#![allow(dead_code)]
//! Minimal pty harness for tests that must observe rat's interactive path
//! (raw mode, /dev/tty, live escape-sequence injection). assert_cmd's piped
//! `Command` cannot exercise this path at all — see `tests/common/mod.rs`.

use std::ffi::CString;
use std::os::unix::io::RawFd;
use std::time::{Duration, Instant};

pub struct PtySession {
    master_fd: RawFd,
    child_pid: libc::pid_t,
}

impl PtySession {
    /// Forks `program` with `args` attached to a fresh pty as its
    /// controlling terminal on all three stdio streams. The child always
    /// gets `TERM=xterm-256color` and never inherits `RAT_APPEARANCE` or
    /// `COLORFGBG` from the test process's own environment — auto mode
    /// under test must reach the real probe, not an accidental pin — plus
    /// whatever `envs` the caller adds.
    ///
    /// `argv`/`envp` are built as C strings *before* `fork()`. `fork()`
    /// only guarantees the calling thread survives into the child; the
    /// test binary is multithreaded, so an allocator or lock call in the
    /// child can deadlock against a lock some other thread held at the
    /// instant of the fork. Building every C string in the parent and
    /// using `execve` with an explicit `envp` keeps the child's post-fork
    /// path to bare syscalls only.
    pub fn spawn(program: &str, args: &[&str], envs: &[(&str, &str)]) -> std::io::Result<Self> {
        let mut master: libc::c_int = -1;
        let mut slave: libc::c_int = -1;
        // A real window size: a null winp yields a 0x0 terminal, and a
        // frame renderer truncates everything against zero rows.
        let mut winsize = libc::winsize {
            ws_row: 24,
            ws_col: 80,
            ws_xpixel: 0,
            ws_ypixel: 0,
        };
        // A raw pointer rather than `&mut`: the winp parameter is
        // `*mut winsize` on macOS but `*const winsize` on Linux, and a
        // `*mut` coerces to both.
        let winp: *mut libc::winsize = &mut winsize;
        // SAFETY: out-params are stack locals; a null termp means "use the
        // platform's default modes," which every unix `openpty` accepts.
        let rc = unsafe {
            libc::openpty(
                &mut master,
                &mut slave,
                std::ptr::null_mut(),
                std::ptr::null_mut(),
                winp,
            )
        };
        if rc != 0 {
            return Err(std::io::Error::last_os_error());
        }
        // Non-blocking master: `read_available` polls first and only reads
        // once `POLLIN` is set, but a non-blocking fd turns any race into a
        // clean `EAGAIN` instead of a hang.
        unsafe {
            let flags = libc::fcntl(master, libc::F_GETFL);
            libc::fcntl(master, libc::F_SETFL, flags | libc::O_NONBLOCK);
        }

        let c_program = CString::new(program).expect("program has no NUL bytes");
        let argv_owned: Vec<CString> = std::iter::once(program)
            .chain(args.iter().copied())
            .map(|a| CString::new(a).expect("arg has no NUL bytes"))
            .collect();
        let mut argv: Vec<*const libc::c_char> = argv_owned.iter().map(|a| a.as_ptr()).collect();
        argv.push(std::ptr::null());

        let mut env_pairs: Vec<(String, String)> =
            vec![("TERM".to_string(), "xterm-256color".to_string())];
        env_pairs.extend(envs.iter().map(|(k, v)| (k.to_string(), v.to_string())));
        let envp_owned: Vec<CString> = env_pairs
            .iter()
            .map(|(k, v)| CString::new(format!("{k}={v}")).expect("env has no NUL bytes"))
            .collect();
        let mut envp: Vec<*const libc::c_char> = envp_owned.iter().map(|e| e.as_ptr()).collect();
        envp.push(std::ptr::null());

        let pid = unsafe { libc::fork() };
        if pid < 0 {
            let err = std::io::Error::last_os_error();
            unsafe {
                libc::close(master);
                libc::close(slave);
            }
            return Err(err);
        }

        if pid == 0 {
            // Child: syscalls only from here to exec. `login_tty(3)` makes
            // the slave our controlling terminal and dup2s it onto fd
            // 0/1/2 in one call (setsid + TIOCSCTTY + dup2 + close), so
            // all three stdio streams become the pty.
            unsafe {
                libc::close(master);
                if libc::login_tty(slave) != 0 {
                    libc::_exit(127);
                }
                libc::execve(c_program.as_ptr(), argv.as_ptr(), envp.as_ptr());
                // execve only returns on failure.
                libc::_exit(127);
            }
        }

        // Parent.
        unsafe {
            libc::close(slave);
        }
        Ok(PtySession {
            master_fd: master,
            child_pid: pid,
        })
    }

    /// Change the pty's window size mid-run — the resize the session
    /// under test observes on its next terminal-size read. The one
    /// addition dashboards needed here; everything else is shared with
    /// the watch suite unchanged.
    pub fn set_winsize(&self, rows: u16, cols: u16) {
        let winsize = libc::winsize {
            ws_row: rows,
            ws_col: cols,
            ws_xpixel: 0,
            ws_ypixel: 0,
        };
        // SAFETY: master_fd is a live pty master for self's lifetime;
        // the winsize is a stack local the call copies from.
        unsafe {
            libc::ioctl(self.master_fd, libc::TIOCSWINSZ as _, &winsize);
        }
    }

    pub fn write_bytes(&self, bytes: &[u8]) {
        unsafe {
            libc::write(self.master_fd, bytes.as_ptr().cast(), bytes.len());
        }
    }

    /// Read whatever arrives within `timeout`, polling in short slices.
    /// Distinguishes a spurious `EAGAIN` (keep polling) from real
    /// EOF/error (stop) so a non-blocking master never gets misread as
    /// "child gone."
    pub fn read_available(&self, timeout: Duration) -> Vec<u8> {
        let deadline = Instant::now() + timeout;
        let mut out = Vec::new();
        loop {
            let now = Instant::now();
            if now >= deadline {
                break;
            }
            let remaining_ms = (deadline - now).as_millis().min(i32::MAX as u128) as i32;
            let mut fds = [libc::pollfd {
                fd: self.master_fd,
                events: libc::POLLIN,
                revents: 0,
            }];
            let n = unsafe { libc::poll(fds.as_mut_ptr(), 1, remaining_ms) };
            if n <= 0 {
                break; // timed out, or a poll error — return what we have
            }
            if fds[0].revents & libc::POLLIN == 0 {
                break; // POLLHUP/POLLERR with nothing left to read
            }
            let mut buf = [0u8; 4096];
            let read = unsafe { libc::read(self.master_fd, buf.as_mut_ptr().cast(), buf.len()) };
            if read > 0 {
                out.extend_from_slice(&buf[..read as usize]);
                // Return what arrived rather than draining until the
                // deadline: callers scan incrementally and re-call.
                break;
            }
            if read == 0 {
                break; // EOF: the child closed its end
            }
            let err = std::io::Error::last_os_error();
            if err.kind() != std::io::ErrorKind::WouldBlock {
                break; // a real error (e.g. EIO once the slave is gone)
            }
            // EAGAIN/EWOULDBLOCK: spurious wakeup, keep polling.
        }
        out
    }

    /// Kills and reaps the child if it hasn't exited by `deadline`.
    /// Returns whether it was still alive — never rely on a wedged child
    /// to notice its own hang.
    pub fn kill_if_alive(&self, deadline: Duration) -> bool {
        let start = Instant::now();
        loop {
            let mut status: libc::c_int = 0;
            let rc = unsafe { libc::waitpid(self.child_pid, &mut status, libc::WNOHANG) };
            if rc == self.child_pid {
                return false; // exited on its own
            }
            if Instant::now().duration_since(start) >= deadline {
                unsafe {
                    libc::kill(self.child_pid, libc::SIGKILL);
                }
                // Even a SIGKILLed child cannot be reaped while it is
                // blocked in the tty driver flushing output nobody reads;
                // drain until the reap lands.
                loop {
                    let _ = self.read_available(Duration::from_millis(20));
                    let rc = unsafe { libc::waitpid(self.child_pid, &mut status, libc::WNOHANG) };
                    if rc == self.child_pid {
                        return true;
                    }
                }
            }
            // A terminal always reads: draining doubles as the loop's
            // pacing, and an exiting child may be blocked mid-write until
            // its final escape sequences are consumed.
            let _ = self.read_available(Duration::from_millis(20));
        }
    }
}

impl Drop for PtySession {
    fn drop(&mut self) {
        unsafe {
            let mut status: libc::c_int = 0;
            // Reap if already exited (the common case: `kill_if_alive` or
            // a clean quit already did this). If the child is still alive
            // — a test that panicked before checking — force it so no test
            // run leaves an orphaned process behind.
            let reaped = libc::waitpid(self.child_pid, &mut status, libc::WNOHANG);
            if reaped == 0 {
                libc::kill(self.child_pid, libc::SIGKILL);
                // Same drain-to-reap dance as `kill_if_alive`: the child
                // may be blocked writing to a slave nobody reads.
                loop {
                    let _ = self.read_available(Duration::from_millis(20));
                    let rc = libc::waitpid(self.child_pid, &mut status, libc::WNOHANG);
                    if rc == self.child_pid {
                        break;
                    }
                }
            }
            libc::close(self.master_fd);
        }
    }
}

/// Answers the queries a terminal is asked mid-session: `OSC 10 ; ?`
/// (foreground), `OSC 11 ; ?` (background), and DA1 (`CSI c`). Tolerant of
/// either a BEL or an ST terminator on the query side; always replies
/// ST-terminated.
pub struct FakeTerminal {
    fg: String,
    bg: String,
    carry: Vec<u8>,
}

const CARRY_CAP: usize = 64;

impl FakeTerminal {
    pub fn dark() -> Self {
        Self::new("rgb:ffff/ffff/ffff", "rgb:1e1e/1e1e/2e2e")
    }

    pub fn light() -> Self {
        Self::new("rgb:0000/0000/0000", "rgb:ffff/ffff/ffff")
    }

    fn new(fg: &str, bg: &str) -> Self {
        FakeTerminal {
            fg: fg.to_string(),
            bg: bg.to_string(),
            carry: Vec::new(),
        }
    }

    /// Reconfigure the colors a later query is answered with — how a
    /// live-flip test simulates a theme change without respawning the
    /// session.
    pub fn set(&mut self, fg: &str, bg: &str) {
        self.fg = fg.to_string();
        self.bg = bg.to_string();
    }

    /// Feed newly read master-side bytes; answer any complete query found.
    /// Unmatched bytes carry into the next call (bounded) so a query split
    /// across two reads is still recognized. A matched query is drained
    /// from the carry so it cannot be answered twice, while a later,
    /// distinct occurrence of the same pattern still matches.
    pub fn respond(&mut self, session: &PtySession, chunk: &[u8]) {
        self.carry.extend_from_slice(chunk);
        if self.carry.len() > CARRY_CAP {
            let drop_from = self.carry.len() - CARRY_CAP;
            self.carry.drain(..drop_from);
        }
        let fg_reply = format!("\x1b]10;{}\x1b\\", self.fg);
        let bg_reply = format!("\x1b]11;{}\x1b\\", self.bg);
        self.answer(session, b"\x1b]10;?", &fg_reply);
        self.answer(session, b"\x1b]11;?", &bg_reply);
        self.answer(session, b"\x1b[c", "\x1b[?1;2c");
    }

    fn answer(&mut self, session: &PtySession, needle: &[u8], reply: &str) {
        if let Some(pos) = find(&self.carry, needle) {
            session.write_bytes(reply.as_bytes());
            self.carry.drain(..pos + needle.len());
        }
    }
}

fn find(haystack: &[u8], needle: &[u8]) -> Option<usize> {
    if needle.len() > haystack.len() {
        return None;
    }
    haystack.windows(needle.len()).position(|w| w == needle)
}

/// Read master output in short slices, answering `terminal`'s configured
/// queries as they arrive, until `needle` appears in what has been read so
/// far or `timeout` elapses. The ≤50ms slice keeps the responder answering
/// well inside the startup probe's own deadline.
pub fn wait_for(
    session: &PtySession,
    terminal: &mut FakeTerminal,
    needle: &[u8],
    timeout: Duration,
) -> bool {
    let deadline = Instant::now() + timeout;
    let mut seen: Vec<u8> = Vec::new();
    loop {
        let now = Instant::now();
        if now >= deadline {
            return false;
        }
        let slice = (deadline - now).min(Duration::from_millis(50));
        let chunk = session.read_available(slice);
        if chunk.is_empty() {
            continue;
        }
        terminal.respond(session, &chunk);
        seen.extend_from_slice(&chunk);
        if find(&seen, needle).is_some() {
            return true;
        }
    }
}

/// Create a named pipe — trigger tests drive watch through one.
pub fn mkfifo_at(path: &std::path::Path) {
    let cpath =
        std::ffi::CString::new(path.as_os_str().as_encoded_bytes().to_vec()).expect("fifo path");
    assert_eq!(unsafe { libc::mkfifo(cpath.as_ptr(), 0o600) }, 0, "mkfifo");
}

/// Open, write, close: one trigger fire.
pub fn write_fifo(path: &std::path::Path, bytes: &[u8]) {
    use std::io::Write;
    let mut writer = std::fs::OpenOptions::new()
        .write(true)
        .open(path)
        .expect("fifo write end");
    writer.write_all(bytes).expect("fifo write");
}

/// The sh line a counting watch child runs — pty tests are unix-only,
/// so sh is available (the sh-free rule binds the portable cli suite).
/// Prints `count-N` so screen assertions have a distinctive needle.
pub fn counter_cmd(path: &std::path::Path) -> String {
    format!(
        "echo run >> {p}; printf 'count-%s' $(wc -l < {p})",
        p = path.display()
    )
}

/// Child-side evidence: wait until the counter file records `n` runs.
pub fn wait_for_counter(path: &std::path::Path, n: usize) {
    let deadline = Instant::now() + Duration::from_secs(10);
    loop {
        let count = std::fs::read_to_string(path)
            .map(|s| s.lines().count())
            .unwrap_or(0);
        if count >= n {
            return;
        }
        assert!(
            Instant::now() < deadline,
            "counter stuck at {count}, wanted {n}"
        );
        std::thread::sleep(Duration::from_millis(20));
    }
}

/// The count must not move past `n` through one settle window, and must
/// end there — value-based, never a bare sleep assertion.
pub fn assert_counter_settled_at(path: &std::path::Path, n: usize) {
    let deadline = Instant::now() + Duration::from_millis(900);
    while Instant::now() < deadline {
        let count = std::fs::read_to_string(path)
            .map(|s| s.lines().count())
            .unwrap_or(0);
        assert!(count <= n, "counter moved past {n}: {count}");
        std::thread::sleep(Duration::from_millis(30));
    }
    let count = std::fs::read_to_string(path)
        .map(|s| s.lines().count())
        .unwrap_or(0);
    assert_eq!(count, n, "counter settled at the wrong value");
}
