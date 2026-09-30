use crate::{
    config::{Config, WorkMode},
    context::{Artifact, ArtifactStore, build_context},
    error::OscarError,
    planning::{Confidence, Plan, ProviderPreference as P, Task, TaskId, ValidationStrategy},
    providers::{ModelRequest, Providers, TokenUsage},
    routing,
};
use serde::{Deserialize, Serialize};
use std::{
    collections::BTreeMap,
    future::Future,
    pin::Pin,
    sync::{
        Arc,
        atomic::{AtomicU64, Ordering},
    },
    time::{Duration, SystemTime, UNIX_EPOCH},
};
use tokio::{
    sync::{Mutex, Semaphore},
    task::JoinSet,
};
pub use tokio_util::sync::CancellationToken;

/// A host-owned, explicitly registered validator. Implementations must be
/// nonblocking/cancellation-safe and must enforce their own tool approval policy.
/// A plan can name a validator but cannot register or authorize one.
pub trait Validator: Send + Sync {
    fn validate<'a>(
        &'a self,
        artifact: &'a str,
    ) -> Pin<Box<dyn Future<Output = Result<(), OscarError>> + Send + 'a>>;
}
pub type Validators = BTreeMap<String, Arc<dyn Validator>>;
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Outcome {
    Completed,
    Failed,
    Cancelled,
    LimitReached,
}
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum Event {
    Started,
    Routed {
        task: TaskId,
        provider: P,
        attempt: usize,
        reason: String,
    },
    Escalated {
        task: TaskId,
        reason: String,
    },
    Validation {
        task: TaskId,
        passed: bool,
        strong: bool,
    },
    Finished {
        outcome: Outcome,
    },
}
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CallRecord {
    pub task: TaskId,
    pub provider: P,
    pub attempt: usize,
    pub input_bytes: usize,
    pub usage: TokenUsage,
    pub simulation: bool,
    pub status: String,
}
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RunReport {
    pub version: u32,
    pub run_id: String,
    pub outcome: Outcome,
    pub error: Option<String>,
    pub events: Vec<Event>,
    pub calls: Vec<CallRecord>,
    pub artifacts: ArtifactStore,
}
#[derive(Default)]
struct Ledger {
    events: Vec<Event>,
    calls: Vec<CallRecord>,
    remote_calls: usize,
    remote_bytes: usize,
}
struct RunContext {
    config: Config,
    plan: Plan,
    providers: Providers,
    validators: Validators,
    local_slots: Semaphore,
    remote_slots: Semaphore,
    ledger: Mutex<Ledger>,
    cancel: CancellationToken,
}
static RUN_SEQUENCE: AtomicU64 = AtomicU64::new(0);

