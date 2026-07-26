use crate::cli::LogArgs;
use crate::color::ColorProfile;
use crate::exit::{AppError, AppResult};

pub fn run(_args: LogArgs, _profile: ColorProfile) -> AppResult {
    Err(AppError::Fail(anyhow::anyhow!("not implemented")))
}
