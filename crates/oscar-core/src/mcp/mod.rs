//! Opt-in MCP tools through the ordinary registry. No capabilities are loaded
//! from server notifications, annotations, prompts, or model-produced settings.
mod schema;
mod transport;

use crate::{
    config::{Config, McpEffect, McpServer, McpSettings, WorkMode},
    error::{McpError, OscarError},
    tools::{
        Effect, Tool, ToolContext, ToolDefinition, ToolError, ToolFuture, ToolRegistry,
        ValidatedInput,
    },
};
use serde_json::{Value, json};
use std::{
    collections::{BTreeMap, BTreeSet},
    future::Future,
    sync::Arc,
    time::Duration,
};
use tokio_util::sync::CancellationToken;
use transport::Connection;

fn protocol() -> OscarError {
    OscarError::Mcp(McpError::Protocol)
}
async fn bounded<T>(
    token: &CancellationToken,
    timeout_ms: u64,
    future: impl Future<Output = Result<T, OscarError>>,
) -> Result<T, OscarError> {
    tokio::select! {
        biased;
        _ = token.cancelled() => Err(OscarError::Cancelled),
        result = tokio::time::timeout(Duration::from_millis(timeout_ms), future) => result.unwrap_or(Err(OscarError::Mcp(McpError::Timeout))),
    }
}
async fn discover(
    connection: &mut Connection,
    settings: &McpSettings,
) -> Result<BTreeMap<String, Value>, OscarError> {
    let mut tools = BTreeMap::new();
    let mut cursor = None;
    let mut cursors = BTreeSet::new();
    for page in 0..settings.max_pages {
        let params = cursor
            .as_ref()
            .map_or_else(|| json!({}), |c| json!({"cursor":c}));
        let result = connection
            .request(2 + page as u64, "tools/list", params)
            .await?;
        for tool in result
            .get("tools")
            .and_then(Value::as_array)
            .ok_or_else(protocol)?
        {
            let name = tool
                .get("name")
                .and_then(Value::as_str)
                .ok_or_else(protocol)?;
            if name.is_empty()
                || name.len() > 128
                || tools.len() >= settings.max_discovered_tools
                || tools.insert(name.into(), tool.clone()).is_some()
            {
                return Err(protocol());
            }
        }
        cursor = match result.get("nextCursor") {
            None | Some(Value::Null) => return Ok(tools),
            Some(Value::String(c))
                if !c.is_empty() && c.len() <= 1024 && cursors.insert(c.clone()) =>
            {
                Some(c.clone())
            }
            _ => return Err(protocol()),
        };
    }
    Err(OscarError::Limit(
        "MCP discovery page limit exceeded".into(),
    ))
}

/// Discover enabled, explicitly configured tools and return a new registry.
/// On failure the owned registry is discarded, so callers cannot use partial
/// registration. Names are `mcp_<server>_<alias>` and effects come only from TOML.
pub async fn register_servers(
    mut registry: ToolRegistry,
    config: &Config,
    cancellation: CancellationToken,
) -> Result<ToolRegistry, OscarError> {
    config.validate()?;
    if !config.features.mcp_enabled {
        return Ok(registry);
    }
    bounded(&cancellation, config.limits.run_timeout_ms, async {
        for (name, server) in config.mcp.servers.iter().filter(|(_, s)| s.enabled) {
            let tools = bounded(&cancellation, config.mcp.timeout_ms, async {
                let mut connection = Connection::open(
                    server,
                    &config.mcp,
                    config.work_mode,
                    config.planning.local_only_data,
                )
                .await?;
                let tools = discover(&mut connection, &config.mcp).await;
                let cleanup = connection.close().await;
                tools.and_then(|tools| cleanup.map(|_| tools))
            })
            .await?;
            for (alias, selected) in &server.tools {
                let remote = tools.get(&selected.name).ok_or_else(protocol)?;
                let input_schema = remote.get("inputSchema").ok_or_else(protocol)?;
                let input =
                    schema::convert(input_schema, config.limits.max_input_bytes).map_err(|_| {
                        OscarError::Config(format!(
                            "MCP tool mcp_{name}_{alias} has an unsupported schema; see docs/mcp.md"
                        ))
                    })?;
                let description = remote
                    .get("description")
                    .and_then(Value::as_str)
                    .filter(|s| !s.trim().is_empty())
                    .unwrap_or("Host-registered MCP tool.");
                if description.len() > 1024 {
                    return Err(OscarError::Limit(
                        "MCP tool description exceeds byte limit".into(),
                    ));
                }
                let effect = match selected.effect {
                    McpEffect::ReadOnly => Effect::ReadOnly,
                    McpEffect::SideEffect => Effect::SideEffect,
                };
                registry
                    .register(
                        ToolDefinition {
                            name: format!("mcp_{name}_{alias}"),
                            description: description.into(),
                            input,
                            effect,
                        },
                        Arc::new(RemoteTool {
                            server: server.clone(),
                            settings: config.mcp.clone(),
                            mode: config.work_mode,
                            local_only: config.planning.local_only_data,
                            remote_name: selected.name.clone(),
                            schema: input_schema.clone(),
                        }),
                    )
                    .map_err(|_| {
                        OscarError::Config(
                            "MCP registration exceeds registry limits or has invalid metadata"
                                .into(),
                        )
                    })?;
            }
        }
        Ok(registry)
    })
    .await
}
struct RemoteTool {
    server: McpServer,
    settings: McpSettings,
    mode: WorkMode,
    local_only: bool,
    remote_name: String,
    schema: Value,
}
impl Tool for RemoteTool {
    fn execute(&self, input: ValidatedInput, context: ToolContext) -> ToolFuture<'_> {
        Box::pin(async move {
            let result = bounded(&context.cancellation, self.settings.timeout_ms, async {
                // Fresh short-lived sessions avoid idle processes and a cancelled
                // response being mistaken for a later request. Never retry calls.
                let mut connection =
                    Connection::open(&self.server, &self.settings, self.mode, self.local_only)
                        .await?;
                let outcome = async {
                    let tools = discover(&mut connection, &self.settings).await?;
                    if tools
                        .get(&self.remote_name)
                        .and_then(|t| t.get("inputSchema"))
                        != Some(&self.schema)
                    {
                        return Err(protocol());
                    }
                    let result = connection
                        .request(
                            100,
                            "tools/call",
                            json!({"name":self.remote_name,"arguments":input}),
                        )
                        .await?;
                    if result.get("isError").is_some_and(|v| !v.is_boolean())
                        || result.get("isError").and_then(Value::as_bool) == Some(true)
                    {
                        return Err(OscarError::Mcp(McpError::ToolFailed));
                    }
                    let content = result
                        .get("content")
                        .filter(|v| v.is_array())
                        .ok_or_else(protocol)?;
                    let mut output = json!({"content":content});
                    if let Some(structured) = result.get("structuredContent") {
                        output["structuredContent"] = structured.clone();
                    }
                    let output = output.to_string();
                    if output.len() > context.max_output_bytes {
                        return Err(OscarError::Limit(
                            "MCP tool output exceeds byte limit".into(),
                        ));
                    }
                    Ok(output)
                }
                .await;
                let cleanup = connection.close().await;
                outcome.and_then(|output| cleanup.map(|_| output))
            })
            .await;
            result.map_err(|error| match error {
                OscarError::Cancelled => ToolError::Cancelled,
                OscarError::Mcp(McpError::Timeout) => ToolError::Timeout,
                OscarError::Limit(_) => ToolError::OutputLimit,
                _ => ToolError::Failed,
            })
        })
    }
}
