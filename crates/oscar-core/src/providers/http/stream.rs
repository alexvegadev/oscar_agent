use super::*;
use serde::Deserialize;

pub(super) fn invalid(message: &str) -> OscarError {
    OscarError::Provider {
        transient: false,
        message: message.into(),
    }
}

#[derive(Deserialize)]
struct Chunk {
    choices: Vec<Choice>,
    usage: Option<Usage>,
}
#[derive(Deserialize)]
struct Choice {
    index: usize,
    delta: Delta,
    finish_reason: Option<String>,
}
#[derive(Deserialize)]
struct Delta {
    content: Option<String>,
    tool_calls: Option<Vec<CallDelta>>,
    function_call: Option<serde_json::Value>,
    refusal: Option<String>,
}
#[derive(Deserialize)]
struct CallDelta {
    index: usize,
    id: Option<String>,
    #[serde(rename = "type")]
    kind: Option<String>,
    function: Option<FunctionDelta>,
}
#[derive(Deserialize)]
struct FunctionDelta {
    name: Option<String>,
    arguments: Option<String>,
}
#[derive(Default)]
struct PendingCall {
    id: String,
    name: String,
    arguments: String,
    function: bool,
}
#[derive(Deserialize)]
struct Usage {
    prompt_tokens: Option<u64>,
    completion_tokens: Option<u64>,
}

