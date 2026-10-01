# OSCAR

**OSCAR — Orchestrated System for Context, Actions & Reasoning** — is an experimental AI orchestration runtime written in Rust.

OSCAR aims to help developers build agents that can reason over a task, create and revise a plan, use explicitly registered tools, keep useful context, and report what they did. Its initial goal is a dependable local-first developer assistant with provider-independent model access and bounded, observable execution.

> **Project status:** Hybrid planning and proposal execution are implemented. Workers can use local or remote models, pass artifacts, and report validation and inference usage. Automatic repository exploration, patch application, shell execution, MCP integration, and persistent sessions remain future work.

## Product principles

- **Local first:** use local models whenever they can do useful work; reserve cloud inference for meaningful reasoning gains and escalation.
- **Explicit capabilities:** select providers by declared capabilities and context limits, not model names.
- **Inspectable execution:** expose plans, routing decisions, retries, validation results, and terminal outcomes.
- **Bounded work:** limit concurrency, attempts, elapsed time, input/output size, and remote usage.
- **Provider independence:** keep orchestration separate from provider-specific HTTP formats.
- **Controlled actions:** model output cannot authorize tools, register validators, or grant filesystem permissions.

## Quick start

The workspace uses **Rust edition 2024** and declares **MSRV 1.85**. Run these commands from the repository root.

The supplied [example configuration](example_config.toml) uses offline mock providers. No API keys, local model server, or cloud tokens are required:

```bash
# Generate a plan without inference.
cargo run -p oscar-cli -- plan "Add JWT authentication and protect admin endpoints" --out auth-plan

# Execute the saved plan with the configured mock workers.
cargo run -p oscar-cli -- run --plan auth-plan/.plan/plan.json --out auth-run
```

You can also plan and execute in one command:

```bash
cargo run -p oscar-cli -- run "Document a parser interface" --out docs-run
```

Each `--out` directory must be new and have an existing parent. Existing output directories are never overwritten. Mock workers return demonstration artifacts; they do not implement the requested feature.

### Generated files

```text
OUTPUT_DIRECTORY/
  PLAN.md              Human-readable plan
  .plan/
    plan.json          Canonical, versioned execution plan
  run.json             Execution outcome, events, calls, usage, and artifacts
  artifacts/
    T01.md             Collected worker proposals
    ...
```

`plan` writes only the plan files. `run` also writes the report and collected artifacts. `PLAN.md` is rendered from the structured plan, so there is one source of truth.

With inference planning enabled, both commands additionally write `planning.json`
with the planner's outcome, routing, elapsed time and token usage.

See the generated [JWT example plan](examples/hybrid-plan/PLAN.md) and its [canonical JSON](examples/hybrid-plan/.plan/plan.json).

## Work modes

Set `work_mode` in your TOML configuration:

```toml
work_mode = "mixed"
```

| Mode | Behavior |
| --- | --- |
| `local` | All inference stays local. Capability/context limitations and exhausted retries fail explicitly; no remote escalation. |
| `full_remote` | All inference uses the remote provider; no local inference fallback. |
| `mixed` | Local-first execution with remote reasoning when quality, capability/context gaps, or failed local attempts justify it. |

The legacy TOML value `remote` is accepted as an alias for `full_remote`. Plan validation runs on the host in every mode; optional inference planning follows the same strict work modes as workers.

Use a different configuration file with `--config`:

```bash
cargo run -p oscar-cli -- run "Explain this interface: ..." --config oscar.toml --out interface-run
```

The example local provider does not declare advanced reasoning. Complex local-only requests require a capable local provider; changing the work mode does not bypass capability checks.

### Mixed-mode routing

Ordinary analysis, implementation proposals, tests, and documentation prefer local execution. High-risk reasoning and review may prefer remote inference. Selection also considers required capabilities, context size, prior failures, data restrictions, and configured cost limits.

The default escalation path is:

```text
Local attempt
    ↓ validation failure, low confidence, or transient inference failure
Local retry with feedback
    ↓ attempts exhausted
One remote attempt, if permitted by mode, policy, and budgets
```

