use crate::error::OscarError;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum WorkMode {
    Local,
    #[serde(alias = "remote")]
    FullRemote,
    Mixed,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Capability {
    CodeGeneration,
    CodeAnalysis,
    RepositorySearch,
    Refactoring,
    Documentation,
    Testing,
    Summarization,
    Debugging,
    AdvancedReasoning,
    Architecture,
    ComplexDebugging,
    Review,
    Planning,
    LargeContext,
    Vision,
    ToolUse,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Config {
    pub work_mode: WorkMode,
    #[serde(default)]
    pub features: Features,
    #[serde(default)]
    pub planning: Planning,
    pub providers: HashMap<String, Provider>,
    #[serde(default)]
    pub routing: Routing,
    #[serde(default)]
    pub limits: Limits,
    #[serde(default)]
    pub mcp: McpSettings,
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PlannerMode {
    #[default]
    Heuristic,
    Inference,
}

#[derive(Debug, Clone, Default, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct Planning {
    pub mode: PlannerMode,
    /// Restrict both planning inference and every generated task to local data use.
    pub local_only_data: bool,
}

#[derive(Debug, Clone, Default, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Features {
    pub skills_enabled: bool,
    pub mcp_enabled: bool,
}

// Deliberately no derived Debug: legacy plaintext credentials must stay redacted.
#[derive(Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Provider {
    pub enabled: bool,
    pub provider: Option<String>,
    /// Explicit protocol for custom provider names. Currently `openai_chat`.
    /// A vendor name alone cannot add a new wire format or permissions.
    pub protocol: Option<String>,
    pub model: String,
    pub context_window: u32,
    pub capabilities: Vec<Capability>,
    pub api_key: Option<String>,
    pub api_key_env: Option<String>,
    pub api_url: Option<String>,
    #[serde(default = "one")]
    pub max_concurrency: usize,
    #[serde(default)]
    pub cost: ProviderCost,
}
fn one() -> usize {
    1
}
impl std::fmt::Debug for Provider {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Provider")
            .field("enabled", &self.enabled)
            .field("context_window", &self.context_window)
            .field("capabilities", &self.capabilities)
            .finish_non_exhaustive()
    }
}

