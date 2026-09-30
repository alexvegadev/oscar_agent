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

#[cfg(feature = "http")]
mod http;

pub type ModelFuture<'a> =
    Pin<Box<dyn Future<Output = Result<ModelOutput, OscarError>> + Send + 'a>>;
/// Implementations must be nonblocking, cancellation-safe on drop, and inference-only.
/// Never execute model-generated tools here. Host adapters must enforce output limits.
pub trait ModelProvider: Send + Sync {
    fn infer(&self, request: ModelRequest) -> ModelFuture<'_>;
}
#[derive(Clone)]
pub struct ModelRequest {
    pub context: String,
    pub max_output_bytes: usize,
    pub max_output_tokens: u32,
}
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct TokenUsage {
    pub input: Option<u64>,
    pub output: Option<u64>,
}
#[derive(Debug, Clone)]
pub struct ModelOutput {
    pub content: String,
    pub confidence: Confidence,
    pub usage: TokenUsage,
}

/// Scripted provider for deterministic offline testing. Exhausted scripts fail closed.
pub struct MockProvider {
    replies: Mutex<VecDeque<Result<ModelOutput, OscarError>>>,
    demo: bool,
}
impl MockProvider {
    pub fn scripted(replies: Vec<Result<ModelOutput, OscarError>>) -> Self {
        Self {
            replies: Mutex::new(replies.into()),
            demo: false,
        }
    }
    pub fn demo() -> Self {
        Self {
            replies: Mutex::new(VecDeque::new()),
            demo: true,
        }
    }
}
impl ModelProvider for MockProvider {
    fn infer(&self, request: ModelRequest) -> ModelFuture<'_> {
        Box::pin(async move {
            let reply = if self.demo {
                Ok(ModelOutput {
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
            if reply.content.len() > request.max_output_bytes {
                return Err(OscarError::Limit("model output exceeds byte limit".into()));
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
    match p.provider.as_deref() {
        Some("mock") => Ok(Arc::new(MockProvider::demo())),
        Some("openai_compatible") => {
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
                    "openai_compatible requires the http Cargo feature".into(),
                ))
            }
        }
        _ => Err(OscarError::Config(
            "provider must be mock or openai_compatible".into(),
        )),
    }
}
