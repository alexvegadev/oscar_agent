# Hybrid orchestration implementation plan

Repository inspection found two crates: a synchronous Clap command registry and
core TOML configuration. There is no implemented inference, tool execution,
task runtime, or persistence. Existing files are untracked and must be preserved.

1. Extend existing configuration with strict work modes (legacy `remote` alias),
   routing policy, endpoint metadata, and validated resource budgets.
2. Add typed task/plan models, deterministic request analysis, capability-aware
   local-first routing, DAG validation, and JSON-derived Markdown rendering.
3. Introduce a provider-neutral asynchronous inference boundary, deterministic
   mocks, and an optional bounded OpenAI-compatible HTTP adapter.
4. Execute DAG waves with bounded Tokio concurrency, cancellation, validation,
   feedback-based local retries, remote escalation, and typed lifecycle events.
5. Pass bounded in-memory artifacts; distill dependency context deterministically
   before remote calls. Keep outputs as proposals: no arbitrary tool or shell
   execution before the tool policy boundary exists.
6. Add explicit CLI plan/run commands that write a new output directory containing
   PLAN.md, canonical .plan/plan.json, and a run report. Do not log configuration
   secrets or prompt contents.
7. Test routing, invalid plans/configuration, dependency scheduling, validation,
   escalation, cancellation, budgets, serialization, and CLI artifacts. Run fmt,
   all-feature Clippy/tests and default-feature tests; document remaining limits.

Architecture remains within oscar-core and oscar-cli. Configuration capabilities
are operator assertions, never inferred from model names. All modes enforce
capability/context limits; strict modes fail instead of crossing providers.
