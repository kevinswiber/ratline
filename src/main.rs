mod cli;
mod color;
mod commands;
mod core;
mod exit;
mod style_spec;
mod term;
// Consumed by the interactive commands, which land next.
#[allow(dead_code)]
mod ui;

use clap::Parser;

use crate::cli::{Cli, Command};
use crate::color::ColorProfile;
use crate::exit::{AppError, OK};
use crate::term::tty::UiStream;

fn main() {
    std::process::exit(real_main());
}

fn real_main() -> i32 {
    let cli = Cli::parse();
    // Capability comes from the UI stream's ttyness, never stdout's, so
    // command substitution keeps color.
    let is_tty = UiStream::open().is_tty();
    let profile = color::resolve_profile(cli.color, &color::SystemEnv, is_tty);
    match dispatch(cli.command, profile) {
        Ok(()) => OK,
        Err(err) => {
            match &err {
                AppError::Fail(inner) => eprintln!("rat: {inner:#}"),
                AppError::Timeout => eprintln!("timed out"),
                AppError::NoSelection | AppError::Aborted | AppError::Child(_) => {}
            }
            err.code()
        }
    }
}

fn dispatch(command: Command, profile: ColorProfile) -> exit::AppResult {
    match command {
        Command::Style(args) => commands::style::run(args, profile),
        Command::Bar(args) => commands::bar::run(args, profile),
        Command::Duration(args) => commands::duration::run(args, profile),
        Command::Date(args) => commands::date::run(args, profile),
        Command::Spark(args) => commands::spark::run(args, profile),
        Command::Log(args) => commands::log::run(args, profile),
        Command::Frame(args) => commands::frame::run(args, profile),
        Command::Watch(args) => commands::watch::run(args, profile),
        Command::Doctor(args) => commands::doctor::run(args, profile),
        Command::Choose(args) => commands::choose::run(args, profile),
        Command::Confirm(args) => commands::confirm::run(args, profile),
        Command::Input(args) => commands::input::run(args, profile),
        Command::Filter(args) => commands::filter::run(args, profile),
        Command::Spin(args) => commands::spin::run(args, profile),
        Command::Completion(args) => commands::completion::run(args, profile),
        #[cfg(debug_assertions)]
        Command::ExitCode(args) => match args.code {
            0 => Ok(()),
            1 => Err(AppError::NoSelection),
            124 => Err(AppError::Timeout),
            130 => Err(AppError::Aborted),
            n => Err(AppError::Child(n)),
        },
    }
}
