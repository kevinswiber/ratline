use crate::cli::FilterArgs;
use crate::exit::{AppError, AppResult};

pub fn run(_args: FilterArgs) -> AppResult {
    Err(AppError::Fail(anyhow::anyhow!("not implemented")))
}
