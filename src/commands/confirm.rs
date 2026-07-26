use crate::cli::ConfirmArgs;
use crate::exit::{AppError, AppResult};

pub fn run(_args: ConfirmArgs) -> AppResult {
    Err(AppError::Fail(anyhow::anyhow!("not implemented")))
}
