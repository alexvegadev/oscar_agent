use oscar_core::{
    config::{Capability, Config, RemoteReview, WorkMode},
    error::OscarError,
    execution::{self, CancellationToken, Outcome, Validators},
    planning::{Confidence, ProviderPreference, TaskId, ValidationStrategy, inference},
    providers::{ModelFuture, ModelOutput, ModelProvider, ModelRequest, Providers, TokenUsage},
};
use serde_json::{Value, json};
use std::sync::{Arc, Mutex};

fn config() -> Config {
    let mut c = Config::parse(include_str!("../../../example_config.toml")).unwrap();
    c.providers
        .get_mut("local")
        .unwrap()
        .capabilities
        .push(Capability::Planning);
    c
}
fn draft() -> Value {
    json!({"tasks":[
        {"id":"schema","title":"Describe parser schema","description":"Specify parser fields and error semantics.",
         "kind":"document","difficulty":"low","risk":"low","dependencies":[],
         "required_capabilities":["documentation"],"context_requirements":["User requirements"],"expected_outputs":["Field specification"]},
        {"id":"examples","title":"Explain invalid inputs","description":"Give examples matching the schema and error semantics.",
         "kind":"document","difficulty":"low","risk":"low","dependencies":["schema"],
         "required_capabilities":["documentation"],"context_requirements":["Schema proposal"],"expected_outputs":["Invalid input examples"]}
    ]})
}
struct Recorder {
    content: String,
    requests: Mutex<Vec<ModelRequest>>,
    delay: u64,
    fail: bool,
}
impl ModelProvider for Recorder {
    fn infer(&self, request: ModelRequest) -> ModelFuture<'_> {
        Box::pin(async move {
            self.requests.lock().unwrap().push(request);
            tokio::time::sleep(std::time::Duration::from_millis(self.delay)).await;
            if self.fail {
                return Err(OscarError::Provider {
                    transient: false,
                    message: "fixture failure".into(),
                });
            }
            // Intentionally ignores max_output_bytes, exercising host-side bounds.
            Ok(ModelOutput {
                content: self.content.clone(),
                confidence: Confidence::High,
                usage: TokenUsage {
                    input: Some(10),
                    output: Some(20),
                },
            })
        })
    }
}
fn providers(content: String, delay: u64, fail: bool) -> (Providers, Arc<Recorder>, Arc<Recorder>) {
    let make = || {
        Arc::new(Recorder {
            content: content.clone(),
            requests: Mutex::default(),
            delay,
            fail,
        })
    };
    let local = make();
    let remote = make();
    (
        Providers {
            local: Some(local.clone()),
            remote: Some(remote.clone()),
        },
        local,
        remote,
    )
}

#[tokio::test]
async fn full_model_dag_uses_host_policy_and_exact_goal_in_each_mode() {
    for (mode, expected) in [
        (WorkMode::Local, ProviderPreference::Local),
        (WorkMode::Mixed, ProviderPreference::Local),
        (WorkMode::FullRemote, ProviderPreference::Remote),
    ] {
        let mut c = config();
        c.work_mode = mode;
        c.providers.get_mut(expected.key()).unwrap().api_key = Some("SECRET_FIXTURE".into());
        let (providers, local, remote) = providers(draft().to_string(), 0, false);
        let result = inference::generate(
            "Document parser schema",
            &c,
            &providers,
            CancellationToken::new(),
        )
        .await;
        let plan = result.plan.unwrap();
        assert_eq!(plan.goal, "Document parser schema");
        assert_eq!(plan.work_mode, mode);
        assert_eq!(plan.tasks.len(), 2); // Heuristic would create one generic task.
        assert_eq!(plan.tasks[0].id, TaskId("schema".into()));
        assert_eq!(plan.tasks[1].dependencies, vec![TaskId("schema".into())]);
        assert_eq!(plan.tasks[1].validation, ValidationStrategy::NonEmpty);
        assert_eq!(plan.waves().unwrap().len(), 2);
        assert_eq!(result.report.outcome, Outcome::Completed);
        assert_eq!(result.report.provider, Some(expected));
        assert_eq!(result.report.usage.output, Some(20));
        let chosen = if expected == ProviderPreference::Local {
            &local
        } else {
            &remote
        };
        let requests = chosen.requests.lock().unwrap();
        assert_eq!(requests.len(), 1);
        assert!(!requests[0].context.contains("SECRET_FIXTURE"));
        assert!(requests[0].context.contains("Document parser schema"));
        assert_eq!(requests[0].context.len(), result.report.input_bytes);
        let other = if expected == ProviderPreference::Local {
            &remote
        } else {
            &local
        };
        assert!(other.requests.lock().unwrap().is_empty());
    }
}

