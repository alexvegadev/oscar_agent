use oscar_code::{
    session::{Role, RunState, SessionState},
    settings::Settings,
};
use oscar_core::{
    config::{Limits, WorkMode},
    error::OscarError,
    planning::{Confidence, ProviderPreference},
    providers::{MockProvider, ModelFuture, ModelOutput, ModelProvider, ModelRequest, TokenUsage},
};
use std::sync::Arc;

fn output(text: &str) -> ModelOutput {
    ModelOutput {
        tool_calls: Vec::new(),
        content: text.into(),
        confidence: Confidence::High,
        usage: TokenUsage::default(),
    }
}

#[tokio::test]
async fn mock_chat_completes_and_reset_preserves_budget_and_ids() {
    let mut session = SessionState::new(
        Limits::default(),
        Settings {
            max_turns: 2,
            ..Settings::default()
        },
    );
    let provider = Arc::new(MockProvider::scripted(vec![
        Ok(output("# Proposal")),
        Ok(output("Follow-up")),
    ]));
    let run = session.begin("Write a parser").unwrap();
    assert_eq!(session.state, RunState::Pending);
    let id = run.id;
    session.running(id);
    session.complete(id, run.execute(provider.clone()).await);
    assert_eq!(session.state, RunState::Completed);
    assert_eq!(session.messages()[1].role(), Role::Model);
    session.reset();
    assert!(session.messages().is_empty());
    assert_eq!(session.turns, 1);
    let run = session.begin("Try again").unwrap();
    assert!(run.id > id);
    let id = run.id;
    session.complete(id, run.execute(provider).await);
    session.reset();
    assert!(matches!(
        session.begin("No budget"),
        Err(OscarError::Limit(_))
    ));
    assert_eq!(session.state, RunState::LimitReached);
}

#[test]
fn invalid_input_duplicate_submit_and_stale_completions_are_rejected() {
    let mut session = SessionState::new(Limits::default(), Settings::default());
    assert!(session.begin(" \n").is_err());
    assert_eq!(session.turns, 0);
    assert!(session.begin(&"x".repeat(24_001)).is_err());
    assert_eq!(session.turns, 0);
    let run = session.begin("First").unwrap();
    assert!(session.begin("Second").is_err());
    session.stop();
    assert_eq!(session.state, RunState::Cancelled);
    session.complete(run.id, Ok(output("late result")));
    assert!(
        !session
            .messages()
            .iter()
            .any(|m| m.text() == Some("late result"))
    );
    session.reset();
    let next = session.begin("Next").unwrap();
    session.complete(run.id, Ok(output("stale")));
    assert!(session.state.active());
    session.complete(next.id, Ok(output("current")));
    assert_eq!(session.state, RunState::Completed);
}

struct Pending;
impl ModelProvider for Pending {
    fn infer(&self, _: ModelRequest) -> ModelFuture<'_> {
        Box::pin(std::future::pending())
    }
}

#[tokio::test]
async fn deadlines_and_cancelled_runs_stop_waiting() {
    let mut session = SessionState::new(
        Limits::default(),
        Settings {
            timeout_ms: 1,
            ..Settings::default()
        },
    );
    let run = session.begin("Wait").unwrap();
    let id = run.id;
    session.complete(id, run.execute(Arc::new(Pending)).await);
    assert_eq!(session.state, RunState::LimitReached);
    let run = session.begin("Cancel").unwrap();
    session.stop();
    assert!(matches!(
        run.execute(Arc::new(Pending)).await,
        Err(OscarError::Cancelled)
    ));
}

#[tokio::test]
async fn adapter_errors_are_redacted_and_output_is_independently_bounded() {
    let mut session = SessionState::new(Limits::default(), Settings::default());
    let run = session.begin("Fail").unwrap();
    let id = run.id;
    session.complete(
        id,
        run.execute(Arc::new(MockProvider::scripted(vec![Err(
            OscarError::Provider {
                transient: false,
                message: "secret credential".into(),
            },
        )])))
        .await,
    );
    assert_eq!(session.state, RunState::Failed);
    assert!(!session.messages().iter().any(|m| {
        m.text()
            .is_some_and(|text| text.contains("secret credential"))
    }));
    let run = session.begin("Oversized").unwrap();
    session.complete(run.id, Ok(output(&"x".repeat(16_385))));
    assert_eq!(session.state, RunState::LimitReached);
}

