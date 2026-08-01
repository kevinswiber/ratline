use std::io::Write;
use std::time::{Duration, Instant};

use anyhow::{Context, anyhow};

use crate::cli::SpinArgs;
use crate::color::ColorProfile;
use crate::core::duration::parse_interval;
use crate::core::retain::{Keep, Retention, compact_count, read_all};
use crate::exit::{AppError, AppResult};
use crate::term::inline::InlineRenderer;
use crate::term::tty::UiStream;
use crate::theme::Palette;

/// How much of the child's output `spin` retains, and from which end.
///
/// **Ten times what a watch pane keeps, because the bargain is
/// different.** A pane renders its own height and has scrollback behind
/// it, so a thousand lines is already more than it can show; `spin`
/// REPLAYS what it kept as the command's answer, and a line it drops is
/// a line the user asked for and does not get. Ten thousand covers a
/// full build or test log without truncating anything anyone would call
/// output, and still bounds a child that never stops — which is the
/// whole point, since an unbounded drain reached 4.4 GiB in 1.8 s
/// against `yes`.
///
/// **What this actually costs, stated honestly, because a line count is
/// not a byte budget.** Ordinary output runs about 1 MiB: terminal lines
/// average well under 100 bytes. The WORST case is far larger — a line
/// is dropped only past `retain::MAX_LINE_BYTES` (64 KiB), so a child emitting
/// ten thousand lines of that size retains **625 MiB per stream, 1.22
/// GiB across both**. Reaching it takes 625 MB of uniformly enormous
/// lines, which is pathological rather than merely large, and the same
/// input consumed unbounded memory before this bound existed. But it is
/// a ceiling of that height, not the couple of MiB the typical case
/// suggests, and anyone raising `max_lines` is multiplying that number.
///
/// The tail survives. The dominant use is `--show-error`, and why a
/// command failed is usually the last thing it said. This costs the
/// case where a compiler dies on its first error and prints a wall of
/// cascade after it; the notice at least says so.
const RETENTION: Retention = Retention {
    max_lines: 10_000,
    keep: Keep::Bottom,
};

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

    // Drain both pipes on threads so the child never blocks on a full
    // pipe, and bound what each retains. The draining and the retaining
    // are separate promises: `read_all` reads to EOF whatever the bound
    // says, so a child that outruns the cap is truncated rather than
    // starved.
    let child_stdout = child.stdout.take().expect("stdout piped");
    let child_stderr = child.stderr.take().expect("stderr piped");
    let stdout_thread = std::thread::spawn(move || read_all(Some(child_stdout), RETENTION));
    let stderr_thread = std::thread::spawn(move || read_all(Some(child_stderr), RETENTION));

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

    let (stdout_lines, stdout_dropped) = stdout_thread.join().unwrap_or_default();
    let (stderr_lines, stderr_dropped) = stderr_thread.join().unwrap_or_default();

    if timed_out {
        return Err(AppError::Timeout);
    }
    let code = status.code().unwrap_or(1); // signal death becomes 1, like gum
    let failed = code != 0;
    let show_stdout = args.show_output || args.show_stdout || (args.show_error && failed);
    let show_stderr = args.show_output || args.show_stderr || (args.show_error && failed);
    // A notice is only owed for a stream being SHOWN, and the reason is
    // what kind of message it would otherwise be. Both pipes are drained
    // whatever the flags say, so the default invocation — no `--show-*`
    // at all — discards plenty. Announcing that is debug output, not
    // normal output: it tells a user about the fate of bytes they chose
    // not to look at, on every long-running command they wrap. Ratified
    // rather than assumed; the alternative costs one line here and one
    // test if it is ever wanted.
    if show_stdout {
        report_dropped(stdout_dropped, stdout_lines.len(), "stdout");
        std::io::stdout()
            // Concatenated rather than written line by line: the retained
            // bytes are exactly what the child wrote, terminators
            // included, so joining them reproduces the stream.
            .write_all(&stdout_lines.concat())
            .context("writing child stdout")?;
    }
    if show_stderr {
        report_dropped(stderr_dropped, stderr_lines.len(), "stderr");
        std::io::stderr()
            .write_all(&stderr_lines.concat())
            .context("writing child stderr")?;
    }
    if failed {
        return Err(AppError::Child(code));
    }
    Ok(())
}

/// Say what the bound cost, before the stream it cost it from.
///
/// **Always stderr, even for a truncated stdout.** The child's stdout is
/// whatever the user is piping, and rat's own voice belongs beside it
/// rather than in it. It goes first because the tail is what survived,
/// so the missing part is the BEGINNING — a reader wants to know that
/// before reading, not after.
///
/// `kept` is counted rather than assumed to be the bound: a single
/// over-long line is dropped whole by the byte backstop, which can
/// report a drop with the line count nowhere near full.
fn report_dropped(dropped: usize, kept: usize, stream: &str) {
    if let Some(notice) = dropped_notice(dropped, kept, stream) {
        eprintln!("{notice}");
    }
}

fn dropped_notice(dropped: usize, kept: usize, stream: &str) -> Option<String> {
    (dropped > 0).then(|| {
        format!(
            "rat: {} {} dropped from {stream} — kept the newest {kept}",
            compact_count(dropped),
            if dropped == 1 { "line" } else { "lines" },
        )
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_stream_that_fit_is_not_reported() {
        assert_eq!(dropped_notice(0, 12, "stdout"), None);
    }

    #[test]
    fn the_notice_names_the_stream_the_loss_and_what_survived() {
        assert_eq!(
            dropped_notice(2_000, 10_000, "stdout").as_deref(),
            Some("rat: 2.0k lines dropped from stdout — kept the newest 10000")
        );
        assert_eq!(
            dropped_notice(2_000, 10_000, "stderr").as_deref(),
            Some("rat: 2.0k lines dropped from stderr — kept the newest 10000")
        );
    }

    #[test]
    fn one_dropped_line_is_a_line() {
        // The byte backstop drops a single over-long line whole, so the
        // count of one is reachable and reads as English when it happens.
        assert_eq!(
            dropped_notice(1, 3, "stdout").as_deref(),
            Some("rat: 1 line dropped from stdout — kept the newest 3")
        );
    }

    #[test]
    fn what_survived_is_counted_rather_than_assumed_to_be_the_bound() {
        // One monster line dropped by the byte backstop, with the count
        // bound nowhere near full: claiming the bound here would be a
        // plain lie about what follows.
        assert_eq!(
            dropped_notice(1, 0, "stdout").as_deref(),
            Some("rat: 1 line dropped from stdout — kept the newest 0")
        );
    }
}
