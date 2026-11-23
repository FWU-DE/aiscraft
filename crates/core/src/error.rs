use serde::{Deserialize, Serialize};

/// What went wrong, in terms a person or an agent can act on.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ErrorKind {
    /// Not signed in, session expired or key rejected.
    Auth,
    /// The server could not be reached or the connection broke.
    Network,
    /// The server answered something we do not understand.
    Protocol,
    /// The server refused for a reason it named (quota, content filter, not allowed).
    Refused,
    NotFound,
    /// Bad parameters from the UI, CLI, control channel or a file.
    Invalid,
    Unsupported,
    Storage,
    Cancelled,
}

#[derive(Debug, Clone, thiserror::Error, Serialize, Deserialize)]
#[error("{message}")]
pub struct Error {
    pub kind: ErrorKind,
    pub message: String,
}

pub type Result<T, E = Error> = std::result::Result<T, E>;

impl Error {
    pub fn new(kind: ErrorKind, message: impl Into<String>) -> Self {
        Self { kind, message: message.into() }
    }
    pub fn auth(m: impl Into<String>) -> Self {
        Self::new(ErrorKind::Auth, m)
    }
    pub fn network(m: impl Into<String>) -> Self {
        Self::new(ErrorKind::Network, m)
    }
    pub fn protocol(m: impl Into<String>) -> Self {
        Self::new(ErrorKind::Protocol, m)
    }
    pub fn refused(m: impl Into<String>) -> Self {
        Self::new(ErrorKind::Refused, m)
    }
    pub fn not_found(m: impl Into<String>) -> Self {
        Self::new(ErrorKind::NotFound, m)
    }
    pub fn invalid(m: impl Into<String>) -> Self {
        Self::new(ErrorKind::Invalid, m)
    }
    pub fn unsupported(m: impl Into<String>) -> Self {
        Self::new(ErrorKind::Unsupported, m)
    }
    pub fn storage(m: impl Into<String>) -> Self {
        Self::new(ErrorKind::Storage, m)
    }
    pub fn cancelled() -> Self {
        Self::new(ErrorKind::Cancelled, "Abgebrochen")
    }
}

impl From<serde_json::Error> for Error {
    fn from(e: serde_json::Error) -> Self {
        Self::invalid(format!("JSON: {e}"))
    }
}
