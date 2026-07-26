use clap::CommandFactory;

use crate::cli::{Cli, CompletionArgs};
use crate::color::ColorProfile;
use crate::exit::AppResult;

pub fn run(args: CompletionArgs, _profile: ColorProfile) -> AppResult {
    let mut cmd = Cli::command();
    clap_complete::generate(args.shell, &mut cmd, "rat", &mut std::io::stdout());
    Ok(())
}
