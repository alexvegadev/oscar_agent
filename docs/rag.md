# Session RAG with Rig

The optional `rag` feature adds semantic retrieval to interactive `agent` sessions.
Only files explicitly loaded with `/read PATH` enter the in-memory index. File
access still uses OSCAR's registered, workspace-constrained read tool.

Start a local Ollama server and install an embedding model yourself, for example
`ollama pull nomic-embed-text`. OSCAR does not download models. Then run:

```bash
cargo run -p oscar-cli --features rag -- agent --workspace . --rag
```

```text
/read crates/oscar-core/src/context.rs
/read crates/oscar-core/src/routing.rs
How is dependency context prepared before routing?
/status
/reset
/exit
```

The supplied settings still use mock **completion** providers. For real coding
answers, configure a completion provider as described in [agent mode](agent-mode.md)
and enable `--features http,rag`. Embeddings and completions are separate models.

`--embedding-model` defaults to `nomic-embed-text`; `--embedding-url` defaults to
`http://127.0.0.1:11434/api/embed`. Both options require `--rag`. The endpoint must
use a literal loopback IP and the `/api/embed` path. DNS names (including
`localhost`), credentials, queries, fragments, redirects and proxies are rejected
or disabled. The adapter uses Ollama's [embedding API](https://docs.ollama.com/api/embed)
with `truncate = false`, so excessive model input fails explicitly.

Use `local` or `mixed` work mode. Strict `full_remote` rejects this local embedding
adapter. In `mixed` mode, remote completion still requires `--allow-remote-context`
and no TOML local-only restriction. Embeddings never go to a remote endpoint.
Enabling RAG grants no extra filesystem access, tools or execution permissions.

Each loaded file is split at UTF-8 boundaries, embedded in one bounded call, and
stored in Rig's in-memory vector store. Re-reading the same source replaces its
snapshot atomically after validation; failures leave the previous snapshot intact.
On each prompt, the query is embedded locally and Rig selects the nearest chunks.
JSON evidence contains source path, start/end byte offsets and the complete chunk
text. Offsets describe the loaded snapshot, not a later on-disk revision.

File contents are kept in the index instead of being appended to conversation
history. Only retrieved chunks accompany each request. Prompts and replies remain
complete and bounded by the existing session context limit; they are not silently
evicted or summarized. If the complete encoded context cannot fit, inference is
rejected. Use `/reset` to clear conversation, evidence and index. Reset preserves
all call budgets and deadlines. Exit discards the index; nothing is persisted.

Default RAG limits:

| Limit | Default |
| --- | --- |
| Document bytes | 16384, also constrained by the existing file tool/input limit |
| Total indexed text bytes | 131072 |
| Total chunks | 128 |
| Chunk bytes | 2048 (no overlap) |
| Nearest chunks per prompt | 4 |
| Encoded evidence array bytes | 8192, capped by `max_input_bytes` |
| Embedding attempts across resets | 256 |
| Embedding dimensions | 4096 |
| Embedding response wire bytes | 4194304 |
| Embedding request timeout | Existing `limits.call_timeout_ms` (60000 by default) |

Failed calls consume the embedding budget. No automatic retries occur. The overall
agent deadline also bounds retrieval and input waiting. Cancellation drops client
work but cannot guarantee the local server stops computation. No response bodies,
prompts, source contents or vectors are included in transport errors or CLI status.
Status/progress reports chunk counts, omissions and embedding-call counts. Hosts
installing a Rig tracing subscriber should also review its source-ID tracing.

This first slice does not index whole repositories, implement durable vector
storage, rerank results, summarize history, or integrate retrieval into `plan`/`run`.
Similarity alone does not establish relevance. Fixed byte chunks can split code
constructs, and matching chunks can omit surrounding definitions. The model is
told to treat evidence as untrusted and report missing context. Citations are
supplied to the model; generated citation correctness is not independently checked.

Library hosts can construct `oscar_core::rag::RagSession` with an injected
`EmbeddingProvider` and `RagLimits`. OSCAR-owned public types hide Rig internals.
Hosts are responsible for injected provider locality/policy and transport bounds;
the session enforces count, shape, finite nonzero vectors, stable dimensions,
timeouts, cancellation, capacity and encoded evidence limits. Tests use mock
vectors and loopback HTTP fixtures without credentials or model downloads.

Rig is pinned to `rig-core = 0.11.1` with default features disabled, verified on
Rust 1.85. Only its vector store and embedding contracts are used. Upgrading Rig
requires an explicit MSRV/API/dependency review; see [ADR 0007](adr/0007-session-rag.md).
