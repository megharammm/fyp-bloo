use crate::layout::{build_blocks, TextRun};
use crate::{Block, ParsedDoc, Parser};
use anyhow::{anyhow, Context, Result};
use pdfium_render::prelude::*;

/// PDF: glyphs with coordinates are collapsed into words, then handed to
/// [`crate::layout`] which reconstructs lines, columns, headings and tables.
///
/// Requires the native `pdfium` library. `scripts/fetch-pdfium.ps1` installs it into
/// `data/pdfium/`; `PDFIUM_PATH` overrides.
pub struct PdfParser;

impl Parser for PdfParser {
    fn parse(&self, bytes: &[u8]) -> Result<ParsedDoc> {
        let bindings = bind()?;
        let pdfium = Pdfium::new(bindings);

        let document = pdfium
            .load_pdf_from_byte_slice(bytes, None)
            .context("pdfium could not open this PDF")?;

        let page_count = document.pages().len() as u32;
        let mut runs: Vec<TextRun> = Vec::new();

        for (index, page) in document.pages().iter().enumerate() {
            let page_number = (index + 1) as u32;
            let text = page
                .text()
                .with_context(|| format!("cannot read text on page {page_number}"))?;

            let mut word = String::new();
            let mut word_x0 = 0.0f32;
            let mut word_x1 = 0.0f32;
            let mut word_y = 0.0f32;
            let mut word_size = 10.0f32;
            let mut word_bold = false;
            let mut prev_right: Option<f32> = None;

            let flush = |word: &mut String,
                         x0: f32,
                         x1: f32,
                         y: f32,
                         size: f32,
                         bold: bool,
                         runs: &mut Vec<TextRun>| {
                let t = word.trim();
                if t.is_empty() {
                    word.clear();
                    return;
                }
                let mut run = TextRun::new(t.to_string(), x0, y, size, page_number);
                run.width = (x1 - x0).max(0.0);
                run.bold = bold;
                runs.push(run);
                word.clear();
            };

            for ch in text.chars().iter() {
                let c = ch.unicode_char().unwrap_or(' ');
                let bounds = ch.loose_bounds()?;
                let left = bounds.left().value;
                let right = bounds.right().value;
                let bottom = bounds.bottom().value;
                let size = ch.scaled_font_size().value;

                let gap = prev_right.map(|p| left - p).unwrap_or(0.0);
                let is_space = c.is_whitespace();
                let wide = prev_right.is_some() && gap > size * 0.6;

                if (is_space || wide) && !word.is_empty() {
                    flush(
                        &mut word, word_x0, word_x1, word_y, word_size, word_bold, &mut runs,
                    );
                }

                prev_right = Some(right);
                if is_space {
                    continue;
                }

                if word.is_empty() {
                    word_x0 = left;
                    word_y = bottom;
                    word_size = size;
                    word_bold = ch.font_is_bold_reenforced();
                    word_x1 = right;
                } else {
                    word_x1 = right;
                    word_bold &= ch.font_is_bold_reenforced();
                }
                word.push(c);
            }

            flush(
                &mut word, word_x0, word_x1, word_y, word_size, word_bold, &mut runs,
            );
        }

        let blocks: Vec<Block> = build_blocks(&runs);
        Ok(ParsedDoc {
            blocks,
            page_count,
            title: None,
        })
    }
}

/// Bind to pdfium, preferring `PDFIUM_PATH`, then `data/pdfium/`, then the system loader.
fn bind() -> Result<Box<dyn PdfiumLibraryBindings>> {
    if let Ok(path) = std::env::var("PDFIUM_PATH") {
        let path = path.trim().to_string();
        if !path.is_empty() {
            let shown = path.clone();
            return Pdfium::bind_to_library(path)
                .map_err(|e| anyhow!("PDFIUM_PATH={shown} could not be loaded: {e}"));
        }
    }

    for candidate in local_candidates() {
        if candidate.exists() {
            if let Ok(bindings) = Pdfium::bind_to_library(candidate.to_string_lossy().to_string()) {
                return Ok(bindings);
            }
        }
    }

    Pdfium::bind_to_system_library().map_err(|e| {
        anyhow!(
            "pdfium library not found ({e}). Run `scripts\\fetch-pdfium.ps1` to install it into \
             data/pdfium/, or set PDFIUM_PATH to pdfium.dll."
        )
    })
}

fn local_candidates() -> Vec<std::path::PathBuf> {
    let mut v = vec![
        std::path::PathBuf::from("data/pdfium/pdfium.dll"),
        std::path::PathBuf::from("data/pdfium/libpdfium.so"),
        std::path::PathBuf::from("data/pdfium/libpdfium.dylib"),
    ];
    // When running from target/debug, the repo root is two levels up.
    if let Ok(cwd) = std::env::current_dir() {
        v.push(cwd.join("../../data/pdfium/pdfium.dll"));
        v.push(cwd.join("../../data/pdfium/libpdfium.so"));
    }
    v
}
