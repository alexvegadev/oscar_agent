# ADR 0006: Interactive proposal agent

## Context

The plan/run CLI persists DAG-based proposal artifacts. Users also need a terminal
conversation that can inspect selected source files and refine edit proposals.
The current model contract supports bounded text inference, not native tool calls.

## Decision

Add `oscar agent` as an in-memory CLI session using existing provider adapters and
routing. Host commands load explicit workspace files through the common tool
registry. The CLI owns session history, counters, terminal interaction and
outcomes. Each turn requires code-analysis and code-generation capabilities.
Remote sharing of conversation/file context requires a CLI flag and remains
subject to TOML work mode and local-only policy.

Use pinned `cap-std` 3.4.4 in the CLI for directory-capability file opens rather
than canonicalize-then-open checks vulnerable to symlink replacement. Its extra
dependencies protect a real filesystem boundary without expanding core APIs.
Keep reads and inference bounded and side effects unavailable. Budgets survive
context reset. No automatic inference retries or model-initiated tools are added.

## Alternatives

- Reuse a complete proposal DAG on every message: adds unrelated artifacts and
  per-turn budgeting without a natural conversational response.
- Implement a full autonomous coding loop: requires a new structured model/tool
  protocol, reviewed write boundary and separate shell threat review.
- Check canonical paths with standard-library file opens: does not adequately
  protect against filesystem path replacement between validation and open.

## Consequences

The CLI gains useful file-grounded edit proposals without claiming patches were
applied. Context exhaustion is explicit; session persistence, streaming and
autonomous tool execution remain future work. OS file I/O and stdin are not
forcibly cancellable; a bounded blocking pool, session termination and bounded
runtime shutdown prevent unlimited waiting. Limits include idle input time.

## Migration impact

No public core APIs, configuration format or existing plan/run commands change.
The CLI adds a filesystem dependency, a subcommand and an explicit remote-sharing
flag. See [agent mode](../agent-mode.md) for commands and supported boundaries.

The interactive prompt uses Crossterm event streaming for immediate slash-command
recommendations and bounded Unicode-aware line editing. A scoped raw-mode guard
restores the terminal when its future is dropped. Redirected input retains the
plain line protocol; no terminal history is persisted.
