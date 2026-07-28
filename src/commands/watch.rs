use std::io::Write;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::{Duration, Instant};

use anyhow::{Context, anyhow};
use crossterm::tty::IsTty;

use crate::cli::WatchArgs;
use crate::color::{ColorProfile, SystemEnv};
use crate::core::duration::parse_interval;
use crate::core::pager::{PagerCommand, resolve_pagers};
use crate::exit::{AppError, AppResult};
use crate::style_spec::StyleSpec;
use crate::term::inline::{InlineRenderer, truncate_to_rows};
use crate::term::tap::TapEvent;
#[cfg(unix)]
use crate::term::tap::{TapScanner, TtyTap};
#[cfg(unix)]
use crate::term::theme_notify::{OscColorKind, ThemeNotifyGuard, classify_colors, may_subscribe};
use crate::term::tty::{ConsoleUtf8Guard, RawModeGuard};
use crate::theme::{Appearance, AppearanceSource, Palette};
use crate::ui::key::{Key, from_crossterm};

// The palette follows the terminal only where the reader can see its
// reports; elsewhere it stays the startup verdict for the whole run.
#[cfg_attr(windows, allow(unused_mut))]
pub fn run(args: WatchArgs, profile: ColorProfile, mut palette: Palette) -> AppResult {
    let interval = parse_interval(&args.interval)?;
    let (interrupted, terminated) = register_signals()?;

    let stdout = std::io::stdout();
    let is_tty = stdout.is_tty();
    // Framing only makes sense on a terminal; piped output gets the plain
    // content so `rat watch | tee log` stays readable.
    let mut renderer = InlineRenderer::new(stdout.lock())
        .with_cursor_hidden(is_tty && !args.no_hide_cursor)
        .with_sync_output(is_tty && !args.no_sync)
        .with_clear_screen(is_tty && args.clear);

    // The looping tty mode reads keys (q quits, v pages the full frame), so
    // it owns the terminal input; children must not compete for it.
    let interactive = is_tty && !args.once;
    let _raw_guard = if interactive {
        Some(RawModeGuard::enable().context("enabling raw mode")?)
    } else {
        None
    };
    // Watch owns the terminal's input while it loops. On unix it reads the
    // device itself: the terminal can send escape sequences unprompted, and
    // those have to be parsed by whoever owns the input stream. Exactly one
    // reader is attached at a time — see the pager arm below.
    #[cfg(unix)]
    let tap = if interactive {
        // When the device cannot be opened, this run keeps the event
        // library's pump instead.
        TtyTap::spawn().ok()
    } else {
        None
    };
    // Declared outside the tick loop so a report split across a tick
    // boundary still reassembles.
    #[cfg(unix)]
    let mut scanner = TapScanner::new();
    // Declared after the raw-mode guard so it drops first: the unsubscribe
    // is written while echo is still suppressed. The terminal only pushes
    // theme changes while this is live, and only the reader above ever
    // sees them.
    #[cfg(unix)]
    let mut theme_sub = may_subscribe(palette.source, profile, interactive && tap.is_some())
        .then(|| ThemeNotifyGuard::subscribe(std::io::stdout()))
        .transpose()
        .context("subscribing to theme notifications")?;
    #[cfg(unix)]
    let mut verify = VerifyState::default();

    let title_line = args.title.as_ref().map(|title| {
        StyleSpec {
            bold: true,
            ..StyleSpec::default()
        }
        .render(title, profile)
    });
    let faint = StyleSpec {
        faint: true,
        ..StyleSpec::default()
    };

    let mut previous_key: Option<(u64, u16, u16, Appearance)> = None;
    let mut full_lines: Vec<String> = Vec::new();
    let mut notice: Option<String> = None;
    loop {
        let output = run_child(&args, interactive, palette.appearance)?;
        let mut combined = output.stdout.clone();
        combined.extend_from_slice(&output.stderr);
        let hash = signature(&combined);
        // The terminal size joins the change key: a resize must repaint
        // even when the content is unchanged. So does the appearance: a
        // palette swap must repaint even when the child prints the same
        // bytes.
        let size = crossterm::terminal::size().unwrap_or((80, 24));
        if previous_key != Some((hash, size.0, size.1, palette.appearance)) || notice.is_some() {
            previous_key = Some((hash, size.0, size.1, palette.appearance));
            let body = String::from_utf8_lossy(&output.stdout);
            let mut lines: Vec<String> = Vec::new();
            if let Some(title) = &title_line {
                lines.push(title.clone());
            }
            lines.extend(body.trim_end_matches('\n').split('\n').map(str::to_string));
            // Child stderr joins the frame; a raw write to the terminal
            // would shift the cursor and corrupt the relative repaint.
            if is_tty && !output.stderr.is_empty() {
                let err_body = String::from_utf8_lossy(&output.stderr);
                lines.extend(
                    err_body
                        .trim_end_matches('\n')
                        .split('\n')
                        .map(str::to_string),
                );
            }
            if is_tty {
                full_lines.clone_from(&lines);
                let (cols, rows) = size;
                let max_rows = args.max_height.unwrap_or_else(|| rows.saturating_sub(2));
                let (mut kept, hidden) = truncate_to_rows(lines, max_rows, cols);
                if hidden > 0 {
                    kept.push(faint.render(
                        &format!("… {hidden} more lines · v views all · q quits"),
                        profile,
                    ));
                }
                if let Some(text) = notice.take() {
                    kept.push(faint.render(&text, profile));
                }
                renderer.draw(&kept, cols).context("writing frame")?;
            } else {
                let mut out = std::io::stdout().lock();
                for line in &lines {
                    writeln!(out, "{line}").context("writing")?;
                }
                out.flush().context("flushing")?;
                // Piped mode keeps the streams separate for log readability.
                if !output.stderr.is_empty() {
                    let mut err = std::io::stderr().lock();
                    err.write_all(&output.stderr).context("writing stderr")?;
                    err.flush().context("flushing stderr")?;
                }
            }
        }

        if args.once {
            break;
        }
        // Wait for the next tick in slices, watching signals and (on a tty)
        // the keyboard.
        let deadline = Instant::now() + interval;
        'wait: loop {
            if interrupted.load(Ordering::Relaxed) {
                renderer.finish().context("restoring terminal")?;
                return Err(AppError::Aborted);
            }
            if terminated.load(Ordering::Relaxed) {
                renderer.finish().context("restoring terminal")?;
                return Ok(());
            }
            let now = Instant::now();
            if now >= deadline {
                break;
            }
            let nap = (deadline - now).min(Duration::from_millis(50));
            if !interactive {
                std::thread::sleep(nap);
                continue;
            }
            #[cfg(unix)]
            if let Some(sub) = theme_sub.as_mut() {
                if verify.pending && verify.in_flight_until.is_none() {
                    verify.pending = false;
                    // Ask once, and only while we own the input: the
                    // replies land in our own reader and nowhere else.
                    if sub.request_colors().is_ok() {
                        verify.fg = None;
                        verify.in_flight_until = Some(Instant::now() + crate::theme::PROBE_TIMEOUT);
                    }
                }
                if verify
                    .in_flight_until
                    .is_some_and(|until| Instant::now() >= until)
                {
                    // The terminal did not answer. A later report can arm
                    // another exchange.
                    verify.in_flight_until = None;
                    verify.fg = None;
                }
            }
            #[cfg(unix)]
            let events = match tap.as_ref() {
                Some(tap) => match tap.recv_timeout(nap) {
                    Some(chunk) => scanner.feed(&chunk),
                    None => scanner.idle(nap),
                },
                None => crossterm_slice(nap)?,
            };
            #[cfg(windows)]
            let events = crossterm_slice(nap)?;
            for event in events {
                match event {
                    TapEvent::Key(Key::CtrlC) => {
                        renderer.finish().context("restoring terminal")?;
                        return Err(AppError::Aborted);
                    }
                    TapEvent::Key(Key::Char('q')) => {
                        renderer.finish().context("restoring terminal")?;
                        return Ok(());
                    }
                    TapEvent::Key(Key::Char('v')) | TapEvent::Key(Key::Enter) => {
                        // The pager reads the same terminal. Stop the
                        // pushes first, then park our reader, so a report
                        // can never land in a foreign reader's input.
                        #[cfg(unix)]
                        if let Some(sub) = theme_sub.as_mut() {
                            let _ = sub.suspend();
                        }
                        // Park our reader and require its confirmation
                        // before handing the input stream over.
                        // Unconfirmed means a reader may still be attached
                        // — never spawn a second one against it.
                        #[cfg(unix)]
                        let handed_off = tap.as_ref().is_none_or(|tap| tap.pause());
                        #[cfg(windows)]
                        let handed_off = true;
                        notice = if handed_off {
                            page_frame(&full_lines, &mut renderer)
                        } else {
                            Some(
                                "pager unavailable: the input reader did not yield; try again"
                                    .to_string(),
                            )
                        };
                        // Reader first, then pushes: a report always has
                        // someone to read it.
                        #[cfg(unix)]
                        {
                            if let Some(tap) = tap.as_ref() {
                                tap.resume();
                            }
                            if let Some(sub) = theme_sub.as_mut() {
                                let _ = sub.resume();
                            }
                            // Whatever was in flight belongs to a terminal
                            // state we stopped listening to.
                            verify = VerifyState::default();
                        }
                        // Repaint immediately with fresh data.
                        previous_key = None;
                        break 'wait;
                    }
                    // The reported value is ignored on purpose: it can
                    // disagree with the colors actually on screen. Every
                    // report re-arms, including one that arrives while an
                    // exchange is already out — a terminal's first report
                    // of a change can be measured too early.
                    #[cfg(unix)]
                    TapEvent::ThemeNotification(_) => {
                        verify.pending = true;
                    }
                    #[cfg(unix)]
                    TapEvent::OscColor(kind, color) => {
                        if let Some(verdict) = verify.reply(kind, color)
                            && adopt(&mut palette, verdict)
                        {
                            if std::env::var_os("RAT_DEBUG_APPEARANCE").is_some() {
                                notice = Some(format!("appearance → {}", verdict.as_str()));
                            }
                            // Repaint now rather than at the next tick.
                            break 'wait;
                        }
                    }
                    // Every other key, as before.
                    _ => {}
                }
            }
        }
    }
    renderer.finish().context("restoring terminal")?;
    Ok(())
}

