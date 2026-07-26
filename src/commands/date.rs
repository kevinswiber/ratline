use crate::cli::DateArgs;
use crate::exit::{AppError, AppResult};

pub fn run(_args: DateArgs) -> AppResult {
    Err(AppError::Fail(anyhow::anyhow!("not implemented")))
}
