use oscar_core::{
    config::{Config, WorkMode},
    error::OscarError,
    execution::{self, CancellationToken, ExecutionOptions, Outcome, Validator, Validators},
    planning::{Confidence, ProviderPreference, ValidationStrategy, plan_request},
    providers::{MockProvider, ModelOutput, ModelRequest, Providers, TokenUsage},
    session::{CaptureMode, Journal, JournalEvent, JournalLimits, JournalRecord, verify},
    validation::ValidationPolicy,
};
use std::{
    future::Future,
    path::PathBuf,
    pin::Pin,
    sync::{
        Arc,
        atomic::{AtomicUsize, Ordering},
    },
};

static NEXT: AtomicUsize = AtomicUsize::new(0);

struct StreamingFixture {
    burst: bool,
}

struct IgnoresSink;
impl oscar_core::providers::ModelProvider for IgnoresSink {
    fn infer(&self, _: ModelRequest) -> oscar_core::providers::ModelFuture<'_> {
        panic!("stream expected")
    }
    fn infer_stream<'a>(
        &'a self,
        _: ModelRequest,
        sink: &'a mut oscar_core::providers::ModelTextSink<'_>,
    ) -> oscar_core::providers::ModelFuture<'a> {
        Box::pin(async move {
            let _ = sink(&"x".repeat(4097));
            output("ignored sink error")
        })
    }
}

#[tokio::test]
async fn ignored_sink_failure_cannot_become_a_successful_response() {
    let path = path();
    let journal = Journal::create(&path, CaptureMode::Content, JournalLimits::default())
        .await
        .unwrap();
    let providers = Providers {
        local: Some(Arc::new(IgnoresSink)),
        remote: None,
    }
    .with_journal(journal.clone());
    assert!(matches!(
        providers
            .get(ProviderPreference::Local)
            .unwrap()
            .infer(request())
            .await,
        Err(OscarError::Limit(_))
    ));
    journal.close().await.unwrap();
    let rows = records(&path);
    assert!(
        rows.iter()
            .any(|r| matches!(r.event, JournalEvent::CallFailed { .. }))
    );
    assert!(
        !rows
            .iter()
            .any(|r| matches!(r.event, JournalEvent::Response { .. }))
    );
}
impl oscar_core::providers::ModelProvider for StreamingFixture {
    fn infer(&self, _: ModelRequest) -> oscar_core::providers::ModelFuture<'_> {
        panic!("must preserve native streaming");
    }
    fn infer_stream<'a>(
        &'a self,
        _: ModelRequest,
        sink: &'a mut oscar_core::providers::ModelTextSink<'_>,
    ) -> oscar_core::providers::ModelFuture<'a> {
        Box::pin(async move {
            sink("partial")?;
            if self.burst {
                for _ in 0..128 {
                    sink("x")?;
                }
                output("unreachable")
            } else {
                std::future::pending().await
            }
        })
    }
}

#[tokio::test]
async fn cancelled_stream_keeps_provisional_evidence_without_fabricating_response() {
    let path = path();
    let journal = Journal::create(&path, CaptureMode::Content, JournalLimits::default())
        .await
        .unwrap();
    let providers = Providers {
        local: Some(Arc::new(StreamingFixture { burst: false })),
        remote: None,
    }
    .with_journal(journal.clone());
    let provider = providers.get(ProviderPreference::Local).unwrap();
    let (sender, observed) = tokio::sync::oneshot::channel();
    let task = tokio::spawn(async move {
        let mut sender = Some(sender);
        provider
            .infer_stream(request(), &mut |_| {
                if let Some(sender) = sender.take() {
                    let _ = sender.send(());
                }
                Ok(())
            })
            .await
    });
    tokio::time::timeout(std::time::Duration::from_secs(5), observed)
        .await
        .unwrap()
        .unwrap();
    task.abort();
    assert!(task.await.unwrap_err().is_cancelled());
    journal.close().await.unwrap();
    let rows = records(&path);
    assert!(
        rows.iter()
            .any(|r| matches!(r.event, JournalEvent::ResponseDelta { .. }))
    );
    assert!(
        !rows
            .iter()
            .any(|r| matches!(r.event, JournalEvent::Response { .. }))
    );
    verify(&path, JournalLimits::default()).await.unwrap();
}

