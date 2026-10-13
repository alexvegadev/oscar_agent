# Interactive agent mode

Optional [RAG with Rig](rag.md) indexes `/read` files in memory and supplies
relevant cited chunks per prompt. Enable `--features rag` and `agent --rag`
with an installed local Ollama embedding model. The full-file context behavior
below describes the default session without RAG.

Start a coding conversation from the repository root:

```bash
cargo run -p oscar-cli -- agent --workspace .
```

The default configuration uses mock providers and returns demonstration proposals.
For real inference, configure an OpenAI-compatible provider with both
`code_analysis` and `code_generation` capabilities and enable the `http` feature:

```bash
cargo run -p oscar-cli --features http -- agent --config oscar.toml --workspace .
```

An optional positional request runs before reading stdin. Interactive terminals
show a cyan `oscar ›` prompt; redirected input accepts the same newline-delimited commands.
Prompts occupy one line. File paths may contain spaces; `/read` takes the entire
remaining line without shell quoting.

Typing `/` immediately opens command recommendations with descriptions. The
list narrows as you type, for example `/re` offers `/read` and `/reset`. Use
Up/Down to choose, Tab to complete, Enter to select and Esc to dismiss. Selecting
`/read` leaves a space for the path; selecting another command runs it. Left/Right,
Home/End, Backspace/Delete and Ctrl+U edit the current line. Ctrl+D on an empty
prompt ends the session. Bracketed paste turns line breaks into spaces and
removes control characters. Input stays bounded and is not saved in a history
file. Redirected streams and `TERM=dumb` use plain input without colors or menus.
Terminal mode and recommendations are restored/cleared on submit, cancellation,
timeout or input failure. Model replies have a separate `OSCAR` label.
In plain input mode, submitting `/` prints the command recommendations.

```text
/read crates/oscar-core/src/config.rs
Propose a unified diff that improves configuration validation, with tests.
Explain the compatibility impact of your proposed change.
/status
/reset
/exit
```

`/read PATH` loads a UTF-8 regular file beneath the explicitly selected workspace.
Reads pass through the host-registered `read_file` tool, schema validation,
read-only allowlist, call and output limits, cancellation and deadlines.
Capability-relative filesystem access prevents path traversal and symlink or
junction escapes. Absolute paths, `..`, Windows prefixes and alternate streams
are rejected. Directories, binary content and oversized files are rejected rather
than truncated. Files are read only at the user's request; models cannot invoke
tools, expand the workspace, apply patches or run commands.

Replies are edit proposals, preferably unified diffs, with suggested checks.
The assistant receives loaded files, earlier prompts and earlier replies as
explicitly marked untrusted JSON evidence. Terminal control characters are
removed from displayed model output. Generated patches are not verified by a
compiler and must be reviewed before applying them.

Conversation and file context are **local-only by default**. To allow remote
inference, pass `--allow-remote-context`. This authorizes sending the entire
current context, including loaded files, to a remote provider when routing
selects it. Strict `local` mode and TOML `planning.local_only_data = true` remain
enforced. Strict `full_remote` mode requires that flag and no TOML local-only
restriction. Provider credentials and HTTP security follow the existing
[configuration rules](hybrid.md). There is no automatic retry or escalation
after a failed model call in this first interactive slice.

`/help` lists commands. `/status` reports turn, remote and file-call counters,
context bytes and the local-only boundary. `/reset` discards loaded files and
conversation; it does not reset budgets or extend deadlines. `/exit`, `/quit`
and EOF finish successfully. Ctrl+C cancels the whole session. Failures and
exhausted execution budgets exit nonzero, with a session identifier and terminal
outcome on stderr. Prompts, file contents and responses are not logged to stderr.

Limits use the existing `[limits]` configuration:

| Setting | Agent behavior | Default |
| --- | --- | --- |
| `max_tasks` | Maximum inference turns | 32 |
| `run_timeout_ms` | Entire session, including time waiting for input | 300000 |
| `call_timeout_ms` | Each inference or file read | 60000 |
| `max_input_bytes` | Input line and total encoded model context | 24000 |
| `max_output_bytes` | Each model reply | 16384 |
| `max_output_tokens` | Requested model output token ceiling | 2048 |
| `max_remote_calls` | Total remote calls across resets | 8 |
| `max_remote_input_bytes` | Total context bytes sent remotely | 96000 |

File output is additionally capped at 16384 bytes, file attempts at 64 and
session input lines at 1024. JSON escaping and instructions count toward context
size. A rejected context addition leaves existing history intact; use `/reset`
when context is full. If a reply cannot fit, it is displayed and the user is
explicitly told that this latest exchange was not retained. No context is
silently distilled or evicted. Provider context-window and remote cost limits
are checked through the existing routing policy.

OS file operations and Tokio stdin use a blocking pool capped at two threads.
They cannot be forcibly cancelled at the OS boundary. Cancellation or a file
timeout ends the session, prevents additional work and waits at most 10 ms for
runtime shutdown; the process then exits. Use ordinary local workspace files,
not device files or a filesystem with unbounded I/O. Provider cancellation drops
client work but cannot guarantee that remote inference or billing stops.

There is no automatic repository discovery, model-to-tool loop, patch application,
shell execution, streaming, terminal editor or durable session resume in this
slice. Session content remains in memory and is discarded on reset or exit;
there is no automatic persistence. Terminal scrollback may still contain replies.