/// Hand the full untruncated frame to the user's pager (RAT_PAGER, PAGER,
/// then less -R, then more.com on Windows), bat-style. The loop resumes
/// when the pager exits; a failure to launch becomes a status line, never
/// an error exit.
fn page_frame(
    lines: &[String],
    renderer: &mut InlineRenderer<std::io::StdoutLock<'static>>,
) -> Option<String> {
    let pagers = resolve_pagers(&SystemEnv);
    let mut used = pagers.first().map(|p| p.bin.clone()).unwrap_or_default();
    let _ = crossterm::terminal::disable_raw_mode();
    let _ = renderer.finish();
    // The pager inherits the console; keep it decoding UTF-8 while the
    // pager owns the screen (more.com garbles the frame otherwise).
    let _console_utf8 = ConsoleUtf8Guard::enable();

    let result = (|| -> std::io::Result<()> {
        let (bin, mut child) = spawn_first(&pagers)?;
        used = bin;
        // Quitting the pager before it reads everything is normal; do not
        // let the default SIGPIPE disposition kill the watch for it.
        #[cfg(unix)]
        unsafe {
            libc::signal(libc::SIGPIPE, libc::SIG_IGN);
        }
        let write_result = (|| -> std::io::Result<()> {
            let mut stdin = child.stdin.take().expect("stdin piped");
            for line in lines {
                writeln!(stdin, "{line}")?;
            }
            Ok(())
        })();
        #[cfg(unix)]
        unsafe {
            libc::signal(libc::SIGPIPE, libc::SIG_DFL);
        }
        match write_result {
            Err(err) if err.kind() != std::io::ErrorKind::BrokenPipe => return Err(err),
            _ => {}
        }
        child.wait()?;
        Ok(())
    })();

    let _ = crossterm::terminal::enable_raw_mode();
    renderer.resume_over_own_frame();
    match result {
        Ok(()) => None,
        Err(err) => Some(format!(
            "pager {used:?} failed ({err}) — set RAT_PAGER or install less"
        )),
    }
}

