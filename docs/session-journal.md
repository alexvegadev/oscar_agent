# Session journal and required proposal gates

This is the first implementation slice of the short-term hardening cycle.
Journaling is opt-in; existing commands persist no additional session log.

## CLI

Plan/run commands write a new journal in their exclusive output directory:

```sh
cargo run -p oscar-cli -- run "Document the parser" --out parser-run --journal metadata
cargo run -p oscar-cli -- run "Document the parser" --out parser-trace --journal content
```

`content` explicitly retains prompts, injected evidence, system instructions in
built-in HTTP wire requests, tool schemas and replies. Choose a private output
directory. Metadata stores byte counts/lifecycle events, but no payloads or payload
hashes. Authentication headers and provider configuration are excluded. Secrets
included by the user in prompts are preserved when content capture is enabled.

Interactive sessions accept a new file under an existing trusted private parent:

```sh
cargo run -p oscar-cli -- agent --workspace . --session-log PRIVATE_DIR/session.jsonl
cargo run -p oscar-cli -- agent --workspace . --session-log PRIVATE_DIR/exact.jsonl --capture-session-content
```

The file must not exist. `/read` calls/results and `/reset` are recorded. Reset
clears active context and preserves budgets as before; it does not erase the log.
Delete a closed log explicitly to remove its contents. No hidden retention service,
database, automatic replay or resume operation is added. All work-mode/egress checks
remain in place; logging neither exports data nor authorizes remote inference.

## Library hosts

Create `session::Journal` with `CaptureMode` and `JournalLimits`. Wrap providers
once with `Providers::with_journal` before planning/execution. Pass the same journal
in `execution::ExecutionOptions` for DAG lifecycle/validation events. Other hosts
can use the provider wrapper independently. Await `Journal::close` after all
producers finish and before shutting down the runtime.

`session::verify` reads without modifying a file. It checks sequence/hash links and
reports the last verified hash, writer closure and incomplete tail. Store the hash
independently for a trusted comparison checkpoint. Never deserialize a journal as
authorization, validation receipts or executable tool input.

Request sequences correlate wire and response records. DAG calls include
run/task/attempt identities; other calls have no DAG origin. `Opened` supplies a
host-generated SessionId. Completed responses preserve raw tool argument strings.
The exact wire body is observable only for OSCAR's built-in HTTP adapter.

## Bounds and failures

| Limit | Default | Maximum |
| --- | ---: | ---: |
| Encoded record | 256 KiB | 4 MiB |
| Journal bytes | 64 MiB | 1 GiB |
| Queued events | 8 | 64 |
| Individual I/O wait | 5 seconds | 60 seconds |

All limits are positive; record minimum is 1 KiB. Serialization accounts for JSON
escaping and reserves 512 bytes for framing. Verification uses bounded lines and
one total I/O deadline. There is one writer per explicitly created journal; hosts
must bound concurrent sessions. The CLI creates one per invocation.

Requests are acknowledged durably before dispatch. Provisional text uses a bounded
nonblocking queue; overflow aborts inference. A later acknowledged response/close
establishes durability of earlier queued text. Write errors, timeout and quota
exhaustion poison the writer. Existing call/run deadlines still apply; final journal
shutdown can additionally take the configured I/O timeout. OS I/O may outlive
cancellation; incomplete tails remain available for inspection.

## Mandatory host gates

Construct `ValidationPolicy::require_all`, register implementations in
`execution::Validators`, and pass the policy in `ExecutionOptions`. Unknown IDs
fail before inference. At most 32 unique IDs of 64 ASCII identifier characters
are allowed. The policy has no Deserialize implementation.

Every admitted artifact must pass its declared validation and all host requirements.
Duplicate checks run once. Confidence, task kind, local repair and remote escalation
cannot remove gates. The whole validation sequence shares the existing call timeout
and run cancellation. Rejection can trigger bounded repair; infrastructure errors,
limits and cancellation stop immediately. Raw validator diagnostics are redacted.

This interface validates proposal strings. It does not execute commands, contain
processes, validate materialized workspaces or authorize applying edits. The CLI
does not install command validators implicitly.

## Remaining cycle work

- Desktop journal wiring and approval/diff UI.
- Per-validator receipts bound to candidate snapshots and host policy versions.
- Isolated approved cargo/test runners with process-tree cleanup.
- Prepare/validate/approve/apply transactions and crash recovery.
- Persistent memory storage, retrieval, retention, reset/resume and deletion.
- Compaction provenance and shared cumulative token-budget services.

See [ADR 0016](adr/0016-session-journal-and-required-gates.md) for architectural and
traceability boundaries.
