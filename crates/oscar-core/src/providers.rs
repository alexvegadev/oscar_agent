use crate::{
    config::{Config, Provider},
    error::OscarError,
    planning::{Confidence, ProviderPreference},
};
use serde::{Deserialize, Serialize};
use std::{
    collections::VecDeque,
    future::Future,
    pin::Pin,
    sync::{Arc, Mutex},
};

mod chat;
#[cfg(feature = "http")]
mod http;

pub type ModelFuture<'a> =
    Pin<Box<dyn Future<Output = Result<ModelOutput, OscarError>> + Send + 'a>>;
/// Inline text delivery with no queue. Sinks must be nonblocking and bounded.
/// Returning an error aborts inference; dropping the future cancels client work.
/// Deltas are provisional until the future returns a successful final output.
/// A delta may contain multiple tokens and is always complete UTF-8 text.
pub type ModelTextSink<'a> = dyn FnMut(&str) -> Result<(), OscarError> + Send + 'a;
/// Implementations must be nonblocking, cancellation-safe on drop, and inference-only.
/// Never execute model-generated tools here. Host adapters must enforce output limits.
pub trait ModelProvider: Send + Sync {
    /// Native tool adapters return ordinary answer text and typed calls. Legacy
    /// injected providers may retain the desktop JSON-envelope protocol.
    fn native_tool_protocol(&self) -> bool {
        false
    }
    fn infer(&self, request: ModelRequest) -> ModelFuture<'_>;

    /// Native adapters override this to deliver text as it arrives. Legacy
    /// providers deliver their complete response once, without simulated tokens.
    /// Final output carries authoritative content, confidence and token usage.
    fn infer_stream<'a>(
        &'a self,
        request: ModelRequest,
        sink: &'a mut ModelTextSink<'_>,
    ) -> ModelFuture<'a> {
        Box::pin(async move {
            let limit = request.max_output_bytes;
            let output = self.infer(request).await?;
            if output.byte_len() > limit {
                return Err(OscarError::Limit("model output exceeds byte limit".into()));
            }
            if !output.content.is_empty() {
                sink(&output.content)?;
            }
            Ok(output)
        })
    }
}
#[derive(Clone, Serialize)]
pub struct ModelRequest {
    pub context: String,
    /// Structured conversation. Empty keeps the legacy single-user context path.
    pub messages: Vec<ModelMessage>,
    /// Host-selected registry definitions. Empty disables native tool calls.
    pub tools: Vec<crate::tools::ToolDefinition>,
    pub max_output_bytes: usize,
    pub max_output_tokens: u32,
}
impl ModelRequest {
    /// Count serialized native messages, schemas and fixed protocol overhead.
    /// Legacy context is also counted for providers that still consume it.
    pub fn input_bytes(&self, model: &str) -> Result<usize, OscarError> {
        let wire = chat::body(model, self)?;
        Ok(serde_json::to_vec(&wire)
            .map_err(|_| OscarError::Validation("cannot encode model request".into()))?
            .len()
            .max(self.context.len()))
    }
}
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct TokenUsage {
    pub input: Option<u64>,
    pub output: Option<u64>,
}
#[derive(Debug, Clone, Serialize)]
pub struct ModelOutput {
    pub content: String,
    /// Proposed actions only; adapters never execute these calls.
    pub tool_calls: Vec<ModelToolCall>,
    pub confidence: Confidence,
    pub usage: TokenUsage,
}

/// Provider-neutral call. Preserve raw JSON so duplicate argument keys remain
/// visible to registry validation. Debug deliberately redacts payloads.
#[derive(Clone, Serialize, Deserialize)]
pub struct ModelToolCall {
    pub id: String,
    pub name: String,
    pub arguments: String,
}
impl std::fmt::Debug for ModelToolCall {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("ModelToolCall([redacted])")
    }
}

#[derive(Clone, Serialize)]
#[serde(tag = "role", rename_all = "snake_case")]
pub enum ModelMessage {
    User {
        content: String,
    },
    Assistant {
        content: String,
        tool_calls: Vec<ModelToolCall>,
    },
    Tool {
        tool_call_id: String,
        content: String,
    },
}
impl std::fmt::Debug for ModelMessage {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("ModelMessage([redacted])")
    }
}

impl ModelOutput {
    /// Text plus call identifiers, names and raw arguments share one byte budget.
    pub fn byte_len(&self) -> usize {
        self.tool_calls
            .iter()
            .fold(self.content.len(), |bytes, call| {
                bytes
                    .saturating_add(call.id.len())
                    .saturating_add(call.name.len())
                    .saturating_add(call.arguments.len())
            })
    }
    pub fn require_text(&self) -> Result<(), OscarError> {
        if self.tool_calls.is_empty() {
            Ok(())
        } else {
            Err(OscarError::Validation(
                "tool calls are disabled for this request".into(),
            ))
        }
    }
}

