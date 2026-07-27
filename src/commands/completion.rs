use clap::CommandFactory;

use crate::cli::{Cli, CompletionArgs};
use crate::color::ColorProfile;
use crate::exit::AppResult;
use crate::theme::Palette;

pub fn run(args: CompletionArgs, _profile: ColorProfile, _palette: Palette) -> AppResult {
    let mut cmd = Cli::command();
    clap_complete::generate(args.shell, &mut cmd, "rat", &mut std::io::stdout());
    Ok(())
}
