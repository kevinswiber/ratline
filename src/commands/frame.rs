use crate::cli::FrameArgs;
use crate::exit::{AppError, AppResult};

pub fn run(_args: FrameArgs) -> AppResult {
    Err(AppError::Fail(anyhow::anyhow!("not implemented")))
}