#[tokio::test]
async fn rejects_invalid_graphs_policy_injection_and_unavailable_tasks() {
    let mut variants = vec![
        "not JSON SECRET_FIXTURE".into(),
        format!("```json\n{}\n```", draft()),
    ];
    for mutation in 0..8 {
        let mut value = draft();
        match mutation {
            0 => value["tasks"][0]["dependencies"] = json!(["examples"]),
            1 => value["tasks"][1]["id"] = json!("schema"),
            2 => value["tasks"][1]["dependencies"] = json!(["unknown"]),
            3 => value["tasks"][0]["local_only_data"] = json!(false),
            4 => value["tasks"][0]["validation"] = json!({"kind":"registered","value":"shell"}),
            5 => value["work_mode"] = json!("full_remote"),
            6 => value["tasks"][0]["required_capabilities"] = json!(["vision"]),
            _ => value["tasks"] = json!([]),
        }
        variants.push(value.to_string());
    }
    for text in variants {
        let (providers, local, remote) = providers(text, 0, false);
        let result = inference::generate(
            "Document parser schema",
            &config(),
            &providers,
            CancellationToken::new(),
        )
        .await;
        assert!(result.plan.is_err());
        assert_eq!(result.report.outcome, Outcome::Failed);
        assert_eq!(local.requests.lock().unwrap().len(), 1);
        assert!(remote.requests.lock().unwrap().is_empty()); // No hidden retries/fallback.
        assert!(
            !serde_json::to_string(&result.report)
                .unwrap()
                .contains("SECRET_FIXTURE")
        );
    }
}

#[tokio::test]
async fn respects_local_only_capability_and_remote_budgets_before_calling() {
    for case in 0..6 {
        let mut c = config();
        match case {
            0 => {
                c.work_mode = WorkMode::FullRemote;
                c.planning.local_only_data = true;
            }
            1 => {
                c.work_mode = WorkMode::Local;
                c.providers
                    .get_mut("local")
                    .unwrap()
                    .capabilities
                    .retain(|v| *v != Capability::Planning);
            }
            2 => {
                c.work_mode = WorkMode::FullRemote;
                c.limits.max_remote_calls = 0;
            }
            3 => {
                c.work_mode = WorkMode::FullRemote;
                c.limits.max_remote_input_bytes = 1;
            }
            4 => {
                c.work_mode = WorkMode::FullRemote;
                c.routing.max_remote_call_cost = Some(0.0);
            }
            _ => c.limits.max_input_bytes = 256,
        }
        let (providers, local, remote) = providers(draft().to_string(), 0, false);
        let result = inference::generate(
            "Document parser schema",
            &c,
            &providers,
            CancellationToken::new(),
        )
        .await;
        assert!(result.plan.is_err());
        assert!(!result.report.call_started);
        assert!(
            local.requests.lock().unwrap().is_empty() && remote.requests.lock().unwrap().is_empty()
        );
    }
    let mut c = config();
    c.planning.local_only_data = true;
    let (p, _, remote) = providers(draft().to_string(), 0, false);
    let plan = inference::generate("Document parser schema", &c, &p, CancellationToken::new())
        .await
        .plan
        .unwrap();
    assert!(
        plan.tasks
            .iter()
            .all(|t| t.local_only_data && !t.escalation_policy.allow_remote)
    );
    assert!(remote.requests.lock().unwrap().is_empty());
}