Permanent provider failures stop execution. A task's `preferred_provider` can change at runtime; its `local_only_data` boundary cannot. Local-only restrictions propagate through dependent artifacts.

```toml
[routing]
local_first = true
max_local_attempts = 2
remote_review = "auto"
context_distillation = true
distilled_bytes_per_artifact = 2048
```

Before remote inference, eligible dependency artifacts are reduced to bounded, explicitly marked excerpts. The initial distiller is deterministic excerpting, not semantic summarization.

### Real inference

The optional `http` feature enables an OpenAI-compatible text chat-completions adapter:

```bash
cargo run -p oscar-cli --features http -- run "Document this interface: ..." --config oscar.toml --out real-run
```

Configure `provider = "openai_compatible"`, the model, capabilities, context window, and complete `api_url`. Prefer `api_key_env` for credentials. Local endpoints require literal loopback IPs; remote endpoints require HTTPS. Redirects and proxies are disabled.

See [hybrid configuration and execution](docs/hybrid.md) for complete provider examples, resource limits, cost metadata, cancellation semantics, and persistence behavior.

### Generate the full plan with inference

Enable the model planner in your TOML:

```toml
[planning]
mode = "inference"
local_only_data = true # Use false to permit remote inference within your work mode.
```

The model generates goal-specific tasks, dependencies and expected outputs. OSCAR
validates the DAG and supplies routing and policy fields. Configure a generative
provider with the `planning` capability. Start from
[examples/inference-config.toml](examples/inference-config.toml), replacing its
model and endpoint with your local server:

```bash
cargo run -p oscar-cli --features http -- plan "Design a CSV importer with duplicate detection and tests" --config examples/inference-config.toml --out csv-plan
cargo run -p oscar-cli --features http -- run "Design a CSV importer with duplicate detection and tests" --config examples/inference-config.toml --out csv-run
```

`plan` saves the generated plan for inspection; `run` generates and then executes
its proposal workers. `--planner inference` overrides the TOML setting;
`--planner heuristic` keeps the offline templates. `--plan` loads a saved plan
without another planning call. The supplied mock demo does not generate JSON
plans. See [inference planning](docs/inference-planning.md) for limits, reports,
failure behavior and the distinction from proposed Laya/JEV classification.

## Architecture

```text
User request or canonical JSON plan
                 |
                 v
     Request analysis and planning
                 |
                 v
       Validated dependency DAG
                 |
                 v
    Capability-aware routing policy
          /               \
         v                 v
   Local workers      Remote workers
          \               /
                 v
       Bounded artifact store
                 |
                 v
       Validation and feedback
                 |
                 v
    Retry / escalate / terminal report
```

Independent tasks execute concurrently within bounded Tokio execution waves. Provider-specific concurrency limits apply alongside the global limit. Workers consume declared dependency artifacts instead of repeatedly rediscovering the same information.

The workspace currently has two crates:

| Location | Responsibility |
| --- | --- |
| `crates/oscar-core/src/config.rs` | Work modes, provider metadata, routing configuration, and limits. |
| `crates/oscar-core/src/planning/` | Typed tasks, heuristic/inference planners, DAG validation, JSON plans, and Markdown rendering. |
| `crates/oscar-core/src/routing.rs` | Explainable provider selection, context fit, escalation, and cost checks. |
| `crates/oscar-core/src/providers/` and `providers.rs` | Provider-neutral async inference, mocks, and the optional HTTP adapter. |
| `crates/oscar-core/src/context.rs` | Shared artifacts, dependency context, and bounded excerpts. |
| `crates/oscar-core/src/execution.rs` | Scheduling, cancellation, validation, retries, events, and reports. |
| `crates/oscar-core/src/error.rs` | Typed orchestration errors. |
| `crates/oscar-core/src/tools/` | Host-registered tool contract, bounded argument schemas, permissions, approvals, and tool sessions. |
| `crates/oscar-cli` | Clap command registration, configuration loading, output files, and exit codes. |

