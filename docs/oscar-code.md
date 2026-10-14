# OSCAR Code desktop chat and autonomous tools

`oscar-code` is an experimental GPUI desktop conversation UI. It uses OSCAR's
existing inference-only provider boundary. It does not execute repository tools,
apply edits, or run commands. The supplied configuration works without credentials.

```bash
cargo +stable run -p oscar-code --features desktop
# Real OpenAI-compatible providers configured in a separate TOML file:
cargo +stable run -p oscar-code --features desktop,http -- --config settings.toml
```

The desktop feature uses GPUI Kit 0.7.3 and its matching `gpui-pre` 0.3.9 engine.
Use current stable Rust for desktop builds (locally checked with 1.98.1). The
headless session library retains the workspace's Rust 1.85 support. Desktop
dependencies and native platform prerequisites are isolated behind `desktop`;
Windows is the initial verified build target. Linux/macOS desktop builds and
visual review on a native display have not been verified.

## Conversation

Enter creates a new line. In default proposal mode, **Send** submits one inference request using the
conversation so far. HTTP providers stream text as it arrives; the in-progress
Markdown preview is provisional. Only a successful reply enters conversation
history. Stop, Reset, failure or a limit discards the preview. A single latest
bounded snapshot coalesces updates when rendering is slower than inference.
Responses render selectable Markdown, including code
blocks. External images and links are disabled so model-produced Markdown cannot
load files, contact another server, or open applications.

**Stop Generation** cancels the active request. A cancelled or reset run cannot
append a late result. The header shows the run ID, outcome and consumed turns.
Model failure details are reduced to a terminal category rather than displaying
arbitrary adapter error text. There are no automatic retries.

**Reset** clears conversation content. It preserves turn count, monotonic run IDs,
and remote call/input budgets. Restart the application to begin a new budget.
Conversation is kept only in memory; no transcript or prompt logs are written.

## Typed tool messages

The headless session API supports host-supplied `Message::ToolCall` and
`Message::ToolResult` entries. The desktop renders the tool name, call ID and
success/error status, with arguments and successful output as indented JSON trees.
Objects show named fields, arrays show indexed items, and scalars retain their
JSON types, including strings, numbers, booleans and null. Empty containers remain
visible. Tool strings render literally; Markdown, URLs and paths have no actions.
Failures use core `ToolError` categories instead of arbitrary implementation errors.

`StructuredData::from_json(text, max_bytes)` checks UTF-8 input bytes and compact
JSON bytes before returning immutable data. Trees allow at most 16 levels below
the root and 512 values including containers. Arguments must be objects; successful
output may be any JSON value. Call IDs and names accept 1–64 ASCII letters, digits,
underscores, hyphens or periods, matching the core registry's identifier rules.

`record_tool_call(run_id, call)` and `record_tool_result(run_id, result)` accept
only the active run. Call IDs must be unique in retained history; each result must
match a current-run call's ID and name and may appear only once. Late messages
after Stop, Reset or completion fail with a typed `MessageError`. Calls count
against `limits.max_input_bytes` (default 24,000); results count against
`limits.max_output_bytes` (default 16,384), including identifiers and compact
payload bytes. All entries share the existing retained-history byte limit of
input plus output limits. A rejected insertion leaves history unchanged.

These types are also used by the opt-in autonomous runtime below. In proposal
mode, JSON-looking replies remain ordinary model messages and tool entries are
excluded from model context. Message types alone never register, authorize or
execute tools. Nested presentation data does not expand the registry's flat input
schema. Approval dialogs remain a separate step.

For a host-supplied display entry:

```rust
use oscar_code::{
    messages::{StructuredData, ToolCall, ToolOutcome, ToolResult},
    session::SessionState,
    settings::Settings,
};
use oscar_core::config::Limits;

let limits = Limits::default();
let mut session = SessionState::new(limits.clone(), Settings::default());
let run = session.begin("Inspect the project")?;
let arguments = StructuredData::from_json(r#"{"path":"src/lib.rs"}"#, limits.max_input_bytes)?;
session.record_tool_call(run.id, ToolCall::new("call-1", "inspect", arguments)?)?;
let output = StructuredData::from_json(
    r#"{"files":[{"path":"src/lib.rs","lines":42}],"complete":true}"#,
    limits.max_output_bytes,
)?;
session.record_tool_result(run.id, ToolResult::new(
    "call-1", "inspect", ToolOutcome::Success(output),
)?)?;
```

The experimental message API now uses enum variants in place of the old
`Message { role, content }` struct. Read history through `session.messages()`;
use `message.role()` and `message.text()` for text entries, or match typed variants.
Tool payloads and identifiers are redacted in `Debug`; callers remain responsible
for omitting secrets from data explicitly displayed in the UI.

