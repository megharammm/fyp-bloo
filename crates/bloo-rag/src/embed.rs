//! Embedding providers.
//!
//! The default is **local ONNX** (`BAAI/bge-small-en-v1.5`, 384 dimensions): ~35 MB on disk,
//! ~130 MB RSS while running, no network, no quota. It is the one component in the pipeline
//! that must stay deterministic during a demo.
//!
//! A remote OpenAI-compatible backend (Jina, Mistral, OpenAI, ...) exists but is **dormant**:
//! it is only constructed when `BLOO_EMBED_PROVIDER=openai`. Switching providers also changes
//! `BLOO_EMBED_DIM`, which is a schema migration (`vector(N)`) plus a re-index. That is cheap
//! today (empty corpus) and expensive later, so the decision is recorded here.

use anyhow::{bail, Context, Result};
use async_trait::async_trait;
use bloo_core::Config;
use serde::{Deserialize, Serialize};
use std::sync::Arc;

/// A dense vector. Length must equal `Embedder::dim()`.
pub type Embedding = Vec<f32>;

#[async_trait]
pub trait Embedder: Send + Sync {
    /// Number of dimensions this embedder produces. Must match `vector(N)` in Postgres.
    fn dim(&self) -> usize;

    /// Short identifier for logs and `/health`.
    fn name(&self) -> &'static str;

    /// Embed documents for storage. Implementations should batch internally.
    async fn embed_documents(&self, texts: &[String]) -> Result<Vec<Embedding>>;

    /// Embed a single query. Kept separate because some providers use asymmetric models.
    async fn embed_query(&self, text: &str) -> Result<Embedding> {
        let mut out = self
            .embed_documents(std::slice::from_ref(&text.to_string()))
            .await?;
        out.pop().context("embedder returned no vector")
    }
}

/// Deterministic, dependency-free embedder for tests and for `BLOO_EMBED_PROVIDER=mock`.
///
/// Not semantically meaningful. It exists so the ingestion and retrieval plumbing can be
/// exercised without a 35 MB download and without hitting a network.
pub struct MockEmbedder {
    dim: usize,
}

impl MockEmbedder {
    pub fn new(dim: usize) -> Self {
        Self { dim }
    }
}

fn hash_vector(text: &str, dim: usize) -> Embedding {
    let mut out = Vec::with_capacity(dim);
    let mut h: u64 = 0xcbf2_9ce4_8422_2325;
    for b in text.bytes() {
        h ^= b as u64;
        h = h.wrapping_mul(0x0000_0100_0000_01b3);
    }
    for i in 0..dim {
        h ^= i as u64;
        h = h.wrapping_mul(0x0000_0100_0000_01b3);
        // take 24 high bits -> [-0.5, 0.5)
        out.push(((h >> 40) as f32 / 16_777_216.0) - 0.5);
    }
    let norm = out.iter().map(|x| x * x).sum::<f32>().sqrt().max(1e-6);
    for v in &mut out {
        *v /= norm;
    }
    out
}

#[async_trait]
impl Embedder for MockEmbedder {
    fn dim(&self) -> usize {
        self.dim
    }

    fn name(&self) -> &'static str {
        "mock"
    }

    async fn embed_documents(&self, texts: &[String]) -> Result<Vec<Embedding>> {
        Ok(texts.iter().map(|t| hash_vector(t, self.dim)).collect())
    }
}

// ─────────────────────────────────────────────────────────────────────────────
// OpenAI-compatible backend (dormant until explicitly selected)
// ─────────────────────────────────────────────────────────────────────────────

#[derive(Serialize)]
struct EmbedRequest<'a> {
    model: &'a str,
    input: &'a [String],
    #[serde(skip_serializing_if = "Option::is_none")]
    dimensions: Option<usize>,
}

#[derive(Deserialize)]
struct EmbedResponse {
    data: Vec<EmbedItem>,
}

#[derive(Deserialize)]
struct EmbedItem {
    embedding: Vec<f32>,
    #[serde(default)]
    index: usize,
}

pub struct OpenAiCompatEmbedder {
    client: reqwest::Client,
    url: String,
    api_key: Option<String>,
    model: String,
    dim: usize,
    /// Maximum texts per request. Jina allows 96, OpenAI 2048; 64 is safe everywhere.
    batch: usize,
}

impl OpenAiCompatEmbedder {
    pub fn new(base_url: &str, api_key: Option<String>, model: String, dim: usize) -> Result<Self> {
        let base = base_url.trim().trim_end_matches('/');
        if base.is_empty() {
            bail!("BLOO_EMBED_BASE_URL is empty");
        }
        let url = if base.ends_with("/embeddings") {
            base.to_string()
        } else {
            format!("{base}/embeddings")
        };

        Ok(Self {
            client: reqwest::Client::builder()
                .user_agent(concat!("BlooAI/", env!("CARGO_PKG_VERSION")))
                .build()
                .context("cannot build the embedding HTTP client")?,
            url,
            api_key: api_key.filter(|k| !k.trim().is_empty()),
            model,
            dim,
            batch: 64,
        })
    }
}

