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
            if profile == ColorProfile::Ascii {
                parse_block(&crate::core::measure::strip_escapes(text))
            } else {
                parse_block(text)
            }
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
