use oscar_core::{
    config::{Capability, Config, WorkMode},
    context::{Artifact, ArtifactStore, build_context, excerpt},
    error::OscarError,
    execution::{self, CancellationToken, Outcome, Validator, Validators},
    planning::*,
    providers::*,
    routing,
};
use std::{
    collections::BTreeMap,
    sync::{
        Arc, Mutex,
        atomic::{AtomicUsize, Ordering},
    },
    time::Duration,
};

fn config() -> Config {
    Config::parse(include_str!("../../../example_config.toml")).unwrap()
}
fn single(c: &Config) -> Plan {
    plan_request("Write README documentation", c).unwrap()
}
fn output(text: &str) -> Result<ModelOutput, OscarError> {
    Ok(ModelOutput {
        content: text.into(),
        confidence: Confidence::High,
        usage: TokenUsage {
            input: Some(20),
            output: Some(10),
        },
    })
}
fn providers(
    local: Vec<Result<ModelOutput, OscarError>>,
    remote: Vec<Result<ModelOutput, OscarError>>,
) -> Providers {
    Providers {
        local: Some(Arc::new(MockProvider::scripted(local))),
        remote: Some(Arc::new(MockProvider::scripted(remote))),
    }
}
async fn run(plan: Plan, c: Config, p: Providers) -> execution::RunReport {
    execution::execute(plan, c, p, Validators::new(), CancellationToken::new())
        .await
        .unwrap()
}

