# ADR 0009: Native provider text streaming

## Context

The provider trait exposed only complete text. CLI and desktop consumers could not
show early output even though OpenAI-compatible endpoints support native SSE.
Planning and artifact validation still need a complete, successful response.
This decision supersedes the streaming deferral in ADRs 0006 and 0008.

## Decision

Add an object-safe `infer_stream` method with an inline fallible `ModelTextSink`
and a final `ModelOutput` carrying content, confidence and usage. Retain `infer`
and provide a default whole-reply streaming implementation for existing providers.
Override streaming in the HTTP adapter and implement its complete API by collecting
that same native SSE path. Keep all inference-only policy boundaries in place.

Bound decoded text, individual SSE lines/events, total wire bytes and request time.
Reject incomplete streams, tool calls and malformed or unsupported protocol events.
Do not retry with non-streaming requests. Treat delivered text as provisional until
successful completion; final content alone is eligible for validation or history.

CLI and desktop use one latest bounded full-text watch snapshot to coalesce updates
without unbounded queues or blocking callbacks. CLI flushes sanitized text deltas;
desktop renders the preview separately from history and rejects stale run IDs.
Stop, Reset and failure discard previews without resetting consumed budgets.

## Alternatives

- Split complete replies into timed chunks: cannot improve first-token latency.
- Replace `infer` with a mandatory stream-only trait: breaks existing injected
  providers and adds migration work to consumers that need complete output.
- Buffer every token in an unbounded channel: slow rendering can retain unlimited
  events. A latest snapshot retains complete provisional text with bounded backlog.
- Silently fall back after streaming failure: can hide incompatibilities, duplicate
  billing and complicate cancellation and budget accounting.

## Consequences

All configured HTTP inference uses native SSE, including planning and artifact
execution. Interactive clients can show incremental Markdown/text. Deltas may
contain multiple tokens. Usage remains optional, and cancellation only guarantees
client-side termination. The mock/legacy-provider fallback emits one complete reply.
There are no new dependencies, credential flows, configuration fields or tool powers.

## Migration impact

Existing `ModelProvider::infer` implementations remain source compatible. Native
adapters can override `infer_stream`; sinks must be nonblocking, bounded and treat
text as provisional. HTTP endpoints must support the documented streaming protocol
and usage option. JSON-only completion endpoints now fail explicitly. Existing
plan/run output formats and complete-response validation remain unchanged.
See [provider contract and limits](../providers.md).
