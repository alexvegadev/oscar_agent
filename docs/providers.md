# Provider text streaming

The experimental `ModelProvider` boundary supports both `infer` (complete output)
and `infer_stream` (incremental text plus final output). The HTTP adapter uses
native OpenAI-compatible chat-completion SSE for both paths. Planning and artifact
execution collect the stream before applying their existing validation; interactive
CLI and desktop consumers render provisional text during inference.

```rust
use oscar_core::providers::{ModelProvider, ModelRequest};
async fn example(provider: &dyn ModelProvider) -> Result<(), oscar_core::error::OscarError> {
let request = ModelRequest {
    context: "Propose a parser".into(),
    max_output_bytes: 16_384,
    max_output_tokens: 2048,
};
let mut preview = String::new();
let output = provider.infer_stream(request, &mut |delta| {
    preview.push_str(delta);
    Ok(())
}).await?;
// Only now is output.content an accepted reply; usage may be unknown.
assert_eq!(preview, output.content);
Ok(())
}
```

`ModelTextSink` receives borrowed UTF-8 text deltas, which may span multiple tokens.
Its callbacks run inline and must be nonblocking and bounded; returning an error
aborts the request. There is no internal event queue. A provider must enforce the
request's cumulative byte limit before delivery. CLI and desktop also enforce it
independently. The returned `ModelOutput` supplies final content, confidence and
usage. Earlier deltas are provisional and cannot authorize tools or enter retained
conversation context until inference succeeds. Existing third-party providers
that implement only `infer` remain source compatible: the default streaming method
checks the byte limit and delivers their whole reply once. The mock provider uses
this path; no token timing is simulated.

## HTTP protocol and bounds

The existing `http` feature, URL restrictions, credential handling, redirect policy
and timeouts remain in force. No new dependencies or configuration fields are
required. Configured endpoints must support `stream: true`,
`stream_options: {"include_usage": true}` and `text/event-stream` responses.
There is no silent non-streaming fallback or retry. The implementation follows the
[OpenAI chat-completion streaming format](https://developers.openai.com/api/reference/resources/chat/subresources/completions/streaming-events).

The decoder accepts arbitrary transport boundaries (including within UTF-8), LF,
CRLF or CR line endings, SSE comments and multiline data events. It accepts only
one choice with index 0, text deltas, a `stop` finish reason and a final `[DONE]`.
Optional final usage-only events update token counts; omitted usage stays unknown.
Malformed frames, invalid UTF-8, unexpected choices, refusals, tool/function calls,
unsupported finish reasons and EOF before completion fail closed. A `length`
finish or excessive reported output-token usage produces a limit error. Provider
error payloads are never included in diagnostics.

| Bound | Default or rule |
| --- | --- |
| Decoded output | `max_output_bytes`, default 16,384 bytes |
| Requested output tokens | `max_output_tokens`, default 2,048; excessive reported usage fails |
| SSE line and event | 65,536 bytes each |
| Total response wire bytes | `1024 * max_output_bytes + 65,536` (16,842,752 bytes at default) |
| HTTP request lifetime | `call_timeout_ms`, default 60,000 ms, including streamed body |
| CLI/desktop waiting | Existing call/run deadlines and cancellation |
| UI/terminal update backlog | One latest bounded full-text snapshot, coalesced by a watch channel |

The wire allowance covers per-token JSON overhead while bounding comments and
metadata. Missing token usage cannot independently establish the actual server
token count; the request ceiling, byte/wire limits and deadlines still apply.
Dropping the inference future releases client work and the response. It cannot
guarantee remote computation or billing stops. Terminal writes use Tokio's bounded
blocking pool and cannot be forcibly cancelled at the OS boundary; existing session
shutdown bounds still apply. Desktop previews are discarded on Stop, Reset, failure
or limit; late deltas and completions are rejected by run ID and active state.

Deterministic tests use loopback HTTP and scripted providers. They cover early text
before server completion, cancellation/drop, arbitrary splits and Unicode, metadata,
malformed events, limits, errors, stale previews and final history commit. Live cloud
conformance and native visual QA remain unverified.
