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
    assert_eq!(session.messages[1].role, Role::Model);
    session.reset();
    assert!(session.messages.is_empty());
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
    assert!(!session.messages.iter().any(|m| m.content == "late result"));
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
    assert!(
        !session
            .messages
            .iter()
            .any(|m| m.content.contains("secret credential"))
    );
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
