use serde::{Serialize, Serializer};

#[derive(Debug, thiserror::Error)]
#[error("{0}")]
pub struct AppError(pub String);

impl Serialize for AppError {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        serializer.serialize_str(&self.0)
    }
}

pub type AppResult<T> = Result<T, AppError>;

#[derive(Debug, Serialize, thiserror::Error)]
#[error("{message}")]
pub struct McpSyncError {
    pub message: String,
    pub config_invalid: bool,
}

impl McpSyncError {
    pub fn invalid_config(error: AppError) -> Self {
        Self {
            message: error.0,
            config_invalid: true,
        }
    }
}

impl From<AppError> for McpSyncError {
    fn from(error: AppError) -> Self {
        Self {
            message: error.0,
            config_invalid: false,
        }
    }
}

macro_rules! app_err {
    ($($arg:tt)*) => {
        crate::error::AppError(format!($($arg)*))
    };
}

pub(crate) use app_err;
