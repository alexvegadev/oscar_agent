# MCP tools

OSCAR can register explicitly selected tools from local stdio servers and local
or remote Streamable HTTP servers. Enable the `mcp` Cargo feature and set
`features.mcp_enabled = true` in the provider configuration. Default settings do
not connect to servers or start processes.

```bash
cargo run -p oscar-cli -- mcp list --config settings.toml
cargo run -p oscar-cli --features mcp -- mcp check --config settings.toml
cargo +stable run -p oscar-code --features desktop,mcp -- --config settings.toml
```

`list` prints server names, transports, enablement and selection counts without
connecting or reading credentials. `check` connects enabled servers, initializes
MCP, discovers tools and validates registration; it never calls a tool. It is
cancellable with Ctrl+C. For model-initiated use, enable autonomous tools in the
desktop Settings. CLI proposal sessions and artifact workers remain inference-only.

## Register a local process

Use an installed executable directly, with separate arguments. OSCAR does not
invoke a shell or install/download servers. On Windows use an executable such as
`node.exe` with the installed server's JavaScript entry point, rather than a
`.cmd` launcher. Paths below are examples to replace with your installation.

```toml
[features]
mcp_enabled = true
skills_enabled = false

[mcp]
timeout_ms = 10000
max_response_bytes = 65536
max_discovered_tools = 128
max_pages = 8

[mcp.servers.files]
enabled = true
transport = "stdio"
command = 'C:\Program Files\nodejs\node.exe'
args = ['C:\mcp\filesystem\dist\index.js', 'C:\projects\my-project']
cwd = 'C:\projects\my-project'

[mcp.servers.files.tools.read]
name = "read_text_file"
effect = "read_only"

[mcp.servers.files.tools.list]
name = "list_directory"
effect = "read_only"
```

The provider sees `mcp_files_read` and `mcp_files_list`. Aliases and server names
use ASCII letters/digits/underscores; the remote tool name must match discovery
exactly. Every enabled server needs at least one explicit selection. OSCAR
discards unselected tools. Server permission hints are never authoritative:
review actual behavior before declaring a tool `read_only`.

Processes inherit only PATH, SystemRoot, WINDIR, TEMP and TMP. Map additional
environment variables by name; secret values stay in the host environment:

```toml
[mcp.servers.files.env]
SERVER_API_TOKEN = "MY_SERVER_API_TOKEN"
```

The left name is passed to the child; the right name is read from OSCAR's host
environment. Missing variables fail closed. Launching a configured local server
executes its installed code with the current user's permissions. MCP is not a
filesystem/process sandbox; set the server's workspace/root and own permission
controls. OSCAR does not enforce workspace roots inside third-party servers or
terminate their independently spawned descendants.

## Register an HTTP server

```toml
[mcp.servers.docs]
enabled = true
transport = "http"
url = "https://mcp.example.com/mcp"
allow_remote = true
bearer_token_env = "OSCAR_DOCS_MCP_TOKEN"

[mcp.servers.docs.tools.search]
name = "search_docs"
effect = "read_only"
```

Use the server's actual endpoint and tool names. Omit `bearer_token_env` for an
unauthenticated server. Credentials are never written to TOML, logs or diagnostics.
Remote access requires HTTPS and explicit `allow_remote = true`. Local work mode
and `planning.local_only_data` deny remote servers before credential reads or
network access. This opt-in covers discovery and sending selected tool arguments
to that server; it is independent of the model provider choice. For local HTTP,
use a literal loopback IP, for example `http://127.0.0.1:8000/mcp`. URL credentials,
query strings, fragments, redirects and automatic proxy use are rejected/disabled.

## Supported boundary and limits

- MCP protocol `2025-06-18`: initialize, initialized notification, paginated
  `tools/list`, and `tools/call`. HTTP supports JSON and POST response SSE with
  LF/CRLF framing, session IDs and protocol-version headers. Session DELETE is
  attempted when discovery or execution finishes, including ordinary failures.
- No OAuth flow, custom auth headers, old HTTP+SSE transport, background GET
  streams, sampling, elicitation, roots, prompts, resources, notifications that
  register tools, automatic retries or reconnection are implemented.
- Tool inputs use OSCAR's existing flat-object schema: strings, bounded integers,
  booleans and string enums, at most 32 fields. String length and integer range
  constraints translate to registry validation. Unsupported constraints, nested
  objects, arrays, nullable unions and `$ref` fail registration for selected
  tools; unselected schemas do not need conversion. Defaults are metadata and
  are not injected. Byte bounds may be stricter than JSON Schema character bounds.
- Up to 16 configured servers, 127 selected MCP tools (leaving a desktop starter
  slot), 128 discovered tools per server, eight pages by default (maximum 32),
  and 64 stdio messages or HTTP SSE events per response. Discovery rejects duplicate tool names and
  repeated cursors. Response bytes are bounded before parsing/allocation growth.
- A fresh session is initialized and the selected schema is rechecked for each
  execution. A changed schema fails the call rather than widening validation.
  This trades extra startup/discovery work for simple cancellation and no idle
  child processes or stale-response reuse.
- Discovery and each execution share a 10-second default MCP deadline (up to
  60 seconds). Registration also shares the configured run deadline. Registry
  call/output limits, approval hooks and run cancellation still apply.
- Local subprocesses use kill-on-drop. Remote HTTP waits stop on cancellation
  and timeout, but OSCAR cannot guarantee remote execution/session cancellation
  after disconnect; server work may continue. Timeout/drop can prevent session
  DELETE. Use server-side deadlines and session expiration. No retry is performed.
- MCP output is bounded JSON containing `content` and optional
  `structuredContent`; `isError` becomes a fixed failure category. Content is
  untrusted evidence, cannot grant permissions, and is never fetched or executed.
  The desktop additionally bounds rendered trees and per-tool output to 4 KiB.
- Desktop registration advertises only host-declared read-only tools. Host-declared
  side effects can be registered through the core API but still need core policy
  opt-in and exact-call approval. The desktop does not offer that approval flow.

See [ADR 0013](adr/0013-mcp-registry-adapters.md) and the disabled examples in
[mcp-config.toml](../examples/mcp-config.toml). Live third-party compatibility has
not been evaluated; transport and policy tests use Rust fixtures and loopback HTTP.
