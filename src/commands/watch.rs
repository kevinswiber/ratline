use std::io::Write;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};

use anyhow::{Context, anyhow};
use crossterm::tty::IsTty;

use crate::cli::WatchArgs;
use crate::color::ColorProfile;
use crate::core::duration::parse_interval;
use crate::exit::{AppError, AppResult};
use crate::style_spec::StyleSpec;
use crate::term::inline::{InlineRenderer, truncate_to_rows};

pub fn run(args: WatchArgs, profile: ColorProfile) -> AppResult {
    let interval = parse_interval(&args.interval)?;
    let interrupted = Arc::new(AtomicBool::new(false));
    let terminated = Arc::new(AtomicBool::new(false));
    signal_hook::flag::register(signal_hook::consts::SIGINT, Arc::clone(&interrupted))
        .context("registering SIGINT")?;
    signal_hook::flag::register(signal_hook::consts::SIGTERM, Arc::clone(&terminated))
        .context("registering SIGTERM")?;
    signal_hook::flag::register(signal_hook::consts::SIGHUP, Arc::clone(&terminated))
        .context("registering SIGHUP")?;

    let stdout = std::io::stdout();
    let is_tty = stdout.is_tty();
    // Framing only makes sense on a terminal; piped output gets the plain
    // content so `rat watch | tee log` stays readable.
    let mut renderer = InlineRenderer::new(stdout.lock())
        .with_cursor_hidden(is_tty && !args.no_hide_cursor)
        .with_sync_output(is_tty && !args.no_sync)
        .with_clear_screen(is_tty && args.clear);

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

    let mut previous_hash: Option<u64> = None;
    loop {
        let output = run_child(&args)?;
        let hash = signature(&output);
        if previous_hash != Some(hash) {
            previous_hash = Some(hash);
            let body = String::from_utf8_lossy(&output);
            let mut lines: Vec<String> = Vec::new();
            if let Some(title) = &title_line {
                lines.push(title.clone());
            }
            lines.extend(body.trim_end_matches('\n').split('\n').map(str::to_string));
            if is_tty {
                let (cols, rows) = crossterm::terminal::size().unwrap_or((80, 24));
                let max_rows = args.max_height.unwrap_or_else(|| rows.saturating_sub(2));
                let (mut kept, hidden) = truncate_to_rows(lines, max_rows, cols);
                if hidden > 0 {
                    kept.push(faint.render(&format!("… {hidden} more lines"), profile));
                }
                renderer.draw(&kept, cols).context("writing frame")?;
            } else {
                let mut out = std::io::stdout().lock();
                for line in &lines {
                    writeln!(out, "{line}").context("writing")?;
                }
                out.flush().context("flushing")?;
            }
        }

        if args.once {
            break;
        }
        // Sleep in slices so ctrl-c stays responsive.
        let mut remaining = interval;
        let slice = std::time::Duration::from_millis(50);
        while !remaining.is_zero() {
            if interrupted.load(Ordering::Relaxed) || terminated.load(Ordering::Relaxed) {
                break;
            }
            let nap = remaining.min(slice);
            std::thread::sleep(nap);
            remaining = remaining.saturating_sub(nap);
        }
        if interrupted.load(Ordering::Relaxed) {
            renderer.finish().context("restoring terminal")?;
            return Err(AppError::Aborted);
        }
        if terminated.load(Ordering::Relaxed) {
            break;
        }
    }
    renderer.finish().context("restoring terminal")?;
    Ok(())
}

/// Run one tick of the child, capturing stdout; stderr passes through.
/// Loop mode renders spawn/read failures as content so a transient failure
/// does not tear down the dashboard; once mode fails loudly.
fn run_child(args: &WatchArgs) -> Result<Vec<u8>, AppError> {
    let output = if args.shell {
        std::process::Command::new("sh")
            .arg("-c")
            .arg(args.command.join(" "))
            .stderr(std::process::Stdio::inherit())
            .output()
    } else {
        std::process::Command::new(&args.command[0])
            .args(&args.command[1..])
            .stderr(std::process::Stdio::inherit())
            .output()
    };
    match output {
        Ok(out) => Ok(out.stdout),
        Err(err) => {
            if args.once {
                Err(anyhow!("running {:?}: {err}", args.command[0]).into())
            } else {
                Ok(format!("watch: {:?}: {err}", args.command[0]).into_bytes())
            }
        }
    }
}

fn signature(bytes: &[u8]) -> u64 {
    use std::hash::{Hash, Hasher};
    let mut hasher = std::hash::DefaultHasher::new();
    bytes.hash(&mut hasher);
    hasher.finish()
}
