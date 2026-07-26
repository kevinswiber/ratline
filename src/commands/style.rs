use std::io::Read;

use anyhow::{Context, anyhow};

use crate::cli::StyleArgs;
use crate::color::ColorProfile;
use crate::exit::AppResult;
use crate::style_spec::{StyleSpec, parse_color};

pub fn run(args: StyleArgs, profile: ColorProfile) -> AppResult {
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
        strip_ansi_escapes::strip_str(&text)
    };

    let mut out = String::new();
    for line in text.split('\n') {
        let line = if args.trim { line.trim() } else { line };
        out.push_str(&spec.render(line, profile));
        out.push('\n');
    }
    print!("{out}");
    Ok(())
}
