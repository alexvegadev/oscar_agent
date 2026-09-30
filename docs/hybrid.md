# Hybrid planning and execution

OSCAR implements an experimental proposal-oriented hybrid runtime in `oscar-core`.
It invokes local and remote models, schedules a task DAG, passes artifacts, validates
outputs and records usage. It does not yet read repositories automatically, apply
patches, execute model-proposed tools, or run shell commands. Supply relevant source
excerpts in the request or a task's description/context requirements. A host can
construct a richer `Plan` and register deterministic validators.

## Quick start

From the repository root, the supplied configuration uses two **mock** providers:

```powershell
cargo run -p oscar-cli -- plan "Add JWT authentication and protect admin endpoints" --out auth-plan
cargo run -p oscar-cli -- run --plan auth-plan/.plan/plan.json --out auth-run
```

Each output directory must be new, with an existing parent. `plan` performs no
inference. `run` writes the plan before invoking workers, then writes `run.json`
and `artifacts/<task-id>.md`. Expected output names in tasks are descriptive;
they never authorize arbitrary filesystem writes. Failed/cancelled runs also
write their terminal report and any collected artifacts. A configuration/plan
error before execution returns a nonzero exit code. Partial output directories
are retained after I/O errors; choose a new directory when retrying.

The JSON at `.plan/plan.json` is the canonical version-1 plan. `PLAN.md` is a
pure deterministic renderer with dependencies, execution waves, reasons,
validation strategies, escalation limits and expected remote calls. Runtime
routing may change the preferred provider based on actual context and failures.

## Configuration and strict modes

Use `--config path/to/oscar.toml` to choose configuration. There is no implicit
environment override of configuration, except explicitly named secret variables.

```toml
work_mode = "mixed" # or "local" or "full_remote"
```

- `local`: all inference stays local. Insufficient capability/context or exhausted
  retries fails visibly. It never initializes the remote adapter.
- `full_remote`: all inference stays remote. No local inference or local fallback.
  Deterministic planning and excerpting still run on the host.
- `mixed`: local first. Local is required; remote may be disabled. Ordinary
  implementation, documentation and tests stay local when capabilities fit.
  High/critical reasoning or review, capability/context gaps, and exhausted local
  attempts can justify remote use. Difficulty alone does not select remote.

Legacy TOML `work_mode = "remote"` remains an alias for `full_remote`. JSON writes
`full_remote`. Existing feature flags are accepted but skills/MCP are not implemented.
Only provider entries named `local` and `remote` are supported. Unknown fields,
invalid modes and zero/out-of-range limits fail rather than being ignored.

The mock example is intentionally asymmetric: local lacks advanced reasoning.
For complex local-only requests configure a local provider that actually supports
that capability; OSCAR will not pretend a capability exists to make a plan work.
Capabilities are operator assertions, independent of model names.

For real inference build with `--features http` and replace the provider entries:

```toml
[providers.local]
enabled = true
provider = "openai_compatible"
model = "your-local-model"
api_url = "http://127.0.0.1:1234/v1/chat/completions"
context_window = 32768
max_concurrency = 2
capabilities = ["code_analysis", "code_generation", "documentation", "testing", "summarization", "advanced_reasoning", "review"]

[providers.remote]
enabled = true
provider = "openai_compatible"
model = "your-remote-model"
api_url = "https://your-provider.example/v1/chat/completions"
api_key_env = "OSCAR_REMOTE_API_KEY"
context_window = 128000
max_concurrency = 1
capabilities = ["code_analysis", "code_generation", "documentation", "testing", "advanced_reasoning", "review"]
```

`api_url` is the complete chat-completions endpoint. Servers must accept text
messages and `max_tokens` and return text choices; not every vendor/model supports
this wire format. Local URLs require a literal loopback IP (not `localhost` or a
remote host). Remote URLs require HTTPS. URL credentials/query strings/fragments,
redirects and proxies are disabled. A local server is trusted to perform inference
locally; OSCAR cannot prevent that server from forwarding requests elsewhere.
Legacy `api_key` is accepted and redacted from Debug, but prefer `api_key_env`.
Secrets are never written back into config. The CLI no longer prints config.

```powershell
cargo run -p oscar-cli --features http -- run "Document this interface: ..." --config oscar.toml --out real-run
```

## Routing, validation and escalation

The default planner uses documented keyword heuristics for risk/complexity and
selects a small DAG: documentation alone, or analysis followed by implementation
and test proposals in parallel. Complex requests add a design task. Sensitive
requests add review in `auto`; `always` adds review even for documentation; `never`
omits that stage. These options never override strict work modes or data boundaries.
Keyword classification can miss risks: inspect/edit the JSON plan for important work.

`preferred_provider` is a preference. `local_only_data` is an enforced boundary:
it propagates through dependencies and prevents remote transfer in every mode.
Use it for private task inputs. Distillation cannot remove this restriction.
All required capabilities must match; input plus output allowance must fit.