#[tokio::test]
async fn streaming_queue_overflow_fails_closed() {
    let path = path();
    let journal = Journal::create(
        &path,
        CaptureMode::Content,
        JournalLimits {
            queue_capacity: 1,
            ..Default::default()
        },
    )
    .await
    .unwrap();
    let providers = Providers {
        local: Some(Arc::new(StreamingFixture { burst: true })),
        remote: None,
    }
    .with_journal(journal.clone());
    assert!(
        providers
            .get(ProviderPreference::Local)
            .unwrap()
            .infer(request())
            .await
            .is_err()
    );
    assert!(journal.append(JournalEvent::ContextReset).await.is_err());
}
fn path() -> PathBuf {
    let directory = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../target/session-tests");
    std::fs::create_dir_all(&directory).unwrap();
    directory.join(format!(
        "{}-{}.jsonl",
        std::process::id(),
        NEXT.fetch_add(1, Ordering::Relaxed)
    ))
}
fn request() -> ModelRequest {
    ModelRequest {
        context: "PRIVATE_PROMPT".into(),
        messages: vec![],
        tools: vec![],
        max_output_bytes: 4096,
        max_output_tokens: 256,
    }
}
fn output(text: &str) -> Result<ModelOutput, OscarError> {
    Ok(ModelOutput {
        content: text.into(),
        tool_calls: vec![],
        confidence: Confidence::High,
        usage: TokenUsage::default(),
    })
}
fn providers(replies: Vec<Result<ModelOutput, OscarError>>) -> Providers {
    Providers {
        local: Some(Arc::new(MockProvider::scripted(replies))),
        remote: None,
    }
}
fn records(path: &std::path::Path) -> Vec<JournalRecord> {
    std::fs::read_to_string(path)
        .unwrap()
        .lines()
        .map(|line| serde_json::from_str(line).unwrap())
        .collect()
}

#[tokio::test]
async fn content_capture_reconstructs_requests_and_responses_and_never_overwrites() {
    let path = path();
    let log = Journal::create(&path, CaptureMode::Content, JournalLimits::default())
        .await
        .unwrap();
    let p = providers(vec![output("PRIVATE_REPLY")]).with_journal(log.clone());
    p.get(ProviderPreference::Local)
        .unwrap()
        .infer(request())
        .await
        .unwrap();
    log.close().await.unwrap();
    let state = verify(&path, JournalLimits::default()).await.unwrap();
    assert!(state.closed && !state.incomplete_tail);
    let rows = records(&path);
    let call = rows
        .iter()
        .find_map(|r| match &r.event {
            JournalEvent::Request { payload, .. } => {
                let value: serde_json::Value =
                    serde_json::from_str(payload.content.as_ref().unwrap()).unwrap();
                assert_eq!(value["context"], "PRIVATE_PROMPT");
                assert!(payload.sha256.is_some());
                Some(r.sequence)
            }
            _ => None,
        })
        .unwrap();
    assert!(rows.iter().any(|r| matches!(&r.event, JournalEvent::Response { call: id, payload } if *id == call && payload.content.as_ref().unwrap().contains("PRIVATE_REPLY"))));
    assert!(
        Journal::create(&path, CaptureMode::Content, JournalLimits::default())
            .await
            .is_err()
    );
    assert!(log.append(JournalEvent::ContextReset).await.is_err());
}

#[tokio::test]
async fn metadata_does_not_persist_content_or_content_hashes() {
    let path = path();
    let log = Journal::create(&path, CaptureMode::Metadata, JournalLimits::default())
        .await
        .unwrap();
    let p = providers(vec![output("PRIVATE_REPLY")]).with_journal(log.clone());
    p.get(ProviderPreference::Local)
        .unwrap()
        .infer(request())
        .await
        .unwrap();
    log.close().await.unwrap();
    let text = std::fs::read_to_string(path).unwrap();
    assert!(!text.contains("PRIVATE_PROMPT") && !text.contains("PRIVATE_REPLY"));
    assert!(text.contains("\"content\":null") && text.contains("\"sha256\":null"));
}

