use serde::{Serialize, Serializer};

#[derive(Debug, thiserror::Error)]
pub enum CoreError {
    #[error("validation failed: {0}")]
    Validation(String),
    #[error("not found: {0}")]
    NotFound(String),
    #[error("conflict: {0}")]
    Conflict(String),
    #[error("arithmetic overflow")]
    Overflow,
    #[error("database error: {0}")]
    Database(#[from] sqlx::Error),
    #[error("migration error: {0}")]
    Migration(#[from] sqlx::migrate::MigrateError),
    #[error("io error: {0}")]
    Io(#[from] std::io::Error),
    #[error("integrity check failed: {0}")]
    Integrity(String),
}

pub type CoreResult<T> = Result<T, CoreError>;

/// Errors cross the Tauri boundary as `{ code, message }`.
#[derive(Serialize)]
struct Wire<'a> {
    code: &'a str,
    message: String,
}

impl CoreError {
    pub fn code(&self) -> &'static str {
        match self {
            Self::Validation(_) => "validation",
            Self::NotFound(_) => "not_found",
            Self::Conflict(_) => "conflict",
            Self::Overflow => "overflow",
            Self::Database(_) => "database",
            Self::Migration(_) => "migration",
            Self::Io(_) => "io",
            Self::Integrity(_) => "integrity",
        }
    }
}

impl Serialize for CoreError {
    fn serialize<S: Serializer>(&self, s: S) -> Result<S::Ok, S::Error> {
        Wire {
            code: self.code(),
            message: self.to_string(),
        }
        .serialize(s)
    }
}