#[test]
fn settings_reject_policy_conflicts_unknown_fields_and_invalid_limits() {
    let mut settings = Settings::default();
    assert!(settings.validate(WorkMode::Local).is_ok());
    settings.provider = ProviderPreference::Remote;
    assert!(settings.validate(WorkMode::Local).is_err());
    assert!(settings.validate(WorkMode::FullRemote).is_ok());
    settings.max_turns = 0;
    assert!(settings.validate(WorkMode::Mixed).is_err());
    assert!(toml::from_str::<Settings>("api_key = 'secret'").is_err());
    let text = toml::to_string(&Settings::default()).unwrap();
    let restored: Settings = toml::from_str(&text).unwrap();
    assert_eq!(restored.max_turns, 32);
    assert!(!text.contains("api_key"));
}

#[tokio::test]
async fn follow_up_context_is_bounded_without_silently_discarding_history() {
    let limits = Limits {
        max_input_bytes: 256,
        ..Limits::default()
    };
    let mut session = SessionState::new(limits, Settings::default());
    let run = session.begin("A").unwrap();
    session.complete(run.id, Ok(output(&"x".repeat(200))));
    assert!(matches!(
        session.begin("Follow up"),
        Err(OscarError::Limit(_))
    ));
    assert_eq!(session.turns, 1);
    session.reset();
    assert!(session.begin("After reset").is_ok());
}

#[test]
fn remote_budget_survives_reset_and_provider_switches() {
    let limits = Limits {
        max_remote_calls: 1,
        ..Limits::default()
    };
    let remote = Settings {
        provider: ProviderPreference::Remote,
        ..Settings::default()
    };
    let mut session = SessionState::new(limits, remote.clone());
    let run = session.begin("Share once").unwrap();
    session.complete(run.id, Ok(output("Reply")));
    session.reset();
    session.update_settings(Settings::default()).unwrap();
    let local = session.begin("Local is still allowed").unwrap();
    session.complete(local.id, Ok(output("Local reply")));
    session.reset();
    session.update_settings(remote).unwrap();
    assert!(matches!(
        session.begin("Share twice"),
        Err(OscarError::Limit(_))
    ));
    assert_eq!(session.turns, 2);
}

struct Streaming;
impl ModelProvider for Streaming {
    fn infer(&self, _: ModelRequest) -> ModelFuture<'_> {
        panic!("interactive runs must use the streaming contract")
    }
    fn infer_stream<'a>(
        &'a self,
        _: ModelRequest,
        sink: &'a mut oscar_core::providers::ModelTextSink<'_>,
    ) -> ModelFuture<'a> {
        Box::pin(async move {
            sink("hé")?;
            tokio::task::yield_now().await;
            sink("🦀")?;
            Ok(output("hé🦀"))
        })
    }
}

#[tokio::test]
async fn native_stream_commits_once_and_provisional_text_is_not_history() {
    let mut session = SessionState::new(Limits::default(), Settings::default());
    let run = session.begin("Stream").unwrap();
    let id = run.id;
    session.running(id);
    let mut text = String::new();
    let result = run
        .execute_stream(Arc::new(Streaming), &mut |delta| {
            text.push_str(delta);
            session.preview(id, text.clone());
            assert_eq!(session.messages().len(), 1);
            Ok(())
        })
        .await;
    assert_eq!(session.preview, "hé🦀");
    session.complete(id, result);
    assert!(session.preview.is_empty());
    assert_eq!(session.messages().len(), 2);
    assert_eq!(session.messages()[1].text(), Some("hé🦀"));
}