/// Spawn the first launchable pager candidate; on Windows the default chain
/// ends in the stock more.com, so this only fails when every candidate is
/// missing (or a configured pager is).
fn spawn_first(pagers: &[PagerCommand]) -> std::io::Result<(String, std::process::Child)> {
    let mut last_err =
        std::io::Error::new(std::io::ErrorKind::NotFound, "no pager candidates resolved");
    for pager in pagers {
        match std::process::Command::new(&pager.bin)
            .args(&pager.args)
            .stdin(std::process::Stdio::piped())
            .spawn()
        {
            Ok(child) => return Ok((pager.bin.clone(), child)),
            Err(err) => last_err = err,
        }
    }
    Err(last_err)
}

struct ChildOutput {
    stdout: Vec<u8>,
    stderr: Vec<u8>,
}

/// Run one tick of the child, capturing both streams. On a tty the stderr
/// joins the painted frame (raw terminal writes would corrupt the repaint);
/// piped mode forwards it to our stderr. Interactive mode nulls the child's
/// stdin so it cannot eat keystrokes. Loop mode renders spawn failures as
/// content so a transient failure does not tear down the dashboard; once
/// mode fails loudly.
fn run_child(
    args: &WatchArgs,
    interactive: bool,
    appearance: Appearance,
) -> Result<ChildOutput, AppError> {
    let mut command = if args.shell {
        shell_command(&args.command.join(" "))
    } else {
        let mut cmd = std::process::Command::new(&args.command[0]);
        cmd.args(&args.command[1..]);
        cmd
    };
    if interactive {
        command.stdin(std::process::Stdio::null());
    }
    // Children lay out against the frame without a tty side channel: the
    // size is re-measured every tick, so scripts adapt to resizes live.
    let (cols, rows) = crossterm::terminal::size().unwrap_or((80, 24));
    command.env("RAT_WIDTH", cols.to_string());
    command.env("RAT_HEIGHT", rows.to_string());
    // Children inherit the controlling terminal, so a child that resolved its
    // own appearance would query a terminal this process is reading from.
    // Hand it the verdict instead.
    command.env("RAT_APPEARANCE", appearance.as_str());
    match command.output() {
        Ok(out) => Ok(ChildOutput {
            stdout: out.stdout,
            stderr: out.stderr,
        }),
        Err(err) => {
            if args.once {
                Err(anyhow!("running {:?}: {err}", args.command[0]).into())
            } else {
                Ok(ChildOutput {
                    stdout: format!("watch: {:?}: {err}", args.command[0]).into_bytes(),
                    stderr: Vec::new(),
                })
            }
        }
    }
}

