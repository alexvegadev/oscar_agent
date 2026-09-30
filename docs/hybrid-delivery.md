# Hybrid orchestration delivery record

## Existing architecture

The inspected workspace had two Rust 2024/MSRV 1.85 crates: a synchronous Clap CLI
with a command registry and oscar-core with TOML configuration. There was no
implemented model invocation, tool registry, task runtime, artifact persistence,
error hierarchy for inference, or async architecture. Context/provider files were
placeholders. All repository files were untracked before this change; no existing
work was reset or committed. Graph discovery was followed by direct source reads
when the service disconnected; its later coverage metadata was stale, so no claim
of exhaustive graph verification is made.

## Decisions and delivered behavior

- Keep focused modules inside the existing crates; introduce Tokio for bounded
  inference/concurrency and feature-gate HTTP/TLS dependencies.
- Use typed tasks and a versioned JSON Plan as canonical execution data. Render
  PLAN.md from that data, including execution waves and expected remote calls.
- Use deterministic request analysis for a small replaceable default DAG. Ordinary
  tasks prefer local execution; high-risk reasoning, capability/context gaps and
  exhausted attempts can justify remote inference. Exact transmitted contexts
  determine runtime fit and estimated cost after distillation.
- In mixed mode permit two local attempts by default, with validation feedback,
  then at most one remote attempt. Retry transient inference/validation failures;
  stop permanent errors. Strict modes never cross providers.
- Produce bounded proposal artifacts. Reject model-generated tool calls. Pass
  dependency artifacts by shared ownership and propagate local-only restrictions.
- Built-in validators check output format; explicitly registered host validators
  can provide deterministic evidence. Strong evidence overrides low confidence;
  confidence never overrides validation failure.
- Persist only on explicit CLI commands to a new output directory; record run
  outcome, routing/escalation/validation events, invocation byte counts, optional
  token usage and mock status. Keep secrets out of logs and error bodies.

Important types: WorkMode, Capability, Routing, Limits, ProviderCost, TaskId, Task,
Difficulty, RiskLevel, ProviderPreference, ValidationStrategy, EscalationPolicy,
ContextStrategy, Confidence, Plan, Decision, ModelProvider, ModelRequest,
ModelOutput, TokenUsage, Providers, Artifact, ArtifactStore, Validator, Event,
CallRecord, RunReport, Outcome and OscarError.

## Added files

- `crates/oscar-core/src/error.rs`
- `crates/oscar-core/src/routing.rs`
- `crates/oscar-core/src/execution.rs`
- `crates/oscar-core/src/planning/mod.rs`
- `crates/oscar-core/src/planning/renderer.rs`
- `crates/oscar-core/src/providers/http.rs`
- `crates/oscar-core/tests/hybrid.rs`
- `crates/oscar-cli/src/commands/orchestration.rs`
- `crates/oscar-cli/tests/orchestration.rs`
- `docs/hybrid-implementation-plan.md`
- `docs/hybrid.md`
- `docs/adr/0002-hybrid-orchestration.md`
- `docs/hybrid-delivery.md` (this record)
- `examples/hybrid-plan/PLAN.md`
- `examples/hybrid-plan/.plan/plan.json`

## Modified files

- `crates/oscar-core/Cargo.toml`
- `crates/oscar-core/src/lib.rs`
- `crates/oscar-core/src/config.rs`
- `crates/oscar-core/src/context.rs`
- `crates/oscar-core/src/providers.rs`
- `crates/oscar-cli/Cargo.toml`
- `crates/oscar-cli/src/main.rs`
- `crates/oscar-cli/src/error.rs`
- `crates/oscar-cli/src/commands/mod.rs`
- `Cargo.lock`
- `example_config.toml`
- `README.md`
- `AGENTS.md`
- `docs/README.md`
- `examples/README.md`

## Configuration and plan generation

Set work_mode to local, full_remote, or mixed in the TOML file passed via --config.
The legacy remote spelling remains accepted. The supplied example is mixed with
mock providers and consumes no real cloud tokens. See hybrid.md for complete real
local/remote examples, secret environment variables and limits.

`oscar plan REQUEST --out NEW_DIRECTORY` writes PLAN.md and .plan/plan.json.
`oscar run --plan PATH --out NEW_DIRECTORY` loads/validates canonical JSON and
executes it. The sample JWT plan has five tasks: local analysis, remote design,
local implementation/test proposals in parallel, and remote review. It was
generated with the actual CLI without inference.

## Verification

- `cargo check --workspace --all-features`: passed.
- `cargo fmt --all --check`: passed.
- `cargo clippy --workspace --all-targets --all-features -- -D warnings`: passed.
- `cargo test --workspace --all-features`: 25 passed (22 added, 3 existing).
- `cargo test --workspace`: 22 passed (HTTP tests excluded).
- After tightening the public excerpt helper for budgets below the truncation
  marker size, its UTF-8/budget regression test and fmt/Clippy passed again.
- CLI generation of examples/hybrid-plan: passed, five tasks, no inference.

The 22 added tests cover routing modes, capabilities, risk versus difficulty,
price/budget limits, retries/escalation, confidence and registered validators,
DAG rejection/waves, JSON round trips, deterministic Markdown, parallelism and
provider slots, dependency artifact passing, privacy propagation, UTF-8 excerpt
limits, exact-context routing, cancellation/deadlines, CLI persistence/no-overwrite,
HTTP endpoint policy, usage parsing, malformed/tool responses and wire limits.
Default tests are mock-backed; feature-enabled HTTP tests use loopback fixtures.

## Remaining limitations and next steps

This is a working planning/routing/inference system, not an autonomous repository
editing agent. Default workers produce proposals; format validation does not prove
code correctness. Repository reading, patch application and shell execution need
the registered tool/approval boundary. No live cloud provider or separate Rust
1.85 toolchain was tested. Request classification is heuristic, distillation uses
marked excerpts, and provider capabilities/prices are operator assertions.
There is no durable resume, semantic retrieval, requests-per-minute scheduler,
streaming, automatic test runner, or billing reconciliation. Server-side inference
may continue after client cancellation.

Next implement authorized read-only repository tools and host compiler/test
validators, then measure planner accuracy and context savings before adding
model-driven planning/distillation. Broader roadmap gates remain incomplete.
