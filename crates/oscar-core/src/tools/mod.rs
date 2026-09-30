//! Experimental host-owned tool boundary. Models may submit calls, never policy.
mod schema;
pub use schema::{Field, InputSchema, InputType, InputValue, ValidatedInput};
use serde::{Deserialize, Serialize};
use std::{
    collections::{BTreeMap, BTreeSet},
    fmt,
    future::Future,
    pin::Pin,
    sync::Arc,
    time::Duration,
};
use tokio::time::Instant;
use tokio_util::sync::CancellationToken;

/// Errors contain fixed diagnostics, never arguments or implementation error text.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum ToolError {
    InvalidDefinition,
    DuplicateName,
    RegistryFull,
    InvalidCall,
    DuplicateCall,
    UnknownTool,
    InvalidArguments,
    Denied,
    ApprovalRequired,
    ApprovalDenied,
    CallLimit,
    InputLimit,
    OutputLimit,
    Timeout,
    Cancelled,
    Failed,
}
impl fmt::Display for ToolError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(match self {
            Self::InvalidDefinition => "invalid tool definition or limits",
            Self::DuplicateName => "tool name already registered",
            Self::RegistryFull => "tool registry limit reached",
            Self::InvalidCall => "invalid tool call JSON or identifier",
            Self::DuplicateCall => "tool call ID already used in this session",
            Self::UnknownTool => "tool is not registered",
            Self::InvalidArguments => "arguments do not match the registered schema",
            Self::Denied => "tool denied by host policy",
            Self::ApprovalRequired => "side-effecting tool requires a host approval handler",
            Self::ApprovalDenied => "host approval denied",
            Self::CallLimit => "tool session call limit reached",
            Self::InputLimit => "tool call exceeds input byte limit",
            Self::OutputLimit => "tool output exceeds byte limit",
            Self::Timeout => "tool session or call deadline exceeded",
            Self::Cancelled => "tool call cancelled",
            Self::Failed => "tool execution failed",
        })
    }
}
impl std::error::Error for ToolError {}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum Effect {
    ReadOnly,
    SideEffect,
}
/// Trusted metadata registered by the host, not deserialized from model output.
#[derive(Debug, Clone, Serialize)]
pub struct ToolDefinition {
    pub name: String,
    pub description: String,
    pub input: InputSchema,
    pub effect: Effect,
}
pub type ToolFuture<'a> = Pin<Box<dyn Future<Output = Result<String, ToolError>> + Send + 'a>>;
/// Tools must be nonblocking and cancellation-safe on drop. Enforce output bounds
/// while producing output, not just after allocation. No detached work is allowed.
/// Side effects must be accurately declared by the trusted registering host.
pub trait Tool: Send + Sync {
    fn execute(&self, input: ValidatedInput, context: ToolContext) -> ToolFuture<'_>;
}
#[derive(Clone)]
pub struct ToolContext {
    pub cancellation: CancellationToken,
    pub max_output_bytes: usize,
}
/// Host implementation must bind approval to this exact definition and input.
/// Returning true is explicit host approval, never a model confidence judgment.
pub trait Approval: Send + Sync {
    fn approve<'a>(
        &'a self,
        call_id: &'a str,
        definition: &'a ToolDefinition,
        input: &'a ValidatedInput,
    ) -> Pin<Box<dyn Future<Output = bool> + Send + 'a>>;
}

/// There is intentionally no Deserialize implementation for policy.
#[derive(Debug, Clone, Default)]
pub struct ToolPolicy {
    pub allowed_tools: BTreeSet<String>,
    pub allow_side_effects: bool,
}
#[derive(Debug, Clone)]
pub struct ToolLimits {
    pub max_calls: usize,
    pub max_input_bytes: usize,
    pub max_output_bytes: usize,
    pub call_timeout_ms: u64,
    pub session_timeout_ms: u64,
}
impl Default for ToolLimits {
    fn default() -> Self {
        Self {
            max_calls: 64,
            max_input_bytes: 16_384,
            max_output_bytes: 16_384,
            call_timeout_ms: 5_000,
            session_timeout_ms: 60_000,
        }
    }
}
impl ToolLimits {
    fn validate(&self) -> Result<(), ToolError> {
        if !(1..=1024).contains(&self.max_calls)
            || !(1..=1_048_576).contains(&self.max_input_bytes)
            || !(1..=1_048_576).contains(&self.max_output_bytes)
            || !(1..=3_600_000).contains(&self.session_timeout_ms)
            || self.call_timeout_ms == 0
            || self.call_timeout_ms > self.session_timeout_ms
        {
            return Err(ToolError::InvalidDefinition);
        }
        Ok(())
    }
}
struct Entry {
    definition: ToolDefinition,
    tool: Arc<dyn Tool>,
}
/// Up to 128 explicit tools; discovery is ordered by name. Registration stops
/// while any session borrows the registry, so in-flight definitions cannot change.
#[derive(Default)]
pub struct ToolRegistry {
    entries: BTreeMap<String, Entry>,
}
impl ToolRegistry {
    pub fn register(
        &mut self,
        definition: ToolDefinition,
        tool: Arc<dyn Tool>,
    ) -> Result<(), ToolError> {
        if !identifier(&definition.name)
            || definition.description.trim().is_empty()
            || definition.description.len() > 1024
        {
            return Err(ToolError::InvalidDefinition);
        }
        definition.input.validate_definition()?;
        if self.entries.contains_key(&definition.name) {
            return Err(ToolError::DuplicateName);
        }
        if self.entries.len() >= 128 {
            return Err(ToolError::RegistryFull);
        }
        self.entries
            .insert(definition.name.clone(), Entry { definition, tool });
        Ok(())
    }
    pub fn definitions(&self) -> impl Iterator<Item = &ToolDefinition> {
        self.entries.values().map(|entry| &entry.definition)
    }
    /// One session per host run. Calls execute serially; no automatic retries.
    pub fn session(
        &self,
        policy: ToolPolicy,
        limits: ToolLimits,
        approval: Option<Arc<dyn Approval>>,
        cancellation: CancellationToken,
    ) -> Result<ToolSession<'_>, ToolError> {
        limits.validate()?;
        if policy.allowed_tools.len() > 128
            || policy
                .allowed_tools
                .iter()
                .any(|name| !self.entries.contains_key(name))
        {
            return Err(ToolError::InvalidDefinition);
        }
        Ok(ToolSession {
            registry: self,
            policy,
            deadline: Instant::now() + Duration::from_millis(limits.session_timeout_ms),
            limits,
            approval,
            cancellation,
            calls: 0,
            ids: BTreeSet::new(),
            events: Vec::new(),
        })
    }
}
fn identifier(name: &str) -> bool {
    !name.is_empty()
        && name.len() <= 64
        && name
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || matches!(b, b'_' | b'-' | b'.'))
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Call {
    id: String,
    name: String,
    arguments: schema::RawInput,
}
/// Redacted lifecycle events: sequence numbers avoid recording model-supplied IDs.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum ToolEvent {
    Requested {
        sequence: usize,
    },
    Finished {
        sequence: usize,
        error: Option<ToolError>,
    },
}
/// Dropping the host's call future cancels the child token as well as the tool future.
struct CancelOnDrop(CancellationToken);
impl Drop for CancelOnDrop {
    fn drop(&mut self) {
        self.0.cancel();
    }
}

