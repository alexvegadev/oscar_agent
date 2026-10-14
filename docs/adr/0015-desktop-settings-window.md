# ADR-0015: Themed desktop shell and transactional preferences

## Context

Desktop preferences were interleaved with chat controls and changed runtime
behavior before persistence. Fixed colors also diverged from GPUI Kit controls.

## Decision

Use GPUI Kit theme tokens across the conversation shell and a separate native
Settings window, with Kit inputs, switches and buttons. Keep one Settings window
per chat and observe the chat for status and policy locks. Keep edits in a draft;
validate host policy and all desktop limits, then use the existing bounded writer.
Apply preferences and the global theme only after a successful write. Closing
and reopening restores the applied settings. Active runs lock behavior controls.

## Alternatives

An inline panel consumes conversation space. A global mutable preference store
would widen scope beyond the existing single-session design. Immediate application
would leave behavior changed after a failed save.

## Consequences

Settings remain non-secret and obey host work modes, local-only restrictions,
registry permissions and shared budgets. Models and credential environment
references stay in provider configuration. Writes retain the existing five-second
waiting limit and uncancellable-write limitation. Native platform visual review
remains necessary; headless GPUI tests verify interaction and rendering boundaries.

## Migration impact

No preference schema or public session API changes. Existing oscar-code.toml files
load unchanged. Users now choose Apply & save before edits affect future runs.
New conversation retains the previous Reset budget semantics.
