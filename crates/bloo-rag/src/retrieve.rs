//! End-to-end answering: route → embed → hybrid retrieve → prompt → generate → cite.

use crate::llm::LlmClient;
use crate::prompt::{self, REFUSAL};
use crate::store::{hybrid_search, RetrievedChunk};
use crate::thx::ThxClient;
use crate::Embedder;
use anyhow::Result;
use bloo_core::{Citation, Config, PgPool};
use std::time::Instant;

/// Minimum dense cosine similarity for an answer. Below this we refuse rather than hallucinate.
const MIN_SIMILARITY: f32 = 0.40;
const CONTEXT_CHUNKS: usize = 6;
const MAX_CITATIONS: usize = 4;

#[derive(Debug, Clone)]
pub struct Prepared {
    /// Set when the pipeline decided not to generate (nothing retrieved, or chit-chat).
    pub refusal: Option<String>,
    pub chunks: Vec<RetrievedChunk>,
    pub route: Option<String>,
    pub topic: Option<String>,
    pub best_similarity: f32,
    pub latency_ms: u64,
}

#[derive(Debug, Clone)]
pub struct AnswerOut {
    pub answer: String,
    pub refused: bool,
    pub citations: Vec<Citation>,
    pub retrieved: usize,
    pub best_similarity: f32,
    pub route: Option<String>,
    pub topic: Option<String>,
    pub latency_ms: u64,
    pub model: String,
}

/// Route + retrieve. No generation, so the streaming path can reuse it.
pub async fn prepare(
    pool: &PgPool,
    _cfg: &Config,
    embedder: &dyn Embedder,
    thx: &ThxClient,
    question: &str,
) -> Result<Prepared> {
    let started = Instant::now();
    let question = question.trim();

    if question.is_empty() {
        return Ok(Prepared {
            refusal: Some(REFUSAL.to_string()),
            chunks: Vec::new(),
            route: None,
            topic: None,
            best_similarity: 0.0,
            latency_ms: 0,
        });
    }

    // 1. Optional routing. A failure here must not break answering.
    let route = thx.route(question).await;
    let route_name = route.as_ref().map(|r| r.route.clone());
    let topic = route.as_ref().and_then(|r| r.topic.clone());

    if let Some(r) = &route {
        if r.route == "chitchat" && r.confidence >= 0.8 {
            return Ok(Prepared {
                refusal: Some(
                    "Hi! I can answer questions about admissions, fees, hostel, exams, the \
                     library and transport. What would you like to know?"
                        .to_string(),
                ),
                chunks: Vec::new(),
                route: route_name,
                topic,
                best_similarity: 0.0,
                latency_ms: started.elapsed().as_millis() as u64,
            });
        }
    }

    // 2. Embed the query.
    let embedding = embedder.embed_query(question).await?;

    // 3. Hybrid retrieval (published chunks only).
    let hits = hybrid_search(pool, question, &embedding, 40).await?;
    let best_similarity = hits.iter().map(|h| h.similarity).fold(0.0f32, f32::max);
    let fused_nonempty = !hits.is_empty();
    let chunks: Vec<RetrievedChunk> = hits.into_iter().take(CONTEXT_CHUNKS).collect();

    // 4. Refuse when retrieval found nothing usable.
    if !fused_nonempty || best_similarity < MIN_SIMILARITY {
        return Ok(Prepared {
            refusal: Some(REFUSAL.to_string()),
            chunks,
            route: route_name,
            topic,
            best_similarity,
            latency_ms: started.elapsed().as_millis() as u64,
        });
    }

    Ok(Prepared {
        refusal: None,
        chunks,
        route: route_name,
        topic,
        best_similarity,
        latency_ms: started.elapsed().as_millis() as u64,
    })
}

pub async fn answer(
    pool: &PgPool,
    cfg: &Config,
    embedder: &dyn Embedder,
    thx: &ThxClient,
    llm: &LlmClient,
    question: &str,
) -> Result<AnswerOut> {
    let started = Instant::now();
    let prepared = prepare(pool, cfg, embedder, thx, question).await?;

    if let Some(refusal) = prepared.refusal {
        return Ok(AnswerOut {
            answer: refusal,
            refused: true,
            citations: Vec::new(),
            retrieved: prepared.chunks.len(),
            best_similarity: prepared.best_similarity,
            route: prepared.route,
            topic: prepared.topic,
            latency_ms: started.elapsed().as_millis() as u64,
            model: llm.model().to_string(),
        });
    }

    let user = prompt::user_prompt(question, &prepared.chunks);
    let answer_text = llm.complete(prompt::system_prompt(), &user).await?;
    let refused = answer_text.contains("couldn't find that in the campus documents");

    // Cite only the passages the answer actually referenced ([1], [2], ...). If the model
    // forgot its markers, fall back to the top two rather than dumping every retrieved chunk.
    let mut indices = used_indices(&answer_text);
    if indices.is_empty() {
        indices = (0..prepared.chunks.len().min(2)).collect();
    }
    indices.retain(|i| *i < prepared.chunks.len());
    indices.truncate(MAX_CITATIONS);

    let mut citations = Vec::new();
    if !refused {
        for (pos, idx) in indices.iter().enumerate() {
            let chunk = &prepared.chunks[*idx];
            // THX-01 excerpt is opt-in: each pass is ~2-3 s on CPU. The default path uses a
            // query-relevant sentence, which is instant and equally verbatim.
            let quote = if pos == 0 && cfg.thx_citations {
                match thx.excerpt(chunk_body(&chunk.text), question).await {
                    Some(q) if chunk.text.contains(&q) => q,
                    _ => pick_quote(&chunk.text, question),
                }
            } else {
                pick_quote(&chunk.text, question)
            };
            if quote.trim().is_empty() {
                continue;
            }
            citations.push(citation_for(chunk, quote));
        }
    }

    Ok(AnswerOut {
        answer: answer_text,
        refused,
        citations,
        retrieved: prepared.chunks.len(),
        best_similarity: prepared.best_similarity,
        route: prepared.route,
        topic: prepared.topic,
        latency_ms: started.elapsed().as_millis() as u64,
        model: llm.model().to_string(),
    })
}

