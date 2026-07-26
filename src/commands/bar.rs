use crate::cli::BarArgs;
use crate::exit::{AppError, AppResult};

pub fn run(_args: BarArgs) -> AppResult {
    Err(AppError::Fail(anyhow::anyhow!("not implemented")))
}
