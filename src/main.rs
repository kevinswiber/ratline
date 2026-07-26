mod cli;
// Detection is consumed by the style command, which lands next.
#[allow(dead_code)]
mod color;
mod commands;
mod exit;

use clap::Parser;

use crate::cli::{Cli, Command};
use crate::exit::{AppError, OK};

fn main() {
    std::process::exit(real_main());
}

fn real_main() -> i32 {
    let cli = Cli::parse();
    match dispatch(cli.command) {
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

fn dispatch(command: Command) -> exit::AppResult {
    match command {
        Command::Style(args) => commands::style::run(args),
        Command::Bar(args) => commands::bar::run(args),
        Command::Duration(args) => commands::duration::run(args),
        Command::Date(args) => commands::date::run(args),
        Command::Spark(args) => commands::spark::run(args),
        Command::Log(args) => commands::log::run(args),
        Command::Frame(args) => commands::frame::run(args),
        Command::Watch(args) => commands::watch::run(args),
        Command::Doctor(args) => commands::doctor::run(args),
        Command::Choose(args) => commands::choose::run(args),
        Command::Confirm(args) => commands::confirm::run(args),
        Command::Input(args) => commands::input::run(args),
        Command::Filter(args) => commands::filter::run(args),
        Command::Spin(args) => commands::spin::run(args),
        Command::Completion(args) => commands::completion::run(args),
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