/// Zero-based indices of `[n]` citation markers, deduplicated and in order.
pub fn used_indices(answer: &str) -> Vec<usize> {
    let mut out: Vec<usize> = Vec::new();
    let bytes = answer.as_bytes();
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] == b'[' {
            let mut j = i + 1;
            let mut n: usize = 0;
            let mut digits = 0;
            while j < bytes.len() && bytes[j].is_ascii_digit() {
                n = n
                    .saturating_mul(10)
                    .saturating_add((bytes[j] - b'0') as usize);
                j += 1;
                digits += 1;
            }
            if digits > 0 && j < bytes.len() && bytes[j] == b']' && n > 0 {
                let idx = n - 1;
                if !out.contains(&idx) {
                    out.push(idx);
                }
                i = j;
            }
        }
        i += 1;
    }
    out
}

/// Build a citation, enforcing "no link for non-web sources".
pub fn citation_for(chunk: &RetrievedChunk, quote: String) -> Citation {
    Citation {
        title: chunk.title.clone(),
        section_path: chunk.section_path.clone(),
        kind: chunk.kind.clone(),
        url: if chunk.kind == "html" {
            chunk.canonical_url.clone()
        } else {
            None
        },
        page_label: chunk.page_from.map(|p| format!("p. {p}")),
        quote,
        score: chunk.similarity,
    }
}

/// Pick the sentence with the most query terms. Always a verbatim slice of `text`.
pub fn pick_quote(text: &str, question: &str) -> String {
    let text = chunk_body(text);
    let terms: Vec<String> = question
        .split_whitespace()
        .map(|w| {
            w.trim_matches(|c: char| !c.is_alphanumeric())
                .to_lowercase()
        })
        .filter(|w| w.len() > 2)
        .collect();

    let mut best = String::new();
    let mut best_score = 0usize;

    for sentence in sentences(text) {
        let s = sentence.trim();
        if s.chars().count() < 20 {
            continue;
        }
        let lower = s.to_lowercase();
        let score = terms.iter().filter(|t| lower.contains(t.as_str())).count();
        if score > best_score {
            best_score = score;
            best = s.to_string();
        }
    }

    if best.is_empty() {
        best = text.chars().take(240).collect();
    }
    if best.chars().count() > 320 {
        best = best.chars().take(320).collect();
    }
    best
}

/// Strip the breadcrumb prefix that chunking adds (`Fees > Hostel\n\nbody`).
///
/// The breadcrumb is part of the embedded text, but citing it would produce a quote that
/// does not exist in the source document.
pub fn chunk_body(text: &str) -> &str {
    match text.split_once("\n\n") {
        Some((head, body)) if head.contains(" > ") && !body.trim().is_empty() => body,
        _ => text,
    }
}

/// Sentence boundaries that respect abbreviations like `B.Tech` and `Rs.`:
/// a period only ends a sentence when whitespace follows.
fn sentences(text: &str) -> Vec<String> {
    let mut out = Vec::new();
    let mut buf = String::new();
    let mut chars = text.char_indices().peekable();

    while let Some((_, c)) = chars.next() {
        buf.push(c);
        let boundary = match c {
            '.' | '!' | '?' => chars
                .peek()
                .map(|(_, next)| next.is_whitespace())
                .unwrap_or(true),
            '\n' => true,
            _ => false,
        };
        if boundary {
            out.push(std::mem::take(&mut buf));
        }
    }
    if !buf.trim().is_empty() {
        out.push(buf);
    }
    out
}

#[cfg(test)]
mod tests {
    use super::pick_quote;

    #[test]
    fn quote_is_a_verbatim_substring() {
        let text = "The hostel fee is 48000 per semester. Mess charges are separate.";
        let q = pick_quote(text, "hostel fee semester");
        assert!(text.contains(&q), "quote must be verbatim: {q}");
        assert!(q.contains("48000"));
    }

    #[test]
    fn falls_back_to_the_start_of_the_chunk() {
        let text = "A".repeat(400);
        let q = pick_quote(&text, "zzz");
        assert!(text.contains(&q));
        assert!(q.chars().count() <= 320);
    }

    #[test]
    fn abbreviations_do_not_split_sentences() {
        let text = "The B.Tech fee is 120000 per year. Mess is billed separately.";
        let q = pick_quote(text, "B.Tech fee year");
        assert!(
            q.contains("B.Tech"),
            "quote was cut at the abbreviation: {q}"
        );
    }

    #[test]
    fn breadcrumb_is_not_cited() {
        let text = "Hostel Rules > Mess Timings\n\nBreakfast is served from 07:30 to 09:30.";
        let q = pick_quote(text, "mess timings breakfast");
        assert!(!q.contains(" > "), "breadcrumb leaked into the quote: {q}");
        assert!(q.contains("Breakfast"));
    }

    #[test]
    fn citation_markers_are_parsed() {
        use super::used_indices;
        assert_eq!(used_indices("a [1] b [3] c [1]"), vec![0, 2]);
        assert!(used_indices("no markers here").is_empty());
        assert!(used_indices("[0] is not a marker").is_empty());
    }
}
