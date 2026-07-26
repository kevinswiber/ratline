use crate::cli::SparkArgs;
use crate::exit::{AppError, AppResult};

pub fn run(_args: SparkArgs) -> AppResult {
    Err(AppError::Fail(anyhow::anyhow!("not implemented")))
}
