use super::*;
use futures_util::StreamExt;
use reqwest::{Client, Url};
use rig::{completion::CompletionModel, prelude::CompletionClient};
use std::{net::IpAddr, time::Duration};
use tracing::instrument::WithSubscriber;

mod transport;

pub(super) struct HttpProvider {
    client: Client,
    url: Url,
    model: String,
    key: Option<String>,
    timeout_ms: u64,
}
impl HttpProvider {
    pub(super) fn new(p: &Provider, local: bool, timeout_ms: u64) -> Result<Self, OscarError> {
        let invalid = || {
            OscarError::Config("api_url must be a credential-free absolute chat/completions URL; local requires a loopback IP, remote requires HTTPS".into())
        };
        let endpoint = p
            .api_url
            .as_deref()
            .or_else(|| default_endpoint(p.provider.as_deref()));
        let url = Url::parse(endpoint.ok_or_else(invalid)?).map_err(|_| invalid())?;
        if !url.path().ends_with("/chat/completions") || timeout_ms == 0 {
            return Err(invalid());
        }
        if !url.username().is_empty()
            || url.password().is_some()
            || url.query().is_some()
            || url.fragment().is_some()
            || !matches!(url.scheme(), "http" | "https")
            || url.host_str().is_none()
        {
            return Err(invalid());
        }
        if local {
            // Literal IPs avoid DNS rebinding and accidental local-mode cloud egress.
            let host = url.host_str().unwrap_or_default().trim_matches(['[', ']']);
            if !host.parse::<IpAddr>().is_ok_and(|ip| ip.is_loopback()) {
                return Err(invalid());
            }
        } else if url.scheme() != "https" {
            return Err(invalid());
        }
        let key = if let Some(env) = &p.api_key_env {
            Some(std::env::var(env).map_err(|_| {
                OscarError::Config("configured API key environment variable is missing".into())
            })?)
        } else {
            p.api_key.clone()
        };
        let client = Client::builder()
            .no_proxy()
            .redirect(reqwest::redirect::Policy::none())
            .retry(reqwest::retry::never())
            .timeout(Duration::from_millis(timeout_ms))
            .connect_timeout(Duration::from_millis(timeout_ms.min(10_000)))
            .build()
            .map_err(|_| OscarError::Config("cannot initialize HTTP client".into()))?;
        Ok(Self {
            client,
            url,
            model: p.model.clone(),
            key,
            timeout_ms,
        })
    }
}
impl ModelProvider for HttpProvider {
    fn native_tool_protocol(&self) -> bool {
        true
    }
    fn infer(&self, request: ModelRequest) -> ModelFuture<'_> {
        Box::pin(async move { self.infer_stream(request, &mut |_| Ok(())).await })
    }

    fn infer_stream<'a>(
        &'a self,
        request: ModelRequest,
        sink: &'a mut ModelTextSink<'_>,
    ) -> ModelFuture<'a> {
        Box::pin(async move {
            let transport = transport::Transport {
                client: Some(self.client.clone()),
                url: Some(self.url.clone()),
                max_bytes: request.max_output_bytes,
                max_tokens: request.max_output_tokens,
                tools: !request.tools.is_empty(),
                ..Default::default()
            };
            let base = self
                .url
                .as_str()
                .strip_suffix("/chat/completions")
                .ok_or_else(|| OscarError::Config("invalid completion endpoint".into()))?;
            let client = rig::providers::openai::CompletionsClient::builder()
                .api_key(self.key.clone().unwrap_or_default())
                .base_url(base)
                .http_client(transport.clone())
                .build()
                .map_err(|_| {
                    OscarError::Config("cannot initialize Rig client (details redacted)".into())
                })?;
            let model = client.completion_model(&self.model);
            let rig_request = super::chat::rig_request(&request)?;
            // Rig's trace spans can capture message content. Disable its subscriber
            // only while polling this request, without changing the host subscriber.
            let inference = async {
                let mut response = model
                    .stream(rig_request)
                    .await
                    .map_err(|_| stream::invalid("Rig request failed (details redacted)"))?;
                while let Some(chunk) = response.next().await {
                    match chunk {
                        Ok(rig::streaming::StreamedAssistantContent::Text(text)) => {
                            sink(&text.text)?
                        }
                        Ok(_) => {}
                        Err(_) => {
                            return Err(stream::invalid("Rig stream failed (details redacted)"));
                        }
                    }
                }
                transport
                    .state
                    .lock()
                    .map_err(|_| stream::invalid("Rig transport state unavailable"))?
                    .result
                    .take()
                    .ok_or_else(|| stream::invalid("incomplete inference stream"))?
            };
            tokio::time::timeout(
                Duration::from_millis(self.timeout_ms),
                inference.with_subscriber(tracing::subscriber::NoSubscriber::default()),
            )
            .await
            .map_err(|_| OscarError::Provider {
                transient: true,
                message: "inference deadline exceeded".into(),
            })?
        })
    }
}

