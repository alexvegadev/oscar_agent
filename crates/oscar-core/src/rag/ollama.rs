use super::{EmbeddingFuture, EmbeddingProvider};
use crate::error::OscarError;
use reqwest::{Client, Url};
use serde::Deserialize;
use std::{net::IpAddr, time::Duration};

/// Bounded local Ollama `/api/embed` transport. No redirects, DNS, proxies,
/// credentials, retries or automatic model downloads.
pub struct LocalOllamaEmbeddings {
    client: Client,
    url: Url,
    model: String,
}
impl LocalOllamaEmbeddings {
    pub fn new(endpoint: &str, model: &str, timeout_ms: u64) -> Result<Self, OscarError> {
        let invalid = || {
            OscarError::Config(
                "embedding endpoint requires a credential-free literal loopback /api/embed URL"
                    .into(),
            )
        };
        let url = Url::parse(endpoint).map_err(|_| invalid())?;
        let host = url.host_str().unwrap_or_default().trim_matches(['[', ']']);
        if !matches!(url.scheme(), "http" | "https")
            || !host.parse::<IpAddr>().is_ok_and(|ip| ip.is_loopback())
            || !url.username().is_empty()
            || url.password().is_some()
            || url.query().is_some()
            || url.fragment().is_some()
            || url.path() != "/api/embed"
            || model.trim().is_empty()
            || model.len() > 256
            || timeout_ms == 0
        {
            return Err(invalid());
        }
        let client = Client::builder()
            .no_proxy()
            .redirect(reqwest::redirect::Policy::none())
            .timeout(Duration::from_millis(timeout_ms))
            .connect_timeout(Duration::from_millis(timeout_ms.min(10_000)))
            .build()
            .map_err(|_| OscarError::Config("cannot initialize embedding client".into()))?;
        Ok(Self {
            client,
            url,
            model: model.into(),
        })
    }
}
#[derive(Deserialize)]
struct Response {
    embeddings: Vec<Vec<f64>>,
}
impl EmbeddingProvider for LocalOllamaEmbeddings {
    fn embed(&self, texts: Vec<String>) -> EmbeddingFuture<'_> {
        Box::pin(async move {
            if texts.is_empty()
                || texts.len() > 128
                || texts.iter().map(String::len).sum::<usize>() > 131_072
            {
                return Err(OscarError::Limit(
                    "embedding request exceeds transport bounds".into(),
                ));
            }
            let mut response = self
                .client
                .post(self.url.clone())
                .json(&serde_json::json!({
                    "model": self.model, "input": texts, "truncate": false
                }))
                .send()
                .await
                .map_err(|_| OscarError::Provider {
                    transient: false,
                    message: "embedding transport failed (details redacted)".into(),
                })?;
            if !response.status().is_success() {
                return Err(OscarError::Provider {
                    transient: false,
                    message: format!("embedding HTTP status {}", response.status().as_u16()),
                });
            }
            let mut bytes = Vec::new();
            // Cap encoded data before JSON allocation; RagSession validates shape.
            while let Some(chunk) = response.chunk().await.map_err(|_| OscarError::Provider {
                transient: false,
                message: "embedding response interrupted".into(),
            })? {
                if bytes.len().saturating_add(chunk.len()) > 4_194_304 {
                    return Err(OscarError::Limit(
                        "embedding response wire limit exceeded".into(),
                    ));
                }
                bytes.extend_from_slice(&chunk);
            }
            let response: Response = serde_json::from_slice(&bytes)
                .map_err(|_| OscarError::Validation("malformed embedding response".into()))?;
            Ok(response.embeddings)
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::{Read, Write};
    #[test]
    fn endpoints_cannot_escape_loopback_or_carry_credentials() {
        for url in [
            "https://example.com/api/embed",
            "http://localhost/api/embed",
            "http://user:password@127.0.0.1/api/embed",
            "http://127.0.0.1/api/embed?key=secret",
            "http://127.0.0.1/api/embed#secret",
            "file:///api/embed",
            "http://127.0.0.1/api/generate",
        ] {
            assert!(LocalOllamaEmbeddings::new(url, "model", 100).is_err());
        }
        assert!(LocalOllamaEmbeddings::new("http://[::1]:11434/api/embed", "model", 100).is_ok());
    }
    async fn response(status: &str, body: &str) -> Result<Vec<Vec<f64>>, OscarError> {
        let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let address = listener.local_addr().unwrap();
        let wire = format!(
            "HTTP/1.1 {status}\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
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
            let n = socket.read(&mut request).unwrap();
            let request = String::from_utf8_lossy(&request[..n]);
            assert!(request.contains("/api/embed"));
            let _ = socket.write_all(wire.as_bytes());
        });
        let model =
            LocalOllamaEmbeddings::new(&format!("http://{address}/api/embed"), "fixture", 2000)
                .unwrap();
        let result = model.embed(vec!["test".into()]).await;
        server.join().unwrap();
        result
    }
    #[tokio::test]
    async fn transport_decodes_embeddings_and_redacts_failures() {
        assert_eq!(
            response("200 OK", r#"{"embeddings":[[1.0,0.0]]}"#)
                .await
                .unwrap(),
            vec![vec![1.0, 0.0]]
        );
        for (status, body) in [
            ("302 Found", "secret"),
            ("500 Error", "secret"),
            ("200 OK", "secret"),
        ] {
            let error = response(status, body).await.unwrap_err();
            assert!(!error.to_string().contains("secret"));
        }
        assert!(matches!(
            response("200 OK", &"x".repeat(4_194_305)).await,
            Err(OscarError::Limit(_))
        ));
    }
}
