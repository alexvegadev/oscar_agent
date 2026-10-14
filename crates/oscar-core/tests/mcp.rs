#![cfg(feature = "mcp")]
use oscar_core::{
    config::{Config, McpEffect, WorkMode},
    error::{McpError, OscarError},
    mcp,
    tools::{Effect, ToolError, ToolLimits, ToolPolicy, ToolRegistry},
};
use serde_json::{Value, json};
use std::{
    io::{Read, Write},
    sync::{Arc, Mutex},
    time::{Duration, Instant},
};

type Requests = Arc<Mutex<Vec<(String, Value)>>>;
fn server(sse: bool, mode: &'static str) -> (String, Requests, std::thread::JoinHandle<()>) {
    let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
    let address = listener.local_addr().unwrap();
    listener.set_nonblocking(true).unwrap();
    let requests = Arc::new(Mutex::new(Vec::new()));
    let observed = requests.clone();
    let worker = std::thread::spawn(move || {
        let deadline = Instant::now() + Duration::from_secs(5);
        let mut sessions = 0;
        loop {
            let (mut socket, _) = loop {
                match listener.accept() {
                    Ok(socket) => break socket,
                    Err(e) if e.kind() == std::io::ErrorKind::WouldBlock => {
                        if Instant::now() >= deadline {
                            return;
                        }
                        std::thread::sleep(Duration::from_millis(5));
                    }
                    Err(_) => return,
                }
            };
            socket.set_nonblocking(false).unwrap();
            socket
                .set_read_timeout(Some(Duration::from_secs(2)))
                .unwrap();
            socket
                .set_write_timeout(Some(Duration::from_secs(2)))
                .unwrap();
            let mut bytes = Vec::new();
            let mut buffer = [0; 4096];
            let (end, length) = loop {
                let n = socket.read(&mut buffer).unwrap();
                assert!(n > 0);
                bytes.extend_from_slice(&buffer[..n]);
                assert!(bytes.len() <= 65_536);
                if let Some(end) = bytes.windows(4).position(|b| b == b"\r\n\r\n") {
                    let headers = std::str::from_utf8(&bytes[..end]).unwrap();
                    let length = headers
                        .lines()
                        .find_map(|l| {
                            l.to_ascii_lowercase()
                                .strip_prefix("content-length: ")
                                .map(|n| n.parse::<usize>().unwrap())
                        })
                        .unwrap_or(0);
                    break (end + 4, length);
                }
            };
            while bytes.len() < end + length {
                let n = socket.read(&mut buffer).unwrap();
                assert!(n > 0);
                bytes.extend_from_slice(&buffer[..n]);
            }
            let headers = std::str::from_utf8(&bytes[..end])
                .unwrap()
                .to_ascii_lowercase();
            if headers.starts_with("delete ") {
                socket
                    .write_all(b"HTTP/1.1 200 OK\r\nContent-Length: 0\r\nConnection: close\r\n\r\n")
                    .unwrap();
                sessions += 1;
                if sessions
                    == if mode == "ok" || mode == "tool_error" || mode == "large_output" {
                        2
                    } else {
                        1
                    }
                {
                    return;
                }
                continue;
            }
            let request: Value = serde_json::from_slice(&bytes[end..end + length]).unwrap();
            observed
                .lock()
                .unwrap()
                .push((headers.clone(), request.clone()));
            let method = request["method"].as_str().unwrap();
            assert!(headers.contains("application/json, text/event-stream"));
            if method != "initialize" {
                assert!(headers.contains("mcp-session-id: test-session"));
                assert!(headers.contains("mcp-protocol-version: 2025-06-18"));
            }
            if method == "notifications/initialized" {
                socket
                    .write_all(
                        b"HTTP/1.1 202 Accepted\r\nContent-Length: 0\r\nConnection: close\r\n\r\n",
                    )
                    .unwrap();
                continue;
            }
            let result = match method {
                "initialize" => {
                    json!({"protocolVersion":if mode == "version" { "invalid" } else { "2025-06-18" },"capabilities":{"tools":{}}})
                }
                "tools/list" => {
                    if request["params"].get("cursor").is_none() {
                        json!({"tools":[],"nextCursor":"next"})
                    } else {
                        let schema = if mode == "nested" {
                            json!({"type":"object","properties":{"data":{"type":"array"}}})
                        } else {
                            json!({"type":"object","properties":{"text":{"type":"string"}},"required":["text"]})
                        };
                        let tool = json!({"name":"echo","description":"Echo text","inputSchema":schema,"annotations":{"readOnlyHint":true}});
                        json!({"tools":if mode == "duplicate" { vec![tool.clone(),tool] } else { vec![tool] }})
                    }
                }
                "tools/call" => {
                    json!({"isError":mode=="tool_error","content":[{"type":"text","text":if mode=="large_output" { Value::String("x".repeat(5000)) } else { request["params"]["arguments"]["text"].clone() }}],"structuredContent":{"ok":true}})
                }
                _ => panic!("unexpected method"),
            };
            let body = json!({"jsonrpc":"2.0","id":request["id"],"result":result}).to_string();
            let body = if sse {
                format!(": heartbeat\r\n\r\nevent: message\r\ndata: {body}\r\n\r\n")
            } else {
                body
            };
            let wire = format!(
                "HTTP/1.1 200 OK\r\nMCP-Session-Id: test-session\r\nContent-Type: {}\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
                if sse {
                    "text/event-stream"
                } else {
                    "application/json"
                },
                body.len()
            );
            for chunk in wire.as_bytes().chunks(7) {
                if socket.write_all(chunk).is_err() {
                    break;
                }
            }
            if mode == "version" || mode == "duplicate" {
                return;
            }
        }
    });
    (format!("http://{address}/mcp"), requests, worker)
}
fn config(url: &str) -> Config {
    let mut config = Config::parse(include_str!("../../../settings.toml")).unwrap();
    config.features.mcp_enabled = true;
    config.mcp.servers.insert("test".into(), serde_json::from_value(json!({"enabled":true,"transport":"http","url":url,"tools":{"echo":{"name":"echo","effect":"read_only"}}})).unwrap());
    config
}
#[tokio::test]
async fn http_json_and_sse_handshake_pagination_session_headers_and_registry_calls() {
    for sse in [false, true] {
        let (url, requests, server) = server(sse, "ok");
        let registry = mcp::register_servers(Default::default(), &config(&url), Default::default())
            .await
            .unwrap();
        assert_eq!(registry.definitions().count(), 1);
        let mut session = registry
            .session(
                ToolPolicy {
                    allowed_tools: ["mcp_test_echo".into()].into(),
                    allow_side_effects: false,
                },
                ToolLimits::default(),
                None,
                Default::default(),
            )
            .unwrap();
        let result = session
            .execute_json(r#"{"id":"a","name":"mcp_test_echo","arguments":{"text":"hello"}}"#)
            .await
            .unwrap();
        assert_eq!(
            serde_json::from_str::<Value>(&result).unwrap()["content"][0]["text"],
            "hello"
        );
        assert_eq!(
            session
                .execute_json(
                    r#"{"id":"b","name":"mcp_test_echo","arguments":{"text":"x","text":"y"}}"#
                )
                .await
                .unwrap_err(),
            ToolError::InvalidCall
        );
        server.join().unwrap();
        assert_eq!(
            requests
                .lock()
                .unwrap()
                .iter()
                .filter(|(_, r)| r["method"] == "tools/call")
                .count(),
            1
        );
    }
}
#[tokio::test]
async fn unsupported_schemas_versions_and_duplicate_discovery_fail_closed() {
    for mode in ["nested", "version", "duplicate"] {
        let (url, _, server) = server(false, mode);
        let result =
            mcp::register_servers(Default::default(), &config(&url), Default::default()).await;
        assert!(result.is_err());
        server.join().unwrap();
    }
}
#[tokio::test]
async fn tool_errors_and_output_limits_are_redacted() {
    for (mode, expected) in [
        ("tool_error", ToolError::Failed),
        ("large_output", ToolError::OutputLimit),
    ] {
        let (url, _, server) = server(false, mode);
        let registry = mcp::register_servers(Default::default(), &config(&url), Default::default())
            .await
            .unwrap();
        let mut session = registry
            .session(
                ToolPolicy {
                    allowed_tools: ["mcp_test_echo".into()].into(),
                    allow_side_effects: false,
                },
                ToolLimits {
                    max_output_bytes: 1024,
                    ..Default::default()
                },
                None,
                Default::default(),
            )
            .unwrap();
        assert_eq!(
            session
                .execute_json(r#"{"id":"a","name":"mcp_test_echo","arguments":{"text":"SECRET"}}"#)
                .await
                .unwrap_err(),
            expected
        );
        server.join().unwrap();
    }
}
#[tokio::test]
async fn disabled_servers_do_not_connect_and_annotations_never_grant_permissions() {
    let mut config = config("http://127.0.0.1:1/mcp");
    config.features.mcp_enabled = false;
    assert_eq!(
        mcp::register_servers(Default::default(), &config, Default::default())
            .await
            .unwrap()
            .definitions()
            .count(),
        0
    );
    let (url, requests, server) = server(false, "side_effect");
    let mut config = self::config(&url);
    config
        .mcp
        .servers
        .get_mut("test")
        .unwrap()
        .tools
        .get_mut("echo")
        .unwrap()
        .effect = McpEffect::SideEffect;
    let registry = mcp::register_servers(ToolRegistry::default(), &config, Default::default())
        .await
        .unwrap();
    assert_eq!(
        registry.definitions().next().unwrap().effect,
        Effect::SideEffect
    );
    let mut session = registry
        .session(
            ToolPolicy {
                allowed_tools: ["mcp_test_echo".into()].into(),
                allow_side_effects: false,
            },
            ToolLimits::default(),
            None,
            Default::default(),
        )
        .unwrap();
    assert_eq!(
        session
            .execute_json(r#"{"id":"a","name":"mcp_test_echo","arguments":{"text":"x"}}"#)
            .await
            .unwrap_err(),
        ToolError::Denied
    );
    server.join().unwrap();
    assert!(
        !requests
            .lock()
            .unwrap()
            .iter()
            .any(|(_, r)| r["method"] == "tools/call")
    );
}
#[tokio::test]
async fn remote_urls_are_denied_before_credentials_or_network_access() {
    for url in [
        "http://example.invalid/mcp",
        "https://secret:token@example.invalid/mcp",
        "https://example.invalid/mcp?token=secret",
    ] {
        let mut config = config(url);
        let server = config.mcp.servers.get_mut("test").unwrap();
        server.bearer_token_env = Some("OSCAR_MCP_MISSING_TEST_SECRET".into());
        assert!(
            mcp::register_servers(Default::default(), &config, Default::default())
                .await
                .is_err()
        );
    }
    let mut config = config("https://example.invalid/mcp");
    config.work_mode = WorkMode::Local;
    config.mcp.servers.get_mut("test").unwrap().allow_remote = true;
    assert!(matches!(
        mcp::register_servers(Default::default(), &config, Default::default()).await,
        Err(OscarError::Mcp(McpError::RemoteDenied))
    ));
}
