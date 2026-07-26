use crate::cli::DoctorArgs;
use crate::exit::{AppError, AppResult};

pub fn run(_args: DoctorArgs) -> AppResult {
    Err(AppError::Fail(anyhow::anyhow!("not implemented")))
}
