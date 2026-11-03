//! Chat completions client (OpenAI-compatible).
//!
//! Non-streaming for tests and deterministic smoke checks; streaming for the UI.
//! OpenCode Go needs `User-Agent` and `x-opencode-session`, which the caller passes in.

use anyhow::{bail, Context, Result};
use bloo_core::Config;
use futures_util::StreamExt;
use serde_json::{json, Value};
use std::time::Duration;
use tokio::sync::mpsc;
use tokio_stream::wrappers::ReceiverStream;

#[derive(Clone)]
pub struct LlmClient {
    http: reqwest::Client,
    url: String,
    key: String,
    model: String,
}

impl LlmClient {
    pub fn from_config(cfg: &Config) -> Result<Self> {
        let key = cfg.require_llm()?.to_string();
        Ok(Self {
            http: reqwest::Client::builder()
                .timeout(Duration::from_secs(180))
                .user_agent(concat!("BlooAI/", env!("CARGO_PKG_VERSION")))
                .build()
                .context("cannot build the LLM client")?,
            url: cfg.chat_url(),
            key,
            model: cfg.llm_model.clone(),
        })
    }

    pub fn model(&self) -> &str {
        &self.model
    }

    fn body(&self, system: &str, user: &str, stream: bool) -> Value {
        json!({
            "model": self.model,
            "messages": [
                { "role": "system", "content": system },
                { "role": "user", "content": user }
            ],
            "temperature": 0.1,
            "max_tokens": 500,
            "stream": stream
        })
    }

    fn request(&self, body: &Value, session: &str) -> reqwest::RequestBuilder {
        self.http
            .post(&self.url)
            .bearer_auth(&self.key)
            // OpenCode Go asks clients to identify themselves and carry a stable session id.
            .header("x-opencode-session", session)
            .json(body)
    }

    pub async fn complete(&self, system: &str, user: &str) -> Result<String> {
        let body = self.body(system, user, false);
        let resp = self
            .request(&body, "bloo-chat")
            .send()
            .await
            .context("LLM request failed")?;

        let status = resp.status();
        let text = resp.text().await.unwrap_or_default();

        if !status.is_success() {
            let hint = match status.as_u16() {
                401 => " (key rejected: create a Console key at opencode.ai/console)",
                402 => " (no credits, or a Go key on the Console base path)",
                404 => " (LLM_BASE_URL is wrong; use the base without /chat/completions)",
                _ => "",
            };
            bail!(
                "LLM HTTP {}{}: {}",
                status.as_u16(),
                hint,
                text.chars().take(300).collect::<String>()
            );
        }

        let parsed: Value = serde_json::from_str(&text).context("LLM returned non-JSON")?;
        let content = parsed["choices"][0]["message"]["content"]
            .as_str()
            .unwrap_or("")
            .trim()
            .to_string();

        if content.is_empty() {
            bail!("LLM returned an empty answer");
        }
        Ok(content)
    }

    /// Stream answer deltas. The channel closes at `[DONE]` or on error.
    pub fn stream(&self, system: String, user: String) -> ReceiverStream<Result<String>> {
        let (tx, rx) = mpsc::channel::<Result<String>>(64);
        let client = self.clone();

        tokio::spawn(async move {
            let body = client.body(&system, &user, true);
            let resp = match client.request(&body, "bloo-chat").send().await {
                Ok(r) => r,
                Err(e) => {
                    let _ = tx
                        .send(Err(anyhow::anyhow!("LLM request failed: {e}")))
                        .await;
                    return;
                }
            };

            if !resp.status().is_success() {
                let status = resp.status();
                let text = resp.text().await.unwrap_or_default();
                let _ = tx
                    .send(Err(anyhow::anyhow!(
                        "LLM HTTP {}: {}",
                        status.as_u16(),
                        text.chars().take(200).collect::<String>()
                    )))
                    .await;
                return;
            }

            let mut buf = String::new();
            let mut bytes = resp.bytes_stream();

            while let Some(chunk) = bytes.next().await {
                let chunk = match chunk {
                    Ok(c) => c,
                    Err(e) => {
                        let _ = tx.send(Err(anyhow::anyhow!("stream error: {e}"))).await;
                        return;
                    }
                };
                buf.push_str(&String::from_utf8_lossy(&chunk));

                while let Some(idx) = buf.find('\n') {
                    let line: String = buf.drain(..=idx).collect();
                    let line = line.trim_end();
                    let Some(data) = line.strip_prefix("data:") else {
                        continue;
                    };
                    let data = data.trim();
                    if data == "[DONE]" {
                        return;
                    }
                    if let Ok(value) = serde_json::from_str::<Value>(data) {
                        if let Some(delta) = value["choices"][0]["delta"]["content"].as_str() {
                            if !delta.is_empty() && tx.send(Ok(delta.to_string())).await.is_err() {
                                return;
                            }
                        }
                    }
                }
            }
        });

        ReceiverStream::new(rx)
    }
}
