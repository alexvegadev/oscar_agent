use std::{error::Error, fmt};

/// Failures crossing an orchestration boundary. Messages never contain prompts.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum OscarError {
    Config(String),
    Plan(String),
    Unavailable(String),
    Provider { transient: bool, message: String },
    Validation(String),
    Limit(String),
    Cancelled,
    Io(String),
}

impl fmt::Display for OscarError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Config(s) => write!(f, "invalid configuration: {s}"),
            Self::Plan(s) => write!(f, "invalid plan: {s}"),
            Self::Unavailable(s) => write!(f, "provider unavailable: {s}"),
            Self::Provider { message, .. } => write!(f, "inference failed: {message}"),
            Self::Validation(s) => write!(f, "validation failed: {s}"),
            Self::Limit(s) => write!(f, "limit reached: {s}"),
            Self::Cancelled => f.write_str("run cancelled"),
            Self::Io(s) => write!(f, "artifact I/O failed: {s}"),
        }
    }
}
impl Error for OscarError {}
