pub mod store;

use thiserror::Error;

#[derive(Debug, Error)]
pub enum Error {
    #[error("{0}")]
    Invalid(String),
    #[error("revision conflict: expected {expected}, current {current}")]
    Conflict { expected: u64, current: u64 },
    #[error("idempotency key already belongs to a different request")]
    KeyConflict,
    #[error(transparent)]
    Sql(#[from] rusqlite::Error),
    #[error(transparent)]
    Json(#[from] serde_json::Error),
    #[error(transparent)]
    Io(#[from] std::io::Error),
    #[error(transparent)]
    Domain(#[from] agentcut_core::AgentCutError),
}

impl Error {
    pub fn code(&self) -> &str {
        match self {
            Self::Invalid(_) => "E_INVALID_REQUEST",
            Self::Conflict { .. } => "E_REVISION_CONFLICT",
            Self::KeyConflict => "E_IDEMPOTENCY_CONFLICT",
            Self::Sql(_) => "E_STORE",
            Self::Json(_) => "E_JSON",
            Self::Io(_) => "E_IO",
            Self::Domain(e) => e.code(),
        }
    }
}

pub type Result<T> = std::result::Result<T, Error>;
