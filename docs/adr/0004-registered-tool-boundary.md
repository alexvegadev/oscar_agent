# ADR 0004: Host-owned tool registry and bounded sessions

## Status

Accepted; experimental Rust API, not yet integrated with proposal workers.

## Context

The hybrid runtime has provider and validator traits but no common tool permission
boundary. Repository tools, agent tool calls, and later MCP integration require
that boundary before implementing actions. This development dependency takes
priority over the remaining release-oriented dependency/API audit; release still
requires that audit. The proposed Laya/JEV classification feature remains planned.

## Decision

Add a focused `tools` module to oscar-core using existing Tokio/Serde dependencies.
Hosts register immutable definitions and implementations; models submit only call
envelopes. Default policy denies all tools. Side-effecting calls require explicit
host opt-in and per-call approval bound to validated input.

Start with a finite flat-object schema: bounded text, signed integer ranges,
booleans and text choices. Reject unsupported structures and unknown fields rather
than claim full JSON Schema support. Reject duplicate JSON keys before conversion.
Expose only validated input to tools; retain fixed redacted diagnostics.

Use a serial session per run with bounded call admissions, input/output, retained
IDs/events, and elapsed time. Approval shares the call deadline. Cancellation or
dropped call futures cancel child tokens. IDs stay consumed after failures; never
retry automatically. Implementations must be nonblocking/cancellation-safe and
bound their own allocations; the registry is not an OS sandbox.

## Alternatives

- Execute provider-returned tool names directly: bypasses host authorization.
- Treat registered tools as implicitly authorized: conflates availability and policy.
- Add a full JSON Schema engine now: unnecessary for the first read-only tools;
  expand deliberately when an actual integration requires nested schemas.
- Implement shell/filesystem tools simultaneously: would combine distinct threat
  boundaries and make registry behavior harder to review.

## Consequences and migration

No existing APIs, config fields, CLI behavior, or model response handling changes.
New APIs are additive and experimental. No dependencies or lockfile changes are
required. The echo example demonstrates host registration without side effects.
Read-only workspace tools and the model-to-tool loop remain separate roadmap
items; future integrations must pass through this registry, not bypass it.
