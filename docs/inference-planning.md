# Full-plan inference (experimental)

The inference planner asks a generative model to decompose the entire user goal
into a task dependency graph. The model supplies task descriptions, acceptance
criteria, dependencies, capabilities, context requirements and expected outputs.
OSCAR converts this draft into its canonical `Plan`, validates its graph and
resource bounds, and renders `PLAN.md`. This is separate from the proposed
Laya/JEV answer-classification stage; a classifier is not a generative planner.

## Configure and run

Use [the complete local configuration](../examples/inference-config.toml),
adjusting its model, endpoint and declared capabilities to match your server:

```bash
cargo run -p oscar-cli --features http -- plan "Design a CSV import pipeline with duplicate detection and validation tests" --config examples/inference-config.toml --out import-plan
cargo run -p oscar-cli --features http -- run "Design a CSV import pipeline with duplicate detection and validation tests" --config examples/inference-config.toml --out import-run
```

In your own TOML, enable:

```toml
[planning]
mode = "inference"       # default: "heuristic" (offline templates)
local_only_data = false  # default; true forbids remote planning and workers
```

The selected provider must declare `planning` in its `capabilities`, alongside
the capabilities its generated workers need. Use `openai_compatible` with the
`http` Cargo feature for real inference. The offline `mock` demo returns prose,
so inference planning with that demo fails draft validation; it does not pretend
to generate a plan. Scripted library mocks support deterministic planner tests.

`--planner inference` or `--planner heuristic` overrides the TOML mode for that
invocation. With `--plan path/to/plan.json`, both commands load the saved plan
without planning inference, regardless of TOML mode; `--plan` conflicts with
`--planner` and with a request. `planning.local_only_data = true` also tightens
loaded plans' data boundaries; false never clears a saved local-only restriction.

Use `plan` to inspect the generated DAG before a later `run --plan`. Use `run`
with a request to generate and execute in one invocation. Workers currently
produce proposals; neither command automatically reads source files, applies
patches, executes commands, or authorizes tools.

## Routing and limits

Planning makes exactly one call. Strict `local`/`full_remote` modes select only
that provider. Mixed mode prefers local when it has the planning capability and
context capacity, otherwise eligible remote inference can be selected. Existing
cost ceilings and remote budgets apply. No automatic repair, retry, remote
escalation after failure, or silent heuristic fallback is performed by the planner.

Planning uses `[limits]`: input bytes (including instructions and capability
metadata), output bytes/tokens, task count, call timeout, run timeout and remote
calls/input bytes. Defaults remain 24,000 input bytes, 16,384 output bytes, 2,048
output tokens, 32 tasks, 60-second calls and a 300-second run. Larger plans may
need larger output budgets; the example uses 65,536 bytes and 8,192 tokens.
Provider context must fit the prompt, output token budget and envelope reserve.
Truncated JSON fails explicitly rather than producing a partial executable plan.

For `run` with inference planning, the planning call consumes the same remote
call/input budget as workers, and planning elapsed time is subtracted from the
worker deadline. `plan` followed later by `run --plan` represents two separately
budgeted invocations. Library callers composing `inference::generate` and
`execution::execute` should pass `PlanningReport::remaining_config` to execution.
Client cancellation drops the inference future; remote server work or billing
may continue. Host-supplied adapters must remain nonblocking and cancellation-safe.

## Validation and reports

The accepted draft is a JSON object containing only `tasks`. Each task has exactly:
`id`, `title`, `description`, `kind`, `difficulty`, `risk`, `dependencies`,
`required_capabilities`, `context_requirements`, `expected_outputs`.
Unknown fields, Markdown fences, invalid enum values, oversized metadata,
duplicate IDs, missing dependencies, cycles, unsupported capabilities and low
model confidence fail closed. A final review covering all branches is required
when `routing.remote_review = "always"`, or with `auto` if a task is high/critical
risk. Model risk estimates and plan completeness still require human judgment.

The host sets goal, version, work mode, routing preference/reason, nonempty
artifact validation, retry limits, context strategy and local-only restrictions.
The model cannot grant permissions, register tools/validators or change policy.
Descriptions and expected outputs are untrusted proposal text, never commands.
Structural validation does not prove semantic completeness or code correctness.

Both commands reserve a new output directory before calling a model and write
`planning.json` for a planning attempt, including failure or cancellation. It
contains terminal outcome, selected provider, routing reason, whether a call
started, simulation status, input byte count, token usage when known, elapsed
time and a sanitized error. Rejected model output and raw prompts are not saved
in that report. Configuration/adapter setup failures can leave an empty directory.
Successful planning also writes `.plan/plan.json` and `PLAN.md`; `run` additionally
writes `run.json` and worker artifacts. Read both reports for total inference
usage. Saved successful plans include the goal and generated content; `--out`
opts into persistence. Delete that directory to remove its saved content.

There is no live-provider accuracy certification, automated repository discovery,
plan revision loop or classifier integration in this feature.
