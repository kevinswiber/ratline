use std::io::Write;

use anyhow::Context;

use crate::cli::{LogArgs, LogLevel};
use crate::color::ColorProfile;
use crate::core::datetime::format_timestamp;
use crate::exit::AppResult;
use crate::style_spec::StyleSpec;

pub fn run(args: LogArgs, profile: ColorProfile) -> AppResult {
    // Unleveled messages bypass the minimum-level filter.
    if let (Some(level), Some(min)) = (args.level, args.min_level)
        && (level as u8) < (min as u8)
    {
        return Ok(());
    }

    let text = args.text.join(" ");
    let mut prefix = String::new();
    let mut plain_prefix = String::new();
    if let Some(fmt) = &args.time {
        let now = jiff::Timestamp::now();
        let stamp = format_timestamp(now, fmt, false)?;
        prefix.push_str(&stamp);
        prefix.push(' ');
        plain_prefix.clone_from(&prefix);
    }
    if let Some(level) = args.level {
        prefix.push_str(&level.styled_tag(profile));
        prefix.push(' ');
        plain_prefix.push_str(level.tag());
        plain_prefix.push(' ');
    }

    if let Some(path) = &args.file {
        let mut file = std::fs::OpenOptions::new()
            .create(true)
            .append(true)
            .open(path)
            .with_context(|| format!("opening {path:?}"))?;
        writeln!(file, "{plain_prefix}{text}").context("writing log file")?;
        return Ok(());
    }
    // Logs go to stderr, keeping stdout free for results.
    eprintln!("{prefix}{text}");
    Ok(())
}

impl LogLevel {
    /// Four-character tag, charmbracelet/log style.
    pub fn tag(self) -> &'static str {
        match self {
            LogLevel::Debug => "DEBU",
            LogLevel::Info => "INFO",
            LogLevel::Warn => "WARN",
            LogLevel::Error => "ERRO",
            LogLevel::Fatal => "FATA",
        }
    }

    fn color(self) -> ratatui::style::Color {
        let index = match self {
            LogLevel::Debug => 63,
            LogLevel::Info => 86,
            LogLevel::Warn => 192,
            LogLevel::Error => 204,
            LogLevel::Fatal => 134,
        };
        ratatui::style::Color::Indexed(index)
    }

    fn styled_tag(self, profile: ColorProfile) -> String {
        StyleSpec {
            bold: true,
            foreground: Some(self.color()),
            ..StyleSpec::default()
        }
        .render(self.tag(), profile)
    }
}
