# OSCAR documentation

This directory contains design records and future user/developer documentation.

- `adr/` holds concise architecture decision records.

Documentation must distinguish implemented behavior from the roadmap in the root README.

## Implemented hybrid orchestration

See [configuration and usage](hybrid.md), [implementation plan](hybrid-implementation-plan.md),
and [ADR 0002](adr/0002-hybrid-orchestration.md).

## Foundation

See [contributing](../CONTRIBUTING.md), [community conduct](../CODE_OF_CONDUCT.md),
[security reporting](../SECURITY.md), and [ADR 0003](adr/0003-foundation-and-msrv.md)
for CI, toolchain compatibility, and license metadata decisions.

## Tool boundary

See [registered tool sessions](tools.md) and [ADR 0004](adr/0004-registered-tool-boundary.md)
for the experimental library API, permissions, limits, and remaining integration work.

## Provider streaming

See [the provider contract and bounds](providers.md) and
[ADR 0009](adr/0009-native-provider-streaming.md) for native SSE, CLI/desktop
previews and complete-output compatibility.