## Settings and limits

Provider configuration defaults to `settings.toml`; select another file with
`--config PATH`. Configure models, URLs, capabilities and `api_key_env` using the
same [core provider settings](hybrid.md). The settings panel displays the selected
model. It does not edit or persist API keys. Environment variables must be set
before launching the application. The optional `http` feature enables real adapters.

In mixed mode, **Use remote (share chat)** explicitly selects the configured remote
provider and shares subsequent conversation context with it. Local/full-remote
modes and `planning.local_only_data` constrain selection. There is no automatic
fallback or escalation. Providers must declare code analysis and code generation.
The provider's context window is checked conservatively using input bytes as a
token upper bound plus output tokens and 256 tokens of margin. Configured remote
cost caps require both input/output prices and must pass before inference.

Desktop preferences default to `oscar-code.toml` in the current directory; select
another path with `--ui-settings PATH`. **Save preferences** explicitly writes only
these fields, and rejects reuse of the provider config path at startup:

```toml
dark = true
max_turns = 32
timeout_ms = 60000
provider = "local"
autonomous = false
max_tool_calls = 16
```

Turn limits accept 1–256; timeouts accept 1–3,600,000 ms. The effective inference
deadline is the minimum of the desktop timeout and core call/run timeouts.
Core input/output byte bounds and output token limits apply unchanged. Core
remote call/input budgets also apply and count failed/cancelled admitted requests.
Only one request is active. Context exhaustion asks the user to reset rather
than silently dropping earlier messages. Raising the turn preference is an
explicit host action; consumed turns never decrease.

## MCP servers

Build with `--features desktop,mcp` and configure explicit server/tool selections
in the provider TOML. The desktop registers MCP tools before startup and exposes
only host-declared read-only selections to autonomous runs. See [MCP setup](mcp.md)
for stdio, Streamable HTTP, environment credentials, supported schemas and limits.
The default configuration starts no MCP servers.

## Autonomous runtime and events

Open **Settings**, select **Enable autonomous tools**, then **Send**. This is an
explicit opt-in to model-requested, registered read-only actions. The desktop
registers only `text_stats`, a pure tool that counts UTF-8 bytes, characters and
lines of supplied text and returns nested JSON. It has no filesystem, process,
network or credential access. The supplied mock configuration completes an
offline model → registry → model demo and displays the actual tool result.

Real configured providers must declare `tool_use` as well as `code_analysis` and
`code_generation`. With the `http` feature, OpenAI-compatible Chat Completions
providers receive host-allowed registry definitions as native `tools` with JSON
Schema parameters. Conversation history uses user/assistant messages, assistant
`tool_calls` and correlated `role: tool` messages with `tool_call_id` and string
content. Final responses are ordinary Markdown. Enable HTTP alongside the desktop:

```bash
cargo +stable run -p oscar-code --features desktop,http
```

The SSE adapter assembles fragmented function names/arguments by call index.
Only complete, valid call streams are returned to the runtime. A response may
contain up to 32 calls, further constrained by the remaining run tool budget;
the runtime executes them serially and sends all successful results in the next
request. Text, call IDs, names and raw arguments share the output byte limit.
Schemas and serialized conversation overhead count toward input/context and
remote budgets. UTF-8 byte bounds are enforced by the registry even though JSON
Schema string lengths count characters.

Injected providers can opt into native answers with
`ModelProvider::native_tool_protocol() = true`; `ModelOutput::tool_calls` holds
proposed calls, with argument JSON retained as a string through registry
validation. `MockProvider::scripted_native` and the desktop offline demo exercise
this typed flow without credentials. Existing injected providers default to the
legacy strict JSON envelopes (`kind: tool_call` / `kind: final`). Ordinary answer
text is never parsed as an executable call by a native provider.

Calls run serially through `ToolRegistry` with the original argument JSON. Unknown
tools, unauthorized tools, bad schemas and duplicate keys/IDs fail closed.
Side-effecting tools are denied; even a host policy requesting side effects is
rejected by this desktop runtime. Every tool failure terminates the run with a
fixed category. There are no automatic retries. A successful tool result enters
the next model request as explicitly labelled, untrusted evidence. Subsequent
autonomous chat turns include retained typed tool history; proposal turns continue
to include only user/model text. Tool results cannot grant permissions or add tools.

Core adapters remain inference-only: decoding calls grants no execution permission.
CLI proposals, planner inference, artifact workers and desktop proposal mode send
no tool definitions and reject typed tool outputs. Autonomous replies are buffered
until validated; proposal mode retains native text streaming. Live-provider
compatibility and native visual/platform QA have not been evaluated.