pub struct ToolSession<'a> {
    registry: &'a ToolRegistry,
    policy: ToolPolicy,
    limits: ToolLimits,
    approval: Option<Arc<dyn Approval>>,
    cancellation: CancellationToken,
    deadline: Instant,
    calls: usize,
    ids: BTreeSet<String>,
    events: Vec<ToolEvent>,
}
impl ToolSession<'_> {
    pub fn events(&self) -> &[ToolEvent] {
        &self.events
    }
    pub fn calls_used(&self) -> usize {
        self.calls
    }
    /// Accept a bounded JSON envelope. Duplicate top-level keys/argument keys,
    /// unknown fields, malformed identifiers, and unregistered names fail closed.
    /// Failed admissions consume a slot; IDs remain consumed after errors/cancellation.
    pub async fn execute_json(&mut self, json: &str) -> Result<String, ToolError> {
        if self.calls >= self.limits.max_calls {
            return Err(ToolError::CallLimit);
        }
        self.calls += 1;
        let sequence = self.calls;
        self.events.push(ToolEvent::Requested { sequence });
        // If the host drops this future, its reserved terminal record remains
        // cancelled. The exclusive session borrow hides it while work is active.
        let terminal = self.events.len();
        self.events.push(ToolEvent::Finished {
            sequence,
            error: Some(ToolError::Cancelled),
        });
        let result = self.execute_inner(json).await;
        self.events[terminal] = ToolEvent::Finished {
            sequence,
            error: result.as_ref().err().copied(),
        };
        result
    }
    async fn execute_inner(&mut self, json: &str) -> Result<String, ToolError> {
        if self.cancellation.is_cancelled() {
            return Err(ToolError::Cancelled);
        }
        if Instant::now() >= self.deadline {
            return Err(ToolError::Timeout);
        }
        if json.len() > self.limits.max_input_bytes {
            return Err(ToolError::InputLimit);
        }
        let call: Call = serde_json::from_str(json).map_err(|_| ToolError::InvalidCall)?;
        if !identifier(&call.id) || !identifier(&call.name) {
            return Err(ToolError::InvalidCall);
        }
        if !self.ids.insert(call.id.clone()) {
            return Err(ToolError::DuplicateCall);
        }
        let entry = self
            .registry
            .entries
            .get(&call.name)
            .ok_or(ToolError::UnknownTool)?;
        if !self.policy.allowed_tools.contains(&call.name)
            || (entry.definition.effect == Effect::SideEffect && !self.policy.allow_side_effects)
        {
            return Err(ToolError::Denied);
        }
        let input = entry.definition.input.validate(call.arguments)?;
        let child = self.cancellation.child_token();
        let guard = CancelOnDrop(child.clone());
        let deadline = self
            .deadline
            .min(Instant::now() + Duration::from_millis(self.limits.call_timeout_ms));
        let operation = async {
            if entry.definition.effect == Effect::SideEffect {
                let approval = self.approval.as_ref().ok_or(ToolError::ApprovalRequired)?;
                if !approval.approve(&call.id, &entry.definition, &input).await {
                    return Err(ToolError::ApprovalDenied);
                }
            }
            if child.is_cancelled() {
                return Err(ToolError::Cancelled);
            }
            if Instant::now() >= deadline {
                return Err(ToolError::Timeout);
            }
            let output = entry
                .tool
                .execute(
                    input,
                    ToolContext {
                        cancellation: child.clone(),
                        max_output_bytes: self.limits.max_output_bytes,
                    },
                )
                .await?;
            if output.len() > self.limits.max_output_bytes {
                return Err(ToolError::OutputLimit);
            }
            Ok(output)
        };
        let result = tokio::select! {
            biased;
            _ = self.cancellation.cancelled() => Err(ToolError::Cancelled),
            result = tokio::time::timeout_at(deadline, operation) => result.unwrap_or(Err(ToolError::Timeout)),
        };
        drop(guard);
        result
    }
}
