use crate::cli::WatchArgs;
use crate::exit::{AppError, AppResult};

pub fn run(_args: WatchArgs) -> AppResult {
    Err(AppError::Fail(anyhow::anyhow!("not implemented")))
}