Keep these responsibilities as modules until independent dependency or API boundaries justify additional crates. See [ADR 0002](docs/adr/0002-hybrid-orchestration.md) for the current decisions; [ADR 0001](docs/adr/0001-initial-workspace-layout.md) records the earlier scaffold.

## Validation and current limitations

Built-in validators check nonempty output, valid JSON, or required text. They do **not** establish that generated code compiles or behaves correctly. Hosts can explicitly register stronger deterministic validators through the Rust API. Unknown validator names fail before inference, and plans cannot register or authorize validators.

High model confidence never overrides failed validation. A passing registered validator can carry more weight than low model confidence.

Current limitations:

- Workers produce proposal artifacts; they do not automatically read repositories, apply patches, or run compiler/test commands.
- The default planner uses keyword heuristics. The inference planner generates a full DAG, but structural validation does not prove plan completeness or risk accuracy.
- Context distillation uses excerpts and can omit relevant evidence.
- Provider capabilities and prices are operator-supplied metadata.
- HTTP behavior is tested with loopback fixtures, not certified against live providers.
- Cancellation stops client work but cannot guarantee that a remote server stops inference or billing.
- There is no durable resume, session database, streaming, semantic retrieval, or requests-per-minute scheduler.

Saved plans and artifacts contain user content. `--out` explicitly opts into persistence; delete a saved run's output directory to remove it. The library keeps artifacts in memory without silently persisting them.

## Roadmap

Checked items describe the implemented hybrid proposal slice. Broader agent features remain separate acceptance gates.

Full-plan inference is available ahead of the remaining tool work at user request.
The next tool boundary work unlocks safe repository access and the later agent loop.
The dependency/API release audit remains open and
must finish before release; it does not block this incremental boundary work.

### Implemented hybrid slice

- [x] Typed work modes and capability-aware local-first routing.
- [x] Canonical JSON plans and deterministic human-readable Markdown.
- [x] Dependency validation and bounded parallel execution waves.
- [x] Async model boundary, mock providers, and optional HTTP inference.
- [x] Artifact passing and bounded context excerpts.
- [x] Validation hooks, local retries, and controlled remote escalation.
- [x] Cancellation, deadlines, remote budgets, and usage reports.
- [x] CLI plan/run workflow and deterministic boundary tests.

### Foundation and release policies

- [x] Align Apache-2.0 license metadata and add contribution, conduct, and security policies.
- [x] Add Linux/Windows CI for stable and Rust 1.85.0; verify the MSRV locally.
- [x] Confirm the first hosted CI matrix run after publishing the workflow.
- [ ] Review dependency licensing and public API compatibility.

### Safe repository tools and end-to-end changes

- [x] Add a registered tool contract with schema validation and permission policy.
- [x] Enforce tool call/output limits, cancellation/deadlines, duplicate-call rejection,
  and explicit host approval for side effects.
- [ ] Implement bounded read-only repository tools with canonicalized workspace roots.
- [ ] Attach compiler/test validation through an explicitly authorized execution boundary.
- [ ] Add reviewed patch application with approval and filesystem safeguards.
- [ ] Complete model-to-tool-to-model agent flow and adversarial boundary tests.
- [ ] Keep shell execution disabled until a separate threat review and policy exist.

The registry is an experimental library API; the CLI's proposal workers still do
not invoke tools. See [tool boundary documentation](docs/tools.md). Run its
credential-free example with `cargo run -p oscar-core --example tool_registry`.

### Planning and memory

- [x] Add opt-in full-plan inference to `plan` and `run`, with host-owned policy,
  validated dependencies, bounded calls, usage reports and TOML/CLI selection.
- [ ] Evaluate planner accuracy and routing quality on representative development tasks.
- [ ] Add observable plan revisions based on execution evidence.
- [ ] Measure semantic distillation against its extra inference cost.
- [ ] Define session retention, reset, deletion, and persistence interfaces.

### Optional inference-based answer classification (proposed)

