//! Error type shared across the crate.

use std::fmt;

pub type Result<T> = std::result::Result<T, GtError>;

#[derive(Debug)]
pub enum GtError {
    /// A `git` invocation failed, or git itself is unusable.
    Git(String),
    /// A `gh` invocation failed, or gh is unavailable.
    Gh(String),
    /// The user invoked the CLI incorrectly.
    Usage(String),
    /// Repository / metadata state prevents the requested operation.
    State(String),
    /// A precondition was not met (dirty tree, detached HEAD, ...).
    Precondition(String),
    /// The user declined to proceed at a prompt.
    Aborted,
    /// A rebase paused on conflicts; details were already printed.
    Paused,
    Io(std::io::Error),
    Json(serde_json::Error),
}

impl fmt::Display for GtError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            GtError::Git(m) => write!(f, "{m}"),
            GtError::Gh(m) => write!(f, "{m}"),
            GtError::Usage(m) => write!(f, "{m}"),
            GtError::State(m) => write!(f, "{m}"),
            GtError::Precondition(m) => write!(f, "{m}"),
            GtError::Aborted => write!(f, "aborted"),
            GtError::Paused => write!(f, "paused on conflicts"),
            GtError::Io(e) => write!(f, "io: {e}"),
            GtError::Json(e) => write!(f, "json: {e}"),
        }
    }
}

impl std::error::Error for GtError {}

impl From<std::io::Error> for GtError {
    fn from(e: std::io::Error) -> Self {
        GtError::Io(e)
    }
}

impl From<serde_json::Error> for GtError {
    fn from(e: serde_json::Error) -> Self {
        GtError::Json(e)
    }
}
