//! Heading-aware chunking.
//!
//! Rules that matter for retrieval quality on campus documents:
//! * every chunk carries its breadcrumb (`Fees > Hostel > Refund policy`)
//! * tables are never cut mid-row; a long table repeats its header row
//! * chunks are ~450 tokens with a 60-token overlap, hard cap 600
//! * headings are never emitted as their own chunk

use bloo_parse::{Block, BlockKind};

/// Hard cap. Must stay under THX-01's 1 024-token read budget with room for the question.
pub const MAX_TOKENS: usize = 600;
pub const TARGET_TOKENS: usize = 450;
pub const OVERLAP_TOKENS: usize = 60;

/// `chars / 4` — documented approximation; no tokenizer dependency in the ingestion path.
pub fn estimate_tokens(text: &str) -> usize {
    text.chars().count().div_ceil(4)
}

#[derive(Debug, Clone, PartialEq)]
pub struct Chunk {
    pub text: String,
    pub section_path: String,
    pub page_from: Option<i32>,
    pub page_to: Option<i32>,
    pub chunk_type: String,
    pub token_count: i32,
}

#[derive(Debug, Clone)]
pub struct ChunkOptions {
    pub max_tokens: usize,
    pub target_tokens: usize,
    pub overlap_tokens: usize,
}

impl Default for ChunkOptions {
    fn default() -> Self {
        Self {
            max_tokens: MAX_TOKENS,
            target_tokens: TARGET_TOKENS,
            overlap_tokens: OVERLAP_TOKENS,
        }
    }
}

struct Builder<'a> {
    opts: &'a ChunkOptions,
    breadcrumb: String,
    buffer: String,
    page_from: Option<i32>,
    page_to: Option<i32>,
    kind: &'a str,
    out: Vec<Chunk>,
}

impl<'a> Builder<'a> {
    fn flush(&mut self) {
        let body = self.buffer.trim().to_string();
        if body.is_empty() {
            return;
        }

        let text = if self.breadcrumb.is_empty() {
            body
        } else {
            format!("{}\n\n{}", self.breadcrumb, body)
        };

        self.out.push(Chunk {
            token_count: estimate_tokens(&text) as i32,
            text,
            section_path: self.breadcrumb.clone(),
            page_from: self.page_from,
            page_to: self.page_to,
            chunk_type: self.kind.to_string(),
        });

        self.buffer.clear();
        self.page_from = None;
        self.page_to = None;
    }

    fn note_page(&mut self, page: u32) {
        if page == 0 {
            return;
        }
        let page = page as i32;
        self.page_from = Some(self.page_from.map_or(page, |p| p.min(page)));
        self.page_to = Some(self.page_to.map_or(page, |p| p.max(page)));
    }

    /// Tokens the breadcrumb (and the blank line after it) will add to every chunk.
    fn reserved(&self) -> usize {
        estimate_tokens(&self.breadcrumb) + 2
    }

    /// Budget for the body, so `breadcrumb + body` stays inside the hard cap.
    fn body_cap(&self) -> usize {
        self.opts.max_tokens.saturating_sub(self.reserved()).max(64)
    }

    /// Last `overlap_tokens` worth of text, cut on a word boundary, so the next chunk
    /// repeats a little context.
    fn tail(&self) -> String {
        if self.opts.overlap_tokens == 0 || self.buffer.is_empty() {
            return String::new();
        }
        let keep = self.opts.overlap_tokens * 4;
        let chars: Vec<char> = self.buffer.chars().collect();
        if chars.len() <= keep {
            return self.buffer.clone();
        }
        let start = chars.len() - keep;
        let tail: String = chars[start..].iter().collect();
        // start at the first word boundary so we do not slice a word in half
        match tail.find(' ') {
            Some(i) if i + 1 < tail.len() => tail[i + 1..].to_string(),
            _ => tail,
        }
    }
}

