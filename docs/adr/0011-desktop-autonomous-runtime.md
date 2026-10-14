# ADR 0011: Opt-in desktop registry loop and ordered live events

The text-only provider bridge below is superseded by [ADR 0012](0012-native-provider-tool-calls.md). Runtime policy, limits and event decisions remain in force.

## Context

Core inference providers produce bounded text and reject provider-native tool
responses. The registry independently validates names, flat argument schemas,
permissions, approval, cancellation and tool limits. OSCAR Code has typed
call/result presentation but needs the model → registry → model runtime and
an event subscription before adding high-impact approval dialogs.

## Decision

Add an opt-in headless runtime in `oscar-code` and subscribe to it from GPUI.
Use strict JSON final/call envelopes over the existing inference adapter as an
experimental bridge. Keep raw call arguments intact until registry admission so
duplicate fields cannot disappear during presentation normalization. Advertise
only host-allowed read-only tools. Never derive policy, schemas or tool
registration from model replies, retrieved evidence or tool results.

Execute one call at a time through a run-scoped registry session and return
successful output as explicitly labelled untrusted evidence. Preserve typed tool
history across autonomous follow-ups. Stop on every protocol, provider or tool
failure; add no retries or provider fallback. Refuse side-effect opt-in in this
runtime, including host attempts, until the separate approval UI step. Register
only a pure text-statistics starter tool in the desktop; no new repository or
command permissions are implied. Mock desktop runs use scripted inference
responses through the same boundary for an offline end-to-end demonstration.

Share model admission counters across proposal/autonomous runs. Every continuation
rechecks work mode, local-only data, capabilities, context, remote cost and byte
budgets. Stop/Reset never reset counters. Bound turns, tool calls, input/output,
tree complexity and retained history. Reserve result capacity before execution.
Use one wall-clock run deadline covering inference, tools and event backpressure,
with tighter per-call deadlines. Cancellation and lost subscribers drop current
futures and propagate cancellation to tool tokens.

Send ordered typed events on an eight-slot bounded channel. GPUI drains queued
events before applying the authoritative report. A best-effort terminal event
never blocks shutdown; the report is the fallback terminal authority. Guard UI
insertion by run ID, active state and increasing event sequence. Retain only
bounded redacted event metadata; display typed tool data through bounded history.
Buffer legacy autonomous JSON protocol replies until validated. Native tool
providers now expose bounded, per-turn provisional text previews through a
nonblocking sink and one latest watch snapshot; typed calls still wait for complete
validation. Preserve ordered lifecycle/tool events independently, retaining native streaming
for proposal mode.

## Alternatives

- Parse arbitrary proposal text as executable calls: silently changes the user's
  permission scope and blurs the proposal/runtime boundary.
- Add provider-native function calling first: requires core HTTP/stream protocol
  changes and provider compatibility work beyond this desktop slice.
- Use an unbounded or lossy event channel: permits memory growth or loses tool
  lifecycle entries needed to correlate history.
- Enable repository/side-effect tools immediately: combines runtime integration
  with filesystem and exact-call approval decisions reserved for later steps.

## Consequences

The desktop completes real registry-backed mock flows without credentials and
can use configured inference providers that obey the JSON protocol and declare
`tool_use`. Native function calling, real-model autonomy evaluation, repository
exploration, patching, command execution and approval dialogs remain separate.
Custom host tools still must honor core's cancellation-safe, nonblocking contract.
Event Debug/report metadata redacts payloads; hosts must filter secrets before
explicit display or remote sharing.

## Migration impact

Add default-off `autonomous` and `max_tool_calls` desktop preferences. Existing
preference files still load; proposal chat and core/CLI contracts remain intact.
Turn budgets now have shared internal admission state so autonomous continuations
cannot bypass Reset limits. Add `begin_agent`, ordered event application and final
report APIs. ADR 0010's presentation boundary remains valid; its runtime deferral
is superseded by this decision. Serialized transcripts or a stable external wire
protocol are not introduced.
