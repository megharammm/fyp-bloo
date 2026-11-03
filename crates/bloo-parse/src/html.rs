use crate::{Block, BlockKind, ParsedDoc, Parser};
use anyhow::Result;
use scraper::{ElementRef, Html, Selector};

/// HTML: prefers `<article>` / `<main>`, drops navigation chrome, keeps headings, paragraphs,
/// list items and tables.
pub struct HtmlParser;

impl Parser for HtmlParser {
    fn parse(&self, bytes: &[u8]) -> Result<ParsedDoc> {
        let html = String::from_utf8_lossy(bytes);
        let doc = Html::parse_document(&html);

        let root = doc
            .select(&sel("article, main"))
            .next()
            .unwrap_or_else(|| doc.root_element());

        let title = doc
            .select(&sel("title"))
            .next()
            .map(|t| collapse(&t.text().collect::<String>()))
            .filter(|t| !t.is_empty());

        let mut blocks = Vec::new();
        for el in root.select(&sel("h1, h2, h3, h4, h5, h6, p, li, table")) {
            if in_boilerplate(&el) {
                continue;
            }

            let tag = el.value().name();
            match tag {
                "table" => {
                    if let Some(text) = table_text(&el) {
                        blocks.push(Block::new(BlockKind::Table, text, 1));
                    }
                }
                _ if tag.starts_with('h') && tag.len() == 2 => {
                    let level = tag[1..].parse::<u8>().unwrap_or(2).clamp(1, 6);
                    let text = collapse(&el.text().collect::<String>());
                    if !text.is_empty() {
                        blocks.push(Block::new(BlockKind::Heading(level), text, 1));
                    }
                }
                _ => {
                    let text = collapse(&el.text().collect::<String>());
                    if !text.is_empty() {
                        let kind = if tag == "li" {
                            BlockKind::ListItem
                        } else {
                            BlockKind::Para
                        };
                        blocks.push(Block::new(kind, text, 1));
                    }
                }
            }
        }

        Ok(ParsedDoc {
            blocks,
            page_count: 1,
            title,
        })
    }
}

fn sel(s: &str) -> Selector {
    Selector::parse(s).expect("static selector")
}

fn in_boilerplate(el: &ElementRef) -> bool {
    let mut cur = el.parent();
    while let Some(node) = cur {
        if let scraper::Node::Element(element) = node.value() {
            if matches!(
                element.name(),
                "nav" | "footer" | "aside" | "script" | "style" | "noscript"
            ) {
                return true;
            }
        }
        cur = node.parent();
    }
    false
}

fn table_text(el: &ElementRef) -> Option<String> {
    let mut rows = Vec::new();
    for row in el.select(&sel("tr")) {
        let cells: Vec<String> = row
            .select(&sel("th, td"))
            .map(|c| collapse(&c.text().collect::<String>()))
            .collect();
        if !cells.is_empty() {
            rows.push(cells.join(" | "));
        }
    }
    if rows.is_empty() {
        None
    } else {
        Some(rows.join("\n"))
    }
}

fn collapse(s: &str) -> String {
    s.split_whitespace().collect::<Vec<_>>().join(" ")
}

#[cfg(test)]
mod tests {
    use super::*;

    const PAGE: &str = r#"
        <html><head><title>Hostel Rules 2026</title></head>
        <body>
          <nav><a href="/">Home</a><a href="/fees">Fees</a></nav>
          <article>
            <h1>Hostel Rules</h1>
            <p>Quiet hours start at <strong>ten</strong> in the evening.</p>
            <ul><li>No visitors after 9 pm</li><li>Keep corridors clear</li></ul>
            <table>
              <tr><th>Item</th><th>Fee</th></tr>
              <tr><td>Hostel</td><td>48000</td></tr>
            </table>
          </article>
          <footer>Copyright 2026</footer>
        </body></html>
    "#;

    #[test]
    fn extracts_content_and_ignores_chrome() {
        let doc = HtmlParser.parse(PAGE.as_bytes()).unwrap();
        let text = doc.text();

        assert_eq!(doc.title.as_deref(), Some("Hostel Rules 2026"));
        assert_eq!(doc.blocks[0].kind, BlockKind::Heading(1));
        assert_eq!(doc.blocks[0].text, "Hostel Rules");
        assert!(text.contains("Quiet hours start at ten in the evening."));
        assert!(text.contains("No visitors after 9 pm"));
        assert!(!text.contains("Home"), "nav must be dropped: {text}");
        assert!(
            !text.contains("Copyright"),
            "footer must be dropped: {text}"
        );
    }

    #[test]
    fn extracts_tables() {
        let doc = HtmlParser.parse(PAGE.as_bytes()).unwrap();
        let table = doc
            .blocks
            .iter()
            .find(|b| b.kind == BlockKind::Table)
            .expect("table");
        assert!(table.text.contains("Item | Fee"), "{}", table.text);
        assert!(table.text.contains("Hostel | 48000"), "{}", table.text);
    }

    #[test]
    fn falls_back_to_body_without_article() {
        let html = b"<html><body><h2>Notice</h2><p>Exams start on Monday.</p></body></html>";
        let doc = HtmlParser.parse(html).unwrap();
        assert_eq!(doc.blocks.len(), 2);
        assert_eq!(doc.blocks[0].kind, BlockKind::Heading(2));
    }
}
