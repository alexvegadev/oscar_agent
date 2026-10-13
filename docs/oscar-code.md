# OSCAR Code desktop proposal chat

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

Enter creates a new line. **Send** submits one inference request using the
conversation so far. Responses render selectable Markdown, including code
blocks. External images and links are disabled so model-produced Markdown cannot
load files, contact another server, or open applications.

**Stop Generation** cancels the active request. A cancelled or reset run cannot
append a late result. The header shows the run ID, outcome and consumed turns.
Model failure details are reduced to a terminal category rather than displaying
arbitrary adapter error text. There are no automatic retries.

**Reset** clears conversation content. It preserves turn count, monotonic run IDs,
and remote call/input budgets. Restart the application to begin a new budget.
Conversation is kept only in memory; no transcript or prompt logs are written.

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
```

Turn limits accept 1–256; timeouts accept 1–3,600,000 ms. The effective inference
deadline is the minimum of the desktop timeout and core call/run timeouts.
Core input/output byte bounds and output token limits apply unchanged. Core
remote call/input budgets also apply and count failed/cancelled admitted requests.
Only one request is active. Context exhaustion asks the user to reset rather
than silently dropping earlier messages. Raising the turn preference is an
explicit host action; consumed turns never decrease.

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
- [ ] Native provider token streaming (core currently exposes complete responses).
- [ ] Typed tool-call/result messages with nested structured output.
- [ ] Autonomous model → registry tool → model runtime and runtime event subscription.
- [ ] High-impact tool opt-in and exact-call approval dialogs; no such tools enabled.
- [ ] In-app model and credential configuration editing.
- [ ] Virtualized long histories and native visual/accessibility/platform QA.

## Validation

The default tests use scripted core providers, including failure, cancellation,
deadline, stale result, context/output bounds and reset/remote budget cases.
The optional GPUI test renders the actual view and clicks its controls without
live credentials:

```bash
cargo test -p oscar-code
cargo +stable test -p oscar-code --features ui-tests --lib
cargo +stable clippy -p oscar-code --all-targets --all-features -- -D warnings
```

This slice does not complete the broader end-to-end tool execution roadmap gate.
