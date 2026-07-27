use std::io::{Read, Write};
use std::time::{Duration, Instant};

use anyhow::{Context, anyhow};

use crate::cli::SpinArgs;
use crate::color::ColorProfile;
use crate::core::duration::parse_interval;
use crate::exit::{AppError, AppResult};
use crate::term::inline::InlineRenderer;
use crate::term::tty::UiStream;
use crate::theme::Palette;

pub fn run(args: SpinArgs, profile: ColorProfile, _palette: Palette) -> AppResult {
    let timeout = args.timeout.as_deref().map(parse_interval).transpose()?;

    let mut command = std::process::Command::new(&args.command[0]);
    command
        .args(&args.command[1..])
        .stdin(std::process::Stdio::inherit())
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::piped());
    // Children colorize for a pipe they cannot see through; force color when
    // our own profile has it. (gum solves this with a PTY; we document it.)
    if profile != ColorProfile::Ascii {
        command.env("CLICOLOR_FORCE", "1");
    }
    let mut child = command
        .spawn()
        .map_err(|err| anyhow!("running {:?}: {err}", args.command[0]))?;

    // Drain both pipes on threads so the child never blocks on a full pipe.
    let mut child_stdout = child.stdout.take().expect("stdout piped");
    let mut child_stderr = child.stderr.take().expect("stderr piped");
    let stdout_thread = std::thread::spawn(move || {
        let mut buf = Vec::new();
        let _ = child_stdout.read_to_end(&mut buf);
        buf
    });
    let stderr_thread = std::thread::spawn(move || {
        let mut buf = Vec::new();
        let _ = child_stderr.read_to_end(&mut buf);
        buf
    });

    let ui = UiStream::open();
    let animate = ui.is_tty();
    let mut renderer = InlineRenderer::new(ui)
        .with_cursor_hidden(animate)
        .with_sync_output(animate);

    let started = Instant::now();
    let mut tick: u64 = 0;
    let mut timed_out = false;
    let status = loop {
        if let Some(status) = child.try_wait().context("waiting for child")? {
            break status;
        }
        if let Some(limit) = timeout
            && started.elapsed() >= limit
        {
            let _ = child.kill();
            let _ = child.wait();
            timed_out = true;
            break std::process::ExitStatus::default();
        }
        if animate {
            let frame = format!("{} {}", args.spinner.frame(tick), args.title);
            let (cols, _) = crossterm::terminal::size().unwrap_or((80, 24));
            renderer.draw(&[frame], cols).context("painting spinner")?;
            tick += 1;
        }
        std::thread::sleep(Duration::from_millis(80));
    };
    if animate {
        renderer.clear().context("clearing spinner")?;
    }
    renderer.finish().context("restoring terminal")?;

    let stdout_bytes = stdout_thread.join().unwrap_or_default();
    let stderr_bytes = stderr_thread.join().unwrap_or_default();

    if timed_out {
        return Err(AppError::Timeout);
    }
    let code = status.code().unwrap_or(1); // signal death becomes 1, like gum
    let failed = code != 0;
    let show_stdout = args.show_output || args.show_stdout || (args.show_error && failed);
    let show_stderr = args.show_output || args.show_stderr || (args.show_error && failed);
    if show_stdout {
        std::io::stdout()
            .write_all(&stdout_bytes)
            .context("writing child stdout")?;
    }
    if show_stderr {
        std::io::stderr()
            .write_all(&stderr_bytes)
            .context("writing child stderr")?;
    }
    if failed {
        return Err(AppError::Child(code));
    }
    Ok(())
}
