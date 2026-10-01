//! Host-requested file context, admitted through the common tool registry.
use cap_std::{ambient_authority, fs::Dir};
use oscar_core::tools::{
    Effect, Field, InputSchema, InputType, InputValue, Tool, ToolContext, ToolDefinition,
    ToolError, ToolFuture, ToolRegistry, ValidatedInput,
};
use std::{
    io::Read,
    path::{Component, Path},
    sync::Arc,
};

struct ReadFile(Arc<Dir>);

pub(super) fn registry(root: &Path) -> Result<ToolRegistry, ToolError> {
    let root = root.canonicalize().map_err(|_| ToolError::Failed)?;
    let dir = Dir::open_ambient_dir(root, ambient_authority()).map_err(|_| ToolError::Failed)?;
    let mut registry = ToolRegistry::default();
    registry.register(
        ToolDefinition {
            name: "read_file".into(),
            description: "Read one UTF-8 regular file relative to the host workspace.".into(),
            effect: Effect::ReadOnly,
            input: InputSchema {
                fields: [(
                    "path".into(),
                    Field {
                        required: true,
                        value_type: InputType::Text { max_bytes: 4096 },
                    },
                )]
                .into(),
            },
        },
        Arc::new(ReadFile(Arc::new(dir))),
    )?;
    Ok(registry)
}

impl Tool for ReadFile {
    fn execute(&self, input: ValidatedInput, context: ToolContext) -> ToolFuture<'_> {
        Box::pin(async move {
            let Some(InputValue::Text(path)) = input.get("path") else {
                return Err(ToolError::InvalidArguments);
            };
            let path = Path::new(path);
            // Reject prefixes, traversal, ADS and ambiguous Windows path spellings.
            if path.as_os_str().is_empty()
                || path
                    .components()
                    .any(|c| !matches!(c, Component::Normal(_)))
                || path.to_string_lossy().contains(':')
            {
                return Err(ToolError::Denied);
            }
            let dir = self.0.clone();
            let path = path.to_owned();
            // The runtime caps its blocking pool at two threads. OS file I/O is not
            // cancellable; the enclosing tool/session deadline stops waiting and
            // terminates this CLI session on timeout (no further file work).
            tokio::task::spawn_blocking(move || {
                if context.cancellation.is_cancelled() {
                    return Err(ToolError::Cancelled);
                }
                let metadata = dir.metadata(&path).map_err(|_| ToolError::Denied)?;
                if !metadata.is_file() {
                    return Err(ToolError::Denied);
                }
                if metadata.len() > context.max_output_bytes as u64 {
                    return Err(ToolError::OutputLimit);
                }
                // Capability-relative open prevents traversal and symlink escapes,
                // including replacement between metadata and open.
                let file = dir.open(&path).map_err(|_| ToolError::Denied)?;
                if !file.metadata().map_err(|_| ToolError::Failed)?.is_file() {
                    return Err(ToolError::Denied);
                }
                let mut bytes = Vec::new();
                file.take(context.max_output_bytes as u64 + 1)
                    .read_to_end(&mut bytes)
                    .map_err(|_| ToolError::Failed)?;
                if context.cancellation.is_cancelled() {
                    return Err(ToolError::Cancelled);
                }
                if bytes.len() > context.max_output_bytes {
                    return Err(ToolError::OutputLimit);
                }
                String::from_utf8(bytes).map_err(|_| ToolError::Failed)
            })
            .await
            .map_err(|_| ToolError::Failed)?
        })
    }
}