/// Optional USD per million tokens; routing uses a conservative byte-based bound.
#[derive(Debug, Clone, Default, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ProviderCost {
    pub input_per_million: Option<f64>,
    pub output_per_million: Option<f64>,
}
#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RemoteReview {
    Never,
    Auto,
    Always,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct Routing {
    pub local_first: bool,
    pub max_local_attempts: usize,
    pub remote_review: RemoteReview,
    pub context_distillation: bool,
    pub distilled_bytes_per_artifact: usize,
    pub max_remote_call_cost: Option<f64>,
}
impl Default for Routing {
    fn default() -> Self {
        Self {
            local_first: true,
            max_local_attempts: 2,
            remote_review: RemoteReview::Auto,
            context_distillation: true,
            distilled_bytes_per_artifact: 2048,
            max_remote_call_cost: None,
        }
    }
}
#[derive(Debug, Clone, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct Limits {
    pub max_tasks: usize,
    pub max_concurrency: usize,
    pub run_timeout_ms: u64,
    pub call_timeout_ms: u64,
    pub max_input_bytes: usize,
    pub max_output_bytes: usize,
    pub max_output_tokens: u32,
    pub max_remote_calls: usize,
    pub max_remote_input_bytes: usize,
}
impl Default for Limits {
    fn default() -> Self {
        Self {
            max_tasks: 32,
            max_concurrency: 2,
            run_timeout_ms: 300_000,
            call_timeout_ms: 60_000,
            max_input_bytes: 24_000,
            max_output_bytes: 16_384,
            max_output_tokens: 2048,
            max_remote_calls: 8,
            max_remote_input_bytes: 96_000,
        }
    }
}
impl Config {
    pub fn load_from_file(path: &str) -> Result<Self, OscarError> {
        use std::io::Read;
        let file = std::fs::File::open(path)
            .map_err(|_| OscarError::Config("cannot open config file".into()))?;
        let mut text = String::new();
        file.take(65_537)
            .read_to_string(&mut text)
            .map_err(|_| OscarError::Config("cannot read UTF-8 config".into()))?;
        Self::parse(&text)
    }
    pub fn parse(text: &str) -> Result<Self, OscarError> {
        if text.len() > 65_536 {
            return Err(OscarError::Config("file exceeds 64 KiB".into()));
        }
        // toml's Display includes the original line, potentially containing a secret.
        let config: Self = toml::from_str(text).map_err(|_| OscarError::Config(
            "invalid TOML or field value; work_mode must be local, full_remote (legacy remote), or mixed; check field names and types".into()))?;
        config.validate()?;
        Ok(config)
    }
    pub fn validate(&self) -> Result<(), OscarError> {
        self.mcp.validate()?;
        let bad = |s: &str| OscarError::Config(s.into());
        let l = &self.limits;
        if !(1..=256).contains(&l.max_tasks)
            || !(1..=32).contains(&l.max_concurrency)
            || !(1..=10).contains(&self.routing.max_local_attempts)
            || !(1..=3_600_000).contains(&l.run_timeout_ms)
            || l.call_timeout_ms == 0
            || l.call_timeout_ms > l.run_timeout_ms
            || !(256..=1_048_576).contains(&l.max_input_bytes)
            || !(1..=1_048_576).contains(&l.max_output_bytes)
            || !(1..=65_536).contains(&l.max_output_tokens)
            || !(128..=65_536).contains(&self.routing.distilled_bytes_per_artifact)
            || l.max_remote_calls > 256
            || l.max_remote_input_bytes > 268_435_456
        {
            return Err(bad(
                "limits out of range; see docs/hybrid.md for supported bounds",
            ));
        }
        if self.providers.keys().any(|k| k != "local" && k != "remote") {
            return Err(bad("providers must be named local or remote"));
        }
        for p in self.providers.values().filter(|p| p.enabled) {
            if p.model.trim().is_empty()
                || p.context_window <= l.max_output_tokens + 256
                || !(1..=32).contains(&p.max_concurrency)
                || p.capabilities.is_empty()
            {
                return Err(bad(
                    "enabled providers need model, capabilities, positive concurrency and context_window > max_output_tokens + 256",
                ));
            }
            if p.api_key.is_some() && p.api_key_env.is_some() {
                return Err(bad("use api_key_env or legacy api_key, not both"));
            }
            for n in [
                p.cost.input_per_million,
                p.cost.output_per_million,
                self.routing.max_remote_call_cost,
            ]
            .into_iter()
            .flatten()
            {
                if !n.is_finite() || n < 0.0 {
                    return Err(bad("costs must be finite and nonnegative"));
                }
            }
        }
        let enabled = |name| self.providers.get(name).is_some_and(|p| p.enabled);
        match self.work_mode {
            WorkMode::Local if !enabled("local") => {
                return Err(bad("local mode requires an enabled local provider"));
            }
            WorkMode::FullRemote if !enabled("remote") => {
                return Err(bad("full_remote mode requires an enabled remote provider"));
            }
            WorkMode::Mixed if !enabled("local") => {
                return Err(bad("mixed mode requires an enabled local provider"));
            }
            _ => {}
        }
        Ok(())
    }
}

