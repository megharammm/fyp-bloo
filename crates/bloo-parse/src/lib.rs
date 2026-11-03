//! Bloo AI — document parsing.
//!
//! Every supported format is reduced to the same structure: a page-aware, ordered list of
//! blocks (headings, paragraphs, tables, list items). Chunking (WP-02) works on blocks, so the
//! chunker never needs to know where the text came from.
//!
//! Design notes:
//! * PDF layout is reconstructed from positioned text runs. The reconstruction lives in
//!   [`layout`] and is pure, so it is tested without the native PDFium library.
//! * The PDFium adapter is thin and fails with an actionable message when the library is
//!   missing (see `scripts/fetch-pdfium.ps1`).

pub mod docx;
pub mod html;
pub mod layout;
pub mod md;
pub mod pdf;
pub mod text;

use anyhow::{bail, Result};
use serde::Serialize;

/// What a block is. Heading depth is 1-based.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
pub enum BlockKind {
    Heading(u8),
    Para,
    Table,
    ListItem,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct Block {
    pub kind: BlockKind,
    /// Plain text. Tables are rows joined by `\n`, cells by ` | `.
    pub text: String,
    /// 1-based page number. 0 when the format has no pages.
    pub page: u32,
    /// `[x0, y0, x1, y1]` in PDF points, when known.
    pub bbox: Option<[f32; 4]>,
}

impl Block {
    pub fn new(kind: BlockKind, text: impl Into<String>, page: u32) -> Self {
        Self {
            kind,
            text: text.into(),
            page,
            bbox: None,
        }
    }
}

#[derive(Debug, Clone, Default, PartialEq, Serialize)]
pub struct ParsedDoc {
    pub blocks: Vec<Block>,
    pub page_count: u32,
    pub title: Option<String>,
}

impl ParsedDoc {
    /// Concatenated plain text, one block per line. Used by tests and by the admin preview.
    pub fn text(&self) -> String {
        self.blocks
            .iter()
            .map(|b| b.text.as_str())
            .collect::<Vec<_>>()
            .join("\n")
    }

    /// Drop blocks with no alphanumeric content (page numbers, bullets, separators).
    pub fn prune_empty(mut self) -> Self {
        self.blocks
            .retain(|b| b.text.chars().any(|c| c.is_alphanumeric()));
        self
    }
}

pub trait Parser {
    fn parse(&self, bytes: &[u8]) -> Result<ParsedDoc>;
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Format {
    Pdf,
    Docx,
    Html,
    Markdown,
    Text,
}

impl Format {
    /// Infer from a file extension or a caller-provided hint.
    pub fn from_hint(hint: &str) -> Option<Self> {
        let h = hint.trim().trim_start_matches('.').to_ascii_lowercase();
        match h.as_str() {
            "pdf" => Some(Self::Pdf),
            "docx" => Some(Self::Docx),
            "html" | "htm" => Some(Self::Html),
            "md" | "markdown" => Some(Self::Markdown),
            "txt" | "text" => Some(Self::Text),
            _ => None,
        }
    }

    pub fn from_mime(mime: &str) -> Option<Self> {
        let m = mime
            .split(';')
            .next()
            .unwrap_or("")
            .trim()
            .to_ascii_lowercase();
        match m.as_str() {
            "application/pdf" => Some(Self::Pdf),
            "application/vnd.openxmlformats-officedocument.wordprocessingml.document" => {
                Some(Self::Docx)
            }
            "text/html" | "application/xhtml+xml" => Some(Self::Html),
            "text/markdown" => Some(Self::Markdown),
            "text/plain" => Some(Self::Text),
            _ => None,
        }
    }
}

/// Parse bytes in the given format.
pub fn parse(bytes: &[u8], format: Format) -> Result<ParsedDoc> {
    if bytes.is_empty() {
        bail!("refusing to parse an empty document");
    }
    let doc = match format {
        Format::Pdf => pdf::PdfParser.parse(bytes)?,
        Format::Docx => docx::DocxParser.parse(bytes)?,
        Format::Html => html::HtmlParser.parse(bytes)?,
        Format::Markdown => md::MarkdownParser.parse(bytes)?,
        Format::Text => text::TextParser.parse(bytes)?,
    };
    Ok(doc.prune_empty())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn format_detection() {
        assert_eq!(Format::from_hint("PDF"), Some(Format::Pdf));
        assert_eq!(Format::from_hint(".docx"), Some(Format::Docx));
        assert_eq!(Format::from_hint("htm"), Some(Format::Html));
        assert_eq!(Format::from_hint("nope"), None);
        assert_eq!(
            Format::from_mime("application/pdf; charset=binary"),
            Some(Format::Pdf)
        );
        assert_eq!(Format::from_mime("text/html"), Some(Format::Html));
    }

    #[test]
    fn empty_input_is_rejected() {
        assert!(parse(b"", Format::Text).is_err());
    }

    #[test]
    fn prune_drops_noise_blocks() {
        let doc = ParsedDoc {
            blocks: vec![
                Block::new(BlockKind::Para, "— — —", 1),
                Block::new(BlockKind::Para, "Real content", 1),
            ],
            ..Default::default()
        };
        let pruned = doc.prune_empty();
        assert_eq!(pruned.blocks.len(), 1);
        assert_eq!(pruned.blocks[0].text, "Real content");
    }
}