/// Scripted provider for deterministic offline testing. Exhausted scripts fail closed.
pub struct MockProvider {
    replies: Mutex<VecDeque<Result<ModelOutput, OscarError>>>,
    demo: bool,
    native_tools: bool,
}
impl MockProvider {
    pub fn scripted(replies: Vec<Result<ModelOutput, OscarError>>) -> Self {
        Self {
            replies: Mutex::new(replies.into()),
            demo: false,
            native_tools: false,
        }
    }
    /// Script native calls followed by ordinary Markdown answers.
    pub fn scripted_native(replies: Vec<Result<ModelOutput, OscarError>>) -> Self {
        Self {
            native_tools: true,
            ..Self::scripted(replies)
        }
    }
    pub fn demo() -> Self {
        Self {
            replies: Mutex::new(VecDeque::new()),
            demo: true,
            native_tools: false,
        }
    }
}
impl ModelProvider for MockProvider {
    fn native_tool_protocol(&self) -> bool {
        self.native_tools
    }
    fn infer(&self, request: ModelRequest) -> ModelFuture<'_> {
        Box::pin(async move {
            let reply = if self.demo {
                Ok(ModelOutput {
                    tool_calls: Vec::new(),
                    content: "Mock proposal artifact. No repository changes or commands executed."
                        .into(),
                    confidence: Confidence::High,
                    usage: TokenUsage::default(),
                })
            } else {
                self.replies
                    .lock()
                    .map_err(|_| OscarError::Provider {
                        transient: false,
                        message: "mock queue unavailable".into(),
                    })?
                    .pop_front()
                    .unwrap_or_else(|| {
                        Err(OscarError::Provider {
                            transient: false,
                            message: "mock script exhausted".into(),
                        })
                    })
            }?;
            if reply.byte_len() > request.max_output_bytes {
                return Err(OscarError::Limit("model output exceeds byte limit".into()));
            }
            if request.tools.is_empty() {
                reply.require_text()?;
            }
            Ok(reply)
        })
    }
}

#[derive(Clone, Default)]
pub struct Providers {
    pub local: Option<Arc<dyn ModelProvider>>,
    pub remote: Option<Arc<dyn ModelProvider>>,
}
impl Providers {
    pub fn get(&self, choice: ProviderPreference) -> Result<Arc<dyn ModelProvider>, OscarError> {
        match choice {
            ProviderPreference::Local => &self.local,
            ProviderPreference::Remote => &self.remote,
        }
        .clone()
        .ok_or_else(|| {
            OscarError::Unavailable(format!("{} adapter is not configured", choice.key()))
        })
    }
    pub fn from_config(config: &Config) -> Result<Self, OscarError> {
        config.validate()?;
        let build = |name: &str| -> Result<Option<Arc<dyn ModelProvider>>, OscarError> {
            config
                .providers
                .get(name)
                .filter(|p| p.enabled)
                .map(|p| build_provider(p, name, config))
                .transpose()
        };
        // Strict modes never initialize the opposite adapter or read its credentials.
        Ok(Self {
            local: if config.work_mode == crate::config::WorkMode::FullRemote {
                None
            } else {
                build("local")?
            },
            remote: if config.work_mode == crate::config::WorkMode::Local {
                None
            } else {
                build("remote")?
            },
        })
    }
}
fn build_provider(
    p: &Provider,
    name: &str,
    config: &Config,
) -> Result<Arc<dyn ModelProvider>, OscarError> {
    if let Some(protocol) = &p.protocol {
        if protocol != "openai_chat" || p.provider.as_deref() == Some("mock") {
            return Err(OscarError::Config(
                "protocol must be openai_chat for a non-mock provider".into(),
            ));
        }
        #[cfg(feature = "http")]
        {
            return Ok(Arc::new(http::HttpProvider::new(
                p,
                name == "local",
                config.limits.call_timeout_ms,
            )?));
        }
        #[cfg(not(feature = "http"))]
        {
            let _ = (name, config);
            return Err(OscarError::Config(
                "Rig providers require the http Cargo feature".into(),
            ));
        }
    }
    match p.provider.as_deref() {
        Some("mock") => Ok(Arc::new(MockProvider::demo())),
        Some("openai_compatible" | "openai" | "openrouter" | "groq" | "deepseek" | "xai" | "ollama" | "lmstudio") => {
            #[cfg(feature = "http")]
            {
                Ok(Arc::new(http::HttpProvider::new(
                    p,
                    name == "local",
                    config.limits.call_timeout_ms,
                )?))
            }
            #[cfg(not(feature = "http"))]
            {
                let _ = (name, config);
                Err(OscarError::Config(
                    "Rig providers require the http Cargo feature".into(),
                ))
            }
        }
        _ => Err(OscarError::Config(
            "unsupported provider; use mock, openai_compatible, openai, openrouter, groq, deepseek, xai, ollama or lmstudio".into(),
        )),
    }
}