/// Incremental SSE framing. Lines, events, decoded text and total wire bytes are
/// bounded independently. Never decode UTF-8 at an arbitrary network boundary.
pub(super) struct Decoder {
    line: Vec<u8>,
    data: Vec<u8>,
    after_cr: bool,
    error_event: bool,
    wire_bytes: usize,
    wire_limit: usize,
    max_output_bytes: usize,
    max_output_tokens: u32,
    content: String,
    usage: TokenUsage,
    finished: bool,
    saw_text: bool,
    tools_enabled: bool,
    calls: Vec<PendingCall>,
}
const FRAME_LIMIT: usize = 65_536;
impl Decoder {
    pub(super) fn new(max_output_bytes: usize, max_output_tokens: u32) -> Self {
        Self {
            line: Vec::new(),
            data: Vec::new(),
            after_cr: false,
            error_event: false,
            wire_bytes: 0,
            // Token chunks have substantially more JSON overhead than one reply.
            wire_limit: max_output_bytes
                .saturating_mul(1024)
                .saturating_add(FRAME_LIMIT),
            max_output_bytes,
            max_output_tokens,
            content: String::new(),
            usage: TokenUsage::default(),
            finished: false,
            saw_text: false,
            tools_enabled: false,
            calls: Vec::new(),
        }
    }
    pub(super) fn enable_tools(&mut self, enabled: bool) {
        self.tools_enabled = enabled;
    }
    fn output_bytes(&self) -> usize {
        self.calls.iter().fold(self.content.len(), |n, c| {
            n.saturating_add(c.id.len())
                .saturating_add(c.name.len())
                .saturating_add(c.arguments.len())
        })
    }
    fn calls_complete(&self) -> Result<(), OscarError> {
        let identifier = |s: &str| {
            !s.is_empty()
                && s.len() <= 64
                && s.bytes()
                    .all(|b| b.is_ascii_alphanumeric() || matches!(b, b'_' | b'-' | b'.'))
        };
        let mut ids = std::collections::BTreeSet::new();
        for call in &self.calls {
            if !call.function
                || !identifier(&call.id)
                || !identifier(&call.name)
                || !ids.insert(&call.id)
            {
                return Err(invalid("invalid or duplicate streamed tool call"));
            }
            if !serde_json::from_str::<serde_json::Value>(&call.arguments)
                .is_ok_and(|v| v.is_object())
            {
                return Err(invalid("tool arguments must be a JSON object"));
            }
        }
        Ok(())
    }
    pub(super) fn feed(
        &mut self,
        bytes: &[u8],
        sink: &mut ModelTextSink<'_>,
    ) -> Result<bool, OscarError> {
        self.wire_bytes = self.wire_bytes.saturating_add(bytes.len());
        if self.wire_bytes > self.wire_limit {
            return Err(OscarError::Limit("HTTP stream exceeds wire limit".into()));
        }
        for &byte in bytes {
            if self.after_cr {
                self.after_cr = false;
                if byte == b'\n' {
                    continue;
                }
            }
            if byte == b'\r' || byte == b'\n' {
                self.after_cr = byte == b'\r';
                if self.line(sink)? {
                    return Ok(true);
                }
            } else {
                if self.line.len() >= FRAME_LIMIT {
                    return Err(OscarError::Limit("SSE line exceeds byte limit".into()));
                }
                self.line.push(byte);
            }
        }
        Ok(false)
    }
    fn line(&mut self, sink: &mut ModelTextSink<'_>) -> Result<bool, OscarError> {
        let bytes = std::mem::take(&mut self.line);
        if bytes.is_empty() {
            if self.error_event {
                return Err(invalid("provider stream error (details redacted)"));
            }
            if self.data.is_empty() {
                return Ok(false);
            }
            let data = std::mem::take(&mut self.data);
            self.error_event = false;
            return self.event(&data, sink);
        }
        let line =
            std::str::from_utf8(&bytes).map_err(|_| invalid("invalid UTF-8 in SSE stream"))?;
        if line.starts_with(':') {
            return Ok(false);
        }
        let (field, value) = line.split_once(':').unwrap_or((line, ""));
        let value = value.strip_prefix(' ').unwrap_or(value);
        match field {
            "data" => {
                if self
                    .data
                    .len()
                    .saturating_add(value.len())
                    .saturating_add(1)
                    > FRAME_LIMIT
                {
                    return Err(OscarError::Limit("SSE event exceeds byte limit".into()));
                }
                if !self.data.is_empty() {
                    self.data.push(b'\n');
                }
                self.data.extend_from_slice(value.as_bytes());
            }
            "event" => self.error_event = value == "error",
            _ => {}
        }
        Ok(false)
    }
    fn event(&mut self, data: &[u8], sink: &mut ModelTextSink<'_>) -> Result<bool, OscarError> {
        if data == b"[DONE]" {
            if !self.finished || (!self.saw_text && self.calls.is_empty()) {
                return Err(invalid("incomplete inference stream"));
            }
            return Ok(true);
        }
        let chunk: Chunk = serde_json::from_slice(data)
            .map_err(|_| invalid("malformed inference stream event"))?;
        if chunk.choices.len() > 1 {
            return Err(invalid("multiple inference choices are unsupported"));
        }
        if chunk.choices.is_empty() && (!self.finished || chunk.usage.is_none()) {
            return Err(invalid("unexpected empty inference choices"));
        }
        if let Some(usage) = chunk.usage {
            if usage
                .completion_tokens
                .is_some_and(|n| n > self.max_output_tokens as u64)
            {
                return Err(OscarError::Limit("model usage exceeds token limit".into()));
            }
            self.usage = TokenUsage {
                input: usage.prompt_tokens,
                output: usage.completion_tokens,
            };
        }
        if let Some(choice) = chunk.choices.into_iter().next() {
            if self.finished || choice.index != 0 {
                return Err(invalid("unexpected inference choice"));
            }
            if (choice.delta.tool_calls.is_some() && !self.tools_enabled)
                || choice.delta.function_call.is_some()
            {
                return Err(invalid("tool calls are not supported by proposal workers"));
            }
            if let Some(calls) = choice.delta.tool_calls {
                if calls.is_empty() || calls.len() > 32 {
                    return Err(invalid("invalid streamed tool call count"));
                }
                let mut indices = std::collections::BTreeSet::new();
                for delta in calls {
                    if delta.index >= 32
                        || delta.index > self.calls.len()
                        || !indices.insert(delta.index)
                    {
                        return Err(invalid("invalid streamed tool call index"));
                    }
                    if delta.index == self.calls.len() {
                        self.calls.push(PendingCall::default());
                    }
                    let call = &mut self.calls[delta.index];
                    if let Some(kind) = delta.kind {
                        if kind != "function" {
                            return Err(invalid("unsupported tool call type"));
                        }
                        call.function = true;
                    }
                    if let Some(id) = delta.id {
                        call.id.push_str(&id);
                    }
                    if let Some(function) = delta.function {
                        if let Some(name) = function.name {
                            call.name.push_str(&name);
                        }
                        if let Some(arguments) = function.arguments {
                            call.arguments.push_str(&arguments);
                        }
                    }
                    if call.id.len() > 64
                        || call.name.len() > 64
                        || self.output_bytes() > self.max_output_bytes
                    {
                        return Err(OscarError::Limit(
                            "tool calls exceed output byte limit".into(),
                        ));
                    }
                }
            }
            if choice.delta.refusal.as_ref().is_some_and(|s| !s.is_empty()) {
                return Err(invalid("provider refused inference"));
            }
            // Validate the entire event before delivering any of its text.
            match choice.finish_reason.as_deref() {
                Some("stop") if self.calls.is_empty() => self.finished = true,
                Some("tool_calls") if !self.calls.is_empty() => {
                    self.calls_complete()?;
                    self.finished = true;
                }
                Some("length") => {
                    return Err(OscarError::Limit("model token limit reached".into()));
                }
                Some(_) => return Err(invalid("unsupported inference finish reason")),
                None => {}
            }
            if let Some(text) = choice.delta.content {
                if self.output_bytes().saturating_add(text.len()) > self.max_output_bytes {
                    return Err(OscarError::Limit("model output exceeds byte limit".into()));
                }
                self.saw_text = true;
                if !text.is_empty() {
                    sink(&text)?;
                    self.content.push_str(&text);
                }
            }
        }
        Ok(false)
    }
    pub(super) fn output(self) -> ModelOutput {
        ModelOutput {
            tool_calls: self
                .calls
                .into_iter()
                .map(|c| ModelToolCall {
                    id: c.id,
                    name: c.name,
                    arguments: c.arguments,
                })
                .collect(),
            content: self.content,
            confidence: Confidence::Medium,
            usage: self.usage,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    fn event(delta: serde_json::Value, finish: Option<&str>) -> String {
        format!(
            "data: {}\n\n",
            serde_json::json!({"choices":[{"index":0,"delta":delta,"finish_reason":finish}]})
        )
    }
    #[test]
    fn fragmented_native_calls_preserve_raw_arguments_and_complete_without_text() {
        let wire = format!(
            "{}{}{}data: [DONE]\n\n",
            event(
                serde_json::json!({"tool_calls":[{"index":0,"id":"c1","type":"function","function":{"name":"read","arguments":"{\"path\":"}},{"index":1,"id":"c2","type":"function","function":{"name":"stats","arguments":"{}"}}]}),
                None
            ),
            event(
                serde_json::json!({"tool_calls":[{"index":0,"function":{"arguments":"\"a\",\"path\":\"b\"}"}}]}),
                None
            ),
            event(serde_json::json!({}), Some("tool_calls"))
        );
        for split in 1..=wire.len() {
            let mut decoder = Decoder::new(200, 10);
            decoder.enable_tools(true);
            let mut done = false;
            for bytes in wire.as_bytes().chunks(split) {
                if decoder
                    .feed(bytes, &mut |_| panic!("no text expected"))
                    .unwrap()
                {
                    done = true;
                    break;
                }
            }
            assert!(done);
            let output = decoder.output();
            assert_eq!(output.tool_calls.len(), 2);
            assert_eq!(
                output.tool_calls[0].arguments,
                "{\"path\":\"a\",\"path\":\"b\"}"
            );
            assert_eq!(output.tool_calls[1].id, "c2");
        }
    }
    #[test]
    fn native_calls_reject_bad_indices_ids_json_finish_and_limits() {
        for (calls, finish) in [
            (
                serde_json::json!([{"index":1,"id":"c","type":"function","function":{"name":"read","arguments":"{}"}}]),
                "tool_calls",
            ),
            (
                serde_json::json!([{"index":0,"id":"c","type":"shell","function":{"name":"read","arguments":"{}"}}]),
                "tool_calls",
            ),
            (
                serde_json::json!([{"index":0,"id":"c","type":"function","function":{"name":"read","arguments":"[]"}}]),
                "tool_calls",
            ),
            (
                serde_json::json!([{"index":0,"id":"c","type":"function","function":{"name":"read","arguments":"{"}}]),
                "tool_calls",
            ),
            (
                serde_json::json!([{"index":0,"id":"c","type":"function","function":{"name":"read","arguments":"{}"}}]),
                "stop",
            ),
            (
                serde_json::json!([{"index":0,"id":"c","type":"function","function":{"name":"read","arguments":"{}"}},{"index":1,"id":"c","type":"function","function":{"name":"read","arguments":"{}"}}]),
                "tool_calls",
            ),
        ] {
            let mut decoder = Decoder::new(200, 10);
            decoder.enable_tools(true);
            assert!(
                decoder
                    .feed(
                        event(serde_json::json!({"tool_calls":calls}), Some(finish)).as_bytes(),
                        &mut |_| Ok(())
                    )
                    .is_err()
            );
        }
        let mut decoder = Decoder::new(5, 10);
        decoder.enable_tools(true);
        assert!(matches!(decoder.feed(event(serde_json::json!({"tool_calls":[{"index":0,"id":"c","type":"function","function":{"name":"read","arguments":"{}"}}]}), None).as_bytes(), &mut |_| Ok(())), Err(OscarError::Limit(_))));
        let mut decoder = Decoder::new(200, 10);
        decoder.enable_tools(true);
        decoder.feed(event(serde_json::json!({"tool_calls":[{"index":0,"id":"c","type":"function","function":{"name":"read","arguments":"{}"}}]}), None).as_bytes(), &mut |_| Ok(())).unwrap();
        assert!(decoder.feed(b"data: [DONE]\n\n", &mut |_| Ok(())).is_err());
    }
    fn text(content: &str, finish: Option<&str>) -> String {
        format!(
            "data: {}\n\n",
            serde_json::json!({"choices":[{"index":0,"delta":{"content":content},"finish_reason":finish}]})
        )
    }
    #[test]
    fn arbitrary_byte_boundaries_unicode_crlf_comments_multiline_and_usage() {
        let wire = format!(
            ": heartbeat\r\n\r\n{}{}data: {{\r\ndata: \"choices\":[],\"usage\":{{\"prompt_tokens\":12,\"completion_tokens\":2}}}}\r\n\r\ndata: [DONE]\r\n\r\n",
            text("hé🦀", None).replace('\n', "\r\n"),
            text("!", Some("stop")).replace('\n', "\r\n")
        );
        for split in 1..=wire.len() {
            let mut decoder = Decoder::new(100, 10);
            let mut deltas = Vec::new();
            let mut done = false;
            for bytes in wire.as_bytes().chunks(split) {
                if decoder
                    .feed(bytes, &mut |s| {
                        deltas.push(s.to_owned());
                        Ok(())
                    })
                    .unwrap()
                {
                    done = true;
                    break;
                }
            }
            assert!(done, "split {split}");
            assert_eq!(deltas, ["hé🦀", "!"]);
            let output = decoder.output();
            assert_eq!(output.content, "hé🦀!");
            assert_eq!(output.usage.input, Some(12));
            assert_eq!(output.usage.output, Some(2));
        }
    }
    #[test]
    fn failures_never_deliver_invalid_event_text_or_complete_truncation() {
        for payload in [
            r#"{"choices":[{"index":0,"delta":{"content":"secret","tool_calls":[]}}]}"#,
            r#"{"choices":[{"index":0,"delta":{"content":"secret","function_call":{}}}]}"#,
            r#"{"choices":[{"index":1,"delta":{"content":"secret"}}]}"#,
            r#"{"choices":[{"index":0,"delta":{"content":"secret"},"finish_reason":"tool_calls"}]}"#,
            r#"{"choices":[{"index":0,"delta":{"refusal":"secret"}}]}"#,
            r#"{"error":{"message":"TOP_SECRET"}}"#,
            "not-json",
            "[DONE]",
        ] {
            let mut decoder = Decoder::new(100, 10);
            let mut delivered = false;
            let error = decoder
                .feed(format!("data: {payload}\n\n").as_bytes(), &mut |_| {
                    delivered = true;
                    Ok(())
                })
                .unwrap_err();
            assert!(!delivered);
            assert!(!error.to_string().contains("TOP_SECRET"));
        }
        let mut decoder = Decoder::new(100, 10);
        assert!(
            !decoder
                .feed(text("partial", None).as_bytes(), &mut |_| Ok(()))
                .unwrap()
        );
        assert!(decoder.feed(b"data: [DONE]\n\n", &mut |_| Ok(())).is_err());
        let mut decoder = Decoder::new(100, 10);
        assert!(
            decoder
                .feed(b"event: error\ndata: TOP_SECRET\n\n", &mut |_| Ok(()))
                .is_err()
        );
    }
    #[test]
    fn cumulative_output_frames_wire_tokens_and_sink_errors_are_bounded() {
        let mut decoder = Decoder::new(5, 10);
        decoder
            .feed(text("hé", None).as_bytes(), &mut |_| Ok(()))
            .unwrap();
        let mut delivered = false;
        assert!(matches!(
            decoder.feed(text("🦀", None).as_bytes(), &mut |_| {
                delivered = true;
                Ok(())
            }),
            Err(OscarError::Limit(_))
        ));
        assert!(!delivered);
        for wire in [
            text("x", Some("length")),
            format!("data: {}\n\n", "x".repeat(FRAME_LIMIT + 1)),
            format!(
                "data: {}\n\n",
                serde_json::json!({"choices":[{"index":0,"delta":{"content":"x"},"finish_reason":"stop"}],"usage":{"completion_tokens":11}})
            ),
        ] {
            assert!(matches!(
                Decoder::new(100, 10).feed(wire.as_bytes(), &mut |_| Ok(())),
                Err(OscarError::Limit(_))
            ));
        }
        let mut decoder = Decoder::new(0, 10);
        assert!(matches!(
            decoder.feed(&vec![b':'; FRAME_LIMIT + 1], &mut |_| Ok(())),
            Err(OscarError::Limit(_))
        ));
        assert!(matches!(
            Decoder::new(100, 10).feed(text("x", None).as_bytes(), &mut |_| Err(
                OscarError::Cancelled
            )),
            Err(OscarError::Cancelled)
        ));
        let mut decoder = Decoder::new(100, 10);
        decoder
            .feed(text("done", Some("stop")).as_bytes(), &mut |_| Ok(()))
            .unwrap();
        assert!(
            decoder
                .feed(text("late", None).as_bytes(), &mut |_| Ok(()))
                .is_err()
        );
    }
}
