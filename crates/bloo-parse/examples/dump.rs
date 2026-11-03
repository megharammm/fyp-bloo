//! Print the block stream for a document.
//!
//! ```powershell
//! cargo run -p bloo_parse --example dump -- content/seed/fees/hostel.pdf
//! ```
//!
//! Useful for judging parse quality before publishing: every block shows its kind and page.

use anyhow::{Context, Result};
use std::path::Path;

fn main() -> Result<()> {
    let path = std::env::args()
        .nth(1)
        .context("usage: dump <file.pdf|docx|html|md|txt>")?;
    let p = Path::new(&path);
    let hint = p.extension().and_then(|e| e.to_str()).unwrap_or("");
    let format = bloo_parse::Format::from_hint(hint)
        .with_context(|| format!("unsupported extension {hint:?}"))?;

    let bytes = std::fs::read(p).with_context(|| format!("cannot read {path}"))?;
    let doc = bloo_parse::parse(&bytes, format)?;

    println!(
        "format={format:?} pages={} blocks={} title={:?}",
        doc.page_count,
        doc.blocks.len(),
        doc.title
    );

    for (i, block) in doc.blocks.iter().enumerate() {
        let mut preview: String = block.text.chars().take(160).collect();
        preview = preview.replace('\n', " \\n ");
        println!("{i:>4}  {:?}  p{:<3}  {}", block.kind, block.page, preview);
    }

    Ok(())
}
