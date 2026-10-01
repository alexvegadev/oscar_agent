# ADR 0005: Host-validated full-plan inference

Status: accepted; experimental implementation.

## Context

The existing keyword planner emits fixed proposal templates. Users need a model
to decompose a goal into a complete, goal-specific DAG using local or remote
inference, accessible through both CLI commands.

## Decision

Add an opt-in `planning.mode = "inference"` and a CLI override. Reuse the existing
provider-neutral inference interface and routing policy with the `planning`
capability. One bounded call returns a strict typed task draft; the host supplies
all policy fields and validates the canonical Plan. Keep heuristic planning and
saved plans available without model inference. Preserve local-only data and
strict modes. Record planning outcome/usage separately and subtract its remote
usage and elapsed time before worker execution in the same CLI invocation.

## Alternatives

Accepting a complete model-authored canonical Plan would allow untrusted output
to supply policy fields. Dedicated vendor structured-output APIs would narrow
provider compatibility. A repair/escalation loop would add cost and complexity;
the initial planner fails explicitly after one call. No new dependencies or
planner trait are necessary: `ModelProvider` supplies the existing test seam.

## Consequences and migration

Existing TOML defaults and version-1 saved plans remain compatible. Rust callers
constructing Config literals must supply the new `planning` field. CLI output
directories may contain a failed `planning.json` or remain empty on setup errors.
Library callers composing planning/execution must carry forward remaining budgets.
The model can propose an incorrect or incomplete plan; schema/DAG checks cannot
prove semantic accuracy. Classification adapters, plan revisions, and tool use
remain separate roadmap items. New host policy fields require an explicit
mapping here rather than permitting the model to populate them.
