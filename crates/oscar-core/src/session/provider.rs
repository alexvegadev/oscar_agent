//! Provider-neutral tracing wrapper. HTTP adapters additionally capture actual
//! outbound bodies through the task-local scope, excluding transport credentials.
use super::{AttemptOrigin, Journal, JournalEvent};
use crate::{
    error::OscarError,
    planning::ProviderPreference,
    providers::{ModelFuture, ModelProvider, ModelRequest, ModelTextSink, Providers},
};
use std::sync::Arc;

#[derive(Clone)]
struct CallTrace {
    #[cfg(feature = "http")]
    journal: Journal,
    #[cfg(feature = "http")]
    call: u64,
}
tokio::task_local! { static ACTIVE_CALL: CallTrace; }
tokio::task_local! { static ACTIVE_ATTEMPT: AttemptOrigin; }

pub(crate) async fn scope_attempt<T>(
    origin: AttemptOrigin,
    operation: impl std::future::Future<Output = T>,
) -> T {
    ACTIVE_ATTEMPT.scope(origin, operation).await
}

/// Capture the exact UTF-8 JSON body immediately before transport dispatch.
/// No header, URL, credential configuration or response error body is accepted.
#[cfg(feature = "http")]
pub(crate) async fn record_wire_request(body: &[u8]) -> Result<(), OscarError> {
    let Some(trace) = ACTIVE_CALL.try_with(Clone::clone).ok() else {
        return Ok(());
    };
    let text = std::str::from_utf8(body)
        .map_err(|_| OscarError::Validation("non-UTF-8 provider request".into()))?;
    trace
        .journal
        .append(JournalEvent::WireRequest {
            call: trace.call,
            payload: trace.journal.payload(text)?,
        })
        .await?;
    Ok(())
}

struct AuditedProvider {
    inner: Arc<dyn ModelProvider>,
    journal: Journal,
    provider: ProviderPreference,
}
impl ModelProvider for AuditedProvider {
    fn native_tool_protocol(&self) -> bool {
        self.inner.native_tool_protocol()
    }
    fn infer(&self, request: ModelRequest) -> ModelFuture<'_> {
        Box::pin(async move { self.infer_stream(request, &mut |_| Ok(())).await })
    }
    fn infer_stream<'a>(
        &'a self,
        request: ModelRequest,
        sink: &'a mut ModelTextSink<'_>,
    ) -> ModelFuture<'a> {
        Box::pin(async move {
            let output_limit = request.max_output_bytes;
            let call = self
                .journal
                .append(JournalEvent::Request {
                    provider: self.provider,
                    origin: ACTIVE_ATTEMPT.try_with(Clone::clone).ok(),
                    payload: self.journal.serialized_payload(&request)?,
                })
                .await?;
            let mut streamed_bytes = 0usize;
            let mut sink_failure: Option<OscarError> = None;
            let mut trace_sink = |text: &str| {
                if let Some(error) = &sink_failure {
                    return Err(error.clone());
                }
                let result = (|| {
                    streamed_bytes = streamed_bytes
                        .checked_add(text.len())
                        .ok_or_else(|| OscarError::Limit("stream byte limit exceeded".into()))?;
                    if streamed_bytes > output_limit {
                        return Err(OscarError::Limit("stream byte limit exceeded".into()));
                    }
                    self.journal.try_append(JournalEvent::ResponseDelta {
                        call,
                        payload: self.journal.payload(text)?,
                    })?;
                    sink(text)
                })();
                if let Err(error) = &result {
                    sink_failure = Some(error.clone());
                }
                result
            };
            let result = ACTIVE_CALL
                .scope(
                    CallTrace {
                        #[cfg(feature = "http")]
                        journal: self.journal.clone(),
                        #[cfg(feature = "http")]
                        call,
                    },
                    self.inner.infer_stream(request, &mut trace_sink),
                )
                .await;
            let result = match sink_failure {
                Some(error) => Err(error),
                None => result,
            };
            let result = result.and_then(|output| {
                if output.byte_len() > output_limit {
                    Err(OscarError::Limit("model output exceeds byte limit".into()))
                } else {
                    Ok(output)
                }
            });
            match &result {
                Ok(output) => {
                    self.journal
                        .append(JournalEvent::Response {
                            call,
                            payload: self.journal.serialized_payload(output)?,
                        })
                        .await?;
                }
                Err(_) => {
                    self.journal
                        .append(JournalEvent::CallFailed { call })
                        .await?;
                }
            }
            result
        })
    }
}

impl Providers {
    /// Wrap once at the host entry point, before planning and execution. Generic
    /// providers record logical requests; built-in HTTP records wire bodies too.
    pub fn with_journal(self, journal: Journal) -> Self {
        let wrap = |inner, provider| {
            Arc::new(AuditedProvider {
                inner,
                journal: journal.clone(),
                provider,
            }) as Arc<dyn ModelProvider>
        };
        Self {
            local: self.local.map(|p| wrap(p, ProviderPreference::Local)),
            remote: self.remote.map(|p| wrap(p, ProviderPreference::Remote)),
        }
    }
}
