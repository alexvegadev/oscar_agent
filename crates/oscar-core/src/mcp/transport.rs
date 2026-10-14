use crate::{
    config::{McpServer, McpSettings, McpTransport, WorkMode},
    error::{McpError, OscarError},
};
use reqwest::{Client, Url};
use serde_json::{Value, json};
use std::{net::IpAddr, process::Stdio, time::Duration};
use tokio::{
    io::{AsyncBufReadExt, AsyncWriteExt, BufReader},
    process::{Child, ChildStdin, ChildStdout, Command},
};

pub(super) const VERSION: &str = "2025-06-18";
fn protocol() -> OscarError {
    OscarError::Mcp(McpError::Protocol)
}
fn transport() -> OscarError {
    OscarError::Mcp(McpError::Transport)
}
fn limit() -> OscarError {
    OscarError::Limit("MCP response byte/message limit exceeded".into())
}

pub(super) enum Connection {
    Stdio(Box<Process>),
    Http(Box<Http>),
}
pub(super) struct Process {
    _child: Child,
    input: ChildStdin,
    output: BufReader<ChildStdout>,
    limit: usize,
}
pub(super) struct Http {
    client: Client,
    url: Url,
    token: Option<String>,
    session: Option<String>,
    limit: usize,
}

pub(super) fn endpoint(
    server: &McpServer,
    mode: WorkMode,
    local_only: bool,
) -> Result<Url, OscarError> {
    let url = Url::parse(server.url.as_deref().ok_or_else(protocol)?).map_err(|_| protocol())?;
    if !url.username().is_empty()
        || url.password().is_some()
        || url.query().is_some()
        || url.fragment().is_some()
        || url.host_str().is_none()
        || !matches!(url.scheme(), "http" | "https")
    {
        return Err(protocol());
    }
    let loopback = url
        .host_str()
        .unwrap_or_default()
        .trim_matches(['[', ']'])
        .parse::<IpAddr>()
        .is_ok_and(|ip| ip.is_loopback());
    if !loopback
        && (url.scheme() != "https"
            || !server.allow_remote
            || mode == WorkMode::Local
            || local_only)
    {
        return Err(OscarError::Mcp(McpError::RemoteDenied));
    }
    Ok(url)
}
impl Connection {
    pub(super) async fn open(
        server: &McpServer,
        settings: &McpSettings,
        mode: WorkMode,
        local_only: bool,
    ) -> Result<Self, OscarError> {
        let mut connection = match server.transport {
            McpTransport::Stdio => {
                let mut command = Command::new(server.command.as_deref().ok_or_else(protocol)?);
                command
                    .args(&server.args)
                    .env_clear()
                    .stdin(Stdio::piped())
                    .stdout(Stdio::piped())
                    .stderr(Stdio::null())
                    .kill_on_drop(true);
                // Minimal platform environment. Credential variables require explicit mapping.
                for name in ["PATH", "SystemRoot", "WINDIR", "TEMP", "TMP"] {
                    if let Some(value) = std::env::var_os(name) {
                        command.env(name, value);
                    }
                }
                for (child, host) in &server.env {
                    let value = std::env::var_os(host)
                        .ok_or(OscarError::Mcp(McpError::SecretUnavailable))?;
                    command.env(child, value);
                }
                if let Some(cwd) = &server.cwd {
                    command.current_dir(cwd);
                }
                #[cfg(windows)]
                command.creation_flags(0x08000000); // CREATE_NO_WINDOW
                let mut child = command.spawn().map_err(|_| transport())?;
                let input = child.stdin.take().ok_or_else(transport)?;
                let output = BufReader::new(child.stdout.take().ok_or_else(transport)?);
                Self::Stdio(Box::new(Process {
                    _child: child,
                    input,
                    output,
                    limit: settings.max_response_bytes,
                }))
            }
            McpTransport::Http => {
                let url = endpoint(server, mode, local_only)?;
                let token = server
                    .bearer_token_env
                    .as_ref()
                    .map(|name| {
                        std::env::var(name)
                            .map_err(|_| OscarError::Mcp(McpError::SecretUnavailable))
                    })
                    .transpose()?;
                let client = Client::builder()
                    .no_proxy()
                    .redirect(reqwest::redirect::Policy::none())
                    .timeout(Duration::from_millis(settings.timeout_ms))
                    .connect_timeout(Duration::from_millis(settings.timeout_ms.min(5000)))
                    .build()
                    .map_err(|_| transport())?;
                Self::Http(Box::new(Http {
                    client,
                    url,
                    token,
                    session: None,
                    limit: settings.max_response_bytes,
                }))
            }
        };
        let initialized = connection.request(1, "initialize", json!({"protocolVersion":VERSION,"capabilities":{},"clientInfo":{"name":"oscar","version":env!("CARGO_PKG_VERSION")}})).await?;
        if initialized.get("protocolVersion").and_then(Value::as_str) != Some(VERSION)
            || !initialized
                .get("capabilities")
                .and_then(|v| v.get("tools"))
                .is_some_and(Value::is_object)
        {
            return Err(protocol());
        }
        connection
            .notify("notifications/initialized", json!({}))
            .await?;
        Ok(connection)
    }
    pub(super) async fn request(
        &mut self,
        id: u64,
        method: &str,
        params: Value,
    ) -> Result<Value, OscarError> {
        let message = json!({"jsonrpc":"2.0","id":id,"method":method,"params":params});
        match self {
            Self::Stdio(p) => {
                p.write(&message).await?;
                let mut total = 0usize;
                for _ in 0..64 {
                    let line = p.line().await?;
                    total = total.saturating_add(line.len());
                    if total > p.limit {
                        return Err(limit());
                    }
                    let value: Value = serde_json::from_slice(&line).map_err(|_| protocol())?;
                    if let Some(result) = response(&value, id)? {
                        return Ok(result);
                    }
                    if value.get("id").is_some() && value.get("method").is_some() {
                        p.write(&json!({"jsonrpc":"2.0","id":value["id"],"error":{"code":-32601,"message":"Client capability not supported"}})).await?;
                    }
                }
                Err(limit())
            }
            Self::Http(h) => h.request(&message, id).await,
        }
    }
    pub(super) async fn notify(&mut self, method: &str, params: Value) -> Result<(), OscarError> {
        let value = json!({"jsonrpc":"2.0","method":method,"params":params});
        match self {
            Self::Stdio(p) => p.write(&value).await,
            Self::Http(h) => {
                let response = h.post(&value).send().await.map_err(|_| transport())?;
                if response.status() != reqwest::StatusCode::ACCEPTED {
                    return Err(protocol());
                }
                Ok(())
            }
        }
    }
    pub(super) async fn close(&mut self) -> Result<(), OscarError> {
        match self {
            Self::Stdio(p) => {
                p._child.kill().await.map_err(|_| transport())?;
                Ok(())
            }
            Self::Http(h) => {
                if let Some(session) = &h.session {
                    let mut request = h
                        .client
                        .delete(h.url.clone())
                        .header("MCP-Session-Id", session)
                        .header("MCP-Protocol-Version", VERSION);
                    if let Some(token) = &h.token {
                        request = request.bearer_auth(token);
                    }
                    let response = request.send().await.map_err(|_| transport())?;
                    if !response.status().is_success()
                        && ![404, 405].contains(&response.status().as_u16())
                    {
                        return Err(transport());
                    }
                }
                Ok(())
            }
        }
    }
}
impl Process {
    async fn write(&mut self, message: &Value) -> Result<(), OscarError> {
        let mut bytes = serde_json::to_vec(message).map_err(|_| protocol())?;
        if bytes.len() > self.limit {
            return Err(limit());
        }
        bytes.push(b'\n');
        self.input
            .write_all(&bytes)
            .await
            .map_err(|_| transport())?;
        self.input.flush().await.map_err(|_| transport())
    }
    async fn line(&mut self) -> Result<Vec<u8>, OscarError> {
        let mut bytes = Vec::new();
        loop {
            let buffer = self.output.fill_buf().await.map_err(|_| transport())?;
            if buffer.is_empty() {
                return Err(transport());
            }
            let newline = buffer.iter().position(|b| *b == b'\n');
            let take = newline.map_or(buffer.len(), |n| n + 1);
            if bytes.len().saturating_add(take) > self.limit {
                return Err(limit());
            }
            bytes.extend_from_slice(&buffer[..take]);
            self.output.consume(take);
            if newline.is_some() {
                return Ok(bytes);
            }
        }
    }
}
impl Http {
    fn post(&self, value: &Value) -> reqwest::RequestBuilder {
        let mut request = self
            .client
            .post(self.url.clone())
            .header("Accept", "application/json, text/event-stream")
            .json(value);
        if value.get("method").and_then(Value::as_str) != Some("initialize") {
            request = request.header("MCP-Protocol-Version", VERSION);
        }
        if let Some(session) = &self.session {
            request = request.header("MCP-Session-Id", session);
        }
        if let Some(token) = &self.token {
            request = request.bearer_auth(token);
        }
        request
    }
    async fn request(&mut self, value: &Value, id: u64) -> Result<Value, OscarError> {
        if value.to_string().len() > self.limit {
            return Err(limit());
        }
        let mut wire = self.post(value).send().await.map_err(|_| transport())?;
        if !wire.status().is_success() {
            return Err(transport());
        }
        if id == 1
            && let Some(session) = wire.headers().get("MCP-Session-Id")
        {
            let session = session.to_str().map_err(|_| protocol())?;
            if session.is_empty()
                || session.len() > 256
                || !session.bytes().all(|b| (0x21..=0x7e).contains(&b))
            {
                return Err(protocol());
            }
            self.session = Some(session.into());
        }
        let content_type = wire
            .headers()
            .get(reqwest::header::CONTENT_TYPE)
            .and_then(|v| v.to_str().ok())
            .unwrap_or_default()
            .split(';')
            .next()
            .unwrap_or_default()
            .trim();
        let sse = match content_type {
            "application/json" => false,
            "text/event-stream" => true,
            _ => return Err(protocol()),
        };
        let mut bytes = Vec::new();
        let mut total = 0usize;
        let mut scan = 0usize;
        let mut events = 0usize;
        while let Some(chunk) = wire.chunk().await.map_err(|_| transport())? {
            total = total.saturating_add(chunk.len());
            if total > self.limit {
                return Err(limit());
            }
            bytes.extend_from_slice(&chunk);
            if sse {
                // Parse complete events only; UTF-8 and CRLF can cross network chunks.
                let mut consumed = 0;
                while let Some((end, separator)) = event_boundary(&bytes, scan) {
                    events += 1;
                    if events > 64 {
                        return Err(limit());
                    }
                    let text =
                        std::str::from_utf8(&bytes[consumed..end]).map_err(|_| protocol())?;
                    consumed = end + separator;
                    scan = consumed;
                    let data = text
                        .lines()
                        .filter_map(|line| {
                            line.strip_prefix("data:")
                                .map(|v| v.strip_prefix(' ').unwrap_or(v))
                        })
                        .collect::<Vec<_>>()
                        .join("\n");
                    if data.is_empty() {
                        continue;
                    }
                    let value: Value = serde_json::from_str(&data).map_err(|_| protocol())?;
                    if let Some(result) = response(&value, id)? {
                        return Ok(result);
                    }
                    // Client requests (sampling/elicitation/roots) are not advertised.
                    if value.get("id").is_some() {
                        return Err(protocol());
                    }
                }
                if consumed > 0 {
                    bytes.drain(..consumed);
                }
                // Rescan only the possible partial delimiter at the end of a chunk.
                scan = bytes.len().saturating_sub(3);
            }
        }
        if sse {
            return Err(protocol());
        }
        let value: Value = serde_json::from_slice(&bytes).map_err(|_| protocol())?;
        response(&value, id)?.ok_or_else(protocol)
    }
}
fn event_boundary(bytes: &[u8], from: usize) -> Option<(usize, usize)> {
    for i in from..bytes.len() {
        if bytes[i..].starts_with(b"\n\n") {
            return Some((i, 2));
        }
        if bytes[i..].starts_with(b"\r\n\r\n") {
            return Some((i, 4));
        }
    }
    None
}
fn response(value: &Value, id: u64) -> Result<Option<Value>, OscarError> {
    if value.get("jsonrpc").and_then(Value::as_str) != Some("2.0") || !value.is_object() {
        return Err(protocol());
    }
    if value.get("method").is_some() {
        return Ok(None);
    }
    if value.get("id").and_then(Value::as_u64) != Some(id)
        || value.get("result").is_some() == value.get("error").is_some()
    {
        return Err(protocol());
    }
    if value.get("error").is_some() {
        return Err(OscarError::Mcp(McpError::ToolFailed));
    }
    Ok(Some(value["result"].clone()))
}
