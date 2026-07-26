use crate::cli::LogArgs;
use crate::exit::{AppError, AppResult};

pub fn run(_args: LogArgs) -> AppResult {
    Err(AppError::Fail(anyhow::anyhow!("not implemented")))
}
