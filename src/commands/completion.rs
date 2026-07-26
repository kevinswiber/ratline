use crate::cli::CompletionArgs;
use crate::exit::{AppError, AppResult};

pub fn run(_args: CompletionArgs) -> AppResult {
    Err(AppError::Fail(anyhow::anyhow!("not implemented")))
}
