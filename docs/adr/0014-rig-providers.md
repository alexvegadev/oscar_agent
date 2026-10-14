# ADR 0014: Rig completion clients with an OSCAR-owned transport

## Context

The owner requested Rig-backed provider selection and easier integration with RAG,
then explicitly authorized raising the Rust 1.85 minimum. Rig 0.11.1 was previously
pinned for compatibility. Its streaming implementation does not preserve the tool
IDs, raw argument JSON, bounded parsing and transport controls OSCAR requires.

## Decision

Raise the workspace minimum and headless CI matrix to Rust 1.88. Pin rig-core
0.32.0 without default features for both completion and vector retrieval. This
supersedes the compiler/pin decisions in ADRs 0003 and 0007. Desktop validation
continues on stable because its platform dependencies have a separate minimum.

Keep OSCAR-owned public types, routing and registry execution. Convert requests
to Rig's typed completion contract, and use its OpenAI Completions client with an
injected bounded transport. Presets select compatible endpoints; arbitrary vendor
names with `protocol = "openai_chat"` and an endpoint require no source edits.
Other protocols fail closed until their adapters are implemented and verified.

The transport preserves OSCAR's loopback/HTTPS rules, no redirects or proxies,
request deadlines, output/frame/wire limits and raw tool-call arguments. It
validates SSE and forwards only canonical bounded text events to Rig. Authoritative
calls and usage remain in per-request state and return through ModelOutput. Raw
JSON is never reconstructed from Rig's parsed tool arguments. Empty credentials
omit authorization; credential lookup remains explicit and mode-scoped.

One network attempt is allowed per invocation. Transport failures become typed
OSCAR errors through request-local state; the SDK sees an ended stream and cannot
reconnect. The host owns retries and budgets. Suppress Rig's tracing subscriber
while polling inference to prevent its SDK message spans from logging user data.
Use Rig's own wire conversion for input-byte accounting.

## Alternatives

- Use SDK default transports directly: weakens locality, output bounds, retry
  ownership and privacy controls.
- Replace OSCAR's orchestration with Rig agents: bypasses the established registry
  and approval boundary without a much broader migration.
- Keep 0.11.1: retains the compatibility constraint and inadequate streaming hooks.
- Enable every native Rig provider immediately: each protocol needs bounded raw
  validation and conformance tests; a list of compiled SDK modules is not support.

## Consequences

Rig owns completion request conversion and the client interface; reqwest remains
an implementation detail of OSCAR's transport and the existing MCP/embedding
adapters. RAG shares the new dependency without changing retention or locality.
Default builds still avoid optional inference dependencies and use mock providers.
Real vendor conformance remains unverified; loopback tests cover the boundary.

## Migration impact

Rust 1.85 is no longer supported. Existing `openai_compatible` configurations retain
the complete api_url convention. Endpoints must end in `/chat/completions`. New
presets can omit api_url; custom names explicitly select openai_chat. The optional
protocol field extends TOML without changing model, tool, plan or run formats.
Existing injected ModelProvider implementations remain source compatible.
