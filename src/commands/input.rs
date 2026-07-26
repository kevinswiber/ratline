use crate::cli::InputArgs;
use crate::color::ColorProfile;
use crate::exit::{AppError, AppResult};

pub fn run(_args: InputArgs, _profile: ColorProfile) -> AppResult {
    Err(AppError::Fail(anyhow::anyhow!("not implemented")))
}