/// Host-owned MCP registration. Server metadata never controls permissions.
#[derive(Debug, Clone, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct McpSettings {
    pub servers: std::collections::BTreeMap<String, McpServer>,
    pub timeout_ms: u64,
    pub max_response_bytes: usize,
    pub max_discovered_tools: usize,
    pub max_pages: usize,
}
impl Default for McpSettings {
    fn default() -> Self {
        Self {
            servers: Default::default(),
            timeout_ms: 10_000,
            max_response_bytes: 65_536,
            max_discovered_tools: 128,
            max_pages: 8,
        }
    }
}
#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum McpTransport {
    Stdio,
    Http,
}
/// Debug hides URLs, arguments, paths and credential variable names.
#[derive(Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct McpServer {
    #[serde(default)]
    pub enabled: bool,
    pub transport: McpTransport,
    pub command: Option<String>,
    #[serde(default)]
    pub args: Vec<String>,
    pub cwd: Option<String>,
    /// Child variable -> host variable name. Values are never stored in TOML.
    #[serde(default)]
    pub env: std::collections::BTreeMap<String, String>,
    pub url: Option<String>,
    pub bearer_token_env: Option<String>,
    /// Required for non-loopback HTTP, independently of model provider selection.
    #[serde(default)]
    pub allow_remote: bool,
    /// Alias -> exact server tool name plus host-owned effect declaration.
    #[serde(default)]
    pub tools: std::collections::BTreeMap<String, McpTool>,
}
impl std::fmt::Debug for McpServer {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("McpServer")
            .field("enabled", &self.enabled)
            .field("transport", &self.transport)
            .finish_non_exhaustive()
    }
}
#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct McpTool {
    pub name: String,
    pub effect: McpEffect,
}
#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum McpEffect {
    ReadOnly,
    SideEffect,
}
impl McpSettings {
    pub fn validate(&self) -> Result<(), OscarError> {
        let bad = || OscarError::Config("invalid MCP configuration; see docs/mcp.md".into());
        let identifier = |s: &str, max| {
            !s.is_empty()
                && s.len() <= max
                && s.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'_')
        };
        let env_name = |s: &str| {
            !s.is_empty()
                && s.len() <= 128
                && s.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'_')
        };
        if self.servers.len() > 16
            || !(1..=60_000).contains(&self.timeout_ms)
            || !(256..=1_048_576).contains(&self.max_response_bytes)
            || !(1..=128).contains(&self.max_discovered_tools)
            || !(1..=32).contains(&self.max_pages)
        {
            return Err(bad());
        }
        let mut count = 0;
        for (name, server) in &self.servers {
            if !identifier(name, 24)
                || server.tools.len() > 64
                || server.args.len() > 32
                || server
                    .args
                    .iter()
                    .any(|s| s.len() > 4096 || s.contains('\0'))
                || server.env.len() > 32
                || server.env.iter().any(|(k, v)| !env_name(k) || !env_name(v))
                || server
                    .bearer_token_env
                    .as_ref()
                    .is_some_and(|s| !env_name(s))
            {
                return Err(bad());
            }
            for (alias, tool) in &server.tools {
                if !identifier(alias, 32)
                    || tool.name.is_empty()
                    || tool.name.len() > 128
                    || tool.name.chars().any(char::is_control)
                {
                    return Err(bad());
                }
            }
            if !server.enabled {
                continue;
            }
            count += server.tools.len();
            if server.tools.is_empty() {
                return Err(bad());
            }
            match server.transport {
                McpTransport::Stdio
                    if server.command.as_ref().is_none_or(|s| {
                        s.trim().is_empty() || s.len() > 4096 || s.contains('\0')
                    }) || server.url.is_some()
                        || server.bearer_token_env.is_some()
                        || server.allow_remote =>
                {
                    return Err(bad());
                }
                McpTransport::Http
                    if server
                        .url
                        .as_ref()
                        .is_none_or(|s| s.is_empty() || s.len() > 4096)
                        || server.command.is_some()
                        || !server.args.is_empty()
                        || server.cwd.is_some()
                        || !server.env.is_empty() =>
                {
                    return Err(bad());
                }
                _ => {}
            }
        }
        if count > 127 {
            return Err(bad());
        }
        Ok(())
    }
}
