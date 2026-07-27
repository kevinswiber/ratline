use std::io::Read;

use anyhow::Context;

use crate::cli::JoinArgs;
use crate::color::ColorProfile;
use crate::core::join::{JoinAlign, VAlign, join_horizontal, join_vertical, parse_block};
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
            let mut lines = parse_block(text);
            // Strip per line: the stripper eats control bytes, so a whole
            // block through it would lose its newlines.
            if profile == ColorProfile::Ascii {
                for line in lines.iter_mut() {
                    let stripped = strip_ansi_escapes::strip_str(line.as_str());
                    *line = stripped;
                }
            }
            lines
        })
        .collect();

    let lines = if args.vertical {
        let align = args
            .align
            .map(JoinAlign::horizontal)
            .transpose()?
            .unwrap_or(Align::Left);
        join_vertical(&blocks, usize::from(args.gap), align)
    } else {
        let align = args
            .align
            .map(JoinAlign::vertical)
            .transpose()?
            .unwrap_or(VAlign::Top);
        join_horizontal(&blocks, usize::from(args.gap), align)
    };
    for line in lines {
        println!("{line}");
    }
    Ok(())
}
