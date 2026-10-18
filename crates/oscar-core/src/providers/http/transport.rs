//! Rig transport boundary. Only validated, bounded text events reach Rig's
//! parser. OSCAR retains authoritative raw calls and usage for host validation.
use super::{stream, *};
use bytes::Bytes;
use rig::http_client::{
    self, HttpClientExt, LazyBody, MultipartForm, Request, Response, StreamingResponse,
};

#[derive(Default)]
pub(super) struct State {
    attempted: bool,
    pub result: Option<Result<ModelOutput, OscarError>>,
}

#[derive(Clone, Default)]
pub(super) struct Transport {
    pub client: Option<Client>,
    pub url: Option<Url>,
    pub state: Arc<Mutex<State>>,
    pub max_bytes: usize,
    pub max_tokens: u32,
    pub tools: bool,
}
impl std::fmt::Debug for Transport {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("Transport([redacted])")
    }
}
fn unsupported() -> http_client::Error {
    http_client::Error::Instance(Box::new(OscarError::Unavailable(
        "only bounded completion streaming is supported".into(),
    )))
}
fn event(text: &str, finish: bool) -> Bytes {
    let value = serde_json::json!({"choices":[{"delta":{"content":text},"finish_reason":if finish {Some("stop")} else {None}}]});
    Bytes::from(format!("data: {value}\n\n"))
}
impl HttpClientExt for Transport {
    fn send<T, U>(
        &self,
        _: Request<T>,
    ) -> impl Future<Output = http_client::Result<Response<LazyBody<U>>>> + Send + 'static
    where
        T: Into<Bytes> + Send,
        U: From<Bytes> + Send + 'static,
    {
        std::future::ready(Err(unsupported()))
    }
    fn send_multipart<U>(
        &self,
        _: Request<MultipartForm>,
    ) -> impl Future<Output = http_client::Result<Response<LazyBody<U>>>> + Send + 'static
    where
        U: From<Bytes> + Send + 'static,
    {
        std::future::ready(Err(unsupported()))
    }
    fn send_streaming<T>(
        &self,
        req: Request<T>,
    ) -> impl Future<Output = http_client::Result<StreamingResponse>> + Send
    where
        T: Into<Bytes>,
    {
        let (mut parts, body) = req.into_parts();
        if parts
            .headers
            .get("authorization")
            .is_some_and(|v| v.as_bytes() == b"Bearer ")
        {
            parts.headers.remove("authorization");
        }
        let body = body.into();
        let this = self.clone();
        async move {
            {
                let mut state = this.state.lock().map_err(|_| unsupported())?;
                if state.attempted {
                    return Err(unsupported());
                }
                state.attempted = true;
            }
            let client = this.client.as_ref().ok_or_else(unsupported)?;
            let url = this.url.as_ref().ok_or_else(unsupported)?;
            // Rig cannot change the host-selected endpoint or method.
            if parts.method != http_client::Method::POST || parts.uri != url.as_str() {
                return Err(unsupported());
            }
            crate::session::provider::record_wire_request(&body)
                .await
                .map_err(|_| unsupported())?;
            let response = client
                .post(url.clone())
                .headers(parts.headers)
                .body(body)
                .send()
                .await;
            let response = match response {
                Err(e) => Err(OscarError::Provider {
                    transient: e.is_timeout() || e.is_connect(),
                    message: "HTTP transport error (details redacted)".into(),
                }),
                Ok(r) if !r.status().is_success() => Err(OscarError::Provider {
                    transient: r.status().as_u16() == 429 || r.status().is_server_error(),
                    message: format!("HTTP status {}", r.status().as_u16()),
                }),
                Ok(r)
                    if !r
                        .headers()
                        .get(reqwest::header::CONTENT_TYPE)
                        .and_then(|v| v.to_str().ok())
                        .is_some_and(|v| {
                            v.split(';')
                                .next()
                                .unwrap_or_default()
                                .trim()
                                .eq_ignore_ascii_case("text/event-stream")
                        }) =>
                {
                    Err(stream::invalid("expected an SSE inference response"))
                }
                Ok(r) => Ok(r),
            };
            let shared = this.state.clone();
            let output = async_stream::stream! {
                let mut response = match response {
                    Ok(r) => r,
                    Err(e) => {
                        if let Ok(mut state) = shared.lock() { state.result = Some(Err(e)); }
                        // End normally at the SDK boundary: OSCAR reports the typed
                        // error below, without triggering SDK retries or body logs.
                        return;
                    }
                };
                let mut decoder = stream::Decoder::new(this.max_bytes, this.max_tokens);
                decoder.enable_tools(this.tools);
                loop {
                    let mut text = String::new();
                    let decoded = match response.chunk().await {
                        Ok(Some(chunk)) => decoder.feed(&chunk, &mut |delta| { text.push_str(delta); Ok(()) }),
                        Ok(None) => Err(stream::invalid("incomplete inference stream")),
                        Err(_) => Err(OscarError::Provider { transient: true, message: "response stream interrupted".into() }),
                    };
                    match decoded {
                        Err(e) => {
                            if let Ok(mut state) = shared.lock() { state.result = Some(Err(e)); }
                            break;
                        }
                        Ok(done) => {
                            if !text.is_empty() { yield Ok(event(&text, false)); }
                            if done {
                                if let Ok(mut state) = shared.lock() { state.result = Some(Ok(decoder.output())); }
                                yield Ok(event("", true));
                                break;
                            }
                        }
                    }
                }
            };
            Response::builder()
                .status(200)
                .header("content-type", "text/event-stream")
                .body(Box::pin(output) as http_client::sse::BoxedStream)
                .map_err(http_client::Error::Protocol)
        }
    }
}