pub fn chunk_blocks(blocks: &[Block], opts: &ChunkOptions) -> Vec<Chunk> {
    let mut b = Builder {
        opts,
        breadcrumb: String::new(),
        buffer: String::new(),
        page_from: None,
        page_to: None,
        kind: "prose",
        out: Vec::new(),
    };
    let mut stack: Vec<(u8, String)> = Vec::new();

    for block in blocks {
        match block.kind {
            BlockKind::Heading(level) => {
                b.flush();
                stack.retain(|(l, _)| *l < level);
                stack.push((level, block.text.trim().to_string()));
                b.breadcrumb = stack
                    .iter()
                    .map(|(_, t)| t.as_str())
                    .collect::<Vec<_>>()
                    .join(" > ");
            }

            BlockKind::Table => {
                b.flush();
                b.kind = "table";
                push_table(&mut b, block);
                b.kind = "prose";
            }

            BlockKind::Para | BlockKind::ListItem => {
                b.kind = "prose";
                b.note_page(block.page);
                for sentence in sentences(&block.text) {
                    push_sentence(&mut b, &sentence);
                }
            }
        }
    }

    b.flush();
    b.out
}

/// Append one sentence, flushing on the soft target and hard-splitting on the hard cap.
///
/// The body budget subtracts the breadcrumb, so `breadcrumb + body` can never exceed
/// `max_tokens` — the bug that made a 600-token cap produce 659-token chunks.
fn push_sentence(b: &mut Builder<'_>, sentence: &str) {
    let body_cap = b.body_cap();
    let target = b.opts.target_tokens.saturating_sub(b.reserved()).max(64);

    if !b.buffer.is_empty() && estimate_tokens(&b.buffer) + estimate_tokens(sentence) > target {
        let tail = b.tail();
        b.flush();
        b.buffer.push_str(&tail);
    }

    let room = body_cap.saturating_sub(estimate_tokens(&b.buffer)).max(32);
    if estimate_tokens(sentence) > room {
        // A single sentence bigger than a chunk: start clean, then emit each piece without
        // re-adding the overlap tail (otherwise tail + piece overshoots the cap).
        let tail = b.tail();
        b.flush();
        b.buffer.push_str(&tail);

        let room = body_cap.saturating_sub(estimate_tokens(&b.buffer)).max(32);
        for piece in hard_split(sentence, room) {
            if !b.buffer.is_empty() {
                b.buffer.push(' ');
            }
            b.buffer.push_str(&piece);
            b.flush();
        }
    } else {
        if !b.buffer.is_empty() {
            b.buffer.push(' ');
        }
        b.buffer.push_str(sentence);
    }
}

/// Emit a table as one chunk, or split by rows (repeating the header) when it exceeds the cap.
fn push_table(b: &mut Builder<'_>, block: &Block) {
    let rows: Vec<&str> = block
        .text
        .lines()
        .filter(|r| !r.trim().is_empty())
        .collect();
    if rows.is_empty() {
        return;
    }

    let cap = b.body_cap();
    let whole: usize = rows.iter().map(|r| estimate_tokens(r)).sum();
    b.note_page(block.page);

    if whole <= cap || rows.len() == 1 {
        b.buffer = rows.join("\n");
        b.flush();
        return;
    }

    let header = rows[0];
    let mut group = String::from(header);
    for row in rows.iter().skip(1) {
        let projected = estimate_tokens(&group) + estimate_tokens(row);
        if projected > cap && group.lines().count() > 1 {
            b.buffer = std::mem::take(&mut group);
            b.flush();
            group = String::from(header);
        }
        group.push('\n');
        group.push_str(row);
    }
    if !group.is_empty() {
        b.buffer = group;
        b.flush();
    }
}

/// Split into sentences, keeping punctuation. Newlines count as boundaries.
fn sentences(text: &str) -> Vec<String> {
    let mut out = Vec::new();
    let mut current = String::new();

    for ch in text.chars() {
        if ch == '\n' {
            if !current.trim().is_empty() {
                out.push(current.trim().to_string());
            }
            current.clear();
            continue;
        }
        current.push(ch);
        if matches!(ch, '.' | '!' | '?') {
            if !current.trim().is_empty() {
                out.push(current.trim().to_string());
            }
            current.clear();
        }
    }
    if !current.trim().is_empty() {
        out.push(current.trim().to_string());
    }
    out
}

