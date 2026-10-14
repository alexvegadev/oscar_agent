//! Self-hosted Rust fixture: no Python/Node, credentials or model dependency.
use oscar_core::{
    config::Config,
    error::{McpError, OscarError},
    mcp,
    tools::{ToolError, ToolLimits, ToolPolicy, ToolRegistry},
};
use serde_json::{Value, json};
use std::io::{BufRead, Write};
use tokio_util::sync::CancellationToken;

fn fixture(mode: &str) {
    let mut output = std::io::stdout().lock();
    for line in std::io::stdin().lock().lines() {
        let message: Value = serde_json::from_str(&line.unwrap()).unwrap();
        let Some(id) = message.get("id") else {
            continue;
        };
        let result = match message["method"].as_str().unwrap() {
            "initialize" => {
                if mode == "hang" {
                    std::thread::sleep(std::time::Duration::from_secs(10));
                }
                if mode == "oversized" {
                    writeln!(output, "{}", "x".repeat(70_000)).unwrap();
                    output.flush().unwrap();
                    continue;
                }
                json!({"protocolVersion":"2025-06-18","capabilities":{"tools":{}},"serverInfo":{"name":"fixture","version":"1"}})
            }
            "tools/list" => {
                json!({"tools":[{"name":"echo","description":"Echo supplied text","inputSchema":{"type":"object","properties":{"text":{"type":"string"}},"required":["text"]},"annotations":{"readOnlyHint":false}}]})
            }
            "tools/call" => {
                if mode == "error" {
                    json!({"isError":true,"content":[{"type":"text","text":"SECRET"}]})
                } else {
                    json!({"content":[{"type":"text","text":message["params"]["arguments"]["text"]}],"structuredContent":{"echo":message["params"]["arguments"]["text"]}})
                }
            }
            _ => panic!("unexpected method"),
        };
        writeln!(
            output,
            "{}",
            json!({"jsonrpc":"2.0","id":id,"result":result})
        )
        .unwrap();
        output.flush().unwrap();
    }
}
fn config(mode: &str) -> Config {
    let mut config = Config::parse(include_str!("../../../settings.toml")).unwrap();
    config.features.mcp_enabled = true;
    let command = std::env::current_exe()
        .unwrap()
        .to_string_lossy()
        .into_owned();
    let server = serde_json::from_value(json!({"enabled":true,"transport":"stdio","command":command,"args":["--fixture",mode],"tools":{"echo":{"name":"echo","effect":"read_only"}}})).unwrap();
    config.mcp.servers.insert("local".into(), server);
    config
}
async fn test_stdio() {
    for mode in ["ok", "error"] {
        let registry = mcp::register_servers(
            ToolRegistry::default(),
            &config(mode),
            CancellationToken::new(),
        )
        .await
        .unwrap();
        assert_eq!(
            registry.definitions().next().unwrap().name,
            "mcp_local_echo"
        );
        let mut session = registry
            .session(
                ToolPolicy {
                    allowed_tools: ["mcp_local_echo".into()].into(),
                    allow_side_effects: false,
                },
                ToolLimits::default(),
                None,
                Default::default(),
            )
            .unwrap();
        let result = session
            .execute_json(r#"{"id":"call","name":"mcp_local_echo","arguments":{"text":"hello"}}"#)
            .await;
        if mode == "ok" {
            assert_eq!(
                serde_json::from_str::<Value>(&result.unwrap()).unwrap()["structuredContent"]["echo"],
                "hello"
            );
        } else {
            assert_eq!(result.unwrap_err(), ToolError::Failed);
        }
    }
    let mut config = config("hang");
    config.mcp.timeout_ms = 50;
    let result = mcp::register_servers(Default::default(), &config, Default::default()).await;
    assert!(matches!(result, Err(OscarError::Mcp(McpError::Timeout))));
    config.mcp.timeout_ms = 10_000;
    let cancellation = CancellationToken::new();
    cancellation.cancel();
    assert!(matches!(
        mcp::register_servers(Default::default(), &config, cancellation).await,
        Err(OscarError::Cancelled)
    ));
    let oversized = mcp::register_servers(
        Default::default(),
        &self::config("oversized"),
        Default::default(),
    )
    .await;
    assert!(matches!(oversized, Err(OscarError::Limit(_))));
    println!(
        "MCP stdio: handshake, discovery, calls, errors, timeout, cancellation and byte limits passed"
    );
}
fn main() {
    let args: Vec<_> = std::env::args().collect();
    if args.get(1).map(String::as_str) == Some("--fixture") {
        fixture(args.get(2).unwrap());
        return;
    }
    tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .unwrap()
        .block_on(test_stdio());
}
