use std::io::Write;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::{Duration, Instant};

use anyhow::{Context, anyhow};
use crossterm::tty::IsTty;

use crate::cli::WatchArgs;
use crate::color::{ColorProfile, SystemEnv};
use crate::core::duration::parse_interval;
use crate::core::pager::resolve_pager;
use crate::exit::{AppError, AppResult};
use crate::style_spec::StyleSpec;
use crate::term::inline::{InlineRenderer, truncate_to_rows};
use crate::term::tty::{ConsoleUtf8Guard, RawModeGuard};
use crate::ui::key::{Key, from_crossterm};

pub fn run(args: WatchArgs, profile: ColorProfile) -> AppResult {
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

    let mut previous_key: Option<(u64, u16, u16)> = None;
    let mut full_lines: Vec<String> = Vec::new();
    let mut notice: Option<String> = None;
    loop {
        let output = run_child(&args, interactive)?;
        let mut combined = output.stdout.clone();
        combined.extend_from_slice(&output.stderr);
        let hash = signature(&combined);
        // The terminal size joins the change key: a resize must repaint
        // even when the content is unchanged.
        let size = crossterm::terminal::size().unwrap_or((80, 24));
        if previous_key != Some((hash, size.0, size.1)) || notice.is_some() {
            previous_key = Some((hash, size.0, size.1));
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
        loop {
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
            if !crossterm::event::poll(nap).context("polling events")? {
                continue;
            }
            let event = crossterm::event::read().context("reading event")?;
            let crossterm::event::Event::Key(key_event) = event else {
                continue;
            };
            match from_crossterm(key_event) {
                Some(Key::CtrlC) => {
                    renderer.finish().context("restoring terminal")?;
                    return Err(AppError::Aborted);
                }
                Some(Key::Char('q')) => {
                    renderer.finish().context("restoring terminal")?;
                    return Ok(());
                }
                Some(Key::Char('v')) | Some(Key::Enter) => {
                    notice = page_frame(&full_lines, &mut renderer);
                    // Repaint immediately with fresh data.
                    previous_key = None;
                    break;
                }
                _ => {}
            }
        }
    }
    renderer.finish().context("restoring terminal")?;
    Ok(())
}

/// Hand the full untruncated frame to the user's pager (RAT_PAGER, PAGER,
/// then less -R), bat-style. The loop resumes when the pager exits; a
/// failure to launch becomes a status line, never an error exit.
fn page_frame(
    lines: &[String],
    renderer: &mut InlineRenderer<std::io::StdoutLock<'static>>,
) -> Option<String> {
    let pager = resolve_pager(&SystemEnv)?;
    let _ = crossterm::terminal::disable_raw_mode();
    let _ = renderer.finish();
    // The pager inherits the console; keep it decoding UTF-8 while the
    // pager owns the screen (more.com garbles the frame otherwise).
    let _console_utf8 = ConsoleUtf8Guard::enable();

    let result = (|| -> std::io::Result<()> {
        let mut child = std::process::Command::new(&pager.bin)
            .args(&pager.args)
            .stdin(std::process::Stdio::piped())
            .spawn()?;
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
    renderer.reset();
    match result {
        Ok(()) => None,
        Err(err) => Some(format!(
            "pager {:?} failed ({err}) — set RAT_PAGER or install less",
            pager.bin
        )),
    }
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
fn run_child(args: &WatchArgs, interactive: bool) -> Result<ChildOutput, AppError> {
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
