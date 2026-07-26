use crate::cli::ConfirmArgs;
use crate::color::ColorProfile;
use crate::exit::{AppError, AppResult};

pub fn run(_args: ConfirmArgs, _profile: ColorProfile) -> AppResult {
    Err(AppError::Fail(anyhow::anyhow!("not implemented")))
}