/// Unix: restore the terminal on INT/TERM/HUP. Windows: the interactive
/// path reads Ctrl-C as a key event; piped mode has no terminal state to
/// restore, so default console handling suffices.
#[cfg(unix)]
fn register_signals() -> Result<(Arc<AtomicBool>, Arc<AtomicBool>), AppError> {
    let interrupted = Arc::new(AtomicBool::new(false));
    let terminated = Arc::new(AtomicBool::new(false));
    signal_hook::flag::register(signal_hook::consts::SIGINT, Arc::clone(&interrupted))
        .context("registering SIGINT")?;
    signal_hook::flag::register(signal_hook::consts::SIGTERM, Arc::clone(&terminated))
        .context("registering SIGTERM")?;
    signal_hook::flag::register(signal_hook::consts::SIGHUP, Arc::clone(&terminated))
        .context("registering SIGHUP")?;
    Ok((interrupted, terminated))
}

#[cfg(windows)]
fn register_signals() -> Result<(Arc<AtomicBool>, Arc<AtomicBool>), AppError> {
    Ok((
        Arc::new(AtomicBool::new(false)),
        Arc::new(AtomicBool::new(false)),
    ))
}

/// One wait slice through the event library's pump: the Windows input path,
/// and the unix fallback when the terminal device cannot be opened. Non-key
/// events are discarded exactly as they were before the split.
fn crossterm_slice(nap: Duration) -> anyhow::Result<Vec<TapEvent>> {
    if !crossterm::event::poll(nap).context("polling events")? {
        return Ok(Vec::new());
    }
    let crossterm::event::Event::Key(key_event) =
        crossterm::event::read().context("reading event")?
    else {
        return Ok(Vec::new());
    };
    match from_crossterm(key_event) {
        Some(key) => Ok(vec![TapEvent::Key(key)]),
        None => Ok(Vec::new()),
    }
}

/// A terminal's theme notification says *that* something changed, not what
/// the colors now are — it reports the application's appearance, which can
/// disagree with the palette actually on screen. So a notification only
/// arms a measurement: one foreground/background exchange, classified by
/// the same rule the startup probe uses.
#[cfg(unix)]
#[derive(Default)]
struct VerifyState {
    pending: bool,
    fg: Option<xterm_color::Color>,
    in_flight_until: Option<Instant>,
}

#[cfg(unix)]
impl VerifyState {
    /// Feed one color reply. Returns the verdict once the background reply
    /// completes an exchange this loop actually asked for.
    fn reply(&mut self, kind: OscColorKind, color: xterm_color::Color) -> Option<Appearance> {
        // A reply nobody asked for is never adopted.
        self.in_flight_until?;
        match kind {
            OscColorKind::Foreground => {
                self.fg = Some(color);
                None
            }
            OscColorKind::Background => {
                self.in_flight_until = None;
                let verdict = classify_colors(self.fg.as_ref(), &color);
                self.fg = None;
                Some(verdict)
            }
        }
    }
}

