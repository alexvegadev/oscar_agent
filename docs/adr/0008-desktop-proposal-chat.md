# ADR 0008: Feature-gated GPUI desktop proposal chat

## Context

The desktop implementation plan calls for a chat UI, token streaming, an agent
tool loop and approvals. Core currently provides bounded complete-response
inference and a separate tool registry. A desktop chat can use those existing
boundaries while the structured streaming/tool protocol remains future work.

## Decision

Implement an experimental in-memory proposal chat in `oscar-code`, using GPUI Kit
0.7.3 and the matching `gpui-pre` 0.3.9 crate under the `gpui` dependency name.
Gate native dependencies and the binary behind `desktop`. Keep session logic and
mock tests independent of the UI. Use current stable for desktop verification
and maintain Rust 1.85 for the headless library and existing runtime/CLI.

Keep models inference-only. Do not interpret response text as executable tools.
Expose cancellation, run IDs, typed outcomes and bounded context/output. Preserve
budgets across Reset. Enforce strict work modes, local-only data, capabilities,
remote budgets and configured cost bounds. Do not initialize forbidden adapters.

Persist only non-secret desktop preferences on an explicit Save action. Provider
models and API key environment references stay in core configuration. Disable
untrusted Markdown image loads and link actions. Use a separate Tokio runtime for
inference, bounded blocking work for settings writes, and bounded shutdown.

## Alternatives

- Add another incompatible GPUI release: creates duplicate engine types and
  needlessly expands the dependency graph.
- Make GPUI mandatory in the runtime: native dependencies would affect default
  tests and headless MSRV support.
- Emulate streaming by splitting complete responses: does not provide actual
  streaming and would obscure provider behavior.
- Execute model-generated JSON commands immediately: bypasses the missing
  reviewed protocol and tool/approval integration.

## Consequences

The UI can complete offline and real-provider proposal conversations. True token
streaming, autonomous tool calls, approval dialogs and tool lifecycle events
remain unimplemented. Desktop builds require recent stable/native dependencies;
they have a separate Windows CI job. OS preference writes cannot be cancelled
forcibly; the UI timeout and runtime shutdown bound waiting, not underlying I/O.

## Migration impact

Core APIs and provider configuration do not change. Add optional desktop and
UI-test features, a CLI binary, non-secret preference TOML and documentation.
Default workspace tests remain headless. Linux/Windows MSRV jobs test core/CLI
all features and the desktop library's HTTP feature separately; the stable
Windows desktop job checks all `oscar-code` features.

## Subsequent streaming decision

[ADR 0009](0009-native-provider-streaming.md) supersedes the streaming deferral
above. Native SSE and bounded CLI/desktop previews are now implemented.
