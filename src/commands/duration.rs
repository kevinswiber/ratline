use crate::cli::DurationArgs;
use crate::exit::{AppError, AppResult};

pub fn run(_args: DurationArgs) -> AppResult {
    Err(AppError::Fail(anyhow::anyhow!("not implemented")))
}
