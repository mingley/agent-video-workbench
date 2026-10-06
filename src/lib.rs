pub mod analysis;
pub mod animation;
pub mod asr;
pub mod assembly;
pub mod audio;
pub mod color;
pub mod delivery;
pub mod editorial;
pub mod inspect;
pub mod interchange;
pub mod jobs;
pub mod json;
pub mod library;
pub mod mcp;
pub mod media;
pub mod policy;
pub mod preserve;
pub mod process;
pub mod service;
pub mod storage;
pub mod store;
pub mod studio;
pub mod tracking;
pub mod transfer;
pub mod workflow;

use thiserror::Error;

#[derive(Debug, Error)]
pub enum Error {
    #[error("job cancelled")]
    Cancelled,
    #[error("subprocess deadline exceeded")]
    Timeout,
    #[error("{message}: {details}")]
    Execution { message: String, details: String },
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
            Self::Cancelled => "E_JOB_CANCELLED",
            Self::Timeout => "E_JOB_TIMEOUT",
            Self::Execution { .. } => "E_EXECUTION_FAILED",
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
