# ADR 0003: Foundation policies and locked MSRV verification

## Status

Accepted. The workflow is implemented; its first hosted matrix run is pending.

## Context

The README's first unfinished goals cover contributor policies and CI/MSRV.
The repository already contains an Apache-2.0 LICENSE, but Cargo metadata and the
README still said UNLICENSED. Rust 1.85 was declared without a dedicated compiler
run. The runtime has deterministic mocks and loopback HTTP tests suitable for CI.

## Decision

Match workspace license metadata and documentation to the existing LICENSE;
leave the license text unchanged. Add contribution, conduct, and security-reporting
guidance without inventing a maintainer email address, support deadline, or bounty.
Document conditional private-reporting routes and a safe fallback request for a
private contact. Repository settings are not changed by these documents.

Use GitHub Actions on push, pull_request, and manual dispatch. Test default and
all-feature configurations on Windows and Linux, with stable and exactly 1.85.0.
Use stable rustfmt/Clippy for quality checks. Pin checkout to its verified v7 commit,
disable credential persistence, grant contents:read only, cancel superseded runs,
and bound job duration. Do not supply model credentials or execute untrusted PR
code through pull_request_target. No publishing/deployment step is introduced.

Keep Cargo.lock committed and use --locked in CI. An actual Rust 1.85.0 run found
that yoke-derive 0.8.3 used str::from_utf8, unavailable on that compiler. Select
0.8.2 in the lockfile; it satisfies yoke's dependency range and the all-feature
suite passes with Rust 1.85.0. Its dependency graph adds syn 2/synstructure 0.13
alongside the versions used by other macros. Do not patch dependency source in
the Cargo cache or raise the declared MSRV to hide this failure.

MSRV verification currently covers the locked workspace. Dependency updates must
rerun it; a future library release also needs fresh-resolution compatibility and
dependency license/security review. Public Rust APIs remain experimental with
explicit migration notes required for breaking changes.

## Alternatives

- Rely on dependency rust-version metadata alone: the dedicated build disproved
  this assumption for the existing lockfile.
- Raise the MSRV: unnecessary for the project code and breaks the declared target.
- Add a permanent unused direct dependency pin: unnecessary for the current locked
  workspace; reconsider constraints when publishing library crates.
- Check only one feature set/platform: misses optional HTTP and platform behavior.
- Change LICENSE: unnecessary; an explicit license already exists in the repository.

## Consequences and migration

No runtime API or work-mode semantics change. Cargo metadata now reports Apache-2.0.
Dependency builds use the compatible locked macro version. Contributors need
stable and 1.85.0 for the complete local check set; CI installs each explicitly
without changing a contributor's default toolchain.

The hosted workflow cannot be claimed green until it is committed/pushed and run.
Foundation checkboxes distinguish implementation/local verification from that
remaining gate. Later dependency audit, tool boundaries, and release gates remain
open. Original hybrid delivery records describe their historical verification.

References: [rustup toolchains](https://rust-lang.github.io/rustup/concepts/toolchains.html)
and [checkout action configuration](https://github.com/actions/checkout).
