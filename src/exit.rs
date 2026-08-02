pub const OK: i32 = 0;
pub const FAILURE: i32 = 1;
pub const TIMEOUT: i32 = 124;
pub const ABORTED: i32 = 130;

/// Command-level failure carrying its process exit code. Only `main` turns
/// this into `process::exit`; everything else returns `AppResult`.
#[derive(Debug)]
pub enum AppError {
    /// Message printed to stderr, exit 1.
    Fail(anyhow::Error),
    /// Silent, exit 1 (Esc / nothing selected, matching gum).
    NoSelection,
    /// Silent, exit 130 (Ctrl-C).
    Aborted,
    /// Exit 124. `None` prints the shipped "timed out"; `Some` names
    /// what was waited on instead — the code never changes with it.
    Timeout(Option<anyhow::Error>),
    /// Silent, exit with the child's code (`rat spin`).
    Child(i32),
}

impl AppError {
    pub fn code(&self) -> i32 {
        match self {
            AppError::Fail(_) | AppError::NoSelection => FAILURE,
            AppError::Aborted => ABORTED,
            AppError::Timeout(_) => TIMEOUT,
            AppError::Child(code) => *code,
        }
    }
}

impl From<anyhow::Error> for AppError {
    fn from(err: anyhow::Error) -> Self {
        AppError::Fail(err)
    }
}

pub type AppResult = Result<(), AppError>;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn code_per_variant() {
        assert_eq!(AppError::Fail(anyhow::anyhow!("boom")).code(), 1);
        assert_eq!(AppError::NoSelection.code(), 1);
        assert_eq!(AppError::Aborted.code(), 130);
        // With or without a detail, a timeout is 124 — the payload
        // changes what stderr says, never the exit code.
        assert_eq!(AppError::Timeout(None).code(), 124);
        assert_eq!(AppError::Timeout(Some(anyhow::anyhow!("x"))).code(), 124);
        assert_eq!(AppError::Child(7).code(), 7);
    }

    #[test]
    fn from_anyhow_is_fail() {
        let err: AppError = anyhow::anyhow!("boom").into();
        assert_eq!(err.code(), 1);
        assert!(matches!(err, AppError::Fail(_)));
    }

    #[test]
    fn constants() {
        assert_eq!(OK, 0);
        assert_eq!(FAILURE, 1);
        assert_eq!(TIMEOUT, 124);
        assert_eq!(ABORTED, 130);
    }
}