#[test]
fn modes_routing_and_capability_mismatches() {
    for (mode, expected) in [
        (WorkMode::Local, ProviderPreference::Local),
        (WorkMode::FullRemote, ProviderPreference::Remote),
        (WorkMode::Mixed, ProviderPreference::Local),
    ] {
        let mut c = config();
        c.work_mode = mode;
        let mut task = single(&c).tasks.remove(0);
        for difficulty in [Difficulty::Trivial, Difficulty::Low, Difficulty::Medium] {
            task.difficulty = difficulty;
            assert_eq!(
                routing::select(&c, &task, 0, 100).unwrap().provider,
                expected
            );
        }
        c.providers
            .get_mut(expected.key())
            .unwrap()
            .capabilities
            .clear();
        let result = routing::select(&c, &task, 0, 100);
        if mode == WorkMode::Mixed {
            assert_eq!(result.unwrap().provider, ProviderPreference::Remote);
        } else {
            assert!(result.is_err());
        }
    }
}
#[test]
fn risk_reasoning_and_cost_are_distinct_from_implementation() {
    let mut c = config();
    let mut t = single(&c).tasks.remove(0);
    t.risk = RiskLevel::Critical;
    t.difficulty = Difficulty::Low;
    assert_eq!(
        routing::select(&c, &t, 0, 100).unwrap().provider,
        ProviderPreference::Local
    );
    t.kind = TaskKind::Reason;
    assert_eq!(
        routing::select(&c, &t, 0, 100).unwrap().provider,
        ProviderPreference::Remote
    );
    c.routing.max_remote_call_cost = Some(0.0);
    c.providers
        .get_mut("remote")
        .unwrap()
        .cost
        .input_per_million = Some(1.0);
    c.providers
        .get_mut("remote")
        .unwrap()
        .cost
        .output_per_million = Some(1.0);
    assert_eq!(
        routing::select(&c, &t, 0, 100).unwrap().provider,
        ProviderPreference::Local
    );
    assert!(routing::select(&c, &t, 2, 100).is_err());
}
#[test]
fn valid_config_legacy_alias_and_redaction() {
    let original = include_str!("../../../example_config.toml");
    assert_eq!(
        Config::parse(&original.replace("work_mode = \"mixed\"", "work_mode = \"remote\""))
            .unwrap()
            .work_mode,
        WorkMode::FullRemote
    );
    let mut c = config();
    c.providers.get_mut("local").unwrap().api_key = Some("TOP_SECRET".into());
    assert!(!format!("{c:?}").contains("TOP_SECRET"));
    for bad in [
        original.replace("mixed", "invalid"),
        original.replace("max_local_attempts = 2", "max_local_attempts = 0"),
        original.replace("max_concurrency = 2", "max_concurrency = 0"),
        original.replace("context_window = 32768", "context_window = 10"),
        original.replace("local_first = true", "local_frist = true"),
    ] {
        assert!(Config::parse(&bad).is_err());
    }
    assert!(
        !Config::parse("api_key = 'TOP_SECRET'\nwork_mode = 42")
            .unwrap_err()
            .to_string()
            .contains("TOP_SECRET")
    );
}
#[test]
fn plans_round_trip_render_stably_and_identify_waves() {
    let c = config();
    let p = plan_request("Add JWT authentication and protect admin endpoints", &c).unwrap();
    assert_eq!(
        p.tasks
            .iter()
            .filter(|t| t.preferred_provider == ProviderPreference::Remote)
            .count(),
        2
    );
    let restored = Plan::from_json(&p.to_json().unwrap(), &c).unwrap();
    assert_eq!(p, restored);
    assert_eq!(
        p.render_markdown().unwrap(),
        restored.render_markdown().unwrap()
    );
    assert_eq!(
        p.waves().unwrap(),
        vec![
            vec![TaskId("T01".into())],
            vec![TaskId("T02".into())],
            vec![TaskId("T03".into()), TaskId("T04".into())],
            vec![TaskId("T05".into())]
        ]
    );
    let md = p.render_markdown().unwrap();
    assert!(md.contains("## Expected Remote Calls") && md.contains("### Wave 3"));
}
#[test]
fn malformed_graphs_fail_closed() {
    let c = config();
    let p = plan_request("Implement a DTO", &c).unwrap();
    for variant in 0..6 {
        let mut bad = p.clone();
        match variant {
            0 => bad.tasks[1].id = bad.tasks[0].id.clone(),
            1 => bad.tasks[0].dependencies = vec![TaskId("missing".into())],
            2 => bad.tasks[0].dependencies = vec![bad.tasks[1].id.clone()],
            3 => bad.tasks[0].id = TaskId("../escape".into()),
            4 => bad.version = 9,
            _ => bad.tasks[0].escalation_policy.max_local_attempts = 100,
        }
        assert!(bad.validate(&c).is_err());
    }
}
#[tokio::test]
async fn local_retry_then_remote_with_usage_recorded() {
    let c = config();
    let p = single(&c);
    let report = run(
        p,
        c,
        providers(
            vec![output(""), output("")],
            vec![output("remote evidence")],
        ),
    )
    .await;
    assert_eq!(report.outcome, Outcome::Completed);
    assert_eq!(
        report.calls.iter().map(|c| c.provider).collect::<Vec<_>>(),
        vec![
            ProviderPreference::Local,
            ProviderPreference::Local,
            ProviderPreference::Remote
        ]
    );
    assert_eq!(report.calls[2].usage.output, Some(10));
    assert!(
        report
            .events
            .iter()
            .any(|e| matches!(e, execution::Event::Escalated { .. }))
    );
}
#[tokio::test]
async fn strict_modes_do_not_cross_provider_boundaries() {
    for mode in [WorkMode::Local, WorkMode::FullRemote] {
        let mut c = config();
        c.work_mode = mode;
        let p = single(&c);
        let report = run(
            p,
            c,
            providers(vec![output(""), output("")], vec![output("remote")]),
        )
        .await;
        if mode == WorkMode::Local {
            assert_eq!(report.outcome, Outcome::Failed);
            assert_eq!(report.calls.len(), 2);
            assert!(
                report
                    .calls
                    .iter()
                    .all(|c| c.provider == ProviderPreference::Local)
            );
        } else {
            assert_eq!(report.outcome, Outcome::Completed);
            assert_eq!(report.calls.len(), 1);
            assert_eq!(report.calls[0].provider, ProviderPreference::Remote);
        }
    }
}
#[tokio::test]
async fn retries_only_transient_provider_failures() {
    for transient in [false, true] {
        let c = config();
        let p = single(&c);
        let report = run(
            p,
            c,
            providers(
                vec![
                    Err(OscarError::Provider {
                        transient,
                        message: "test failure".into(),
                    }),
                    output("valid"),
                ],
                vec![],
            ),
        )
        .await;
        assert_eq!(report.calls.len(), if transient { 2 } else { 1 });
        assert_eq!(
            report.outcome,
            if transient {
                Outcome::Completed
            } else {
                Outcome::Failed
            }
        );
    }
}
#[tokio::test]
async fn budgets_and_outputs_are_bounded() {
    let mut c = config();
    let p = single(&c);
    c.limits.max_output_bytes = 2;
    assert_eq!(
        run(p, c, providers(vec![output("too big")], vec![]))
            .await
            .outcome,
        Outcome::LimitReached
    );
    let mut c = config();
    let p = single(&c);
    c.limits.max_remote_calls = 0;
    let r = run(p, c, providers(vec![output(""), output("")], vec![])).await;
    assert_eq!(r.outcome, Outcome::LimitReached);
    assert_eq!(r.calls.len(), 2);
    let mut c = config();
    c.work_mode = WorkMode::FullRemote;
    let p = single(&c);
    c.limits.max_remote_input_bytes = 1;
    let r = run(p, c, providers(vec![], vec![output("never called")])).await;
    assert_eq!(r.outcome, Outcome::LimitReached);
    assert!(r.calls.is_empty());
}
struct Capture {
    inputs: Arc<Mutex<Vec<String>>>,
    active: Arc<AtomicUsize>,
    peak: Arc<AtomicUsize>,
}
impl ModelProvider for Capture {
    fn infer(&self, request: ModelRequest) -> ModelFuture<'_> {
        Box::pin(async move {
            self.inputs.lock().unwrap().push(request.context);
            let active = self.active.fetch_add(1, Ordering::SeqCst) + 1;
            self.peak.fetch_max(active, Ordering::SeqCst);
            tokio::time::sleep(Duration::from_millis(20)).await;
            self.active.fetch_sub(1, Ordering::SeqCst);
            output("source-evidence-marker")
        })
    }
}
#[tokio::test]
async fn parallel_workers_pass_artifacts_and_obey_provider_slots() {
    for slots in [1, 2] {
        let mut c = config();
        c.providers.get_mut("local").unwrap().max_concurrency = slots;
        let p = plan_request("Implement a DTO", &c).unwrap();
        let inputs = Arc::new(Mutex::new(vec![]));
        let peak = Arc::new(AtomicUsize::new(0));
        let provider = Capture {
            inputs: inputs.clone(),
            active: Arc::new(AtomicUsize::new(0)),
            peak: peak.clone(),
        };
        let r = run(
            p,
            c,
            Providers {
                local: Some(Arc::new(provider)),
                remote: None,
            },
        )
        .await;
        assert_eq!(r.outcome, Outcome::Completed);
        assert_eq!(peak.load(Ordering::SeqCst), slots);
        let texts = inputs.lock().unwrap();
        assert_eq!(texts.len(), 3);
        assert!(
            texts[1].contains("source-evidence-marker")
                && texts[2].contains("source-evidence-marker")
        );
    }
}
#[test]
fn distillation_is_bounded_utf8_safe_and_retains_source_identity() {
    let c = config();
    let mut p = plan_request("Implement a DTO", &c).unwrap();
    let t = p.tasks.remove(1);
    let mut artifacts = ArtifactStore::new();
    artifacts.insert(
        TaskId("T01".into()),
        Artifact {
            task_id: TaskId("T01".into()),
            content: "界".repeat(3000).into(),
            provider: ProviderPreference::Local,
            confidence: Confidence::High,
            validation: "format".into(),
            local_only_data: false,
        },
    );
    let remote = build_context(&p, &t, &artifacts, &c, true, "").unwrap();
    let local = build_context(&p, &t, &artifacts, &c, false, "").unwrap();
    assert!(remote.len() < local.len());
    assert!(remote.contains("excerpt truncated") && remote.contains("T01"));
    assert!(excerpt(&"界".repeat(1000), 128).len() <= 128);
    for budget in 0..128 {
        assert!(excerpt(&"界".repeat(1000), budget).len() <= budget);
    }
}
#[tokio::test]
async fn sensitive_artifacts_cannot_escalate_or_lose_their_boundary() {
    let c = config();
    let mut p = plan_request("Implement a DTO", &c).unwrap();
    p.tasks[0].local_only_data = true;
    p.tasks[1].required_capabilities = vec![Capability::AdvancedReasoning];
    let r = run(
        p,
        c,
        providers(
            vec![output("private evidence")],
            vec![output("must not run")],
        ),
    )
    .await;
    assert_eq!(r.outcome, Outcome::Failed);
    assert!(
        r.calls
            .iter()
            .all(|c| c.provider == ProviderPreference::Local)
    );
}
struct Pending;
impl ModelProvider for Pending {
    fn infer(&self, _: ModelRequest) -> ModelFuture<'_> {
        Box::pin(std::future::pending())
    }
}
#[tokio::test]
async fn cancellation_and_deadlines_have_terminal_reports() {
    let mut c = config();
    c.limits.run_timeout_ms = 40;
    c.limits.call_timeout_ms = 30;
    let p = single(&c);
    let r = run(
        p,
        c,
        Providers {
            local: Some(Arc::new(Pending)),
            remote: None,
        },
    )
    .await;
    assert_eq!(r.outcome, Outcome::LimitReached);
    let c = config();
    let p = single(&c);
    let cancel = CancellationToken::new();
    let signal = cancel.clone();
    tokio::spawn(async move {
        tokio::time::sleep(Duration::from_millis(10)).await;
        signal.cancel();
    });
    let r = execution::execute(
        p,
        c,
        Providers {
            local: Some(Arc::new(Pending)),
            remote: None,
        },
        Validators::new(),
        cancel,
    )
    .await
    .unwrap();
    assert_eq!(r.outcome, Outcome::Cancelled);
    assert!(r.calls[0].status.contains("interrupted"));
}
struct Strong;
impl Validator for Strong {
    fn validate<'a>(
        &'a self,
        artifact: &'a str,
    ) -> std::pin::Pin<Box<dyn std::future::Future<Output = Result<(), OscarError>> + Send + 'a>>
    {
        Box::pin(async move {
            if artifact == "verified" {
                Ok(())
            } else {
                Err(OscarError::Validation("failed".into()))
            }
        })
    }
}
#[tokio::test]
async fn deterministic_evidence_overrides_low_confidence_and_unknown_validators_fail() {
    let c = config();
    let mut p = single(&c);
    p.tasks[0].validation = ValidationStrategy::Registered("host-test".into());
    let mut out = output("verified").unwrap();
    out.confidence = Confidence::Low;
    let invalid = execution::execute(
        p.clone(),
        c.clone(),
        providers(vec![Ok(out.clone())], vec![]),
        Validators::new(),
        CancellationToken::new(),
    )
    .await;
    assert!(invalid.is_err());
    let mut validators: Validators = BTreeMap::new();
    validators.insert("host-test".into(), Arc::new(Strong));
    let r = execution::execute(
        p,
        c,
        providers(vec![Ok(out)], vec![]),
        validators,
        CancellationToken::new(),
    )
    .await
    .unwrap();
    assert_eq!(r.outcome, Outcome::Completed);
    assert_eq!(r.calls.len(), 1);
}

