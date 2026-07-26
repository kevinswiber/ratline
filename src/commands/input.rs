use crate::cli::InputArgs;
use crate::exit::{AppError, AppResult};

pub fn run(_args: InputArgs) -> AppResult {
    Err(AppError::Fail(anyhow::anyhow!("not implemented")))
}
