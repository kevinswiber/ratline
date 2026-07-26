use crate::cli::ChooseArgs;
use crate::exit::{AppError, AppResult};

pub fn run(_args: ChooseArgs) -> AppResult {
    Err(AppError::Fail(anyhow::anyhow!("not implemented")))
}