/// Parse an OpenAI-style embeddings response, ordering by `index` and checking dimensions.
///
/// Kept pure so it can be unit-tested without a server.
pub fn parse_embeddings(body: &str, expected_dim: usize) -> Result<Vec<Embedding>> {
    let parsed: EmbedResponse =
        serde_json::from_str(body).context("embedding provider returned non-JSON")?;

    if parsed.data.is_empty() {
        bail!("embedding provider returned no vectors");
    }

    let mut items = parsed.data;
    items.sort_by_key(|i| i.index);

    let mut out = Vec::with_capacity(items.len());
    for item in items {
        if item.embedding.len() != expected_dim {
            bail!(
                "embedding dimension mismatch: provider returned {}, schema expects {}. \
                 Change BLOO_EMBED_DIM and add a migration (vector({})), or pick a model \
                 with {} dimensions.",
                item.embedding.len(),
                expected_dim,
                item.embedding.len(),
                expected_dim
            );
        }
        out.push(item.embedding);
    }
    Ok(out)
}

#[async_trait]
impl Embedder for OpenAiCompatEmbedder {
    fn dim(&self) -> usize {
        self.dim
    }

    fn name(&self) -> &'static str {
        "openai-compatible"
    }

    async fn embed_documents(&self, texts: &[String]) -> Result<Vec<Embedding>> {
        let mut out = Vec::with_capacity(texts.len());

        for chunk in texts.chunks(self.batch) {
            let body = EmbedRequest {
                model: &self.model,
                input: chunk,
                // Jina, OpenAI and Mistral accept this; providers that do not will ignore
                // unknown fields or return their native dimension, caught by the check below.
                dimensions: Some(self.dim),
            };

            let mut req = self.client.post(&self.url).json(&body);
            if let Some(key) = &self.api_key {
                req = req.bearer_auth(key);
            }

            let resp = req
                .send()
                .await
                .with_context(|| format!("embedding request to {} failed", self.url))?;

            let status = resp.status();
            let text = resp.text().await.unwrap_or_default();

            if !status.is_success() {
                bail!(
                    "embedding provider returned HTTP {}: {}",
                    status.as_u16(),
                    text.chars().take(400).collect::<String>()
                );
            }

            out.extend(parse_embeddings(&text, self.dim)?);
        }

        Ok(out)
    }
}

// ─────────────────────────────────────────────────────────────────────────────
// Factory
// ─────────────────────────────────────────────────────────────────────────────

/// Build the embedder selected by `BLOO_EMBED_PROVIDER` (`local` | `openai` | `mock`).
///
/// `local` is the default and is implemented in WP-02 with `fastembed` + ONNX. Until the
/// ingestion crate lands there is nothing to embed, so this returns a clear error instead of
/// silently producing meaningless vectors.
pub fn build(cfg: &Config) -> Result<Arc<dyn Embedder>> {
    match cfg.embed_provider.as_str() {
        "mock" => Ok(Arc::new(MockEmbedder::new(cfg.embed_dim))),
        "openai" | "api" => {
            let key = cfg
                .embed_api_key
                .clone()
                .filter(|k| !k.trim().is_empty())
                .context(
                    "BLOO_EMBED_PROVIDER=openai needs BLOO_EMBED_API_KEY \
                     (Jina/Mistral/OpenAI). Leave the provider at `local` to avoid this.",
                )?;
            Ok(Arc::new(OpenAiCompatEmbedder::new(
                &cfg.embed_base_url,
                Some(key),
                cfg.embed_model.clone(),
                cfg.embed_dim,
            )?))
        }
        "local" | "" => bail!(
            "local embeddings are implemented in WP-02 (ingestion) with fastembed + ONNX. \
             Nothing calls the embedder yet. For tests use BLOO_EMBED_PROVIDER=mock; to try a \
             remote model use BLOO_EMBED_PROVIDER=openai with BLOO_EMBED_API_KEY."
        ),
        other => {
            bail!("unknown BLOO_EMBED_PROVIDER {other:?}; expected `local`, `openai` or `mock`")
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn mock_is_deterministic_and_normalised() {
        let e = MockEmbedder::new(384);
        let rt = tokio::runtime::Runtime::new().unwrap();
        let a = rt
            .block_on(e.embed_documents(&["hostel fee".to_string()]))
            .unwrap();
        let b = rt
            .block_on(e.embed_documents(&["hostel fee".to_string()]))
            .unwrap();
        let c = rt
            .block_on(e.embed_documents(&["mess fee".to_string()]))
            .unwrap();

        assert_eq!(a[0].len(), 384);
        assert_eq!(a[0], b[0], "same input must produce the same vector");
        assert_ne!(
            a[0], c[0],
            "different input must produce a different vector"
        );

        let norm: f32 = a[0].iter().map(|x| x * x).sum::<f32>().sqrt();
        assert!(
            (norm - 1.0).abs() < 1e-4,
            "vector must be unit length, got {norm}"
        );
    }

    #[test]
    fn parses_and_orders_by_index() {
        let body = r#"{"data":[
            {"embedding":[3.0,4.0],"index":1},
            {"embedding":[0.0,1.0],"index":0}
        ]}"#;
        let v = parse_embeddings(body, 2).unwrap();
        assert_eq!(v[0], vec![0.0, 1.0]);
        assert_eq!(v[1], vec![3.0, 4.0]);
    }

    #[test]
    fn dimension_mismatch_is_actionable() {
        let body = r#"{"data":[{"embedding":[0.1,0.2,0.3],"index":0}]}"#;
        let err = parse_embeddings(body, 384).unwrap_err().to_string();
        assert!(err.contains("dimension mismatch"));
        assert!(err.contains("384"));
    }

    #[test]
    fn empty_response_is_an_error() {
        assert!(parse_embeddings(r#"{"data":[]}"#, 384).is_err());
    }
}
