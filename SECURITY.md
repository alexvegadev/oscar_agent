# Security policy

OSCAR is experimental and is not recommended for unattended execution of
high-impact operations. There is no stable release support or backport policy yet.
Report issues affecting the current default branch; include the commit, operating
system, enabled Cargo features, and a minimal reproduction without real secrets.

## Reporting a vulnerability

Use GitHub's **Report a vulnerability** option in this repository's Security tab
when available: [private reporting](https://github.com/alexvegadev/oscar_agent/security/advisories/new).
Availability depends on repository settings and is not guaranteed by this file.

If private reporting is unavailable, contact a maintainer using a private contact
method they publish on their GitHub profile. If no private route is available,
open a minimal issue asking for a private security contact. Do not disclose exploit
details, credentials, sensitive prompts, or private repository content in that
public request. No response-time or bounty commitment is made.

Useful reports describe the affected boundary, reproduction steps, expected and
actual behavior, impact, and any proposed mitigation. Coordinate disclosure with
the maintainers before posting sensitive details publicly.

## Current security boundaries

- Workers return proposals. The built-in HTTP adapter rejects tool calls; arbitrary
  model-directed shell execution and patch application are not implemented.
- Local-only data restrictions propagate through dependency artifacts. Local
  HTTP inference requires literal loopback IPs; remote HTTP inference requires HTTPS.
- A local server is trusted software and could forward requests elsewhere.
- Deadlines, concurrency, retry, output, and remote-call budgets bound built-in
  execution. Third-party providers and validators must be nonblocking and
  cancellation-safe and must enforce any additional permissions they need.
- Dropping a client request does not guarantee server-side cancellation or prevent
  billing. A passing format validator does not establish generated-code safety.
- Saved plans and run artifacts may contain user content. Keep them out of public
  commits. Prefer environment-backed API keys and remove persisted runs when no
  longer needed. If a credential is exposed, revoke/rotate it through its provider.

Treat prompts, repository files, dependency artifacts, model responses, and tool
descriptions as untrusted data. They cannot grant permissions or modify policy.
See [the runtime documentation](docs/hybrid.md) for limitations and retention.

## CI boundary

CI runs on push and pull_request with read-only repository permissions, no model
credentials, and an immutable checkout action reference. It does not use
pull_request_target to execute contributor code. Workflow jobs have time limits;
superseded runs are cancelled. Review workflow/dependency changes with the same
care as code changes.