/// Split on whitespace into pieces no longer than `max_tokens`.
fn hard_split(text: &str, max_tokens: usize) -> Vec<String> {
    let mut out = Vec::new();
    let mut current = String::new();
    for word in text.split_whitespace() {
        let projected = estimate_tokens(&current) + estimate_tokens(word) + 1;
        if projected > max_tokens && !current.is_empty() {
            out.push(std::mem::take(&mut current));
        }
        if !current.is_empty() {
            current.push(' ');
        }
        current.push_str(word);
    }
    if !current.is_empty() {
        out.push(current);
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use bloo_parse::Block;

    fn para(text: &str, page: u32) -> Block {
        Block::new(BlockKind::Para, text, page)
    }

    #[test]
    fn headings_become_breadcrumbs_not_chunks() {
        let blocks = vec![
            Block::new(BlockKind::Heading(1), "Fees", 1),
            Block::new(BlockKind::Heading(2), "Hostel", 1),
            para("The hostel fee is 48000 per semester.", 1),
        ];
        let chunks = chunk_blocks(&blocks, &ChunkOptions::default());

        assert_eq!(chunks.len(), 1, "heading must not be its own chunk");
        assert_eq!(chunks[0].section_path, "Fees > Hostel");
        assert!(chunks[0].text.starts_with("Fees > Hostel"));
        assert!(chunks[0].text.contains("48000"));
    }

    #[test]
    fn respects_the_hard_cap() {
        let long = "word ".repeat(2000);
        let chunks = chunk_blocks(&[para(&long, 1)], &ChunkOptions::default());
        assert!(chunks.len() > 1);
        for c in &chunks {
            assert!(
                c.token_count <= MAX_TOKENS as i32,
                "{} tokens",
                c.token_count
            );
        }
    }

    #[test]
    fn tables_stay_whole() {
        let table = "Item | Fee\nHostel | 48000\nMess | 3200";
        let blocks = vec![Block::new(BlockKind::Table, table, 1)];
        let chunks = chunk_blocks(&blocks, &ChunkOptions::default());

        assert_eq!(chunks.len(), 1);
        assert_eq!(chunks[0].chunk_type, "table");
        assert!(chunks[0].text.contains("Hostel | 48000"));
        assert!(chunks[0].text.contains("Mess | 3200"));
    }

    #[test]
    fn long_tables_repeat_the_header() {
        let mut rows = vec!["Item | Fee".to_string()];
        for i in 0..200 {
            rows.push(format!("very long row label number {i} | {}", i * 100));
        }
        let blocks = vec![Block::new(BlockKind::Table, rows.join("\n"), 1)];
        let chunks = chunk_blocks(&blocks, &ChunkOptions::default());

        assert!(chunks.len() > 1);
        for c in &chunks {
            assert!(c.text.contains("Item | Fee"), "header missing:\n{}", c.text);
            assert!(c.token_count <= MAX_TOKENS as i32);
        }
    }

    #[test]
    fn page_range_is_tracked() {
        let blocks = vec![para("Page one sentence.", 1), para("Page two sentence.", 2)];
        let chunks = chunk_blocks(&blocks, &ChunkOptions::default());
        assert_eq!(chunks[0].page_from, Some(1));
        assert_eq!(chunks[0].page_to, Some(2));
    }

    #[test]
    fn overlap_repeats_context() {
        let text = "alpha beta gamma delta. ".repeat(120);
        let chunks = chunk_blocks(&[para(&text, 1)], &ChunkOptions::default());
        assert!(chunks.len() > 1);
        // the tail of chunk 0 should appear at the start of chunk 1
        let first_words: Vec<&str> = chunks[0].text.split_whitespace().rev().take(6).collect();
        let second_start: String = chunks[1].text.chars().take(200).collect();
        assert!(
            first_words.iter().any(|w| second_start.contains(*w)),
            "no overlap detected"
        );
    }
}
