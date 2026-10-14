//! Durable backend primitives. Uploaded data must be processed in a sandbox.
pub mod api;
pub mod intake;
pub mod model;
pub mod protocol;
pub mod publish;
pub mod runner;
pub mod sandbox;
pub mod store;

use serde::Serialize;
use std::{error::Error, fmt};

#[derive(Debug, Serialize)]
pub struct ServiceError {
    pub code: String,
    pub message: String,
}

impl ServiceError {
    pub fn new(code: &str, message: impl Into<String>) -> Self {
        Self {
            code: code.into(),
            message: message.into(),
        }
    }
}
impl fmt::Display for ServiceError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}: {}", self.code, self.message)
    }
}
impl Error for ServiceError {}
impl From<sqlx::Error> for ServiceError {
    fn from(_: sqlx::Error) -> Self {
        Self::new(
            "STORAGE_ERROR",
            "Database operation failed; inspect operator diagnostics",
        )
    }
}
impl From<std::io::Error> for ServiceError {
    fn from(_: std::io::Error) -> Self {
        Self::new("STORAGE_ERROR", "Filesystem operation failed")
    }
}
pub type Result<T> = std::result::Result<T, ServiceError>;
