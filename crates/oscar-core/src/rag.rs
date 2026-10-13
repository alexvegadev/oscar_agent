//! Experimental, session-scoped retrieval. Documents are supplied by the host;
//! this module cannot discover files, invoke tools, or authorize remote inference.
use crate::{error::OscarError, execution::CancellationToken};
use rig::{
    OneOrMany,
    embeddings::{Embedding, EmbeddingError, EmbeddingModel},
    vector_store::{VectorStoreIndex, in_memory_store::InMemoryVectorStore},
};
use serde::{Deserialize, Serialize};
use std::{future::Future, pin::Pin, sync::Arc, time::Duration};

mod ollama;
pub use ollama::LocalOllamaEmbeddings;

pub type EmbeddingFuture<'a> =
    Pin<Box<dyn Future<Output = Result<Vec<Vec<f64>>, OscarError>> + Send + 'a>>;

/// Host-owned embedding seam. Return one vector per input in the same order.
/// Hosts must enforce locality for injected providers; the CLI uses loopback only.
pub trait EmbeddingProvider: Send + Sync {
    fn embed(&self, texts: Vec<String>) -> EmbeddingFuture<'_>;
}

/// Explicit bounds, independent of inference budgets. Defaults fit small sessions.
#[derive(Clone, Debug)]
pub struct RagLimits {
    pub max_document_bytes: usize,
    pub max_index_bytes: usize,
    pub max_chunks: usize,
    pub chunk_bytes: usize,
    pub top_k: usize,
    /// Includes JSON escaping, citation metadata and the enclosing JSON array.
    pub max_context_bytes: usize,
    pub max_embedding_calls: usize,
    pub max_dimensions: usize,
    pub call_timeout_ms: u64,
}
impl Default for RagLimits {
    fn default() -> Self {
        Self {
            max_document_bytes: 16_384,
            max_index_bytes: 131_072,
            max_chunks: 128,
            chunk_bytes: 2048,
            top_k: 4,
            max_context_bytes: 8192,
            max_embedding_calls: 256,
            max_dimensions: 4096,
            call_timeout_ms: 60_000,
        }
    }
}
impl RagLimits {
    fn validate(&self) -> Result<(), OscarError> {
        if self.chunk_bytes < 4
            || self.chunk_bytes > self.max_document_bytes
            || self.max_document_bytes > self.max_index_bytes
            || self.max_chunks == 0
            || self.top_k == 0
            || self.top_k > self.max_chunks
            || self.max_context_bytes < 2
            || self.max_embedding_calls == 0
            || self.max_dimensions == 0
            || self.call_timeout_ms == 0
        {
            return Err(OscarError::Config("invalid RAG limits".into()));
        }
        Ok(())
    }
}

/// Source coordinates are UTF-8 byte offsets in the supplied document snapshot.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Evidence {
    pub source: String,
    pub start_byte: usize,
    pub end_byte: usize,
    pub text: String,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Retrieval {
    pub evidence: Vec<Evidence>,
    /// Retrieved chunks omitted to satisfy the encoded context budget.
    pub omitted: usize,
}

