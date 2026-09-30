# ADR 0002: Bounded local-first proposal orchestration

## Status

Accepted; experimental public Rust API.

## Context

The existing workspace has oscar-cli (Clap registry) and oscar-core (TOML
configuration). Provider/context files are placeholders. There is no model
invocation, tool registry, agent loop, persistence or concurrency architecture.
The requested hybrid feature needs real execution without bypassing the future
permission boundary or creating empty crates.

## Decision

Keep implemented boundaries as core modules: config, planning, routing, providers,
context and execution. Use Tokio, a provider-neutral boxed async inference trait,
explicit task dependencies and host-registered validators. The default planner is
deterministic and replaceable by any validated Plan, avoiding a mandatory planning
model call. Plans are versioned JSON; Markdown is derived.

Mixed-mode routing is local-first with explicit, explainable exceptions for
reasoning/risk, capability/context gaps and failed attempts. Strict modes never
cross providers. Enforce attempt, byte, concurrency, time and remote budgets.
Capabilities are operator metadata, not model-name heuristics. Optional cost
metadata can cap calls without a billing framework.

Workers perform inference only and return proposal artifacts. No model output is
interpreted as filesystem/shell permission. Host validators are explicitly
registered, and plans cannot add them. Deterministic validation overrides model
confidence; built-in format validators cannot establish code correctness.

Share bounded in-memory artifacts, pass declared dependencies only, preserve
local-only data transitively, and excerpt dependencies deterministically for
remote calls. Artifacts/plan persistence is an explicit CLI operation into a new
user-named output directory, with no overwrite. Expected output names are metadata.

Feature-gate HTTP to keep TLS/network dependencies optional. The common
OpenAI-compatible text endpoint adapter uses secure remote configuration, local
literal loopback URLs, no redirect/proxy, output caps, timeouts and cancellation
on future drop. Use deterministic mocks by default. Enable writeable/alloc only
under HTTP to support the ICU 2.0 dependency family selected for the workspace MSRV.

## Alternatives

- A model-generated mandatory scout/planner/distiller pipeline: extra inference
  before simple tasks and a larger untrusted-plan surface.
- Immutable provider assignments: prevent evidence-driven escalation.
- New crates per conceptual component: premature package boundaries.
- Running generated cargo/shell commands: violates the missing tool policy boundary.
- Database persistence: unnecessary for explicit bounded run artifacts.

## Consequences

The full planning/routing/inference lifecycle is usable offline and with configured
text endpoints, but autonomous repository modifications remain outside this slice.
Keyword planning and excerpting need evaluation and can miss important evidence.
The host is responsible for the quality/security of declared provider capabilities,
local servers and registered validators. Cancellation cannot undo server billing.
No automatic retries occur for permanent errors or tool side effects.

Tokio's [JoinSet](https://docs.rs/tokio/latest/tokio/task/struct.JoinSet.html)
provides abort-on-drop task ownership; the runtime also drains it on termination.
HTTP policy uses the documented
[reqwest client controls](https://docs.rs/reqwest/0.12/reqwest/struct.ClientBuilder.html).

## Migration impact

Legacy TOML `remote` aliases `full_remote`; use WorkMode::FullRemote in Rust.
Existing features/provider fields remain readable, including legacy api_key,
although unknown fields now fail closed. Config Debug redacts credentials.
`test` no longer reads example_config.toml or prints its contents. New CLI commands
are `plan` and `run`; no existing command is removed. The old unreferenced
context/context.rs placeholder is preserved. Existing ADR 0001 describes the
historical one-crate scaffold; the implemented workspace now has two members.