Default retries are two local attempts total (including the initial attempt),
then at most one remote attempt. Local retry receives explicit validator feedback.
Only transient inference errors (transport timeout/connect, HTTP 429/5xx), failed
validation or low confidence can retry. Permanent errors stop. Backoff is bounded
at 25 milliseconds times the local failure count. Model calls are proposal-only,
so retries cannot duplicate tool side effects. `allow_remote = false` disables
mixed-mode escalation for a task. Configuration caps task-level attempt limits.

Built-in `non_empty`, `json` and `contains` validators check artifact shape, **not
semantic or code correctness**. High confidence never overrides failed validation.
Low confidence causes a retry unless a registered host validator passes. Use
`ValidationStrategy::Registered(name)` with the `Validators` map in the Rust API
to attach stronger tests. Unknown names fail before inference. Plans cannot
register validators or grant permissions. The CLI deliberately exposes no shell
validator; a host must enforce tool/approval policy itself. Validators must be
nonblocking and cancellation-safe and are subject to a deadline.

## Context and artifacts

The in-memory artifact store shares content via `Arc<str>`. Only declared dependency
artifacts are included. Before remote inference, `Distilled`/`RepositorySlice`
strategies select a bounded UTF-8 excerpt per dependency with its task ID and
validation provenance. The goal and explicit constraints remain intact. `Direct`
or disabled distillation sends the bounded full dependency contents. Oversized
contexts fail rather than silently dropping constraints.

The initial distiller is deterministic excerpting, not semantic summarization or
an automatic call-graph extractor. It can omit relevant evidence, and marks
truncation explicitly. Hosts should supply focused repository slices. Local-model
summarization can be represented as a dependency task using `Summarization`;
there is no mandatory extra model call before every remote call.

## Budgets, concurrency, cancellation and reports

Defaults (all explicit in the example):

| Setting | Default | Accepted bounds |
| --- | ---: | --- |
| `limits.max_tasks` | 32 | 1–256 |
| `limits.max_concurrency` | 2 | 1–32 |
| `providers.*.max_concurrency` | 1 | 1–32 |
| `limits.run_timeout_ms` | 300000 | 1–3600000 |
| `limits.call_timeout_ms` | 60000 | 1–run timeout |
| `limits.max_input_bytes` | 24000 | 256–1048576 |
| `limits.max_output_bytes` | 16384 | 1–1048576 |
| `limits.max_output_tokens` | 2048 | 1–65536 |
| `limits.max_remote_calls` | 8 | 0–256 |
| `limits.max_remote_input_bytes` | 96000 | 0–268435456 |
| `routing.max_local_attempts` | 2 | 1–10 |
| `routing.distilled_bytes_per_artifact` | 2048 | 128–65536 |

Global concurrency and separate provider semaphores bound parallelism; no independent
async executor is introduced. Waves have a barrier between them. There is no
requests-per-minute scheduler yet. Remote budgets are reserved atomically before
invocation; failed/interrupted calls still count because they may incur cost.
Context fit uses a conservative one-token-per-UTF-8-byte estimate with an envelope
reserve. This is not vendor tokenization. Remote `cost.input_per_million` and
`cost.output_per_million` (USD) can be paired with `routing.max_remote_call_cost`;
a configured cost ceiling requires both price fields. Missing usage is `null`,
never a claim of zero tokens. No billing engine or total currency budget exists.

Ctrl+C, `CancellationToken`, and deadlines abort pending workers and drop inference
futures. A remote server may still finish a request or charge for it after the
client disconnects. Custom providers must obey the nonblocking/drop contract;
OSCAR cannot preempt blocking code inside a third-party implementation.

`run.json` contains a unique run ID, terminal outcome, typed events, every attempted
model call, simulation status, byte counts, returned token usage and artifacts.
Routing events record task IDs and reasons without prompts/credentials. The saved
plan and artifacts intentionally contain user content: `--out` is explicit opt-in
to persistence. The library keeps results in memory; drop the report to release
retention. Delete the output directory to remove a saved run. There is no hidden
session database, automatic telemetry, retention service, or resumable execution.

## Validation and next steps

Tests use scripted models and loopback HTTP fixtures, never real model credentials.
Run `cargo fmt --all --check`,
`cargo clippy --workspace --all-targets --all-features -- -D warnings`, and
`cargo test --workspace --all-features`. Default-feature tests exercise the
credential-free build. HTTP adapter behavior is fixture-tested, not live-provider
certified. Rust 1.85 remains the declared MSRV; verification used the installed
compiler, not a separate 1.85 toolchain.

Next: implement the registered read-only repository tool boundary, attach compiler/
test validators with explicit authorization, evaluate planner quality on real tasks,
and add semantic distillation only when measured token savings justify its calls.
Patch application and shell execution require the separate tool threat review.