#[tokio::test]
async fn concurrent_records_have_one_order_and_valid_chain() {
    let path = path();
    let log = Journal::create(&path, CaptureMode::Metadata, JournalLimits::default())
        .await
        .unwrap();
    let mut jobs = tokio::task::JoinSet::new();
    for _ in 0..16 {
        let log = log.clone();
        jobs.spawn(async move { log.append(JournalEvent::ContextReset).await.unwrap() });
    }
    let mut sequences = Vec::new();
    while let Some(result) = jobs.join_next().await {
        sequences.push(result.unwrap());
    }
    sequences.sort();
    assert_eq!(sequences, (2..=17).collect::<Vec<_>>());
    log.close().await.unwrap();
    assert_eq!(
        verify(&path, JournalLimits::default())
            .await
            .unwrap()
            .records,
        18
    );
}

#[tokio::test]
async fn recovery_distinguishes_partial_tail_and_corruption_without_repair() {
    let path = path();
    let log = Journal::create(&path, CaptureMode::Content, JournalLimits::default())
        .await
        .unwrap();
    log.append(JournalEvent::ContextReset).await.unwrap();
    // An unclosed but valid prefix is an interrupted session.
    let original = std::fs::read(&path).unwrap();
    let state = verify(&path, JournalLimits::default()).await.unwrap();
    assert!(!state.closed && !state.incomplete_tail);
    drop(log);
    let mut truncated = original.clone();
    truncated.extend_from_slice(b"{\"version\":");
    std::fs::write(&path, &truncated).unwrap();
    let state = verify(&path, JournalLimits::default()).await.unwrap();
    assert!(state.incomplete_tail);
    assert_eq!(std::fs::read(&path).unwrap(), truncated);
    let corrupt = String::from_utf8(original)
        .unwrap()
        .replace("context_reset", "call_failed");
    std::fs::write(&path, corrupt).unwrap();
    assert!(verify(&path, JournalLimits::default()).await.is_err());
}

#[tokio::test]
async fn capacity_failure_is_sticky_and_blocks_inference() {
    let path = path();
    let limits = JournalLimits {
        max_record_bytes: 1024,
        max_total_bytes: 1024,
        ..Default::default()
    };
    let log = Journal::create(&path, CaptureMode::Content, limits)
        .await
        .unwrap();
    for _ in 0..16 {
        if log.append(JournalEvent::ContextReset).await.is_err() {
            break;
        }
    }
    assert!(log.append(JournalEvent::ContextReset).await.is_err());
    let mock = Arc::new(MockProvider::scripted(vec![output("not consumed")]));
    let p = Providers {
        local: Some(mock.clone()),
        remote: None,
    }
    .with_journal(log);
    assert!(
        p.get(ProviderPreference::Local)
            .unwrap()
            .infer(request())
            .await
            .is_err()
    );
    use oscar_core::providers::ModelProvider;
    assert_eq!(mock.infer(request()).await.unwrap().content, "not consumed");
}

#[tokio::test]
async fn escaped_payload_is_bounded_before_enqueue() {
    let path = path();
    let limits = JournalLimits {
        max_record_bytes: 1024,
        ..Default::default()
    };
    let log = Journal::create(&path, CaptureMode::Content, limits)
        .await
        .unwrap();
    let payload = log.payload(&"\0".repeat(500)).unwrap();
    assert!(
        log.append(JournalEvent::ToolRequested { payload })
            .await
            .is_err()
    );
    log.close().await.unwrap();
}

struct Gate {
    calls: Arc<AtomicUsize>,
    error: Option<OscarError>,
}
impl Validator for Gate {
    fn validate<'a>(
        &'a self,
        _: &'a str,
    ) -> Pin<Box<dyn Future<Output = Result<(), OscarError>> + Send + 'a>> {
        self.calls.fetch_add(1, Ordering::SeqCst);
        Box::pin(async {
            match &self.error {
                Some(error) => Err(error.clone()),
                None => Ok(()),
            }
        })
    }
}