/// Run a validated DAG. Independent tasks execute concurrently in bounded waves.
/// All retained content is bounded by max_tasks, attempt counts and byte limits.
pub async fn execute(
    plan: Plan,
    config: Config,
    providers: Providers,
    validators: Validators,
    cancel: CancellationToken,
) -> Result<RunReport, OscarError> {
    plan.validate(&config)?;
    for task in &plan.tasks {
        if let ValidationStrategy::Registered(name) = &task.validation {
            if !validators.contains_key(name) {
                return Err(OscarError::Plan(
                    "plan names an unregistered validator".into(),
                ));
            }
        }
    }
    let run_id = format!(
        "{}-{}-{}",
        std::process::id(),
        SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap_or_default()
            .as_nanos(),
        RUN_SEQUENCE.fetch_add(1, Ordering::Relaxed)
    );
    let timeout = Duration::from_millis(config.limits.run_timeout_ms);
    let cap = |key| config.providers.get(key).map_or(1, |p| p.max_concurrency);
    let context = Arc::new(RunContext {
        local_slots: Semaphore::new(cap("local")),
        remote_slots: Semaphore::new(cap("remote")),
        config,
        plan,
        providers,
        validators,
        ledger: Mutex::new(Ledger {
            events: vec![Event::Started],
            ..Ledger::default()
        }),
        cancel,
    });
    let mut artifacts = ArtifactStore::new();
    let mut jobs = JoinSet::new();
    let result = tokio::select! {
        biased;
        _ = context.cancel.cancelled() => Err(OscarError::Cancelled),
        result = tokio::time::timeout(timeout, execute_waves(&context, &mut artifacts, &mut jobs)) => {
            result.unwrap_or_else(|_| Err(OscarError::Limit("run deadline exceeded".into())))
        }
    };
    jobs.shutdown().await;
    let outcome = match &result {
        Ok(()) => Outcome::Completed,
        Err(OscarError::Cancelled) => Outcome::Cancelled,
        Err(OscarError::Limit(_)) => Outcome::LimitReached,
        Err(_) => Outcome::Failed,
    };
    let mut ledger = context.ledger.lock().await;
    for call in &mut ledger.calls {
        if call.status == "started" {
            call.status = "interrupted; token usage unknown".into();
        }
    }
    ledger.events.push(Event::Finished { outcome });
    Ok(RunReport {
        version: 1,
        run_id,
        outcome,
        error: result.err().map(|e| e.to_string()),
        events: std::mem::take(&mut ledger.events),
        calls: std::mem::take(&mut ledger.calls),
        artifacts,
    })
}
async fn execute_waves(
    context: &Arc<RunContext>,
    artifacts: &mut ArtifactStore,
    jobs: &mut JoinSet<Result<Artifact, OscarError>>,
) -> Result<(), OscarError> {
    for wave in context.plan.waves()? {
        let inputs = Arc::new(artifacts.clone()); // Arc content avoids copying large prompts.
        let mut pending = wave.into_iter();
        loop {
            while jobs.len() < context.config.limits.max_concurrency {
                let Some(id) = pending.next() else { break };
                let task = context
                    .plan
                    .tasks
                    .iter()
                    .find(|t| t.id == id)
                    .cloned()
                    .ok_or_else(|| OscarError::Plan("missing scheduled task".into()))?;
                let (context, inputs) = (Arc::clone(context), Arc::clone(&inputs));
                jobs.spawn(async move { execute_task(&context, task, &inputs).await });
            }
            let Some(result) = jobs.join_next().await else {
                break;
            };
            let artifact = result.map_err(|_| OscarError::Provider {
                transient: false,
                message: "worker terminated unexpectedly".into(),
            })??;
            artifacts.insert(artifact.task_id.clone(), artifact);
        }
    }
    Ok(())
}
async fn execute_task(
    context: &RunContext,
    mut task: Task,
    artifacts: &ArtifactStore,
) -> Result<Artifact, OscarError> {
    // Sensitivity propagates transitively and cannot be erased by a summary.
    task.local_only_data |= task
        .dependencies
        .iter()
        .filter_map(|d| artifacts.get(d))
        .any(|a| a.local_only_data);
    let config = &context.config;
    let mut local_failures = 0;
    let mut feedback = String::new();
    let mut previous = None;
    let mut attempt = 0;
    loop {
        if context.cancel.is_cancelled() {
            return Err(OscarError::Cancelled);
        }
        attempt += 1;
        let local_context =
            build_context(&context.plan, &task, artifacts, config, false, &feedback);
        let remote_context = if config.work_mode != WorkMode::Local && !task.local_only_data {
            Some(build_context(
                &context.plan,
                &task,
                artifacts,
                config,
                true,
                &feedback,
            ))
        } else {
            None
        };
        // Cost and context checks use the exact candidate prompts, after excerpting.
        let local_bytes = local_context.as_ref().map_or(usize::MAX / 2, String::len);
        let remote_bytes = remote_context
            .as_ref()
            .and_then(|r| r.as_ref().ok())
            .map_or(usize::MAX / 2, String::len);
        let decision = routing::select_with_context_sizes(
            config,
            &task,
            local_failures,
            local_bytes,
            remote_bytes,
        )?;
        let prompt = match decision.provider {
            P::Local => local_context?,
            P::Remote => remote_context.ok_or_else(|| {
                OscarError::Unavailable("remote data transfer forbidden".into())
            })??,
        };
        let provider = context.providers.get(decision.provider)?;
        let slots = if decision.provider == P::Local {
            &context.local_slots
        } else {
            &context.remote_slots
        };
        let _permit = slots.acquire().await.map_err(|_| OscarError::Cancelled)?;
        if context.cancel.is_cancelled() {
            return Err(OscarError::Cancelled);
        }
        let call_index = {
            let mut ledger = context.ledger.lock().await;
            if decision.provider == P::Remote {
                if ledger.remote_calls >= config.limits.max_remote_calls
                    || ledger.remote_bytes.saturating_add(prompt.len())
                        > config.limits.max_remote_input_bytes
                {
                    return Err(OscarError::Limit(
                        "remote call/input budget exhausted".into(),
                    ));
                }
                ledger.remote_calls += 1;
                ledger.remote_bytes += prompt.len();
            }
            if previous == Some(P::Local) && decision.provider == P::Remote {
                ledger.events.push(Event::Escalated {
                    task: task.id.clone(),
                    reason: decision.reason.into(),
                });
            }
            ledger.events.push(Event::Routed {
                task: task.id.clone(),
                provider: decision.provider,
                attempt,
                reason: decision.reason.into(),
            });
            let index = ledger.calls.len();
            ledger.calls.push(CallRecord {
                task: task.id.clone(),
                provider: decision.provider,
                attempt,
                input_bytes: prompt.len(),
                usage: TokenUsage::default(),
                status: "started".into(),
                simulation: config
                    .providers
                    .get(decision.provider.key())
                    .is_some_and(|p| p.provider.as_deref() == Some("mock")),
            });
            index
        };
        let result = tokio::time::timeout(
            Duration::from_millis(config.limits.call_timeout_ms),
            provider.infer(ModelRequest {
                context: prompt,
                max_output_bytes: config.limits.max_output_bytes,
                max_output_tokens: config.limits.max_output_tokens,
            }),
        )
        .await
        .unwrap_or_else(|_| {
            Err(OscarError::Provider {
                transient: true,
                message: "inference deadline exceeded".into(),
            })
        });
        drop(_permit);
        {
            let mut ledger = context.ledger.lock().await;
            ledger.calls[call_index].status =
                if result.is_ok() { "returned" } else { "failed" }.into();
            if let Ok(output) = &result {
                ledger.calls[call_index].usage = output.usage.clone();
            }
        }
        let failure = match result {
            Ok(output) => {
                if output.content.len() > config.limits.max_output_bytes {
                    return Err(OscarError::Limit("worker output exceeds byte limit".into()));
                }
                let validation = tokio::time::timeout(
                    Duration::from_millis(config.limits.call_timeout_ms),
                    validate(&task.validation, &output.content, &context.validators),
                )
                .await
                .map_err(|_| OscarError::Limit("validator deadline exceeded".into()))?;
                let strong = matches!(task.validation, ValidationStrategy::Registered(_));
                let passed = validation.is_ok();
                context.ledger.lock().await.events.push(Event::Validation {
                    task: task.id.clone(),
                    passed,
                    strong,
                });
                if passed && (strong || output.confidence != Confidence::Low) {
                    return Ok(Artifact {
                        task_id: task.id,
                        content: output.content.into(),
                        provider: decision.provider,
                        confidence: output.confidence,
                        validation: if strong {
                            "host deterministic validator passed"
                        } else {
                            "artifact format only; code not executed"
                        }
                        .into(),
                        local_only_data: task.local_only_data,
                    });
                }
                if !passed {
                    OscarError::Validation("artifact failed declared validator; correct the output against its requirements".into())
                } else {
                    OscarError::Validation("low confidence without strong deterministic evidence; revisit assumptions and missing context".into())
                }
            }
            Err(
                e @ OscarError::Provider {
                    transient: true, ..
                },
            ) => e,
            Err(e) => return Err(e),
        };
        if decision.provider == P::Remote {
            return Err(failure);
        }
        local_failures += 1;
        if config.work_mode == WorkMode::Local
            && local_failures >= task.escalation_policy.max_local_attempts
        {
            return Err(OscarError::Unavailable(
                "local attempt limit reached; local mode forbids remote escalation".into(),
            ));
        }
        // Feedback is generated by the runtime, never raw provider or validator errors.
        feedback = match failure {
            OscarError::Validation(_) => format!(
                "Previous artifact failed validation or confidence checks. Recheck {:?}; supply corrected evidence and address missing assumptions.",
                task.validation
            ),
            _ => "Previous inference failed transiently; retry the same proposal-only task.".into(),
        };
        previous = Some(decision.provider);
        tokio::time::sleep(Duration::from_millis(25 * local_failures as u64)).await;
    }
}
async fn validate(
    strategy: &ValidationStrategy,
    content: &str,
    validators: &Validators,
) -> Result<(), OscarError> {
    let pass = match strategy {
        ValidationStrategy::NonEmpty => !content.trim().is_empty(),
        ValidationStrategy::Json => serde_json::from_str::<serde_json::Value>(content).is_ok(),
        ValidationStrategy::Contains(text) => content.contains(text),
        ValidationStrategy::Registered(name) => {
            return validators
                .get(name)
                .ok_or_else(|| OscarError::Plan("unregistered validator".into()))?
                .validate(content)
                .await;
        }
    };
    if pass {
        Ok(())
    } else {
        Err(OscarError::Validation("artifact format rejected".into()))
    }
}
