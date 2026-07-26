use crate::cli::ChooseArgs;
use crate::color::ColorProfile;
use crate::exit::{AppError, AppResult};

pub fn run(_args: ChooseArgs, _profile: ColorProfile) -> AppResult {
    Err(AppError::Fail(anyhow::anyhow!("not implemented")))
}
