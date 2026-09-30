# ADR 0001: Keep the initial workspace small

## Status

Accepted

## Context

OSCAR is at the planning and foundation stage. Its README identifies future boundaries for core, models, tools, runtime, memory, MCP, and CLI, but the current implementation is a single command-line prototype.

## Decision

Use a Cargo workspace immediately, with a single real member: `crates/oscar-cli`. Keep the root manifest for workspace-wide metadata and lints. Place design records in `docs/adr/` and reserve `examples/` for executable examples.

Do not create empty crates for planned components. Add a crate only when it owns implemented code, tests, or a dependency boundary that justifies independent compilation.

## Alternatives considered

- Keep the prototype at the repository root until more crates exist.
- Create every planned crate now as an empty scaffold.

## Consequences

The CLI has a stable, conventional location and future crates can be added under `crates/` without another layout migration. The workspace communicates the intended direction without claiming that planned subsystems are implemented.

## Migration impact

The prototype binary name remains `oscar`; its source moves from `src/main.rs` to `crates/oscar-cli/src/main.rs`. Commands should target the package as `cargo run -p oscar-cli --`.
