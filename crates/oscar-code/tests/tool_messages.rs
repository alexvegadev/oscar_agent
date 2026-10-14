use oscar_code::{
    messages::{Message, MessageError, StructuredData, ToolCall, ToolOutcome, ToolResult},
    session::{RunState, SessionState},
    settings::Settings,
};
use oscar_core::{
    config::Limits,
    planning::Confidence,
    providers::{ModelFuture, ModelOutput, ModelProvider, ModelRequest, TokenUsage},
    tools::ToolError,
};
use serde_json::json;
use std::sync::Arc;

fn data(json: &str) -> StructuredData {
    StructuredData::from_json(json, 16_384).unwrap()
}
fn call(id: &str) -> ToolCall {
    ToolCall::new(id, "read_file", data(r#"{"path":"src/lib.rs"}"#)).unwrap()
}
fn result(id: &str, outcome: ToolOutcome) -> ToolResult {
    ToolResult::new(id, "read_file", outcome).unwrap()
}
fn output(text: &str) -> ModelOutput {
    ModelOutput {
        tool_calls: Vec::new(),
        content: text.into(),
        confidence: Confidence::High,
        usage: TokenUsage::default(),
    }
}

#[test]
fn nested_objects_arrays_and_scalars_preserve_types_and_redact_debug() {
    let input = r#"{"files":[{"path":"hé🦀","lines":[1,null,true,2.5]}],"empty":{},"array":[]}"#;
    let structured = data(input);
    assert_eq!(
        structured.value(),
        &serde_json::from_str::<serde_json::Value>(input).unwrap()
    );
    assert_eq!(structured.byte_len(), structured.value().to_string().len());
    let call = ToolCall::new("call-1", "inspect", structured.clone()).unwrap();
    let result = ToolResult::new("call-1", "inspect", ToolOutcome::Success(structured)).unwrap();
    assert_eq!(
        call.arguments().value()["files"][0]["lines"],
        json!([1, null, true, 2.5])
    );
    assert_eq!(result.call_id(), call.id());
    assert_eq!(result.name(), call.name());
    assert!(!format!("{call:?} {result:?}").contains("hé"));
    for scalar in ["null", "true", "42", "1.5", r#""literal""#, "[]", "{}"] {
        assert!(StructuredData::from_json(scalar, 64).is_ok());
    }
}

#[test]
fn malformed_identifiers_non_object_arguments_and_structure_limits_fail_closed() {
    for json in ["", "{", "NaN", "{} trailing"] {
        assert_eq!(
            StructuredData::from_json(json, 100),
            Err(MessageError::InvalidJson)
        );
    }
    assert_eq!(
        StructuredData::from_json(r#""hé🦀""#, 7),
        Err(MessageError::ByteLimit)
    );
    assert!(StructuredData::from_json(r#""hé🦀""#, 9).is_ok());
    for id in ["", "bad id", "../file", "é", &"x".repeat(65)] {
        assert_eq!(
            ToolCall::new(id, "read_file", data("{}")),
            Err(MessageError::InvalidIdentifier)
        );
        assert_eq!(
            ToolResult::new("valid", id, ToolOutcome::Failure(ToolError::Failed)),
            Err(MessageError::InvalidIdentifier)
        );
    }
    assert_eq!(
        ToolCall::new("valid", "read_file", data("[]")),
        Err(MessageError::ExpectedObject)
    );
    let nested = |depth| format!("{}null{}", "[".repeat(depth), "]".repeat(depth));
    assert!(StructuredData::from_json(&nested(16), 100).is_ok());
    assert_eq!(
        StructuredData::from_json(&nested(17), 100),
        Err(MessageError::DepthLimit)
    );
    let wide = |count| format!("[{}]", vec!["null"; count].join(","));
    assert!(StructuredData::from_json(&wide(511), 5000).is_ok());
    assert_eq!(
        StructuredData::from_json(&wide(512), 5000),
        Err(MessageError::NodeLimit)
    );
}

#[test]
fn results_match_once_in_the_active_run_and_failed_admissions_do_not_mutate_history() {
    let mut session = SessionState::new(Limits::default(), Settings::default());
    let run = session.begin("Inspect").unwrap();
    assert_eq!(
        session.record_tool_result(run.id, result("1", ToolOutcome::Success(data("{}")))),
        Err(MessageError::UnknownCall)
    );
    session.record_tool_call(run.id, call("1")).unwrap();
    assert_eq!(
        session.record_tool_call(run.id, call("1")),
        Err(MessageError::DuplicateCall)
    );
    assert_eq!(
        session.record_tool_result(
            run.id,
            ToolResult::new("1", "other", ToolOutcome::Success(data("{}"))).unwrap()
        ),
        Err(MessageError::MismatchedTool)
    );
    session
        .record_tool_result(run.id, result("1", ToolOutcome::Failure(ToolError::Denied)))
        .unwrap();
    assert_eq!(
        session.record_tool_result(run.id, result("1", ToolOutcome::Success(data("null")))),
        Err(MessageError::DuplicateResult)
    );
    assert_eq!(session.messages().len(), 3);
    assert!(
        matches!(session.messages()[2], Message::ToolResult(ref result) if result.outcome() == &ToolOutcome::Failure(ToolError::Denied))
    );
    session
        .record_tool_call(run.id, call("unfinished"))
        .unwrap();
    session.complete(run.id, Ok(output("proposal")));
    assert_eq!(
        session.record_tool_call(run.id, call("late")),
        Err(MessageError::InactiveRun)
    );
    let next = session.begin("Follow up").unwrap();
    assert_eq!(
        session.record_tool_call(run.id, call("stale")),
        Err(MessageError::InactiveRun)
    );
    assert_eq!(
        session.record_tool_result(
            next.id,
            result("unfinished", ToolOutcome::Success(data("{}")))
        ),
        Err(MessageError::UnknownCall)
    );
    assert_eq!(
        session.record_tool_call(next.id, call("1")),
        Err(MessageError::DuplicateCall)
    );
    session.stop();
    assert_eq!(
        session.record_tool_call(next.id, call("stopped")),
        Err(MessageError::InactiveRun)
    );
    session.reset();
    assert!(session.messages().is_empty());
    assert_eq!(session.turns, 2);
    assert_eq!(
        session.record_tool_result(
            next.id,
            result("unfinished", ToolOutcome::Success(data("{}")))
        ),
        Err(MessageError::InactiveRun)
    );
    let current = session.begin("Restart").unwrap();
    session.record_tool_call(current.id, call("1")).unwrap();
}

#[test]
fn tool_payloads_and_identifiers_count_toward_byte_and_total_history_limits() {
    let mut session = SessionState::new(
        Limits {
            max_input_bytes: 256,
            max_output_bytes: 32,
            ..Limits::default()
        },
        Settings::default(),
    );
    let run = session.begin("a").unwrap();
    let too_large = ToolCall::new(
        "1",
        "read_file",
        data(&format!(r#"{{"text":"{}"}}"#, "x".repeat(256))),
    )
    .unwrap();
    assert_eq!(
        session.record_tool_call(run.id, too_large),
        Err(MessageError::ByteLimit)
    );
    session.record_tool_call(run.id, call("1")).unwrap();
    assert_eq!(
        session.record_tool_result(
            run.id,
            result(
                "1",
                ToolOutcome::Success(data(&format!(r#""{}""#, "x".repeat(32))))
            )
        ),
        Err(MessageError::ByteLimit)
    );
    assert_eq!(session.messages().len(), 2);
    session
        .record_tool_result(run.id, result("1", ToolOutcome::Success(data("null"))))
        .unwrap();
    let bulky = |id| {
        ToolCall::new(
            id,
            "read_file",
            data(&format!(r#"{{"text":"{}"}}"#, "x".repeat(190))),
        )
        .unwrap()
    };
    session.record_tool_call(run.id, bulky("2")).unwrap();
    let before = session.messages().len();
    assert_eq!(
        session.record_tool_call(run.id, bulky("3")),
        Err(MessageError::HistoryLimit)
    );
    assert_eq!(session.messages().len(), before);
    // Model completion must account for structured history too.
    session.complete(run.id, Ok(output(&"x".repeat(32))));
    assert_eq!(session.state, RunState::LimitReached);
}

struct ContextCheck;
impl ModelProvider for ContextCheck {
    fn infer(&self, request: ModelRequest) -> ModelFuture<'_> {
        Box::pin(async move {
            assert!(request.context.contains("USER:\nInspect"));
            assert!(request.context.contains("ASSISTANT:\nproposal"));
            assert!(!request.context.contains("tool-only-marker"));
            assert!(!request.context.contains("read_file"));
            Ok(output("follow-up"))
        })
    }
}

#[tokio::test]
async fn tool_messages_are_display_only_and_model_text_is_never_parsed_as_a_call() {
    let mut session = SessionState::new(Limits::default(), Settings::default());
    let run = session.begin("Inspect").unwrap();
    session.record_tool_call(run.id, call("1")).unwrap();
    session
        .record_tool_result(
            run.id,
            result(
                "1",
                ToolOutcome::Success(data(r#"{"text":"tool-only-marker"}"#)),
            ),
        )
        .unwrap();
    session.complete(run.id, Ok(output("proposal")));
    let next = session.begin("Follow up").unwrap();
    let id = next.id;
    session.complete(id, next.execute(Arc::new(ContextCheck)).await);
    let run = session.begin("Show JSON").unwrap();
    let json = r#"{"id":"2","name":"read_file","arguments":{}}"#;
    session.complete(run.id, Ok(output(json)));
    assert_eq!(
        session.messages().last(),
        Some(&Message::Model(json.into()))
    );
}