Every model request, including continuations, consumes the shared desktop turn
budget (default 32) and applicable remote call/input budgets. Failed/cancelled
admissions remain consumed across Stop, Reset, and provider/mode switches. Context
window, input byte limits and configured remote cost bounds are rechecked for
every request. There is no provider fallback. **Use remote (share chat)** includes
autonomous conversation/tool evidence and remains constrained by work modes and
`planning.local_only_data`.

Each run has at most `max_tool_calls` calls (default 16, configurable 1–64).
Tool input is bounded by core input bytes; tool output is bounded by the smaller
of core output bytes and 4 KiB, including JSON escaping for plain string results.
Existing tree and aggregate history limits still apply. The runtime reserves room
for a bounded result before admitting execution, and ends rather than discarding
old history silently. A run's wall-clock deadline is the smaller of desktop
`timeout_ms` and core `run_timeout_ms`; model/tool calls also obey core
`call_timeout_ms`. Stop, Reset, worker abort and lost UI subscriptions cancel
in-flight work. Host tools must satisfy core's nonblocking, cancellation-safe
contract; this slice adds no blocking filesystem tools.

The desktop subscribes to ordered run-start, model-start/finish, tool-request/result
and terminal events through a bounded eight-slot channel. Queue backpressure shares
the run deadline and cancellation path; no unbounded event log is accumulated.
**Run events** shows up to 642 entries of redacted phase/failure metadata for the
latest run. Typed tool payloads appear separately in bounded chat history. Events
with old run IDs or duplicate sequence numbers cannot append late content. An
authoritative final report supplies the terminal outcome if the final event could
not enter a full queue. Reset clears content and event metadata, preserving budgets.

The headless API is `SessionState::begin_agent(prompt, config, registry, policy)`
with `Settings::autonomous = true`, then `AgentRun::execute(provider, sender)`.
Drain the bounded receiver concurrently, apply events with
`session.apply_runtime_event(event)`, and apply the returned report with
`session.finish_agent(report)`. `runtime::starter_registry`, `starter_policy` and
`demo_provider` support offline integration tests. Custom hosts remain responsible
for selecting trustworthy tool implementations and filtering secrets before display
or remote context sharing.

Preference files are bounded to 4 KiB when read, reject unknown fields and contain
no credentials. Writes run on Tokio's blocking pool with a five-second UI waiting
limit. OS file writes cannot be forcibly cancelled and may finish after that
deadline. Shutdown waits at most one second for blocking work. Use a local path.
After a timed-out write, further actions are disabled until restart to prevent
accumulating uncancellable writes.

## Implementation plan status

- [x] GPUI dependencies, application/window, header, scrollable chat and composer.
- [x] In-memory session state, reactive updates, run IDs and terminal outcomes.
- [x] User/model/system rendering, Markdown, multiline input, Send/Stop/Reset.
- [x] Existing mock and optional HTTP inference providers, bounded background work.
- [x] Non-secret TOML preferences, provider selection, dark/light theme, limits.
- [x] Native provider text streaming, bounded previews, cancellation and final usage metadata.
- [x] Typed tool-call/result messages with nested structured output.
- [x] Autonomous model → registry tool → model runtime and runtime event subscription.
- [x] Native provider schemas, streamed tool calls and correlated result continuations.
- [ ] High-impact tool opt-in and exact-call approval dialogs; no such tools enabled.
- [ ] In-app model and credential configuration editing.
- [ ] Virtualized long histories and native visual/accessibility/platform QA.

## Validation

The default tests use scripted core providers, including failure, cancellation,
deadline, stale result, context/output bounds and reset/remote budget cases,
plus nested tool data, malformed inputs, matching/duplicate results, retention
limits and the proposal display boundary. Autonomous integration tests additionally
cover the real registry loop, ordered subscriptions, protocol/schema/policy
failures, duplicate calls, injection-shaped evidence, provider/queue/tool
cancellation, deadlines, turn/tool limits, continuation context/cost bounds and
shared remote budgets.
The optional GPUI test renders the actual view and clicks its controls without
live credentials. It also renders nested tool successes and failures in both
themes and verifies accessible tree labels and Reset. A second test clicks the
autonomous toggle and Send, waits for the actual subscribed runtime, and inspects
its tool result and event history:

```bash
cargo test -p oscar-code
cargo +stable test -p oscar-code --features ui-tests --lib
cargo +stable clippy -p oscar-code --all-targets --all-features -- -D warnings
```

The broader end-to-end gate still requires a CLI autonomous flow. High-impact
actions, approval dialogs and automatic repository exploration remain separate.
