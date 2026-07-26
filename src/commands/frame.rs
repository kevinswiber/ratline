use std::hash::{Hash, Hasher};
use std::io::{Read, Write};

use anyhow::Context;

use crate::cli::{FrameAction, FrameArgs};
use crate::color::ColorProfile;
use crate::exit::AppResult;
use crate::term::frame_state::{FrameState, STATE_VERSION, default_state_path};
use crate::term::inline::{frame_bytes, rendered_rows};

pub fn run(args: FrameArgs, _profile: ColorProfile) -> AppResult {
    let mut stdout = std::io::stdout().lock();

    match args.action {
        Some(FrameAction::Begin) => {
            stdout.write_all(b"\x1b[?2026h").context("writing")?;
            return Ok(());
        }
        Some(FrameAction::End) => {
            stdout.write_all(b"\x1b[?2026l").context("writing")?;
            return Ok(());
        }
        None => {}
    }

    let state_path = args.state.clone().unwrap_or_else(default_state_path);

    if args.reset {
        let _ = std::fs::remove_file(&state_path);
        return Ok(());
    }
    if args.finish {
        let _ = std::fs::remove_file(&state_path);
        stdout
            .write_all(b"\x1b[?2026l\x1b[?25h")
            .context("writing")?;
        return Ok(());
    }
    if args.clear {
        let prev = FrameState::load(&state_path).map_or(0, |s| s.rows);
        let _ = std::fs::remove_file(&state_path);
        let mut bytes = String::new();
        if prev > 0 {
            bytes.push_str(&format!("\x1b[{prev}A"));
        }
        bytes.push_str("\r\x1b[0J\x1b[?25h");
        stdout.write_all(bytes.as_bytes()).context("writing")?;
        return Ok(());
    }

    let mut body = String::new();
    std::io::stdin()
        .read_to_string(&mut body)
        .context("reading stdin")?;
    let lines: Vec<String> = if body.is_empty() {
        Vec::new()
    } else {
        body.trim_end_matches('\n')
            .split('\n')
            .map(str::to_string)
            .collect()
    };

    let cols = args
        .width
        .unwrap_or_else(|| crossterm::terminal::size().map_or(80, |(w, _)| w));

    let mut hasher = std::hash::DefaultHasher::new();
    body.hash(&mut hasher);
    let hash = hasher.finish();

    let previous = FrameState::load(&state_path);
    if let Some(prev) = previous
        && prev.hash == hash
        && prev.cols == cols
    {
        return Ok(());
    }
    // A width change invalidates the wrap-derived row count: repaint from
    // scratch rather than moving up over stale rows.
    let prev_rows = match previous {
        Some(prev) if prev.cols == cols => prev.rows,
        _ => 0,
    };

    let mut bytes = String::new();
    if !args.no_hide_cursor {
        bytes.push_str("\x1b[?25l");
    }
    bytes.push_str(&frame_bytes(prev_rows, &lines, cols, !args.no_sync, false));
    stdout.write_all(bytes.as_bytes()).context("writing")?;
    stdout.flush().context("flushing")?;

    FrameState {
        version: STATE_VERSION,
        rows: rendered_rows(&lines, cols),
        cols,
        hash,
    }
    .store(&state_path)
    .context("storing frame state")?;
    Ok(())
}
