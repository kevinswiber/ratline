use crate::cli::StyleArgs;
use crate::exit::{AppError, AppResult};

pub fn run(_args: StyleArgs) -> AppResult {
    Err(AppError::Fail(anyhow::anyhow!("not implemented")))
}