/// Adopt an appearance the terminal reported. Returns true when the verdict
/// actually changed; a repeat is a no-op, so a terminal that re-announces an
/// unchanged theme costs nothing.
#[cfg_attr(windows, allow(dead_code))] // Called from the unix input path.
fn adopt(palette: &mut Palette, reported: Appearance) -> bool {
    if palette.appearance == reported {
        return false;
    }
    *palette = Palette::builtin(reported, AppearanceSource::Notification);
    true
}

fn signature(bytes: &[u8]) -> u64 {
    use std::hash::{Hash, Hasher};
    let mut hasher = std::hash::DefaultHasher::new();
    bytes.hash(&mut hasher);
    hasher.finish()
}

#[cfg(unix)]
fn shell_command(script: &str) -> std::process::Command {
    let mut cmd = std::process::Command::new("sh");
    cmd.arg("-c").arg(script);
    cmd
}

#[cfg(windows)]
fn shell_command(script: &str) -> std::process::Command {
    let shell = std::env::var("COMSPEC").unwrap_or_else(|_| "cmd".to_string());
    let mut cmd = std::process::Command::new(shell);
    cmd.arg("/C").arg(script);
    cmd
}

#[cfg(test)]
mod tests {
    use ratatui::style::Color;

    use super::*;

    #[test]
    fn adopting_a_different_appearance_reresolves_the_palette() {
        let mut palette = Palette::builtin(Appearance::Dark, AppearanceSource::Osc);
        assert!(adopt(&mut palette, Appearance::Light));
        assert_eq!(palette.appearance, Appearance::Light);
        assert_eq!(palette.source, AppearanceSource::Notification);
        assert_eq!(palette.accent, Color::Indexed(129));
    }

    #[test]
    fn adopting_the_current_appearance_changes_nothing() {
        let mut palette = Palette::builtin(Appearance::Dark, AppearanceSource::Osc);
        assert!(!adopt(&mut palette, Appearance::Dark));
        // Not even the provenance moves: a repeat report is not a new
        // verdict, and `doctor` must keep reporting how the palette was
        // actually reached.
        assert_eq!(palette.source, AppearanceSource::Osc);
        assert_eq!(palette.accent, Color::Indexed(212));
    }

    #[test]
    fn adopting_back_restores_the_original_tokens() {
        let mut palette = Palette::builtin(Appearance::Dark, AppearanceSource::Osc);
        assert!(adopt(&mut palette, Appearance::Light));
        assert!(adopt(&mut palette, Appearance::Dark));
        assert_eq!(palette.appearance, Appearance::Dark);
        assert_eq!(palette.accent, Color::Indexed(212));
        assert_eq!(palette.on_accent, Color::Indexed(16));
    }

    #[cfg(unix)]
    fn white() -> xterm_color::Color {
        xterm_color::Color::rgb(u16::MAX, u16::MAX, u16::MAX)
    }

    #[cfg(unix)]
    fn black() -> xterm_color::Color {
        xterm_color::Color::rgb(0, 0, 0)
    }

    #[cfg(unix)]
    #[test]
    fn a_reply_nobody_asked_for_is_ignored() {
        let mut verify = VerifyState::default();
        assert_eq!(verify.reply(OscColorKind::Background, black()), None);
    }

    #[cfg(unix)]
    #[test]
    fn a_background_reply_completes_the_exchange() {
        use crate::theme::PROBE_TIMEOUT;

        let mut verify = VerifyState {
            in_flight_until: Some(Instant::now() + PROBE_TIMEOUT),
            ..VerifyState::default()
        };
        assert_eq!(verify.reply(OscColorKind::Foreground, white()), None);
        assert_eq!(
            verify.reply(OscColorKind::Background, black()),
            Some(Appearance::Dark)
        );
        // The exchange is over: a straggler cannot move the verdict again.
        assert!(verify.in_flight_until.is_none());
        assert_eq!(verify.reply(OscColorKind::Background, white()), None);
    }

    #[cfg(unix)]
    #[test]
    fn a_background_reply_alone_still_classifies() {
        use crate::theme::PROBE_TIMEOUT;

        let mut verify = VerifyState {
            in_flight_until: Some(Instant::now() + PROBE_TIMEOUT),
            ..VerifyState::default()
        };
        assert_eq!(
            verify.reply(OscColorKind::Background, white()),
            Some(Appearance::Light)
        );
    }
}