#[tokio::test]
async fn host_gate_cannot_be_bypassed_by_format_confidence_or_provider() {
    for mode in [WorkMode::Local, WorkMode::Mixed, WorkMode::FullRemote] {
        let mut config = Config::parse(include_str!("../../../settings.toml")).unwrap();
        config.work_mode = mode;
        let plan = plan_request("Write README documentation", &config).unwrap();
        assert!(matches!(
            plan.tasks[0].validation,
            ValidationStrategy::NonEmpty
        ));
        let calls = Arc::new(AtomicUsize::new(0));
        let validators: Validators = [(
            "required".into(),
            Arc::new(Gate {
                calls: calls.clone(),
                error: Some(OscarError::Validation("HOST_SECRET".into())),
            }) as Arc<dyn Validator>,
        )]
        .into();
        let providers = Providers {
            local: Some(Arc::new(MockProvider::demo())),
            remote: Some(Arc::new(MockProvider::demo())),
        };
        let report = execution::execute_with_options(
            plan,
            config,
            providers,
            validators,
            CancellationToken::new(),
            ExecutionOptions {
                journal: None,
                validation_policy: ValidationPolicy::require_all(["required".into()]).unwrap(),
            },
        )
        .await
        .unwrap();
        assert_ne!(report.outcome, Outcome::Completed);
        assert!(report.artifacts.is_empty());
        assert!(calls.load(Ordering::SeqCst) > 0);
        assert!(
            !serde_json::to_string(&report)
                .unwrap()
                .contains("HOST_SECRET")
        );
    }
}

#[tokio::test]
async fn missing_gate_fails_before_model_and_infrastructure_errors_do_not_retry() {
    let config = Config::parse(include_str!("../../../settings.toml")).unwrap();
    let plan = plan_request("Write README documentation", &config).unwrap();
    let options = ExecutionOptions {
        journal: None,
        validation_policy: ValidationPolicy::require_all(["required".into()]).unwrap(),
    };
    assert!(
        execution::execute_with_options(
            plan.clone(),
            config.clone(),
            Providers::default(),
            Validators::new(),
            CancellationToken::new(),
            options.clone()
        )
        .await
        .is_err()
    );
    let calls = Arc::new(AtomicUsize::new(0));
    let validators: Validators = [(
        "required".into(),
        Arc::new(Gate {
            calls: calls.clone(),
            error: Some(OscarError::Cancelled),
        }) as Arc<dyn Validator>,
    )]
    .into();
    let report = execution::execute_with_options(
        plan,
        config,
        providers(vec![output("valid proposal")]),
        validators,
        CancellationToken::new(),
        options,
    )
    .await
    .unwrap();
    assert_eq!(report.outcome, Outcome::Cancelled);
    assert_eq!(calls.load(Ordering::SeqCst), 1);
    assert_eq!(report.calls.len(), 1);
}

#[tokio::test]
async fn mandatory_gate_and_declared_gate_are_deduplicated_and_completion_is_durable() {
    let path = path();
    let log = Journal::create(&path, CaptureMode::Content, JournalLimits::default())
        .await
        .unwrap();
    let config = Config::parse(include_str!("../../../settings.toml")).unwrap();
    let mut plan = plan_request("Write README documentation", &config).unwrap();
    plan.tasks[0].validation = ValidationStrategy::Registered("required".into());
    let calls = Arc::new(AtomicUsize::new(0));
    let validators: Validators = [(
        "required".into(),
        Arc::new(Gate {
            calls: calls.clone(),
            error: None,
        }) as Arc<dyn Validator>,
    )]
    .into();
    let report = execution::execute_with_options(
        plan,
        config,
        providers(vec![output("accepted")]).with_journal(log.clone()),
        validators,
        CancellationToken::new(),
        ExecutionOptions {
            journal: Some(log.clone()),
            validation_policy: ValidationPolicy::require_all(["required".into()]).unwrap(),
        },
    )
    .await
    .unwrap();
    assert_eq!(report.outcome, Outcome::Completed);
    assert_eq!(calls.load(Ordering::SeqCst), 1);
    log.close().await.unwrap();
    assert!(records(&path).iter().any(|r| matches!(
        r.event,
        JournalEvent::RunFinished {
            outcome: Outcome::Completed,
            ..
        }
    )));
    verify(&path, JournalLimits::default()).await.unwrap();
}

