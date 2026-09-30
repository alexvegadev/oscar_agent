# Contributing to OSCAR

OSCAR is experimental. Start with a focused issue or a small pull request, and
distinguish implemented behavior from roadmap proposals. Read [AGENTS.md](AGENTS.md)
for the runtime invariants and [CODE_OF_CONDUCT.md](CODE_OF_CONDUCT.md) for community
expectations. Report vulnerabilities using [SECURITY.md](SECURITY.md).

## Development setup

Install Rust with rustup. The workspace uses edition 2024 and supports Rust 1.85.0
or newer. Use current stable for formatting and linting; use 1.85.0 to check the
minimum supported Rust version (MSRV). Keep the workspace edition and MSRV shared
between crates. An MSRV increase needs an explicit compatibility decision and an
update to the CI matrix and documentation.

```bash
rustup toolchain install stable --profile minimal --component rustfmt --component clippy
rustup toolchain install 1.85.0 --profile minimal
cargo +stable fmt --all --check
cargo +stable clippy --workspace --all-targets --all-features --locked -- -D warnings
cargo +stable test --workspace --locked
cargo +stable test --workspace --all-features --locked
cargo +1.85.0 test --workspace --locked
cargo +1.85.0 test --workspace --all-features --locked
```

The CI workflow runs tests on Linux and Windows with both toolchains. Default
tests use mocks; HTTP-feature tests use loopback fixtures. Never add live model
credentials, paid inference, or model downloads to the default suite.

## Making a change

1. Inspect relevant code, tests, manifests, and decision records before editing.
2. Keep changes focused and preserve the registered capability/permission boundary.
3. Include success, failure, authorization, cancellation, and limit tests as relevant.
4. Update configuration examples and user documentation for changed behavior.
5. Add an ADR for changes to public APIs, persistence, permissions, crate boundaries,
   or compatibility. Keep roadmap checkboxes tied to verified acceptance criteria.
6. Run the checks above and describe the result, including anything not verified.

Commit Cargo.lock for reproducible CLI builds. Dependency updates must pass the
MSRV jobs as well as stable jobs; do not raise the MSRV to hide a dependency
resolution failure. Review added dependency features, license metadata, compile
cost, and security surface. The full release dependency audit remains a roadmap
item; a passing build is not a license or security audit.

## Pull requests

Explain the concrete problem, resulting behavior, tests, and remaining limitations.
Include a before/after example when useful. Do not include secrets or private
repository content in logs, fixtures, screenshots, or generated run artifacts.
Do not commit local output directories such as auth-plan or docs-run.

There is no stable public Rust API promise yet. Breaking changes still need an
explicit migration note. Do not introduce arbitrary model-directed command
execution, filesystem writes, or tool registration.

## License

The repository contains the Apache License, Version 2.0 in [LICENSE](LICENSE), and
Cargo metadata uses Apache-2.0. Preserve license notices and identify any third-party
material added in a contribution. Do not submit material you are not authorized to
contribute. There is no additional contributor agreement in this repository.
