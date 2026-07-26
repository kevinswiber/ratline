use crate::cli::DoctorArgs;
use crate::color::ColorProfile;
use crate::exit::{AppError, AppResult};

pub fn run(_args: DoctorArgs, _profile: ColorProfile) -> AppResult {
    Err(AppError::Fail(anyhow::anyhow!("not implemented")))
}
