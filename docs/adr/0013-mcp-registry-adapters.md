# ADR 0013: Explicit MCP registration through the tool boundary

## Context

Native provider calls now pass through a bounded registry. Users need tools from
local and external MCP servers without remote descriptions or annotations changing
host policy. The current registry accepts a bounded flat-object input schema.

## Decision

Add an optional `mcp` feature and host TOML settings for stdio and Streamable HTTP.
Default off; require per-server enablement, per-tool aliases and exact remote names,
host-owned effects, and a separate remote-egress opt-in. Use environment references
for secrets. Reject remote access in local/local-only mode before reading credentials.
Launch installed executables directly with a minimal environment; never download
packages or derive launch commands from model output.

Discover only within bounded pages, tools, bytes and deadlines. Translate supported
scalar schemas without ignoring validation constraints. Fail closed on unsupported
selected schemas. Register namespaced adapters through the existing registry so
authorization, exact-call approval, arguments, cancellation, limits and events remain
common to local and MCP tools. Ignore server permission hints and tool-list changes.

Use fresh short-lived sessions per invocation, rechecking the selected schema before
calling. No retries. Support protocol 2025-06-18 and JSON/SSE HTTP POST responses.
Terminate owned subprocesses on drop; delete HTTP sessions on ordinary completion.
Remote execution cannot be guaranteed cancelled after disconnect; document that
limitation and bound client waiting. Servers must enforce their own workspace and
execution limits. MCP does not sandbox third-party server code.

The desktop prepares registration before opening the UI and advertises only
read-only selected tools. Add `oscar mcp list` for offline inspection and `mcp check`
for discovery/schema validation without tool execution.

## Alternatives

- Trust server read-only hints: external metadata could bypass host policy.
- Expose all discovered tools: changes permissions without explicit selection.
- General JSON Schema support now: substantially expands the validation boundary.
- Persistent multiplexed sessions: introduces cancellation, stale-response and
  lifetime complexity before there is a demonstrated need.
- A general MCP SDK: useful for broader capability support, but this bounded tools
  subset reuses existing HTTP/Tokio dependencies and preserves the current MSRV.

## Consequences

Users can configure supported local and remote tools without provider changes.
More complex schemas, OAuth, old SSE transports, resources, sampling, dynamic
registration and CLI autonomous execution remain separate. Connection startup and
discovery repeat on each call. Third-party server behavior and sandboxing are the
host's responsibility. Rust fixtures and loopback tests cover the implemented
transport/policy boundary; live server compatibility is not implied.

## Migration impact

`Config` gains defaulted `mcp` settings; existing TOML remains compatible. The MCP
feature enables HTTP and Tokio process/I/O support, with no new direct dependencies.
Existing tools, provider contracts and permissions remain intact. Enabling the
feature alone does not connect to any server. No credentials or transcripts persist.
