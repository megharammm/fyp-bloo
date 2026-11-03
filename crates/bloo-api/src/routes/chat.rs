use askama::Template;
use axum::{extract::State, response::IntoResponse};
use serde_json::Value;

use crate::{AppState, Html};

#[derive(Template)]
#[template(path = "chat.html")]
struct ChatTemplate {
    version: &'static str,
    suggestions: Vec<String>,
}

fn default_suggestions() -> Vec<String> {
    vec![
        "What is the hostel fee per semester?".to_string(),
        "How many books can an undergraduate borrow?".to_string(),
        "When does the revaluation window open?".to_string(),
        "Which bus route goes to the north campus?".to_string(),
        "What are the mess timings?".to_string(),
    ]
}

/// Suggestions live in `app_settings` so an admin can change them without a rebuild.
async fn suggestions(state: &AppState) -> Vec<String> {
    let raw: Result<Option<Value>, _> =
        sqlx::query_scalar("select value from app_settings where key = 'suggested_questions'")
            .fetch_optional(&state.pool)
            .await;

    match raw {
        Ok(Some(Value::Array(items))) => {
            let list: Vec<String> = items
                .iter()
                .filter_map(|v| v.as_str().map(str::to_string))
                .take(5)
                .collect();
            if list.is_empty() {
                default_suggestions()
            } else {
                list
            }
        }
        _ => default_suggestions(),
    }
}

pub async fn index(State(state): State<AppState>) -> impl IntoResponse {
    Html(ChatTemplate {
        version: env!("CARGO_PKG_VERSION"),
        suggestions: suggestions(&state).await,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn chat_template_renders() {
        let html = ChatTemplate {
            version: "0.1.0",
            suggestions: default_suggestions(),
        }
        .render()
        .expect("render");

        assert!(html.contains("Bloo"));
        assert!(html.contains("composer"));
        assert!(html.contains("hostel fee"), "suggestions must render");
        assert!(html.contains(r#"aria-live="polite""#));
    }
}
