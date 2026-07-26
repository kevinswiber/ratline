use crate::cli::SpinArgs;
use crate::exit::{AppError, AppResult};

pub fn run(_args: SpinArgs) -> AppResult {
    Err(AppError::Fail(anyhow::anyhow!("not implemented")))
}
