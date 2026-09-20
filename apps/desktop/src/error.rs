use serde::{Deserialize, Serialize};
use thiserror::Error;

#[derive(Debug, Clone, Copy, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ErrorCategory {
    Authentication,
    Api,
    Playback,
    Database,
    Configuration,
    Validation,
    Internal,
}

#[derive(Debug, Clone, Serialize, Deserialize, Error)]
#[error("{message}")]
#[serde(rename_all = "camelCase")]
pub struct AppError {
    pub category: ErrorCategory,
    pub code: String,
    pub message: String,
    pub action: Option<String>,
    pub technical: String,
}

impl AppError {
    pub fn new(
        category: ErrorCategory,
        code: impl Into<String>,
        message: impl Into<String>,
        action: impl Into<Option<String>>,
        technical: impl Into<String>,
    ) -> Self {
        Self {
            category,
            code: code.into(),
            message: message.into(),
            action: action.into(),
            technical: technical.into(),
        }
    }

    pub fn database(
        code: impl Into<String>,
        message: impl Into<String>,
        technical: impl Into<String>,
    ) -> Self {
        Self::new(
            ErrorCategory::Database,
            code,
            message,
            Some("Open the data directory, check that it is writable, then retry.".to_string()),
            technical,
        )
    }

    pub fn validation(message: impl Into<String>) -> Self {
        let message = message.into();
        Self::new(
            ErrorCategory::Validation,
            "invalid_input",
            message.clone(),
            None,
            message,
        )
    }

    pub fn configuration(
        code: impl Into<String>,
        message: impl Into<String>,
        action: impl Into<String>,
    ) -> Self {
        let message = message.into();
        Self::new(
            ErrorCategory::Configuration,
            code,
            message.clone(),
            Some(action.into()),
            message,
        )
    }

    pub fn playback(
        code: impl Into<String>,
        message: impl Into<String>,
        action: impl Into<String>,
    ) -> Self {
        let message = message.into();
        Self::new(
            ErrorCategory::Playback,
            code,
            message.clone(),
            Some(action.into()),
            message,
        )
    }

    pub fn internal(technical: impl Into<String>) -> Self {
        Self::new(
            ErrorCategory::Internal,
            "internal_error",
            "Thelxinoe hit an internal error.",
            Some(
                "Retry the action. If it fails again, open the logs from the data directory."
                    .to_string(),
            ),
            technical,
        )
    }
}

pub type AppResult<T> = Result<T, AppError>;

impl From<rusqlite::Error> for AppError {
    fn from(value: rusqlite::Error) -> Self {
        Self::database(
            "sqlite_error",
            "The local database operation failed.",
            value.to_string(),
        )
    }
}

impl From<std::io::Error> for AppError {
    fn from(value: std::io::Error) -> Self {
        Self::internal(value.to_string())
    }
}