#[tokio::test]
async fn charges_planning_against_execution_and_reports_failed_calls() {
    let mut c = config();
    c.work_mode = WorkMode::FullRemote;
    c.limits.max_remote_calls = 1;
    let (p, _, remote) = providers(draft().to_string(), 0, false);
    let result =
        inference::generate("Document parser schema", &c, &p, CancellationToken::new()).await;
    let remaining = result.report.remaining_config(&c).unwrap();
    assert_eq!(remaining.limits.max_remote_calls, 0);
    assert_eq!(
        remaining.limits.max_remote_input_bytes,
        c.limits.max_remote_input_bytes - result.report.input_bytes
    );
    assert!(remaining.limits.run_timeout_ms <= c.limits.run_timeout_ms);
    let run = execution::execute(
        result.plan.unwrap(),
        remaining,
        p,
        Validators::new(),
        CancellationToken::new(),
    )
    .await
    .unwrap();
    assert_eq!(run.outcome, Outcome::LimitReached);
    assert_eq!(remote.requests.lock().unwrap().len(), 1);
    let (p, _, _) = providers(String::new(), 0, true);
    let result =
        inference::generate("Document parser schema", &c, &p, CancellationToken::new()).await;
    assert!(matches!(result.plan, Err(OscarError::Provider { .. })));
    assert!(result.report.call_started);
    assert_eq!(
        result
            .report
            .remaining_config(&c)
            .unwrap()
            .limits
            .max_remote_calls,
        0
    );
}

#[tokio::test]
async fn enforces_time_cancellation_output_and_task_bounds() {
    let mut c = config();
    c.limits.call_timeout_ms = 5;
    let (p, _, _) = providers(draft().to_string(), 100, false);
    let result =
        inference::generate("Document parser schema", &c, &p, CancellationToken::new()).await;
    assert_eq!(result.report.outcome, Outcome::LimitReached);
    let cancel = CancellationToken::new();
    cancel.cancel();
    let result = inference::generate("Document parser schema", &c, &p, cancel).await;
    assert_eq!(result.report.outcome, Outcome::Cancelled);
    assert!(!result.report.call_started);
    c.limits.call_timeout_ms = 1000;
    let cancel = CancellationToken::new();
    let signal = cancel.clone();
    let canceller = tokio::spawn(async move {
        tokio::time::sleep(std::time::Duration::from_millis(5)).await;
        signal.cancel();
    });
    let result = inference::generate("Document parser schema", &c, &p, cancel).await;
    canceller.await.unwrap();
    assert_eq!(result.report.outcome, Outcome::Cancelled);
    assert!(result.report.call_started);
    c.limits.max_output_bytes = 10;
    let (p, _, _) = providers(draft().to_string(), 0, false);
    assert!(matches!(
        inference::generate("Document parser schema", &c, &p, CancellationToken::new())
            .await
            .plan,
        Err(OscarError::Limit(_))
    ));
    c.limits.max_output_bytes = 16384;
    c.limits.max_tasks = 1;
    assert!(matches!(
        inference::generate("Document parser schema", &c, &p, CancellationToken::new())
            .await
            .plan,
        Err(OscarError::Plan(_))
    ));
}

#[tokio::test]
async fn review_policy_requires_review_of_every_branch() {
    let mut c = config();
    c.routing.remote_review = RemoteReview::Always;
    let mut value = draft();
    let (p, _, _) = providers(value.to_string(), 0, false);
    assert!(
        inference::generate("Document parser schema", &c, &p, CancellationToken::new())
            .await
            .plan
            .is_err()
    );
    value["tasks"][1]["kind"] = json!("review");
    value["tasks"][1]["required_capabilities"] = json!(["review"]);
    let (p, _, _) = providers(value.to_string(), 0, false);
    assert!(
        inference::generate("Document parser schema", &c, &p, CancellationToken::new())
            .await
            .plan
            .is_ok()
    );
    value["tasks"][1]["dependencies"] = json!([]);
    let (p, _, _) = providers(value.to_string(), 0, false);
    assert!(
        inference::generate("Document parser schema", &c, &p, CancellationToken::new())
            .await
            .plan
            .is_err()
    );
}
