pub use crate::messages::{Message, Role};
use crate::messages::{MessageError, ToolCall, ToolResult};
use crate::runtime::{ModelBudget, RuntimeStatus};
use crate::settings::Settings;
use oscar_core::{
    config::Limits,
    error::OscarError,
    providers::{ModelOutput, ModelProvider, ModelRequest, ModelTextSink},
};
use std::{sync::Arc, time::Duration};
use tokio_util::sync::CancellationToken;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RunState {
    Idle,
    Pending,
    Running,
    Completed,
    Failed,
    Cancelled,
    LimitReached,
}

impl RunState {
    pub fn active(self) -> bool {
        matches!(self, Self::Pending | Self::Running)
    }
}

/// One active inference at a time; IDs and consumed turns survive Reset.
pub struct SessionState {
    messages: Vec<Message>,
    active_history_start: usize,
    pub state: RunState,
    pub turns: usize,
    /// Provisional text, rendered separately and never included in model context.
    pub preview: String,
    /// Latest run's bounded, redacted event metadata.
    pub(crate) runtime_events: Vec<RuntimeStatus>,
    pub(crate) runtime_sequence: usize,
    pub(crate) budget: ModelBudget,
    run_id: u64,
    cancellation: CancellationToken,
    pub(crate) limits: Limits,
    pub(crate) settings: Settings,
}

pub struct Run {
    pub id: u64,
    pub(crate) request: ModelRequest,
    pub(crate) cancellation: CancellationToken,
    pub(crate) timeout: Duration,
}

impl SessionState {
    pub fn new(limits: Limits, settings: Settings) -> Self {
        Self {
            messages: Vec::new(),
            active_history_start: 0,
            state: RunState::Idle,
            turns: 0,
            preview: String::new(),
            runtime_events: Vec::new(),
            runtime_sequence: 0,
            budget: ModelBudget::default(),
            run_id: 0,
            cancellation: CancellationToken::new(),
            limits,
            settings,
        }
    }

    pub fn begin(&mut self, prompt: &str) -> Result<Run, OscarError> {
        if prompt.len() > self.limits.max_input_bytes {
            return Err(OscarError::Limit("message exceeds input byte limit".into()));
        }
        // Only text enters proposal context; autonomous runs build a separate protocol.
        let mut context = String::from(
            "You are OSCAR, a coding proposal assistant. Return Markdown proposals. You cannot execute tools or modify files.\n",
        );
        for message in &self.messages {
            if let Message::User(content) | Message::Model(content) = message {
                context.push_str(if message.role() == Role::User {
                    "\nUSER:\n"
                } else {
                    "\nASSISTANT:\n"
                });
                context.push_str(content);
            }
        }
        context.push_str("\nUSER:\n");
        context.push_str(prompt);
        self.begin_context(prompt, context)
    }

    pub(crate) fn begin_context(
        &mut self,
        prompt: &str,
        context: String,
    ) -> Result<Run, OscarError> {
        let bytes = context.len();
        self.begin_context_bytes(prompt, context, bytes)
    }