#[tokio::test]
async fn routing_uses_distilled_context_for_fit_and_cost_without_fake_failures() {
    let mut c = config();
    c.providers.get_mut("local").unwrap().context_window = 4000;
    c.providers
        .get_mut("remote")
        .unwrap()
        .cost
        .input_per_million = Some(1.0);
    c.providers
        .get_mut("remote")
        .unwrap()
        .cost
        .output_per_million = Some(1.0);
    c.routing.max_remote_call_cost = Some(0.006);
    let p = plan_request("Implement a DTO", &c).unwrap();
    let r = run(
        p,
        c,
        providers(
            vec![output(&"source evidence ".repeat(400))],
            vec![output("proposal"), output("tests")],
        ),
    )
    .await;
    assert_eq!(r.outcome, Outcome::Completed);
    assert_eq!(r.calls.len(), 3);
    assert_eq!(
        r.calls
            .iter()
            .filter(|c| c.provider == ProviderPreference::Remote)
            .count(),
        2
    );
    assert!(r.calls.iter().all(|c| c.attempt == 1));
    assert!(r.events.iter().any(|e| matches!(e, execution::Event::Routed { reason, .. } if reason == "local capability or context gap")));
}

#[tokio::test]
async fn confidence_never_overrides_invalid_output_and_low_confidence_retries() {
    let c = config();
    let mut p = single(&c);
    p.tasks[0].validation = ValidationStrategy::Json;
    let mut low = output("{}").unwrap();
    low.confidence = Confidence::Low;
    let r = run(
        p,
        c,
        providers(vec![output("invalid JSON"), Ok(low)], vec![output("{}")]),
    )
    .await;
    assert_eq!(r.outcome, Outcome::Completed);
    assert_eq!(r.calls.len(), 3);
}

#[test]
fn review_policy_and_zero_remote_budget_keep_local_work_available() {
    let mut c = config();
    c.routing.remote_review = oscar_core::config::RemoteReview::Always;
    let p = single(&c);
    assert_eq!(p.tasks.len(), 2);
    c.limits.max_remote_calls = 0;
    let p = single(&c);
    assert!(
        p.tasks
            .iter()
            .all(|t| t.preferred_provider == ProviderPreference::Local)
    );
    c.routing.remote_review = oscar_core::config::RemoteReview::Never;
    assert_eq!(single(&c).tasks.len(), 1);
}