Use a local or remote worker to generate an answer, then classify that answer
with Laya, JEV, or another compatible decision model. OSCAR uses the typed result
to select an allowed next step: accept, retry, gather context, escalate, or request
review. This is a possible future feature, not implemented configuration.

```text
OSCAR plan → local/remote worker inference → generated answer + evidence
           → Laya / JEV / classifier → typed decision
           → OSCAR policy and validation → next execution step
```

- [ ] Add an opt-in TOML classification stage, disabled by default, with provider,
  endpoint/model, decision schema, thresholds, timeouts, and call budgets.
- [ ] Implement a provider-neutral classification contract and a local Laya adapter
  following its state/typed-question usage guidance; support remote JEV and other
  classifiers through adapters and shared conformance tests.
- [ ] Combine classification with deterministic validation, preserve strict work
  modes and local-only data, and handle uncertain/invalid responses explicitly.
- [ ] Evaluate domain/language accuracy and calibration, and record classifier
  decisions, latency, usage, and resulting plan transitions.

See the [answer-classification proposal](docs/decision-classification.md) for the
proposed TOML, Laya usage requirements, compatibility boundaries, and acceptance tests.

### Integrations and hardening

- [ ] Route MCP capabilities through the common tool validation and permission path.
- [ ] Add provider conformance tests and optional streaming where useful.
- [ ] Expand prompt-injection, credential, filesystem, and resource-limit testing.
- [ ] Add rate scheduling, resumable execution, and telemetry only with explicit bounds and privacy controls.

### Release readiness

- [ ] Review public APIs, examples, compatibility policy, and security documentation.
- [ ] Verify clean installation and real-provider quick starts.
- [ ] Publish release notes distinguishing stable and experimental behavior.

See [AGENTS.md](AGENTS.md) for project invariants, acceptance gates, and the definition of done.

## Development

```bash
cargo fmt --all --check
cargo clippy --workspace --all-targets --all-features -- -D warnings
cargo test --workspace --all-features
cargo test --workspace
```

The [CI workflow](.github/workflows/ci.yml) checks formatting and Clippy, plus
default/all-feature tests on Linux and Windows with stable and Rust 1.85.0.
Builds use the committed lockfile (`--locked`). See [CONTRIBUTING.md](CONTRIBUTING.md)
for toolchain setup and the matching local commands. The Rust 1.85.0 all-feature
suite has passed locally on Windows; hosted Linux/Windows CI still needs its first run.

Tests use deterministic mocks and, with HTTP enabled, loopback fixtures. They do not require real API credentials or model downloads. The existing prototype command remains available:

```bash
cargo run -p oscar-cli -- test --name example
```

That command is a CLI scaffold; use `cargo test` to run the Rust test suite.

## Documentation

- [Hybrid configuration, behavior, and limitations](docs/hybrid.md)
- [Architecture decision: hybrid orchestration](docs/adr/0002-hybrid-orchestration.md)
- [Implementation plan](docs/hybrid-implementation-plan.md)
- [Delivery record and verification results](docs/hybrid-delivery.md)
- [Generated example plan](examples/hybrid-plan/PLAN.md)
- [Contributor and agent working agreements](AGENTS.md)
- [Contribution guide](CONTRIBUTING.md)
- [Community conduct](CODE_OF_CONDUCT.md)
- [Security reporting](SECURITY.md)
- [Foundation and MSRV decision](docs/adr/0003-foundation-and-msrv.md)

## Contributing

Start with a focused issue or scoped change. Follow [CONTRIBUTING.md](CONTRIBUTING.md),
[AGENTS.md](AGENTS.md), and the [code of conduct](CODE_OF_CONDUCT.md). Report
vulnerabilities using [SECURITY.md](SECURITY.md), without disclosing sensitive
details publicly.

## License

Licensed under the [Apache License, Version 2.0](LICENSE). Workspace Cargo metadata
uses the matching `Apache-2.0` identifier. Dependency license review remains a
separate release-readiness task.
