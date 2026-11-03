use std::path::PathBuf;

/// Runtime configuration, loaded from environment (see `.env.example`).
#[derive(Debug, Clone)]
pub struct Config {
    /// Address the Axum server binds to. Bloo uses 27810 (never 3000/8000).
    pub bind: String,
    pub database_url: String,
    pub data_dir: PathBuf,
    pub content_dir: PathBuf,
    pub auto_migrate: bool,

    /// HAL-X THX-01 decision API.
    pub halx_base_url: String,
    pub halx_api_key: Option<String>,

    /// Any OpenAI-compatible chat-completions endpoint (prose generation only).
    pub llm_base_url: String,
    pub llm_api_key: Option<String>,
    pub llm_model: String,

    /// Local ONNX embedding model (fastembed).
    pub embed_model: String,
    /// Number of dimensions the embedding model produces. Must match the
    /// `vector(N)` column in `migrations/0001_init.sql`.
    pub embed_dim: usize,
}

fn env_or(key: &str, default: &str) -> String {
    std::env::var(key).unwrap_or_else(|_| default.to_string())
}

fn env_path(key: &str, default: &str) -> PathBuf {
    PathBuf::from(env_or(key, default))
}

impl Config {
    pub fn from_env() -> anyhow::Result<Self> {
        let embed_dim: usize = env_or("BLOO_EMBED_DIM", "384")
            .parse()
            .map_err(|_| anyhow::anyhow!("BLOO_EMBED_DIM must be a number"))?;

        Ok(Self {
            bind: env_or("BLOO_BIND", "127.0.0.1:27810"),
            database_url: env_or(
                "DATABASE_URL",
                "postgres://bloo:bloo_local@127.0.0.1:27432/blooai",
            ),
            data_dir: env_path("BLOO_DATA_DIR", "./data"),
            content_dir: env_path("BLOO_CONTENT_DIR", "./content"),
            auto_migrate: env_or("BLOO_AUTO_MIGRATE", "1") == "1",
            halx_base_url: env_or("HALX_BASE_URL", "https://api.hal-x.ai"),
            halx_api_key: std::env::var("HALX_API_KEY").ok(),
            llm_base_url: env_or("LLM_BASE_URL", "https://api.openai.com/v1"),
            llm_api_key: std::env::var("LLM_API_KEY").ok(),
            llm_model: env_or("LLM_MODEL", "gpt-4o-mini"),
            embed_model: env_or("BLOO_EMBED_MODEL", "BAAI/bge-small-en-v1.5"),
            embed_dim,
        })
    }

    /// Fail fast with a readable message if a required secret is missing.
    pub fn require_halx(&self) -> anyhow::Result<&str> {
        self.halx_api_key
            .as_deref()
            .filter(|k| !k.is_empty())
            .ok_or_else(|| anyhow::anyhow!("HALX_API_KEY is not set (see .env.example)"))
    }

    pub fn require_llm(&self) -> anyhow::Result<&str> {
        self.llm_api_key
            .as_deref()
            .filter(|k| !k.is_empty())
            .ok_or_else(|| anyhow::anyhow!("LLM_API_KEY is not set (see .env.example)"))
    }
}
