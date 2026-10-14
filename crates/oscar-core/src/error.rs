use std::{error::Error, fmt};

/// Failures crossing an orchestration boundary. Messages never contain prompts.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum OscarError {
    Mcp(McpError),
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
            Self::Mcp(error) => write!(f, "MCP failed: {error}"),
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

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum McpError {
    Timeout,
    Transport,
    Protocol,
    UnsupportedSchema,
    ToolFailed,
    RemoteDenied,
    SecretUnavailable,
}
impl fmt::Display for McpError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(match self {
            Self::Timeout => "operation deadline exceeded",
            Self::Transport => "transport error (details redacted)",
            Self::Protocol => "invalid or unsupported protocol response",
            Self::UnsupportedSchema => "tool schema is outside the supported flat-object subset",
            Self::ToolFailed => "server tool failed (details redacted)",
            Self::RemoteDenied => "remote MCP access is not permitted by host policy",
            Self::SecretUnavailable => "configured credential environment variable is unavailable",
        })
    }
}
