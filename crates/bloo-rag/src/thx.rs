//! THX-01 client: routing and verbatim citation spans.
//!
//! Every call is best-effort. If the model is down or slow, retrieval still answers — this is
//! an enhancer, not a dependency (see PLAYBOOK 8.1).

use anyhow::{Context, Result};
use bloo_core::Config;
use serde_json::{json, Value};
use std::time::Duration;

#[derive(Debug, Clone)]
pub struct Route {
    pub route: String,
    pub topic: Option<String>,
    pub confidence: f32,
}

#[derive(Clone)]
pub struct ThxClient {
    http: reqwest::Client,
    base: String,
    key: Option<String>,
}

impl ThxClient {
    pub fn new(cfg: &Config) -> Self {
        Self {
            http: reqwest::Client::builder()
                .timeout(Duration::from_secs(6))
                .user_agent(concat!("BlooAI/", env!("CARGO_PKG_VERSION")))
                .build()
                .unwrap_or_default(),
            base: cfg.halx_base_url.trim_end_matches('/').to_string(),
            key: cfg.halx_api_key.clone().filter(|k| !k.trim().is_empty()),
        }
    }

    async fn post(&self, body: &Value) -> Result<Value> {
        let mut req = self
            .http
            .post(format!("{}/v1/systemone", self.base))
            .json(body);
        if let Some(key) = &self.key {
            req = req.bearer_auth(key);
        }
        let resp = req.send().await.context("THX-01 request failed")?;
        let status = resp.status();
        let text = resp.text().await.unwrap_or_default();
        if !status.is_success() {
            anyhow::bail!(
                "THX-01 HTTP {}: {}",
                status.as_u16(),
                text.chars().take(200).collect::<String>()
            );
        }
        serde_json::from_str(&text).context("THX-01 returned non-JSON")
    }

    /// Classify the question. Returns `None` on any failure.
    pub async fn route(&self, question: &str) -> Option<Route> {
        let body = json!({
            "model": "thx-01",
            "state": question,
            "questions": {
                "route": {
                    "type": "choice",
                    "question": "What kind of request is this?",
                    "criteria": {
                        "lookup": "asks for a specific value, date, fee, contact or deadline",
                        "procedure": "asks how to do something: steps, process, eligibility, forms",
                        "chitchat": "greeting, thanks, small talk, not a campus question",
                        "out_of_scope": "cannot be answered from campus documents"
                    }
                },
                "topic": {
                    "type": "choice",
                    "question": "Which campus area does this concern?",
                    "criteria": {
                        "admissions": "application, eligibility, entrance, document checklist",
                        "fees": "tuition, hostel fee, refund, payment, scholarship, fine",
                        "hostel": "rooms, wardens, mess, rules, vacating, room allotment",
                        "exams": "timetable, results, revaluation, backlog, hall ticket",
                        "academics": "courses, credits, attendance, syllabus, departments",
                        "library": "books, journals, access, timings, membership",
                        "transport": "bus routes, timings, passes, parking",
                        "general": "anything else campus related"
                    }
                }
            }
        });

        let out = self.post(&body).await.ok()?;
        let answers = out.get("answers")?;
        let route = answers.get("route")?.get("choice")?.as_str()?.to_string();
        let confidence = answers
            .get("route")?
            .get("probabilities")?
            .get(&route)?
            .as_f64()
            .unwrap_or(0.0) as f32;
        let topic = answers
            .get("topic")?
            .get("choice")?
            .as_str()
            .map(|s| s.to_string());

        Some(Route {
            route,
            topic,
            confidence,
        })
    }

    /// Ask THX-01 for the sentence that answers `question`. The span is cut from the document,
    /// so callers must still verify it is a substring before trusting it.
    pub async fn excerpt(&self, text: &str, question: &str) -> Option<String> {
        let body = json!({
            "model": "thx-01",
            "state": { "document": text },
            "questions": {
                "support": {
                    "type": "excerpt",
                    "question": format!("Which sentence answers: {question}?")
                }
            }
        });

        let out = self.post(&body).await.ok()?;
        let quote = out
            .get("answers")?
            .get("support")?
            .get("text")?
            .as_str()?
            .trim()
            .to_string();

        if quote.len() > 8 && text.contains(&quote) {
            Some(quote)
        } else {
            None
        }
    }
}
