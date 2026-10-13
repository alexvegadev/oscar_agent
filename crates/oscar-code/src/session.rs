use crate::settings::Settings;
use oscar_core::{
    config::Limits,
    error::OscarError,
    planning::ProviderPreference,
    providers::{ModelOutput, ModelProvider, ModelRequest},
};
use std::{sync::Arc, time::Duration};
use tokio_util::sync::CancellationToken;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Role {
    User,
    Model,
    Tool,
    System,
}

#[derive(Debug, Clone)]
pub struct Message {
    pub role: Role,
    pub content: String,
}

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
    pub messages: Vec<Message>,
    pub state: RunState,
    pub turns: usize,
    remote_calls: usize,
    remote_input_bytes: usize,
    run_id: u64,
    cancellation: CancellationToken,
    limits: Limits,
    settings: Settings,
}

pub struct Run {
    pub id: u64,
    request: ModelRequest,
    cancellation: CancellationToken,
    timeout: Duration,
}

impl SessionState {
    pub fn new(limits: Limits, settings: Settings) -> Self {
        Self {
            messages: Vec::new(),
            state: RunState::Idle,
            turns: 0,
            remote_calls: 0,
            remote_input_bytes: 0,
            run_id: 0,
            cancellation: CancellationToken::new(),
            limits,
            settings,
        }
    }

    pub fn begin(&mut self, prompt: &str) -> Result<Run, OscarError> {
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
        // Build only bounded conversation context. No external content can add tools.
        let mut context = String::from(
            "You are OSCAR, a coding proposal assistant. Return Markdown proposals. You cannot execute tools or modify files.\n",
        );
        for message in &self.messages {
            if matches!(message.role, Role::User | Role::Model) {
                context.push_str(if message.role == Role::User {
                    "\nUSER:\n"
                } else {
                    "\nASSISTANT:\n"
                });
                context.push_str(&message.content);
            }
        }
        if prompt.len() > self.limits.max_input_bytes
            || context.len().saturating_add(prompt.len()).saturating_add(8)
                > self.limits.max_input_bytes
        {
            return Err(OscarError::Limit("conversation exceeds input limit; Reset clears context without resetting the budget".into()));
        }
        if self.retained_bytes().saturating_add(prompt.len()) > self.retention_limit() {
            self.state = RunState::LimitReached;
            return Err(OscarError::Limit(
                "desktop retained history limit reached; Reset clears context".into(),
            ));
        }
        context.push_str("\nUSER:\n");
        context.push_str(prompt);
        if self.settings.provider == ProviderPreference::Remote {
            if self.remote_calls >= self.limits.max_remote_calls
                || self.remote_input_bytes.saturating_add(context.len())
                    > self.limits.max_remote_input_bytes
            {
                self.state = RunState::LimitReached;
                return Err(OscarError::Limit("desktop remote budget exhausted".into()));
            }
            self.remote_calls += 1;
            self.remote_input_bytes += context.len();
        }
        self.run_id = self
            .run_id
            .checked_add(1)
            .ok_or_else(|| OscarError::Limit("run IDs exhausted".into()))?;
        self.turns += 1;
        self.cancellation = CancellationToken::new();
        self.messages.push(Message {
            role: Role::User,
            content: prompt.into(),
        });
        self.state = RunState::Pending;
        Ok(Run {
            id: self.run_id,
            request: ModelRequest {
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

    pub fn complete(&mut self, id: u64, result: Result<ModelOutput, OscarError>) {
        // Discard stale results, including results racing with Stop or Reset.
        if id != self.run_id || !self.state.active() {
            return;
        }
        match result {
            Ok(output) if output.content.len() <= self.limits.max_output_bytes => {
                let retained: usize = self.messages.iter().map(|m| m.content.len()).sum();
                if retained.saturating_add(output.content.len())
                    > self
                        .limits
                        .max_input_bytes
                        .saturating_add(self.limits.max_output_bytes)
                {
                    self.state = RunState::LimitReached;
                } else {
                    self.messages.push(Message {
                        role: Role::Model,
                        content: output.content,
                    });
                    self.state = RunState::Completed;
                }
            }
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
            self.state = RunState::Cancelled;
            self.system_message(format!("Run {} cancelled", self.run_id));
        }
    }

    pub fn reset(&mut self) {
        self.stop();
        self.messages.clear();
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

    fn retained_bytes(&self) -> usize {
        self.messages
            .iter()
            .map(|message| message.content.len())
            .sum()
    }
    fn retention_limit(&self) -> usize {
        self.limits
            .max_input_bytes
            .saturating_add(self.limits.max_output_bytes)
    }
    fn system_message(&mut self, content: String) {
        // The header always carries the outcome, even when history is full.
        if self.retained_bytes().saturating_add(content.len()) <= self.retention_limit() {
            self.messages.push(Message {
                role: Role::System,
                content,
            });
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
        tokio::select! {
            biased;
            _ = self.cancellation.cancelled() => Err(OscarError::Cancelled),
            result = tokio::time::timeout(self.timeout, provider.infer(self.request)) => result.unwrap_or_else(|_| Err(OscarError::Limit("desktop inference deadline exceeded".into()))),
        }
    }
}
