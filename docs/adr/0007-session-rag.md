# ADR 0007: Opt-in session retrieval behind OSCAR's context boundary

## Context

Interactive agent mode appends loaded files to every completion request. This
fills the bounded input window quickly. The owner requested RAG and improved
context handling with [Rig](https://rig.rs/). OSCAR must retain its tool policies,
locality rules, provider routing, cancellation and Rust 1.85 minimum.

## Decision

Add an optional `rag` Cargo feature, disabled by default. Rig provides in-memory
vector search; OSCAR owns the embedding provider seam, document admission,
chunking, source coordinates, resource bounds and completion context. Only
explicit `/read` results are indexed. Completion providers and tool registration
remain unchanged. Library types do not expose Rig types.

The CLI uses a bounded loopback-only Ollama embedding adapter, rather than Rig's
provider clients or agent orchestration. This preserves OSCAR's endpoint, response
size and permission controls. Local embeddings require local or mixed work mode;
full_remote fails closed. Remote completion remains separately authorized.

Pin rig-core 0.11.1 without default features and commit its dependency resolution.
This release compiles on Rust 1.85 with the current workspace. Rig's current main
workspace declares Rust 1.95; adopting it would change OSCAR's compatibility
policy. Pinning an older release is an explicit compatibility tradeoff, not a
claim that the latest Rig API is supported. Dependency/license/security review
remains a release gate. No Rig agent/tool execution APIs are enabled by OSCAR.

## Alternatives

- Replace OSCAR orchestration with Rig agents: much broader migration, and would
  require re-establishing policy and execution invariants.
- Add a persistent vector database now: premature retention and schema choices.
- Continue sending all files: simple, but wastes the limited context window.
- Raise MSRV for current Rig: changes the existing supported toolchain contract.

## Consequences

RAG sessions can retain more file evidence than fits into a completion request,
while only selected cited chunks are supplied. Retrieval is approximate; byte
chunks can split definitions and fail to capture surrounding dependencies.
Conversation history is still bounded, without automatic eviction. All content
is discarded on reset/exit, with budgets retained across reset. A local embedding
model is an explicit prerequisite; mocks and HTTP fixtures keep tests offline.

## Migration impact

No existing CLI, settings, provider or serialized format changes. Opt in with
`--features rag` and `agent --rag`. Hosts may inject an OSCAR embedding provider;
they must preserve its locality contract. Persistent stores, semantic summaries
and worker/planner retrieval require separate design and acceptance work.
