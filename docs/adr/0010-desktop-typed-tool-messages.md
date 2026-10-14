# ADR 0010: Typed desktop tool messages and bounded structured rendering

## Context

OSCAR Code's history previously stored only a role and a string. The desktop
implementation plan requires typed tool calls/results and nested output before
the autonomous runtime and approval dialogs. Core tools still return strings,
and their argument schema remains deliberately flat.

## Decision

Use an experimental desktop `Message` enum for user, model, system, tool-call
and tool-result entries. Call/result types carry identifiers, tool names and
immutable `StructuredData`; results distinguish success from fixed core
`ToolError` categories. Parse JSON through a bounded constructor, with a maximum
depth of 16 below the root and 512 values. No unchecked deserializer or mutable
payload accessor is exposed. Render structures as literal, accessible object/array
trees with scalar types preserved and no link, image or command actions.

Make history read-only to consumers. Host insertion methods validate active run
IDs, unique call IDs and exactly one result with a matching ID/name in the current
run. Account for identifiers and compact payloads in input/output and aggregate
retention limits, including subsequent model completions. Reset discards these
entries while preserving existing inference budgets and run IDs.

Keep these entries display-only. They are not included in proposal context, and
model text is never parsed into calls. Execution still requires the registry's
schema validation, host policy and approval path. Core providers/tools, their
schemas and the broader runtime protocol remain unchanged.

## Alternatives

- Keep role/string messages and format JSON as prose: loses type identity,
  result correlation and distinct success/error behavior.
- Admit unrestricted JSON values or render tool output as Markdown: allows
  uncontrolled tree complexity or unintended content actions.
- Introduce a model/tool runtime at the same time: combines a presentation step
  with separate execution, permission and approval decisions.

## Consequences

Hosts can supply nested tool display entries without granting execution rights.
Payload and identifier Debug output is redacted; explicitly displayed data still
requires host secret filtering. The desktop has no automatic producer of tool
messages until runtime integration is implemented. The core flat schema is not
expanded by the desktop's more general presentation data.

## Migration impact

The experimental `oscar-code` message struct becomes an enum. Replace field access
with `message.role()` / `message.text()` or variant matching, and replace direct
history mutation/access with `session.messages()` and typed insertion methods.
`Message` and `Role` remain re-exported from `session`. No persisted transcript or
external wire protocol is introduced; no core/CLI API changes are required.
