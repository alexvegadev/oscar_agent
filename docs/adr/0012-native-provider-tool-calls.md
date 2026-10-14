# ADR 0012: Native provider tool calls and correlated results

## Context

ADR 0011 introduced a strict JSON text bridge for desktop autonomy. HTTP adapters
still rejected native function calls, preventing use of provider tool selection
and streamed argument deltas. The registry already owns execution, schema
validation, authorization, deadlines and cancellation.

## Decision

Extend provider-neutral `ModelRequest` with typed messages and host-selected
registry definitions, and `ModelOutput` with proposed calls carrying raw argument
JSON strings. Keep native wire formatting inside the provider adapter. Translate
the bounded flat registry schema into Chat Completions function parameters;
request tool selection only when the host supplied definitions. Assemble SSE
call fragments by index, validate completion and return calls without executing.

Desktop autonomous runs send assistant `tool_calls` and tool messages correlated
by `tool_call_id`. Buffer full replies before admitting calls, execute batches
serially through the existing registry, and terminate on tool failures without
retry. Bound batches to 32 and the remaining run call budget. Count text and call
payloads together for output limits and serialized schemas/messages for model
input, context and remote budgets. Preserve raw argument JSON so duplicate fields
cannot evade registry validation. Cancelled exchanges without results stay
visible in desktop history but are omitted from the next native conversation.

`native_tool_protocol` distinguishes ordinary Markdown answers from the legacy
injected-provider final envelope. Typed calls are never extracted from ordinary
native answer text. Proposal-only CLI, planner, workers and desktop sessions
continue to disable tools and independently reject typed calls from injected
adapters. No new execution permissions are introduced.

## Alternatives

- Keep only the text envelope: does not integrate provider-native tool APIs.
- Execute calls in adapters: bypasses the registry and host approval boundary.
- Normalize argument JSON before admission: hides duplicate argument keys.
- Execute batches concurrently: adds unnecessary scheduling complexity and makes
  observable tool ordering harder to maintain.

## Consequences

OpenAI-compatible Chat Completions providers can use native tools in desktop
read-only autonomous runs. Offline native mock and loopback HTTP tests cover
call/result continuations. Actual model capability, live-provider compatibility,
CLI autonomy, side-effect approval UI and repository tools remain separate.
The schema's `maxLength` is a character bound; the registry remains authoritative
for the stricter UTF-8 byte bound. Streams require `tool_calls` termination,
valid function calls and `[DONE]`; legacy `function_call` streams fail closed.

## Migration impact

Rust callers constructing `ModelRequest` must supply `messages` and `tools`
(empty vectors preserve proposal behavior). `ModelOutput` constructors supply
`tool_calls` (empty for text). Existing injected providers keep the JSON envelope
by default; native providers override `native_tool_protocol` and return plain
answers. `MockProvider::scripted_native` supplies deterministic native replies.
No new dependencies, credentials, persisted formats or preferences are added.