#[cfg(feature = "http")]
#[tokio::test]
async fn actual_http_body_is_captured_with_system_policy_without_auth_headers() {
    use std::{
        io::{Read, Write},
        time::Duration,
    };
    let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
    listener.set_nonblocking(true).unwrap();
    let address = listener.local_addr().unwrap();
    let server = std::thread::spawn(move || {
        let start = std::time::Instant::now();
        let mut socket = loop {
            match listener.accept() {
                Ok((socket, _)) => break socket,
                Err(e) if e.kind() == std::io::ErrorKind::WouldBlock => {
                    assert!(start.elapsed() < Duration::from_secs(5));
                    std::thread::sleep(Duration::from_millis(5));
                }
                Err(e) => panic!("{e}"),
            }
        };
        socket.set_nonblocking(false).unwrap();
        socket
            .set_read_timeout(Some(Duration::from_secs(3)))
            .unwrap();
        socket
            .set_write_timeout(Some(Duration::from_secs(3)))
            .unwrap();
        let mut bytes = Vec::new();
        let (head_end, size) = loop {
            let mut buf = [0; 4096];
            let read = socket.read(&mut buf).unwrap();
            assert!(read > 0 && bytes.len() + read < 65_536);
            bytes.extend_from_slice(&buf[..read]);
            if let Some(end) = bytes.windows(4).position(|w| w == b"\r\n\r\n") {
                let headers = String::from_utf8_lossy(&bytes[..end]);
                assert!(
                    headers.to_ascii_lowercase().contains(
                        "authorization: bearer TEST_CREDENTIAL"
                            .to_ascii_lowercase()
                            .as_str()
                    )
                );
                let size = headers
                    .lines()
                    .find_map(|line| {
                        line.to_ascii_lowercase()
                            .strip_prefix("content-length:")
                            .map(|s| s.trim().parse::<usize>().unwrap())
                    })
                    .unwrap();
                break (end + 4, size);
            }
        };
        while bytes.len() < head_end + size {
            let mut buf = [0; 4096];
            let read = socket.read(&mut buf).unwrap();
            assert!(read > 0 && bytes.len() + read < 65_536);
            bytes.extend_from_slice(&buf[..read]);
        }
        let body = String::from_utf8(bytes[head_end..head_end + size].to_vec()).unwrap();
        let data = "data: {\"choices\":[{\"index\":0,\"delta\":{\"content\":\"hello\"},\"finish_reason\":null}]}\n\ndata: {\"choices\":[{\"index\":0,\"delta\":{},\"finish_reason\":\"stop\"}]}\n\ndata: [DONE]\n\n";
        write!(socket, "HTTP/1.1 200 OK\r\nContent-Type: text/event-stream\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{data}", data.len()).unwrap();
        body
    });
    let path = path();
    let journal = Journal::create(&path, CaptureMode::Content, JournalLimits::default())
        .await
        .unwrap();
    let mut config = Config::parse(include_str!("../../../settings.toml")).unwrap();
    config.work_mode = WorkMode::Local;
    let settings = config.providers.get_mut("local").unwrap();
    settings.provider = Some("openai_compatible".into());
    settings.api_url = Some(format!("http://{address}/v1/chat/completions"));
    settings.api_key = Some("TEST_CREDENTIAL".into());
    let providers = Providers::from_config(&config)
        .unwrap()
        .with_journal(journal.clone());
    assert_eq!(
        providers
            .get(ProviderPreference::Local)
            .unwrap()
            .infer(request())
            .await
            .unwrap()
            .content,
        "hello"
    );
    journal.close().await.unwrap();
    let actual = server.join().unwrap();
    let rows = records(&path);
    let wire = rows
        .iter()
        .find_map(|r| match &r.event {
            JournalEvent::WireRequest { payload, .. } => payload.content.as_ref(),
            _ => None,
        })
        .unwrap();
    assert_eq!(wire, &actual);
    assert!(wire.contains("untrusted evidence") && wire.contains("PRIVATE_PROMPT"));
    assert!(
        !std::fs::read_to_string(&path)
            .unwrap()
            .contains("TEST_CREDENTIAL")
    );
    verify(&path, JournalLimits::default()).await.unwrap();
}
