# Proposal: inference-based answer classification

Status: possible future feature; not implemented. Configuration below is a design
sketch and must not be added to a current runtime config, which rejects unknown
fields. This proposal does not change the existing execution path.

## Intended behavior

OSCAR executes a planned task using a local or remote generative model. The worker
produces an answer/proposal. A separately configured classification model examines
that answer with the task goal, constraints, relevant artifacts, and validation
evidence. Its typed result helps OSCAR decide what happens next.

```text
OSCAR plan execution
        |
        v
Local/remote generative model → answer/proposal
        |
        v
Bounded state: goal + constraints + answer + independent evidence
        |
        v
Laya (local) / JEV (remote) / another classifier
        |
        v
Typed classification + probabilities/confidence
        |
        v
OSCAR checks validation, policy, mode, privacy, and budgets
        |
        +→ accept artifact / retry / gather context / escalate / request review
```

The classifier selects among application-defined decisions. OSCAR owns execution
and authorization. A suggested action cannot register tools, grant permissions,
or turn an unsuccessful compiler/test result into success. Unavailable operations
(such as automatic context gathering before repository tools exist) must surface
as limitations rather than being reported as performed.

## Laya usage requirements

Laya takes `state` and typed `questions`, returning structured answers rather than
generating text. Support its `choice`, `score`, and `noul` question forms. Use its
Router or an explicitly appropriate language checkpoint; budget both state and
question/options instead of silently truncating. Its self-hosted server documents
a JEV-compatible `POST /v1/systemone` interface. The card warns about overconfidence,
domain-specific accuracy, `noul` label sensitivity, and unreliable
`action.act_probability`; evaluate/calibrate on OSCAR tasks and do not use that
action field as authorization. [Source: Laya model card](https://huggingface.co/convaiinnovations/laya).

Keep question instructions and decision rubrics under host control, separate from
the worker's untrusted answer. Include an uncertainty/review path and test that
answer text cannot redefine the allowed options. Confidence thresholds are
provider-, task-, and language-dependent; no universal threshold proves correctness.

## Provider compatibility

Define a small Rust classification boundary separate from the text-generation
provider. Normalize typed answers, selected labels, distributions, confidence when
available, provider/model identity, and usage. Reject missing answers, unknown
labels, invalid types, nonfinite/out-of-range probabilities, and malformed
distributions. Do not invent missing confidence or usage values.

JEV's official API evaluates `state` with named typed `questions` at
`POST https://api.typesafe.ai/v1/systemone`, using bearer authentication and a model
identifier. Returned answers correspond to the supplied question IDs.
[Source: TypeSafe API reference](https://docs.typesafe.ai/api).

Laya and JEV should have adapter/conformance tests for the supported common schema;
similar HTTP shapes are not proof of identical semantics or calibration. Other
remote classification models can translate their native API into the same contract.
Do not assume they implement chat-completions or the System One wire protocol.

## Proposed TOML opt-in

Illustrative future settings; exact names will be settled during implementation:

```toml
[classification]
enabled = false                 # Set true to enable once implemented
provider = "laya"              # Proposed adapters: laya, jev, custom
execution = "local"            # Locality is explicit, not inferred from model name
api_url = "http://127.0.0.1:8000/v1/systemone"
decision_schema = "task_outcome_v1"
on_uncertain = "request_review"
on_error = "fail_task"
timeout_ms = 5000
max_calls_per_run = 16
max_input_bytes = 12000
max_output_bytes = 4096
```

For a remote JEV classifier, the proposed replacement settings are:

```toml
[classification]
enabled = true
provider = "jev"
execution = "remote"
api_url = "https://api.typesafe.ai/v1/systemone"
model = "jev-latest"
api_key_env = "OSCAR_CLASSIFIER_API_KEY"
decision_schema = "task_outcome_v1"
on_uncertain = "request_review"
on_error = "fail_task"
timeout_ms = 5000
max_calls_per_run = 16
max_input_bytes = 12000
max_output_bytes = 4096
```

Add schema-specific acceptance thresholds after evaluation; illustrative timeout
and size values above are OSCAR design proposals, not provider context guarantees.
An absent/disabled section must preserve current behavior without extra inference.
Enabling an unavailable adapter or conflicting configuration must fail explicitly.

## Policy, limits, and observability

- `local`: classification inference must also be local; remote JEV is not allowed.
- `full_remote`: classification inference must be remote; a local Laya adapter is
  not silently run as preprocessing.
- `mixed`: prefer a configured capable local classifier; remote classification
  requires explicit configuration and available remote budget.
- Preserve local-only data across the classification state and any excerpts.
  A classifier decision cannot authorize transfer of private artifacts.
- Count classifier calls toward run deadlines and applicable remote call/input/cost
  budgets as well as classification-specific limits. Bound retries and subsequent
  decision cycles to prevent classifier-driven loops.
- Apply cancellation, endpoint restrictions, environment-backed secrets, and
  response-size enforcement at the classifier boundary.
- Record classification events and their resulting transitions, model/schema
  versions, latency, and usage. Avoid logging raw state by default. Display expected
  classifier calls alongside worker calls in plans and actual usage in run reports.
- On uncertainty, failure, or exhausted budgets, follow the configured bounded
  fallback; never silently accept an artifact or bypass deterministic validation.

## Acceptance criteria

- [ ] Disabled configuration makes zero classifier calls and preserves old configs.
- [ ] Local worker output can be classified by local Laya and influence a subsequent
  execution step, with task/evidence provenance retained.
- [ ] JEV and a second mock remote adapter pass the common contract tests without
  provider-name branches in planning logic.
- [ ] All work modes, local-only data, invalid outputs, timeouts, cancellation,
  budgets, uncertain decisions, and bounded retry/escalation paths have tests.
- [ ] Failed deterministic validation cannot be overridden by classifier confidence.
- [ ] Prompt-injection fixtures cannot change question rubrics or permissions.
- [ ] Domain/language evaluation and threshold calibration are documented before
  enabling automatic acceptance; confidence is never treated as proof of correctness.
- [ ] PLAN.md and run reports distinguish generation from classification inference.
- [ ] Live-provider checks are opt-in; the default test suite needs no credentials
  or model downloads.