#[test]
fn stop_reset_failure_and_limits_discard_previews_and_ignore_late_updates() {
    let mut session = SessionState::new(Limits::default(), Settings::default());
    let run = session.begin("First").unwrap();
    session.preview(run.id, "partial".into());
    session.preview(run.id, "x".repeat(16_385));
    assert_eq!(session.preview, "partial");
    session.stop();
    session.preview(run.id, "late".into());
    assert!(session.preview.is_empty());
    session.reset();
    let next = session.begin("Next").unwrap();
    session.preview(run.id, "stale".into());
    assert!(session.preview.is_empty());
    session.preview(next.id, "unfinished".into());
    session.complete(next.id, Err(OscarError::Cancelled));
    assert!(session.preview.is_empty());
    assert!(!session.messages().iter().any(|m| m.role() == Role::Model));
}

#[tokio::test]
async fn host_enforces_cumulative_stream_limit_before_delivery_and_sink_can_abort() {
    let mut session = SessionState::new(
        Limits {
            max_output_bytes: 3,
            ..Limits::default()
        },
        Settings::default(),
    );
    let run = session.begin("Bound").unwrap();
    let mut text = String::new();
    assert!(matches!(
        run.execute_stream(Arc::new(Streaming), &mut |delta| {
            text.push_str(delta);
            Ok(())
        })
        .await,
        Err(OscarError::Limit(_))
    ));
    assert_eq!(text, "hé");
    session.reset();
    let run = session.begin("Abort").unwrap();
    assert!(matches!(
        run.execute_stream(Arc::new(Streaming), &mut |_| Err(OscarError::Cancelled))
            .await,
        Err(OscarError::Cancelled)
    ));
}

struct PartialPending;
impl ModelProvider for PartialPending {
    fn infer(&self, _: ModelRequest) -> ModelFuture<'_> {
        Box::pin(std::future::pending())
    }
    fn infer_stream<'a>(
        &'a self,
        _: ModelRequest,
        sink: &'a mut oscar_core::providers::ModelTextSink<'_>,
    ) -> ModelFuture<'a> {
        Box::pin(async move {
            sink("partial")?;
            std::future::pending().await
        })
    }
}

#[tokio::test]
async fn deadline_and_stop_after_a_delta_discard_partial_text() {
    for stop in [false, true] {
        let mut session = SessionState::new(
            Limits::default(),
            Settings {
                timeout_ms: 1,
                ..Settings::default()
            },
        );
        let run = session.begin("Wait after text").unwrap();
        let id = run.id;
        let result = run
            .execute_stream(Arc::new(PartialPending), &mut |delta| {
                session.preview(id, delta.into());
                if stop {
                    session.stop();
                }
                Ok(())
            })
            .await;
        if stop {
            assert!(matches!(result, Err(OscarError::Cancelled)));
        } else {
            assert!(matches!(result, Err(OscarError::Limit(_))));
        }
        session.complete(id, result);
        assert!(session.preview.is_empty());
        assert!(!session.messages().iter().any(|m| m.role() == Role::Model));
    }
}

struct UnsolicitedCall;
impl ModelProvider for UnsolicitedCall {
    fn infer(&self, request: ModelRequest) -> ModelFuture<'_> {
        assert!(request.tools.is_empty());
        Box::pin(async {
            let mut output = output("claimed answer");
            output
                .tool_calls
                .push(oscar_core::providers::ModelToolCall {
                    id: "hidden".into(),
                    name: "shell".into(),
                    arguments: "{}".into(),
                });
            Ok(output)
        })
    }
}
#[tokio::test]
async fn proposal_mode_rejects_unsolicited_typed_calls_from_injected_providers() {
    let mut session = SessionState::new(Limits::default(), Settings::default());
    let run = session.begin("Propose").unwrap();
    let id = run.id;
    let result = run.execute(Arc::new(UnsolicitedCall)).await;
    assert!(matches!(result, Err(OscarError::Validation(_))));
    session.complete(id, result);
    assert_eq!(session.state, RunState::Failed);
    assert!(
        !session
            .messages()
            .iter()
            .any(|m| m.text() == Some("claimed answer"))
    );
}