    pub(crate) fn begin_context_bytes(
        &mut self,
        prompt: &str,
        context: String,
        bytes: usize,
    ) -> Result<Run, OscarError> {
        self.sync_budget();
        if self.state.active() {
            return Err(OscarError::Unavailable(
                "a desktop run is already active".into(),
            ));
        }
        if prompt.trim().is_empty() {
            return Err(OscarError::Config("enter a message".into()));
        }
        if self.turns >= self.settings.max_turns {
            self.state = RunState::LimitReached;
            return Err(OscarError::Limit(
                "desktop turn budget exhausted; restart to create a new budget".into(),
            ));
        }
        if prompt.len() > self.limits.max_input_bytes || bytes > self.limits.max_input_bytes {
            return Err(OscarError::Limit("conversation exceeds input limit; Reset clears context without resetting the budget".into()));
        }
        if self.retained_bytes().saturating_add(prompt.len()) > self.retention_limit() {
            self.state = RunState::LimitReached;
            return Err(OscarError::Limit(
                "desktop retained history limit reached; Reset clears context".into(),
            ));
        }
        self.budget
            .admit(bytes, &self.limits, &self.settings)
            .inspect_err(|_| {
                self.state = RunState::LimitReached;
            })?;
        self.run_id = self
            .run_id
            .checked_add(1)
            .ok_or_else(|| OscarError::Limit("run IDs exhausted".into()))?;
        self.sync_budget();
        self.cancellation = CancellationToken::new();
        self.active_history_start = self.messages.len();
        self.runtime_events.clear();
        self.runtime_sequence = 0;
        self.messages.push(Message::User(prompt.into()));
        self.preview.clear();
        self.state = RunState::Pending;
        Ok(Run {
            id: self.run_id,
            request: ModelRequest {
                messages: Vec::new(),
                tools: Vec::new(),
                context,
                max_output_bytes: self.limits.max_output_bytes,
                max_output_tokens: self.limits.max_output_tokens,
            },
            cancellation: self.cancellation.clone(),
            timeout: Duration::from_millis(
                self.settings
                    .timeout_ms
                    .min(self.limits.call_timeout_ms)
                    .min(self.limits.run_timeout_ms),
            ),
        })
    }

    pub fn running(&mut self, id: u64) {
        if self.run_id == id && self.state == RunState::Pending {
            self.state = RunState::Running;
        }
    }

    /// Apply a coalesced stream snapshot only to the current active run.
    pub fn preview(&mut self, id: u64, text: String) {
        if id == self.run_id && self.state.active() && text.len() <= self.limits.max_output_bytes {
            self.preview = text;
        }
    }

    pub fn complete(&mut self, id: u64, result: Result<ModelOutput, OscarError>) {
        // Discard stale results, including results racing with Stop or Reset.
        if id != self.run_id || !self.state.active() {
            return;
        }
        self.preview.clear();
        match result {
            Ok(output)
                if output.require_text().is_ok()
                    && output.byte_len() <= self.limits.max_output_bytes =>
            {
                let retained = self.retained_bytes();
                if retained.saturating_add(output.content.len())
                    > self
                        .limits
                        .max_input_bytes
                        .saturating_add(self.limits.max_output_bytes)
                {
                    self.state = RunState::LimitReached;
                } else {
                    self.messages.push(Message::Model(output.content));
                    self.state = RunState::Completed;
                }
            }
            Ok(output) if !output.tool_calls.is_empty() => self.state = RunState::Failed,
            Ok(_) | Err(OscarError::Limit(_)) => self.state = RunState::LimitReached,
            Err(OscarError::Cancelled) => self.state = RunState::Cancelled,
            Err(_) => self.state = RunState::Failed,
        }
        // Do not display provider errors: arbitrary adapters may include secrets.
        if self.state != RunState::Completed {
            self.system_message(format!("Run {}: {:?}", self.run_id, self.state));
        }
    }

    pub fn stop(&mut self) {
        if self.state.active() {
            self.cancellation.cancel();
            self.preview.clear();
            self.state = RunState::Cancelled;
            self.system_message(format!("Run {} cancelled", self.run_id));
        }
    }

    pub fn reset(&mut self) {
        self.stop();
        self.sync_budget();
        self.messages.clear();
        self.runtime_events.clear();
        self.active_history_start = 0;
        self.state = RunState::Idle;
    }

    pub fn update_settings(&mut self, settings: Settings) -> Result<(), OscarError> {
        if self.state.active() {
            return Err(OscarError::Unavailable(
                "stop the active run before changing settings".into(),
            ));
        }
        self.settings = settings;
        Ok(())
    }

    pub fn run_id(&self) -> u64 {
        self.run_id
    }

    /// Read-only history; insertion goes through bounded, typed session methods.
    pub fn messages(&self) -> &[Message] {
        &self.messages
    }

    pub fn runtime_events(&self) -> &[RuntimeStatus] {
        &self.runtime_events
    }

    pub(crate) fn sync_budget(&mut self) {
        self.turns = self.budget.turns();
    }