pub(super) fn default_endpoint(provider: Option<&str>) -> Option<&'static str> {
    match provider? {
        "openai" => Some("https://api.openai.com/v1/chat/completions"),
        "openrouter" => Some("https://openrouter.ai/api/v1/chat/completions"),
        "groq" => Some("https://api.groq.com/openai/v1/chat/completions"),
        "deepseek" => Some("https://api.deepseek.com/v1/chat/completions"),
        "xai" => Some("https://api.x.ai/v1/chat/completions"),
        "ollama" => Some("http://127.0.0.1:11434/v1/chat/completions"),
        "lmstudio" => Some("http://127.0.0.1:1234/v1/chat/completions"),
        _ => None,
    }
}

mod stream;

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::{Read, Write};

    fn provider(url: &str) -> Provider {
        let c = Config::parse(include_str!("../../../../settings.toml")).unwrap();
        let mut p = c.providers["local"].clone();
        p.api_url = Some(url.into());
        p
    }
    #[test]
    fn endpoint_policy_rejects_cloud_local_dns_secrets_and_insecure_remote() {
        for url in [
            "https://api.example.com/v1/chat/completions",
            "http://localhost:1234/v1/chat/completions",
            "http://user:secret@127.0.0.1/v1/chat/completions",
            "http://127.0.0.1/v1/chat/completions?key=secret",
        ] {
            assert!(HttpProvider::new(&provider(url), true, 100).is_err());
        }
        assert!(
            HttpProvider::new(
                &provider("http://127.0.0.1:1234/v1/chat/completions"),
                true,
                100
            )
            .is_ok()
        );
        assert!(
            HttpProvider::new(
                &provider("https://api.example.com/v1/chat/completions"),
                false,
                100
            )
            .is_ok()
        );
        assert!(
            HttpProvider::new(
                &provider("http://api.example.com/v1/chat/completions"),
                false,
                100
            )
            .is_err()
        );
    }
    #[test]
    fn presets_use_the_same_endpoint_policy_and_unknown_protocols_fail() {
        let example =
            Config::parse(include_str!("../../../../examples/rig-providers.toml")).unwrap();
        assert!(Providers::from_config(&example).is_ok());
        for name in ["openai", "openrouter", "groq", "deepseek", "xai"] {
            let mut p = provider("https://example.com/v1/chat/completions");
            p.provider = Some(name.into());
            p.api_url = None;
            assert!(HttpProvider::new(&p, false, 100).is_ok());
            assert!(HttpProvider::new(&p, true, 100).is_err());
        }
        for name in ["ollama", "lmstudio"] {
            let mut p = provider("");
            p.provider = Some(name.into());
            p.api_url = None;
            assert!(HttpProvider::new(&p, true, 100).is_ok());
        }
        let mut config = Config::parse(include_str!("../../../../settings.toml")).unwrap();
        config.providers.get_mut("local").unwrap().provider = Some("unknown_protocol".into());
        assert!(Providers::from_config(&config).is_err());
        let p = config.providers.get_mut("local").unwrap();
        p.protocol = Some("openai_chat".into());
        p.api_url = Some("http://127.0.0.1:1234/v1/chat/completions".into());
        assert!(Providers::from_config(&config).is_ok());
        config.providers.get_mut("local").unwrap().protocol = Some("unknown_wire_format".into());
        assert!(Providers::from_config(&config).is_err());
    }

    #[tokio::test]
    async fn transport_preserves_failure_and_refuses_sdk_retries() {
        use rig::http_client::{HttpClientExt, Request};
        let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let address = listener.local_addr().unwrap();
        let server = std::thread::spawn(move || {
            let (mut socket, _) = listener.accept().unwrap();
            socket
                .set_read_timeout(Some(Duration::from_secs(2)))
                .unwrap();
            socket
                .set_write_timeout(Some(Duration::from_secs(2)))
                .unwrap();
            let mut bytes = [0; 4096];
            assert!(socket.read(&mut bytes).unwrap() > 0);
            socket.write_all(b"HTTP/1.1 503 Unavailable\r\nContent-Length: 6\r\nConnection: close\r\n\r\nSECRET").unwrap();
        });
        let url = format!("http://{address}/v1/chat/completions");
        let transport = transport::Transport {
            client: Some(
                Client::builder()
                    .timeout(Duration::from_secs(2))
                    .build()
                    .unwrap(),
            ),
            url: Some(Url::parse(&url).unwrap()),
            ..Default::default()
        };
        let request = || {
            Request::builder()
                .method("POST")
                .uri(&url)
                .body(Vec::<u8>::new())
                .unwrap()
        };
        let response = transport.send_streaming(request()).await.unwrap();
        assert!(response.into_body().next().await.is_none());
        assert!(matches!(
            transport.state.lock().unwrap().result,
            Some(Err(OscarError::Provider {
                transient: true,
                ..
            }))
        ));
        assert!(transport.send_streaming(request()).await.is_err());
        assert!(!format!("{:?}", transport.state.lock().unwrap().result).contains("SECRET"));
        tokio::task::spawn_blocking(move || server.join().unwrap())
            .await
            .unwrap();
    }

    #[tokio::test]
    async fn rig_sends_native_history_and_schemas_and_preserves_raw_arguments() {
        use crate::tools::{Effect, Field, InputSchema, InputType, ToolDefinition};
        use serde_json::json;
        let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let address = listener.local_addr().unwrap();
        let server = std::thread::spawn(move || {
            let (mut socket, _) = listener.accept().unwrap();
            socket
                .set_read_timeout(Some(Duration::from_secs(2)))
                .unwrap();
            socket
                .set_write_timeout(Some(Duration::from_secs(2)))
                .unwrap();
            let mut bytes = Vec::new();
            let mut chunk = [0; 4096];
            let request = loop {
                let n = socket.read(&mut chunk).unwrap();
                assert!(n > 0);
                bytes.extend_from_slice(&chunk[..n]);
                assert!(bytes.len() < 65_536);
                if let Some(end) = bytes.windows(4).position(|w| w == b"\r\n\r\n") {
                    let headers = std::str::from_utf8(&bytes[..end])
                        .unwrap()
                        .to_ascii_lowercase();
                    assert!(headers.starts_with("post /v1/chat/completions "));
                    assert!(!headers.contains("authorization:"));
                    let length: usize = headers
                        .lines()
                        .find_map(|s| s.strip_prefix("content-length: "))
                        .unwrap()
                        .parse()
                        .unwrap();
                    if bytes.len() >= end + 4 + length {
                        break serde_json::from_slice::<serde_json::Value>(
                            &bytes[end + 4..end + 4 + length],
                        )
                        .unwrap();
                    }
                }
            };
            let body = format!(
                "data: {}\n\ndata: [DONE]\n\n",
                json!({"choices":[{"index":0,"delta":{"tool_calls":[{"index":0,"id":"next","type":"function","function":{"name":"inspect","arguments":"{\"text\":\"first\",\"text\":\"second\"}"}}]},"finish_reason":"tool_calls"}],"usage":{"prompt_tokens":11,"completion_tokens":3}})
            );
            socket.write_all(format!("HTTP/1.1 200 OK\r\nContent-Type: text/event-stream\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}", body.len()).as_bytes()).unwrap();
            request
        });
        let p = HttpProvider::new(
            &provider(&format!("http://{address}/v1/chat/completions")),
            true,
            2000,
        )
        .unwrap();
        let request = ModelRequest {
            context: "legacy context".into(),
            messages: vec![
                ModelMessage::User {
                    content: "inspect data".into(),
                },
                ModelMessage::Assistant {
                    content: String::new(),
                    tool_calls: vec![ModelToolCall {
                        id: "previous".into(),
                        name: "inspect".into(),
                        arguments: r#"{"text":"sample"}"#.into(),
                    }],
                },
                ModelMessage::Tool {
                    tool_call_id: "previous".into(),
                    content: "tool evidence".into(),
                },
            ],
            tools: vec![ToolDefinition {
                name: "inspect".into(),
                description: "Inspect text".into(),
                effect: Effect::ReadOnly,
                input: InputSchema {
                    fields: [(
                        "text".into(),
                        Field {
                            required: true,
                            value_type: InputType::Text { max_bytes: 100 },
                        },
                    )]
                    .into(),
                },
            }],
            max_output_bytes: 1000,
            max_output_tokens: 10,
        };
        let expected_bytes = request.input_bytes(&p.model).unwrap();
        let output = p.infer(request).await.unwrap();
        assert_eq!(output.tool_calls[0].id, "next");
        assert_eq!(
            output.tool_calls[0].arguments,
            r#"{"text":"first","text":"second"}"#
        );
        assert_eq!(output.usage.input, Some(11));
        let wire = tokio::task::spawn_blocking(move || server.join().unwrap())
            .await
            .unwrap();
        assert_eq!(wire.to_string().len(), expected_bytes);
        assert_eq!(wire["messages"][2]["tool_calls"][0]["id"], "previous");
        assert_eq!(wire["messages"][3]["role"], "tool");
        assert_eq!(wire["messages"][3]["tool_call_id"], "previous");
        assert_eq!(
            wire["tools"][0]["function"]["parameters"]["additionalProperties"],
            false
        );
        assert_eq!(wire["stream"], true);
    }
    async fn response(status: &str, body: &str, max: usize) -> Result<ModelOutput, OscarError> {
        let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let address = listener.local_addr().unwrap();
        let body = if serde_json::from_str::<serde_json::Value>(body).is_ok() {
            format!("data: {body}\n\ndata: [DONE]\n\n")
        } else {
            body.to_owned()
        };
        let wire = format!(
            "HTTP/1.1 {status}\r\nContent-Type: text/event-stream\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
            body.len()
        );
        let server = std::thread::spawn(move || {
            let (mut socket, _) = listener.accept().unwrap();
            socket
                .set_read_timeout(Some(Duration::from_secs(2)))
                .unwrap();
            socket
                .set_write_timeout(Some(Duration::from_secs(2)))
                .unwrap();
            let mut request = [0; 8192];
            let _ = socket.read(&mut request);
            let _ = socket.write_all(wire.as_bytes());
        });
        let p = HttpProvider::new(
            &provider(&format!("http://{address}/v1/chat/completions")),
            true,
            2000,
        )
        .unwrap();
        let result = p
            .infer(ModelRequest {
                messages: Vec::new(),
                tools: Vec::new(),
                context: "test".into(),
                max_output_bytes: max,
                max_output_tokens: 10,
            })
            .await;
        server.join().unwrap();
        result
    }
    #[tokio::test]
    async fn http_decodes_usage_and_classifies_status_without_leaking_body() {
        let body = r#"{"choices":[{"index":0,"delta":{"content":"hello"},"finish_reason":"stop"}],"usage":{"prompt_tokens":12,"completion_tokens":2}}"#;
        let result = response("200 OK", body, 100).await.unwrap();
        assert_eq!(result.content, "hello");
        assert_eq!(result.usage.input, Some(12));
        for (status, transient) in [("429 Too Many Requests", true), ("401 Unauthorized", false)] {
            let error = response(status, "TOP_SECRET", 100).await.unwrap_err();
            assert!(matches!(error, OscarError::Provider { transient: t, .. } if t == transient));
            assert!(!error.to_string().contains("TOP_SECRET"));
        }
    }
    #[tokio::test]
    async fn rig_message_tracing_is_suppressed_per_request() {
        use std::sync::atomic::{AtomicUsize, Ordering};
        struct RecordingSubscriber(Arc<AtomicUsize>);
        impl tracing::Subscriber for RecordingSubscriber {
            fn enabled(&self, _: &tracing::Metadata<'_>) -> bool {
                true
            }
            fn new_span(&self, _: &tracing::span::Attributes<'_>) -> tracing::span::Id {
                self.0.fetch_add(1, Ordering::SeqCst);
                tracing::span::Id::from_u64(1)
            }
            fn record(&self, _: &tracing::span::Id, _: &tracing::span::Record<'_>) {
                self.0.fetch_add(1, Ordering::SeqCst);
            }
            fn record_follows_from(&self, _: &tracing::span::Id, _: &tracing::span::Id) {}
            fn event(&self, _: &tracing::Event<'_>) {
                self.0.fetch_add(1, Ordering::SeqCst);
            }
            fn enter(&self, _: &tracing::span::Id) {}
            fn exit(&self, _: &tracing::span::Id) {}
            fn max_level_hint(&self) -> Option<tracing::level_filters::LevelFilter> {
                Some(tracing::level_filters::LevelFilter::TRACE)
            }
        }
        let captured = Arc::new(AtomicUsize::new(0));
        let output = response(
            "200 OK",
            r#"{"choices":[{"index":0,"delta":{"content":"PRIVATE"},"finish_reason":"stop"}]}"#,
            100,
        )
        .with_subscriber(RecordingSubscriber(captured.clone()))
        .await
        .unwrap();
        assert_eq!(output.content, "PRIVATE");
        assert_eq!(captured.load(Ordering::SeqCst), 0);
    }
    #[tokio::test]
    async fn compatible_empty_tool_arrays_do_not_interrupt_text_streaming() {
        let body = r#"data: {"choices":[{"index":0,"delta":{"role":"assistant","content":"","tool_calls":[]}}]}

data: {"choices":[{"index":0,"delta":{"content":"hello","tool_calls":[]}}]}

data: {"choices":[{"index":0,"delta":{"content":" world","tool_calls":[]},"finish_reason":"stop"}]}

data: [DONE]

"#;
        let output = response("200 OK", body, 100).await.unwrap();
        assert_eq!(output.content, "hello world");
        assert!(output.tool_calls.is_empty());
    }
    #[tokio::test]
    async fn malformed_tools_and_oversized_responses_fail_closed() {
        for body in [
            "{}",
            r#"{"choices":[{"index":0,"delta":{"content":"text","tool_calls":[{"name":"shell"}]}}]}"#,
        ] {
            assert!(response("200 OK", body, 100).await.is_err());
        }
        assert!(matches!(
            response(
                "200 OK",
                r#"{"choices":[{"index":0,"delta":{"content":"too long"},"finish_reason":"stop"}]}"#,
                2
            )
            .await,
            Err(OscarError::Limit(_))
        ));
        assert!(matches!(
            response("200 OK", &"x".repeat(70_000), 2).await,
            Err(OscarError::Limit(_))
        ));
    }
    #[tokio::test]
    async fn native_delta_arrives_before_server_completes_and_drop_closes_request() {
        for cancel in [false, true] {
            let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
            let address = listener.local_addr().unwrap();
            let (release, wait) = std::sync::mpsc::channel();
            let server = std::thread::spawn(move || {
                let (mut socket, _) = listener.accept().unwrap();
                socket
                    .set_read_timeout(Some(Duration::from_secs(2)))
                    .unwrap();
                socket
                    .set_write_timeout(Some(Duration::from_secs(2)))
                    .unwrap();
                let mut bytes = [0; 8192];
                let _ = socket.read(&mut bytes).unwrap();
                let first =
                    "data: {\"choices\":[{\"index\":0,\"delta\":{\"content\":\"first\"}}]}\n\n";
                socket.write_all(format!("HTTP/1.1 200 OK\r\nContent-Type: text/event-stream\r\nConnection: close\r\n\r\n{first}").as_bytes()).unwrap();
                // The server cannot finish until the client has delivered text.
                wait.recv_timeout(Duration::from_secs(2)).unwrap();
                if !cancel {
                    socket.write_all(b"data: {\"choices\":[{\"index\":0,\"delta\":{},\"finish_reason\":\"stop\"}]}\n\ndata: [DONE]\n\n").unwrap();
                } else {
                    // No further body is sent; dropping the future releases HTTP.
                    assert_eq!(socket.read(&mut bytes).unwrap(), 0);
                }
            });
            let p = HttpProvider::new(
                &provider(&format!("http://{address}/v1/chat/completions")),
                true,
                2000,
            )
            .unwrap();
            let token = tokio_util::sync::CancellationToken::new();
            let mut deltas = Vec::new();
            let mut sink = |delta: &str| {
                deltas.push(delta.to_owned());
                release.send(()).unwrap();
                if cancel {
                    token.cancel();
                }
                Ok(())
            };
            let result = tokio::select! {
                biased;
                _ = token.cancelled() => Err(OscarError::Cancelled),
                result = p.infer_stream(ModelRequest { messages: Vec::new(), tools: Vec::new(), context: "test".into(), max_output_bytes: 100, max_output_tokens: 10 }, &mut sink) => result,
            };
            assert_eq!(deltas, ["first"]);
            if cancel {
                assert!(matches!(result, Err(OscarError::Cancelled)));
            } else {
                assert_eq!(result.unwrap().content, "first");
            }
            tokio::task::spawn_blocking(move || server.join().unwrap())
                .await
                .unwrap();
        }
    }
}
