//! Experimental, display-only conversation types. These types grant no tool permissions.
use oscar_core::tools::ToolError;
use serde_json::Value;
use std::fmt;

/// Maximum tree depth (root is depth zero) and number of values, including containers.
pub const MAX_STRUCTURE_DEPTH: usize = 16;
pub const MAX_STRUCTURE_NODES: usize = 512;

/// Fixed diagnostics never include untrusted JSON, identifiers or tool error details.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MessageError {
    InvalidIdentifier,
    InvalidJson,
    ExpectedObject,
    ByteLimit,
    DepthLimit,
    NodeLimit,
    InactiveRun,
    DuplicateCall,
    UnknownCall,
    MismatchedTool,
    DuplicateResult,
    HistoryLimit,
}

impl fmt::Display for MessageError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(match self {
            Self::InvalidIdentifier => "invalid tool message identifier",
            Self::InvalidJson => "invalid structured JSON",
            Self::ExpectedObject => "tool arguments must be an object",
            Self::ByteLimit => "tool message exceeds byte limit",
            Self::DepthLimit => "structured value exceeds depth limit",
            Self::NodeLimit => "structured value exceeds node limit",
            Self::InactiveRun => "tool message does not belong to the active run",
            Self::DuplicateCall => "tool call ID already recorded",
            Self::UnknownCall => "tool result has no matching call",
            Self::MismatchedTool => "tool result name does not match its call",
            Self::DuplicateResult => "tool call already has a result",
            Self::HistoryLimit => "desktop retained history limit reached",
        })
    }
}
impl std::error::Error for MessageError {}

/// Immutable JSON checked before retention/rendering. No unchecked deserialization.
/// Objects, arrays and all JSON scalars are supported; Debug redacts the content.
#[derive(Clone, PartialEq, Eq)]
pub struct StructuredData {
    value: Value,
    bytes: usize,
}

impl StructuredData {
    /// Parse at most `max_bytes` UTF-8 bytes and enforce fixed tree complexity limits.
    /// This validates presentation structure, not a registered tool's input schema.
    pub fn from_json(json: &str, max_bytes: usize) -> Result<Self, MessageError> {
        if json.len() > max_bytes {
            return Err(MessageError::ByteLimit);
        }
        let value: Value = serde_json::from_str(json).map_err(|_| MessageError::InvalidJson)?;
        fn check(value: &Value, depth: usize, nodes: &mut usize) -> Result<(), MessageError> {
            if depth > MAX_STRUCTURE_DEPTH {
                return Err(MessageError::DepthLimit);
            }
            *nodes += 1;
            if *nodes > MAX_STRUCTURE_NODES {
                return Err(MessageError::NodeLimit);
            }
            match value {
                Value::Object(values) => {
                    for value in values.values() {
                        check(value, depth + 1, nodes)?;
                    }
                }
                Value::Array(values) => {
                    for value in values {
                        check(value, depth + 1, nodes)?;
                    }
                }
                _ => {}
            }
            Ok(())
        }
        check(&value, 0, &mut 0)?;
        // Count the canonical display payload too: escaping can expand UTF-8 input.
        let bytes = value.to_string().len();
        if bytes > max_bytes {
            return Err(MessageError::ByteLimit);
        }
        Ok(Self { value, bytes })
    }

    pub fn value(&self) -> &Value {
        &self.value
    }

    pub fn byte_len(&self) -> usize {
        self.bytes
    }
}

impl fmt::Debug for StructuredData {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("StructuredData([redacted])")
    }
}

fn identifier(value: &str) -> bool {
    !value.is_empty()
        && value.len() <= 64
        && value
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || matches!(b, b'_' | b'-' | b'.'))
}

/// A proposed call for display. Only the registry can authorize and validate execution.
#[derive(Clone, PartialEq, Eq)]
pub struct ToolCall {
    id: String,
    name: String,
    arguments: StructuredData,
}

impl ToolCall {
    pub fn new(id: &str, name: &str, arguments: StructuredData) -> Result<Self, MessageError> {
        if !identifier(id) || !identifier(name) {
            return Err(MessageError::InvalidIdentifier);
        }
        if !arguments.value().is_object() {
            return Err(MessageError::ExpectedObject);
        }
        Ok(Self {
            id: id.into(),
            name: name.into(),
            arguments,
        })
    }
    pub fn id(&self) -> &str {
        &self.id
    }
    pub fn name(&self) -> &str {
        &self.name
    }
    pub fn arguments(&self) -> &StructuredData {
        &self.arguments
    }
    pub(crate) fn byte_len(&self) -> usize {
        self.id.len() + self.name.len() + self.arguments.byte_len()
    }
}

impl fmt::Debug for ToolCall {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("ToolCall([redacted])")
    }
}

/// Success has nested JSON; failures use the registry's fixed, redacted categories.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ToolOutcome {
    Success(StructuredData),
    Failure(ToolError),
}

#[derive(Clone, PartialEq, Eq)]
pub struct ToolResult {
    call_id: String,
    name: String,
    outcome: ToolOutcome,
}

impl ToolResult {
    pub fn new(call_id: &str, name: &str, outcome: ToolOutcome) -> Result<Self, MessageError> {
        if !identifier(call_id) || !identifier(name) {
            return Err(MessageError::InvalidIdentifier);
        }
        Ok(Self {
            call_id: call_id.into(),
            name: name.into(),
            outcome,
        })
    }
    pub fn call_id(&self) -> &str {
        &self.call_id
    }
    pub fn name(&self) -> &str {
        &self.name
    }
    pub fn outcome(&self) -> &ToolOutcome {
        &self.outcome
    }
    pub(crate) fn byte_len(&self) -> usize {
        self.call_id.len()
            + self.name.len()
            + match &self.outcome {
                ToolOutcome::Success(data) => data.byte_len(),
                ToolOutcome::Failure(error) => error.to_string().len(),
            }
    }
}

impl fmt::Debug for ToolResult {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("ToolResult([redacted])")
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Role {
    User,
    Model,
    Tool,
    System,
}

/// Text cannot masquerade as a tool call/result. Tool content is always typed.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Message {
    User(String),
    Model(String),
    System(String),
    ToolCall(ToolCall),
    ToolResult(ToolResult),
}

impl Message {
    pub fn role(&self) -> Role {
        match self {
            Self::User(_) => Role::User,
            Self::Model(_) => Role::Model,
            Self::System(_) => Role::System,
            Self::ToolCall(_) | Self::ToolResult(_) => Role::Tool,
        }
    }
    pub fn text(&self) -> Option<&str> {
        match self {
            Self::User(text) | Self::Model(text) | Self::System(text) => Some(text),
            _ => None,
        }
    }
    pub(crate) fn byte_len(&self) -> usize {
        match self {
            Self::User(text) | Self::Model(text) | Self::System(text) => text.len(),
            Self::ToolCall(call) => call.byte_len(),
            Self::ToolResult(result) => result.byte_len(),
        }
    }
}
