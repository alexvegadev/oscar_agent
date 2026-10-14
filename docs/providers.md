# Rig providers and text streaming

The experimental `ModelProvider` boundary supports both `infer` (complete output)
and `infer_stream` (incremental text plus final output). The Rig completion adapter uses
native OpenAI-compatible chat-completion SSE for both paths. Planning and artifact
execution collect the stream before applying their existing validation; interactive
CLI and desktop consumers render provisional text during inference.

```rust
use oscar_core::providers::{ModelProvider, ModelRequest};
async fn example(provider: &dyn ModelProvider) -> Result<(), oscar_core::error::OscarError> {
let request = ModelRequest {
    context: "Propose a parser".into(),
    messages: Vec::new(),
    tools: Vec::new(),
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
and timeouts remain in force. Rig handles typed requests and the completion
client; OSCAR supplies a bounded transport using reqwest. Rig and RAG share the
pinned `rig-core = 0.32.0` dependency. Configured endpoints must support `stream: true`,
`stream_options: {"include_usage": true}` and `text/event-stream` responses.
There is no silent non-streaming fallback or retry. The implementation follows the
[OpenAI chat-completion streaming format](https://developers.openai.com/api/reference/resources/chat/subresources/completions/streaming-events).

The decoder accepts arbitrary transport boundaries (including within UTF-8), LF,
CRLF or CR line endings, SSE comments and multiline data events. It accepts only
one choice with index 0, text deltas, a `stop` finish reason and a final `[DONE]`.
Optional final usage-only events update token counts; omitted usage stays unknown.
Malformed frames, invalid UTF-8, unexpected choices, refusals, legacy function calls,
unsupported finish reasons and EOF before completion fail closed. Native tool calls
are accepted only when host registry schemas were supplied, with correlated IDs
and raw argument JSON preserved for registry validation. Empty `tool_calls` arrays
are metadata meaning no calls; they do not interrupt ordinary text streaming. A tool response requires
a `tool_calls` finish reason. Proposal-only requests reject all tool calls. A `length`
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

## Provider selection

Provider configuration stays in the `local` and `remote` routing slots. The `http`
feature enables the Rig completion adapter; default builds remain offline and can
use `provider = "mock"`. All real adapters in this release speak **OpenAI Chat
Completions**, including its native streaming tool protocol. Endpoint presets are
configuration conveniences, not assertions that every model supports every tool.

| `provider` | Default complete `api_url` |
| --- | --- |
| `openai` | `https://api.openai.com/v1/chat/completions` |
| `openrouter` | `https://openrouter.ai/api/v1/chat/completions` |
| `groq` | `https://api.groq.com/openai/v1/chat/completions` |
| `deepseek` | `https://api.deepseek.com/v1/chat/completions` |
| `xai` | `https://api.x.ai/v1/chat/completions` |
| `ollama` | `http://127.0.0.1:11434/v1/chat/completions` |
| `lmstudio` | `http://127.0.0.1:1234/v1/chat/completions` |
| `openai_compatible` | Explicit `api_url` required |

An explicit `api_url` overrides a preset. It is still the **complete** endpoint,
ending in `/chat/completions`; existing compatible-endpoint configurations keep
working. Configure credentials with `api_key_env`; only that named variable is
read, and only for an adapter initialized under the selected work mode. There is
no automatic fallback to a vendor's environment variables.

New vendors need no code changes when they implement this protocol:

```toml
work_mode = "full_remote"

[providers.remote]
enabled = true
provider = "my-provider"
protocol = "openai_chat"
model = "my-model"
api_url = "https://models.example.com/v1/chat/completions"
api_key_env = "OSCAR_MODEL_KEY"
context_window = 32768
capabilities = ["planning", "code_analysis", "documentation", "tool_use"]
```

`protocol = "openai_chat"` explicitly selects the supported wire adapter for any
custom provider name. Unknown protocols and unknown names without an explicit
protocol fail closed. A different native protocol (for example Anthropic Messages
or Gemini GenerateContent) still requires a validated adapter; installing Rig
does not automatically enable its entire provider catalogue. A compatible gateway
can expose those models through the supported protocol. Native adapters remain a
future extension behind OSCAR's `ModelProvider` contract.

Change TOML and restart the CLI or reload the configuration through the host.
Models, capabilities, context windows and prices are operator configuration, not
inferred from provider names. Local presets obey the same literal-loopback policy;
a cloud preset cannot bypass local mode. With `--features http,rag`, completion
selection and Rig vector retrieval can be used together. The embedding endpoint
and RAG permissions remain independent; see [session RAG](rag.md).

OSCAR validates raw SSE before Rig consumes it, forwards only bounded canonical
text events, and retains the authoritative tool calls and optional usage separately.
This preserves duplicate argument keys for registry rejection. SDK request bodies
are included in input-budget accounting. Each transport permits one network
attempt; failures return OSCAR's typed error without SDK reconnects. The host's
existing bounded retry policy is the only retry owner. Rig tracing is disabled
while polling inference so SDK spans cannot capture prompts or responses.

The minimum compiler is Rust 1.88. The pin is a reviewed migration target, not a
claim to support the latest Rig API. Real vendor conformance requires separate
live testing; deterministic tests use loopback servers and offline mocks.
