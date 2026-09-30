# Registered tool boundary

The experimental `oscar_core::tools` module provides a host-owned registry and
bounded execution sessions. It is available with default features and adds no
dependencies. It does not yet add filesystem tools, a CLI tool command, or a
model-to-tool loop; the existing HTTP inference adapter still rejects tool calls.

## Host workflow

1. Implement `Tool` using the existing boxed async pattern. Register it with a
   `ToolDefinition`: unique name, truthful description, input schema, and effect.
2. Construct `ToolPolicy` in host code. Default policy permits nothing. Explicitly
   list permitted names; registration alone is not authorization.
3. Open one `ToolSession` per host run, passing limits and a cancellation token.
4. Submit a JSON envelope containing exactly `id`, `name`, and `arguments`.
5. Inspect the result and redacted lifecycle events. All output remains untrusted.

```json
{"id":"call-1","name":"echo","arguments":{"text":"hello"}}
```

The runnable example registers an in-memory echo tool, grants it permission, and
executes one call without credentials or filesystem access:

```bash
cargo run -p oscar-core --example tool_registry
```

See [the example source](../crates/oscar-core/examples/tool_registry.rs).

## Schema and admission

`InputSchema` is a bounded, OSCAR-owned flat-object schema, not a complete JSON
Schema implementation. Fields may be required or optional and support:

- UTF-8 text with a byte limit;
- signed 64-bit integers with inclusive minimum/maximum bounds;
- booleans;
- text choices from an explicit finite set.

Unknown fields, nested values, arrays, nulls, malformed JSON, duplicate argument
keys, duplicate envelope fields, and type/bound violations fail closed. There is
no coercion, remote schema reference resolution, or regex execution. Schemas have
at most 32 fields; choices have at most 32 unique values of up to 256 bytes each.
The registry validates schemas before registration. A tool receives only
`ValidatedInput`, which callers cannot directly construct or deserialize.

Names/call IDs contain 1–64 ASCII letters, digits, dots, underscores or hyphens.
Discovery is sorted by registered name. At most 128 tools can be registered.
Duplicate registration never replaces an existing tool. A live session borrows
the registry, preventing mutation of its definitions while it runs.

## Permissions and approvals

The host owns policy and tool effect metadata. Neither is deserialized from model
output. Unregistered names, unpermitted tools, and forged `approved` fields fail.
Unknown names in the host allowlist are configuration errors rather than silently
ignored typos.

For a `SideEffect` tool, both `allow_side_effects = true` and an `Approval` handler
are required. The handler receives the exact call ID, registered definition, and
validated arguments. It must obtain the host's configured approval for that call;
model confidence or text is not approval. Approval is bounded by the same deadline
as execution. No approval result is cached across calls.

Trusted implementations must accurately declare effects. This registry is not a
sandbox for malicious Rust code. Future filesystem and shell tools still need
their own workspace confinement and threat review.

## Limits and cancellation

| Limit | Default | Supported range |
| --- | ---: | --- |
| Calls admitted per session | 64 | 1–1024 |
| JSON envelope bytes | 16384 | 1–1048576 |
| Output UTF-8 bytes | 16384 | 1–1048576 |
| Call timeout (including approval) | 5000 ms | 1–session timeout |
| Session lifetime from creation | 60000 ms | 1–3600000 ms |

Calls execute serially per session, and every attempted admission consumes a slot,
including malformed or denied requests. A valid call ID is consumed before name,
permission and schema checks and remains consumed after failure or cancellation.
There are no automatic retries. The host must make an explicit new request with a
new ID if retry is appropriate; it must not blindly replay possible side effects.

Cancellation drops the pending approval/tool future and cancels the child token
provided in `ToolContext`. Dropping the host's call future also cancels the child
token and leaves a terminal cancelled event. Session destruction drops retained
IDs and events; no persistence is automatic.

Tools and approval handlers must be nonblocking and cancellation-safe. No detached
work is allowed. Implementations must bound allocation while constructing output;
the registry also checks returned output before releasing it. It cannot preempt
blocking code or undo effects already performed, including remote server work.

## Errors, events, and tests

`ToolError` distinguishes invalid definitions/calls, unknown tools, schema failures,
policy/approval denials, limits, timeouts, cancellation and execution failures.
Diagnostics contain fixed messages. Events retain only session sequence numbers
and typed outcomes; arguments, outputs and model-supplied IDs are not logged.
Input Debug output is redacted. Hosts may deliberately inspect arguments for an
approval UI, but must handle that content as sensitive and untrusted.

Tests cover registration, stable discovery, schema boundaries, forged approval,
duplicate JSON keys/IDs, default denial, approval combinations, input/output/call
limits, cancellation, dropped futures, session/call/approval deadlines, and tool
failures without retry. Existing runtime tests continue to cover proposal execution.

Next implement confined read-only repository tools, then integrate explicit tool
messages into the model loop through this same policy path.