/// A bounded in-memory Rig index. Reindexing a source replaces its snapshot
/// atomically. Failed/cancelled embedding attempts consume the call budget.
pub struct RagSession {
    provider: Arc<dyn EmbeddingProvider>,
    limits: RagLimits,
    store: InMemoryVectorStore<Evidence>,
    dimensions: Option<usize>,
    calls: usize,
}
impl RagSession {
    pub fn new(
        provider: Arc<dyn EmbeddingProvider>,
        limits: RagLimits,
    ) -> Result<Self, OscarError> {
        limits.validate()?;
        Ok(Self {
            provider,
            limits,
            store: InMemoryVectorStore::from_documents_with_ids(Vec::<(
                String,
                Evidence,
                OneOrMany<Embedding>,
            )>::new()),
            dimensions: None,
            calls: 0,
        })
    }
    pub fn chunks(&self) -> usize {
        self.store.len()
    }
    pub fn calls_used(&self) -> usize {
        self.calls
    }
    /// Discard all content, retaining budgets and embedding-space identity.
    pub fn clear(&mut self) {
        self.store = InMemoryVectorStore::from_documents_with_ids(Vec::<(
            String,
            Evidence,
            OneOrMany<Embedding>,
        )>::new());
    }
    async fn embed(
        &mut self,
        texts: Vec<String>,
        cancel: &CancellationToken,
    ) -> Result<Vec<Embedding>, OscarError> {
        if cancel.is_cancelled() {
            return Err(OscarError::Cancelled);
        }
        if self.calls >= self.limits.max_embedding_calls {
            return Err(OscarError::Limit(
                "RAG embedding call budget exhausted".into(),
            ));
        }
        self.calls += 1;
        let vectors = tokio::select! {
            biased;
            _ = cancel.cancelled() => return Err(OscarError::Cancelled),
            result = tokio::time::timeout(Duration::from_millis(self.limits.call_timeout_ms), self.provider.embed(texts.clone())) =>
                result.map_err(|_| OscarError::Limit("RAG embedding deadline exceeded".into()))??,
        };
        if vectors.len() != texts.len() {
            return Err(OscarError::Validation("embedding count mismatch".into()));
        }
        let dimensions = vectors.first().map_or(0, Vec::len);
        if dimensions == 0
            || dimensions > self.limits.max_dimensions
            || self.dimensions.is_some_and(|d| d != dimensions)
            || vectors.iter().any(|v| {
                let norm: f64 = v.iter().map(|x| x * x).sum();
                v.len() != dimensions
                    || v.iter().any(|x| !x.is_finite())
                    || !norm.is_finite()
                    || norm <= 0.0
            })
        {
            return Err(OscarError::Validation(
                "invalid embedding vectors or changed dimensions".into(),
            ));
        }
        self.dimensions = Some(dimensions);
        // Normalize before Rig similarity search to prevent magnitude overflow.
        Ok(texts
            .into_iter()
            .zip(vectors)
            .map(|(document, mut vec)| {
                let norm = vec.iter().map(|x| x * x).sum::<f64>().sqrt();
                for x in &mut vec {
                    *x /= norm;
                }
                Embedding { document, vec }
            })
            .collect())
    }
    pub async fn index(
        &mut self,
        source: &str,
        text: &str,
        cancel: &CancellationToken,
    ) -> Result<usize, OscarError> {
        if source.is_empty() || source.len() > 1024 || text.trim().is_empty() {
            return Err(OscarError::Validation(
                "RAG requires a source and nonempty document".into(),
            ));
        }
        if text.len() > self.limits.max_document_bytes {
            return Err(OscarError::Limit("RAG document byte limit exceeded".into()));
        }
        let retained: Vec<_> = self
            .store
            .iter()
            .filter(|(_, (d, _))| d.source != source)
            .map(|(id, pair)| (id.clone(), pair.0.clone(), pair.1.clone()))
            .collect();
        let retained_bytes: usize = retained.iter().map(|(_, d, _)| d.text.len()).sum();
        let mut chunks = Vec::new();
        let mut start = 0;
        while start < text.len() {
            let mut end = start
                .saturating_add(self.limits.chunk_bytes)
                .min(text.len());
            while !text.is_char_boundary(end) {
                end -= 1;
            }
            chunks.push(Evidence {
                source: source.into(),
                start_byte: start,
                end_byte: end,
                text: text[start..end].into(),
            });
            start = end;
        }
        if retained.len().saturating_add(chunks.len()) > self.limits.max_chunks
            || retained_bytes.saturating_add(text.len()) > self.limits.max_index_bytes
        {
            return Err(OscarError::Limit("RAG index capacity exceeded".into()));
        }
        let embeddings = self
            .embed(chunks.iter().map(|c| c.text.clone()).collect(), cancel)
            .await?;
        let count = chunks.len();
        let added = chunks.into_iter().zip(embeddings).map(|(d, e)| {
            // JSON tuple encoding prevents source/offset ID collisions.
            let id = serde_json::json!([d.source, d.start_byte]).to_string();
            (id, d, OneOrMany::one(e))
        });
        self.store =
            InMemoryVectorStore::from_documents_with_ids(retained.into_iter().chain(added));
        Ok(count)
    }
    pub async fn retrieve(
        &mut self,
        query: &str,
        cancel: &CancellationToken,
    ) -> Result<Retrieval, OscarError> {
        if cancel.is_cancelled() {
            return Err(OscarError::Cancelled);
        }
        if query.trim().is_empty() || query.len() > self.limits.max_document_bytes {
            return Err(OscarError::Validation("invalid RAG query length".into()));
        }
        if self.store.is_empty() {
            return Ok(Retrieval {
                evidence: vec![],
                omitted: 0,
            });
        }
        let embedding = self.embed(vec![query.into()], cancel).await?.remove(0);
        // Rig performs vector search with a validated precomputed query embedding.
        // No Rig agent, provider HTTP client or tool execution path is used.
        let index = self.store.clone().index(QueryEmbedding(embedding));
        let mut hits = index
            .top_n::<Evidence>(query, self.limits.top_k)
            .await
            .map_err(|_| OscarError::Validation("RAG vector search failed".into()))?;
        hits.sort_by(|a, b| b.0.total_cmp(&a.0).then_with(|| a.1.cmp(&b.1)));
        let mut evidence = Vec::new();
        let mut omitted = 0;
        for (score, _, document) in hits {
            if !score.is_finite() {
                return Err(OscarError::Validation("invalid retrieval score".into()));
            }
            evidence.push(document);
            let bytes = serde_json::to_vec(&evidence)
                .map_err(|_| OscarError::Validation("cannot encode RAG evidence".into()))?;
            if bytes.len() > self.limits.max_context_bytes {
                evidence.pop();
                omitted += 1;
            }
        }
        Ok(Retrieval { evidence, omitted })
    }
}

