# ADR 0016: Opt-in session journals and mandatory host proposal gates

## Context

The short-term cycle requires traceable model inputs, deterministic gates, safe
patch application and persistent memory. The DAG runner already accepts host
validators, but a plan chooses its validation strategy. Session context is volatile
and run reports do not reconstruct submitted requests.

## Decision

Start with an independent bounded JSONL journal and mandatory host gate policy.
Keep them in oscar-core, on edition 2024 / Rust 1.88. Preserve `execute` and add
`execute_with_options`. Required validators apply to every artifact regardless of
task classification, confidence or provider. Duplicate requested/required checks
execute once. Infrastructure errors and cancellation terminate without repair
inference. Model-generated content cannot register validators or weaken policy.

Capture is off unless explicitly enabled; metadata is the enabled default.
Content capture separately opts into storing sensitive inputs and outputs.
A bounded Tokio channel feeds one writer, acknowledging write, flush and sync_data.
SHA-256 chains versioned records. Existing files are never opened for writing.
Read-only verification identifies incomplete tails without truncation or replay.

Wrap providers before planning/execution. Record logical requests, provisional
text deltas and completed outputs. The HTTP transport records the actual outgoing
JSON body before dispatch, after SDK conversion; headers, URLs and credential
configuration are excluded. Generic providers only get logical request capture.
Run/task/attempt correlations use host values. Reset appends an event without
erasing historical records or replenishing budgets.

## Alternatives

- run.json alone cannot reconstruct requests or interrupted work.
- Logging only context misses adapter system instructions and tool schemas.
- Plan-selected gates let models weaken acceptance criteria.
- Unbounded/lossy capture violates bounded work or traceability.
- A memory database now would couple archival storage to retrieval before its
  retention, deletion and projection interfaces are implemented.

## Consequences and threat boundaries

Journal failures abort further model work. Queue overflow aborts inference instead
of silently losing text. OS I/O cannot reliably be cancelled; timeout poisons the
writer, and crashes can lose unacknowledged fragments. Hash chaining is evidence
relative to a trusted checkpoint, not protection against an administrator rewriting
the whole chain. Directory durability across power loss is not guaranteed.
Hosts choose private trusted directories outside tool-writable roots. Unix uses
mode 0600; Windows inherits the parent ACL.

Raw HTTP response bodies and malformed stream frames are not retained. Captured
content means bounded accepted output and submitted requests, not hidden provider
instructions or proof of server receipt. `Closed` means writer shutdown, not that
every started inference finished. Cancelled calls can lack a terminal response.

Command/snapshot validators, patch transactions, desktop journal UI, compaction
events and persistent retrieval remain subsequent work. Proposal gates do not
prove cargo or tests ran. Registered Rust validators remain trusted host code
subject to the existing nonblocking/cancellation contract.

## Migration impact

Existing commands and library callers keep their behavior. New flags explicitly
enable capture. Required validators are a host API, not commands loaded from model
output. The journal is a new versioned format; plan/run formats are unchanged.
SHA-256 adds a small direct dependency already present in the lockfile. No database
or new crate split is introduced.
