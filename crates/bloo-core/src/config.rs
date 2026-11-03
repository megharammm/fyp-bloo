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
    /// `local` (default) | `openai` | `mock`. See `bloo_rag::embed`.
    pub embed_provider: String,
    /// Base URL for the dormant OpenAI-compatible backend.
    pub embed_base_url: String,
    pub embed_api_key: Option<String>,
    /// `auto` (default) | `thx` | `local` | `off`. See PLAYBOOK.md 8.1.
    pub decider: String,
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
            embed_provider: env_or("BLOO_EMBED_PROVIDER", "local"),
            embed_base_url: env_or("BLOO_EMBED_BASE_URL", "https://api.jina.ai/v1"),
            embed_api_key: std::env::var("BLOO_EMBED_API_KEY").ok(),
            decider: env_or("BLOO_DECIDER", "auto"),
        })
    }

    /// Which decision backend the pipeline will use.
    ///
    /// `thx` forces THX-01, `local`/`off` force the offline fallback, `auto` picks THX-01 only
    /// when a key is present. THX-01 is an accelerator, never a hard dependency.
    pub fn decider(&self) -> &'static str {
        let has_key = self
            .halx_api_key
            .as_deref()
            .is_some_and(|k| !k.trim().is_empty());
        resolve_decider(&self.decider, has_key)
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

    /// Full chat-completions URL.
    ///
    /// `LLM_BASE_URL` is a *base* (`https://host/v1`). People paste the full endpoint
    /// often enough that we normalise it instead of failing: both forms work.
    pub fn chat_url(&self) -> String {
        resolve_chat_url(&self.llm_base_url)
    }
}

/// Pure URL normalisation, unit-tested below.
pub fn resolve_chat_url(base: &str) -> String {
    let base = base.trim().trim_end_matches('/');
    if base.ends_with("/chat/completions") {
        base.to_string()
    } else {
        format!("{base}/chat/completions")
    }
}

/// Pure mapping, unit-tested below. Kept out of `Config` so it needs no env access.
pub fn resolve_decider(mode: &str, has_key: bool) -> &'static str {
    match mode {
        "thx" => "thx-01",
        "local" | "off" => "local",
        _ => {
            if has_key {
                "thx-01"
            } else {
                "local"
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::resolve_decider;

    #[test]
    fn auto_follows_the_key() {
        assert_eq!(resolve_decider("auto", true), "thx-01");
        assert_eq!(resolve_decider("auto", false), "local");
    }

    #[test]
    fn explicit_modes_win() {
        assert_eq!(resolve_decider("thx", false), "thx-01");
        assert_eq!(resolve_decider("local", true), "local");
        assert_eq!(resolve_decider("off", true), "local");
    }

    #[test]
    fn unknown_mode_is_auto() {
        assert_eq!(resolve_decider("", true), "thx-01");
        assert_eq!(resolve_decider("nonsense", false), "local");
    }

    #[test]
    fn chat_url_accepts_base_or_full_endpoint() {
        use super::resolve_chat_url;

        assert_eq!(
            resolve_chat_url("https://opencode.ai/inference/openai/v1"),
            "https://opencode.ai/inference/openai/v1/chat/completions"
        );
        // the mistake made in .env: pasting the full endpoint
        assert_eq!(
            resolve_chat_url("https://opencode.ai/inference/openai/v1/chat/completions"),
            "https://opencode.ai/inference/openai/v1/chat/completions"
        );
        assert_eq!(
            resolve_chat_url("https://api.openai.com/v1/"),
            "https://api.openai.com/v1/chat/completions"
        );
    }
}
