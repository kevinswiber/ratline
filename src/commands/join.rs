use std::io::Read;

use anyhow::Context;

use crate::cli::JoinArgs;
use crate::color::ColorProfile;
use crate::core::join::{
    JoinAlign, VAlign, horizontal_width, join_horizontal, join_vertical, parse_block,
};
use crate::core::measure::Align;
use crate::exit::AppResult;

pub fn run(args: JoinArgs, profile: ColorProfile) -> AppResult {
    let texts: Vec<String> = if args.file.is_empty() {
        args.blocks
    } else {
        args.file
            .iter()
            .map(|path| {
                if path.as_os_str() == "-" {
                    let mut buf = String::new();
                    std::io::stdin()
                        .read_to_string(&mut buf)
                        .context("reading stdin")?;
                    Ok(buf)
                } else {
                    std::fs::read_to_string(path)
                        .with_context(|| format!("reading {}", path.display()))
                }
            })
            .collect::<anyhow::Result<_>>()?
    };
    let blocks: Vec<Vec<String>> = texts
        .iter()
        .map(|text| {
            if profile == ColorProfile::Ascii {
                parse_block(&crate::core::measure::strip_escapes(text))
            } else {
                parse_block(text)
            }
        })
        .collect();

    let gap = usize::from(args.gap);
    let lines = if args.vertical {
        let align = args
            .align
            .map(JoinAlign::horizontal)
            .transpose()?
            .unwrap_or(Align::Left);
        join_vertical(&blocks, gap, align)
    } else if fit_wants_stacking(args.fit, args.max_width, &blocks, gap) {
        // The adaptive fallback: alignment is direction-specific, so the
        // stacked layout goes plain left.
        join_vertical(&blocks, gap, Align::Left)
    } else {
        let align = args
            .align
            .map(JoinAlign::vertical)
            .transpose()?
            .unwrap_or(VAlign::Top);
        join_horizontal(&blocks, gap, align)
    };
    for line in lines {
        println!("{line}");
    }
    Ok(())
}

/// Whether --fit should fall back to stacking: the joined width exceeds the
/// available width, resolved as --max-width, then RAT_WIDTH (set by rat
/// watch for its children), then the terminal. No signal keeps blocks
/// beside, so pipelines stay deterministic.
fn fit_wants_stacking(
    fit: bool,
    max_width: Option<u16>,
    blocks: &[Vec<String>],
    gap: usize,
) -> bool {
    if !fit && max_width.is_none() {
        return false;
    }
    let available = max_width
        .map(usize::from)
        .or_else(|| {
            std::env::var("RAT_WIDTH")
                .ok()
                .and_then(|value| value.trim().parse().ok())
        })
        .or_else(|| {
            crossterm::terminal::size()
                .ok()
                .map(|(w, _)| usize::from(w))
        });
    match available {
        Some(max) => horizontal_width(blocks, gap) > max,
        None => false,
    }
}