#[derive(Clone)]
struct QueryEmbedding(Embedding);
impl EmbeddingModel for QueryEmbedding {
    const MAX_DOCUMENTS: usize = 1;
    fn ndims(&self) -> usize {
        self.0.vec.len()
    }
    async fn embed_texts(
        &self,
        _: impl IntoIterator<Item = String> + Send,
    ) -> Result<Vec<Embedding>, EmbeddingError> {
        Ok(vec![self.0.clone()])
    }
    async fn embed_text(&self, _: &str) -> Result<Embedding, EmbeddingError> {
        Ok(self.0.clone())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicUsize, Ordering};
    struct SemanticFixture;
    impl EmbeddingProvider for SemanticFixture {
        fn embed(&self, texts: Vec<String>) -> EmbeddingFuture<'_> {
            Box::pin(async move {
                Ok(texts
                    .iter()
                    .map(|t| {
                        if t.contains("auth") || t.contains("login") {
                            vec![1.0, 0.0]
                        } else {
                            vec![0.0, 1.0]
                        }
                    })
                    .collect())
            })
        }
    }
    fn session(limits: RagLimits) -> RagSession {
        RagSession::new(Arc::new(SemanticFixture), limits).unwrap()
    }
    #[tokio::test]
    async fn semantic_retrieval_citations_and_replacement_are_bounded() {
        let cancel = CancellationToken::new();
        let mut s = session(RagLimits {
            top_k: 1,
            ..RagLimits::default()
        });
        s.index("auth.rs", "authentication rules", &cancel)
            .await
            .unwrap();
        s.index("parser.rs", "parse a number", &cancel)
            .await
            .unwrap();
        let result = s.retrieve("login", &cancel).await.unwrap();
        assert_eq!(result.evidence[0].source, "auth.rs");
        assert_eq!(result.evidence[0].start_byte, 0);
        assert_eq!(result.evidence[0].end_byte, "authentication rules".len());
        s.index("auth.rs", "auth changed", &cancel).await.unwrap();
        assert_eq!(s.chunks(), 2);
        assert_eq!(
            s.retrieve("login", &cancel).await.unwrap().evidence[0].text,
            "auth changed"
        );
        let used = s.calls_used();
        s.clear();
        assert_eq!(s.chunks(), 0);
        assert!(
            s.retrieve("login", &cancel)
                .await
                .unwrap()
                .evidence
                .is_empty()
        );
        assert_eq!(s.calls_used(), used);
    }
    #[tokio::test]
    async fn utf8_chunking_and_json_escaping_obey_exact_budget() {
        let cancel = CancellationToken::new();
        let text = "🦀\"\n🦀";
        let mut s = session(RagLimits {
            chunk_bytes: 4,
            top_k: 3,
            max_context_bytes: 2,
            ..RagLimits::default()
        });
        assert_eq!(s.index("unicode", text, &cancel).await.unwrap(), 3);
        let mut chunks: Vec<_> = s.store.iter().map(|(_, (d, _))| d.clone()).collect();
        chunks.sort_by_key(|c| c.start_byte);
        assert_eq!(
            chunks.iter().map(|c| c.text.as_str()).collect::<String>(),
            text
        );
        for c in chunks {
            assert_eq!(c.text, text[c.start_byte..c.end_byte]);
        }
        let result = s.retrieve("test", &cancel).await.unwrap();
        assert!(result.evidence.is_empty());
        assert_eq!(result.omitted, 3);
        assert_eq!(serde_json::to_vec(&result.evidence).unwrap().len(), 2);
    }
    #[tokio::test]
    async fn capacity_and_call_budget_fail_without_changing_index() {
        let cancel = CancellationToken::new();
        let mut s = session(RagLimits {
            max_chunks: 1,
            top_k: 1,
            max_embedding_calls: 2,
            ..RagLimits::default()
        });
        s.index("one", "auth", &cancel).await.unwrap();
        assert!(matches!(
            s.index("two", "parse", &cancel).await,
            Err(OscarError::Limit(_))
        ));
        assert_eq!(s.calls_used(), 1);
        assert!(s.index("one", &"x".repeat(16_385), &cancel).await.is_err());
        s.retrieve("login", &cancel).await.unwrap();
        s.clear();
        assert!(matches!(
            s.index("one", "auth", &cancel).await,
            Err(OscarError::Limit(_))
        ));
        assert_eq!(s.chunks(), 0);
        assert!(
            RagSession::new(
                Arc::new(SemanticFixture),
                RagLimits {
                    chunk_bytes: 3,
                    ..RagLimits::default()
                }
            )
            .is_err()
        );
    }
    struct Invalid(Vec<Vec<f64>>);
    impl EmbeddingProvider for Invalid {
        fn embed(&self, _: Vec<String>) -> EmbeddingFuture<'_> {
            Box::pin(async { Ok(self.0.clone()) })
        }
    }
    #[tokio::test]
    async fn malformed_vectors_fail_closed_and_consume_budget() {
        for vectors in [
            vec![],
            vec![vec![]],
            vec![vec![0.0, 0.0]],
            vec![vec![f64::NAN]],
            vec![vec![f64::INFINITY]],
            vec![vec![f64::MAX]],
            vec![vec![1.0]; 2],
            vec![vec![1.0; 4097]],
        ] {
            let mut s = RagSession::new(Arc::new(Invalid(vectors)), RagLimits::default()).unwrap();
            assert!(matches!(
                s.index("source", "text", &CancellationToken::new()).await,
                Err(OscarError::Validation(_))
            ));
            assert_eq!(s.chunks(), 0);
            assert_eq!(s.calls_used(), 1);
        }
    }
    struct Changing(AtomicUsize);
    impl EmbeddingProvider for Changing {
        fn embed(&self, _: Vec<String>) -> EmbeddingFuture<'_> {
            let n = self.0.fetch_add(1, Ordering::Relaxed) + 1;
            Box::pin(async move { Ok(vec![vec![1.0; n]]) })
        }
    }
    #[tokio::test]
    async fn dimension_change_does_not_replace_existing_evidence() {
        let mut s = RagSession::new(
            Arc::new(Changing(AtomicUsize::new(0))),
            RagLimits::default(),
        )
        .unwrap();
        let cancel = CancellationToken::new();
        s.index("source", "old text", &cancel).await.unwrap();
        assert!(s.index("source", "new text", &cancel).await.is_err());
        assert_eq!(s.store.iter().next().unwrap().1.0.text, "old text");
    }
    struct Waiting;
    impl EmbeddingProvider for Waiting {
        fn embed(&self, _: Vec<String>) -> EmbeddingFuture<'_> {
            Box::pin(std::future::pending())
        }
    }
    #[tokio::test]
    async fn timeout_and_cancellation_stop_waiting_without_resetting_budget() {
        let mut s = RagSession::new(
            Arc::new(Waiting),
            RagLimits {
                call_timeout_ms: 5,
                ..RagLimits::default()
            },
        )
        .unwrap();
        assert!(matches!(
            s.index("source", "text", &CancellationToken::new()).await,
            Err(OscarError::Limit(_))
        ));
        assert_eq!(s.calls_used(), 1);
        let cancel = CancellationToken::new();
        cancel.cancel();
        assert_eq!(
            s.index("source", "text", &cancel).await,
            Err(OscarError::Cancelled)
        );
        assert_eq!(s.calls_used(), 1);
        let cancel = CancellationToken::new();
        let signal = cancel.clone();
        let cancel_later = async move {
            tokio::task::yield_now().await;
            signal.cancel();
        };
        let (result, ()) = tokio::join!(s.index("source", "text", &cancel), cancel_later);
        assert_eq!(result, Err(OscarError::Cancelled));
        assert_eq!(s.calls_used(), 2);
    }
}