    /// Record a host-supplied proposal for display; does not authorize or execute it.
    /// IDs are unique throughout retained history. Stop/Reset reject late arrivals.
    pub fn record_tool_call(&mut self, id: u64, call: ToolCall) -> Result<(), MessageError> {
        self.check_tool_run(id)?;
        if self.messages.iter().any(
            |message| matches!(message, Message::ToolCall(existing) if existing.id() == call.id()),
        ) {
            return Err(MessageError::DuplicateCall);
        }
        self.retain_tool_message(Message::ToolCall(call), self.limits.max_input_bytes)
    }

    /// Record exactly one result for a call in the current run with a matching name.
    pub fn record_tool_result(&mut self, id: u64, result: ToolResult) -> Result<(), MessageError> {
        self.check_tool_run(id)?;
        let call = self.messages[self.active_history_start..]
            .iter()
            .find_map(|message| match message {
                Message::ToolCall(call) if call.id() == result.call_id() => Some(call),
                _ => None,
            })
            .ok_or(MessageError::UnknownCall)?;
        if call.name() != result.name() {
            return Err(MessageError::MismatchedTool);
        }
        if self.messages[self.active_history_start..].iter().any(|message| {
            matches!(message, Message::ToolResult(existing) if existing.call_id() == result.call_id())
        }) {
            return Err(MessageError::DuplicateResult);
        }
        self.retain_tool_message(Message::ToolResult(result), self.limits.max_output_bytes)
    }

    fn check_tool_run(&self, id: u64) -> Result<(), MessageError> {
        if id != self.run_id || !self.state.active() {
            return Err(MessageError::InactiveRun);
        }
        Ok(())
    }

    fn retain_tool_message(
        &mut self,
        message: Message,
        max_bytes: usize,
    ) -> Result<(), MessageError> {
        if message.byte_len() > max_bytes {
            return Err(MessageError::ByteLimit);
        }
        if self.retained_bytes().saturating_add(message.byte_len()) > self.retention_limit() {
            return Err(MessageError::HistoryLimit);
        }
        self.messages.push(message);
        Ok(())
    }

    pub(crate) fn retained_bytes(&self) -> usize {
        self.messages.iter().fold(0usize, |bytes, message| {
            bytes.saturating_add(message.byte_len())
        })
    }
    fn retention_limit(&self) -> usize {
        self.limits
            .max_input_bytes
            .saturating_add(self.limits.max_output_bytes)
    }
    fn system_message(&mut self, content: String) {
        // The header always carries the outcome, even when history is full.
        if self.retained_bytes().saturating_add(content.len()) <= self.retention_limit() {
            self.messages.push(Message::System(content));
        }
    }
}

impl Drop for SessionState {
    fn drop(&mut self) {
        self.cancellation.cancel();
    }
}

impl Run {
    pub fn input_bytes(&self) -> usize {
        self.request.context.len()
    }
    pub async fn execute(
        self,
        provider: Arc<dyn ModelProvider>,
    ) -> Result<ModelOutput, OscarError> {
        self.execute_stream(provider, &mut |_| Ok(())).await
    }
    pub async fn execute_stream(
        self,
        provider: Arc<dyn ModelProvider>,
        sink: &mut ModelTextSink<'_>,
    ) -> Result<ModelOutput, OscarError> {
        let limit = self.request.max_output_bytes;
        let mut delivered = 0usize;
        let mut bounded_sink = |delta: &str| {
            delivered = delivered.saturating_add(delta.len());
            if delivered > limit {
                return Err(OscarError::Limit("model output exceeds byte limit".into()));
            }
            sink(delta)
        };
        let output = tokio::select! {
            biased;
            _ = self.cancellation.cancelled() => Err(OscarError::Cancelled),
            result = tokio::time::timeout(self.timeout, provider.infer_stream(self.request, &mut bounded_sink)) => result.unwrap_or_else(|_| Err(OscarError::Limit("desktop inference deadline exceeded".into()))),
        }?;
        output.require_text()?;
        if output.byte_len() > limit {
            return Err(OscarError::Limit("model output exceeds byte limit".into()));
        }
        Ok(output)
    }
}
