use std::io::Read;

use anyhow::{Context, anyhow};

use crate::cli::StyleArgs;
use crate::color::ColorProfile;
use crate::core::box_model::{BoxSpec, parse_sides, render_box};
use crate::exit::AppResult;
use crate::style_spec::{StyleSpec, parse_color};
use crate::theme::Palette;

pub fn run(args: StyleArgs, profile: ColorProfile, _palette: Palette) -> AppResult {
    let spec = StyleSpec {
        bold: args.bold,
        faint: args.faint,
        italic: args.italic,
        underline: args.underline,
        strikethrough: args.strikethrough,
        foreground: args.foreground.as_deref().map(parse_color).transpose()?,
        background: args.background.as_deref().map(parse_color).transpose()?,
    };

    let text = if args.text.is_empty() {
        let mut buf = String::new();
        std::io::stdin()
            .read_to_string(&mut buf)
            .context("reading stdin")?;
        buf.truncate(buf.trim_end_matches('\n').len());
        buf
    } else {
        args.text.join("\n")
    };
    if text.is_empty() {
        return Err(anyhow!("no input provided").into());
    }

    let text = if args.no_strip_ansi {
        text
    } else {
        // Escape-only stripping: tabs stay, so tab-delimited rows can be
        // styled on their way to a table.
        crate::core::measure::strip_escapes(&text)
    };

    let lines: Vec<String> = text
        .split('\n')
        .map(|line| {
            let line = if args.trim { line.trim() } else { line };
            line.to_string()
        })
        .collect();

    let boxspec = BoxSpec {
        content_style: spec,
        width: args.width.map(usize::from),
        align: args.align,
        padding: args
            .padding
            .as_deref()
            .map(parse_sides)
            .transpose()
            .context("padding")?
            .unwrap_or_default(),
        border: args.border,
        border_style: StyleSpec {
            foreground: args.border_color.as_deref().map(parse_color).transpose()?,
            ..StyleSpec::default()
        },
        title: args.title.as_deref(),
        margin: args
            .margin
            .as_deref()
            .map(parse_sides)
            .transpose()
            .context("margin")?
            .unwrap_or_default(),
        ellipsis: &args.ellipsis,
    };
    let mut out = String::new();
    for line in render_box(&lines, &boxspec, profile) {
        out.push_str(&line);
        out.push('\n');
    }
    print!("{out}");
    Ok(())
}
