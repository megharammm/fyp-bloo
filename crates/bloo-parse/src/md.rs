use crate::{Block, BlockKind, ParsedDoc, Parser};
use anyhow::Result;
use pulldown_cmark::{Event, Options, Parser as MdParser, Tag, TagEnd};

/// Markdown: headings, paragraphs, list items and pipe tables.
pub struct MarkdownParser;

impl Parser for MarkdownParser {
    fn parse(&self, bytes: &[u8]) -> Result<ParsedDoc> {
        let src = String::from_utf8_lossy(bytes);
        let mut opts = Options::empty();
        opts.insert(Options::ENABLE_TABLES);
        opts.insert(Options::ENABLE_STRIKETHROUGH);

        let mut blocks: Vec<Block> = Vec::new();
        let mut buf = String::new();
        let mut kind: Option<BlockKind> = None;

        // table state
        let mut in_table = false;
        let mut rows: Vec<String> = Vec::new();
        let mut row: Vec<String> = Vec::new();
        let mut cell = String::new();

        for event in MdParser::new_ext(&src, opts) {
            match event {
                Event::Start(Tag::Table(_)) => {
                    in_table = true;
                    rows.clear();
                }
                Event::End(TagEnd::Table) => {
                    in_table = false;
                    let text = rows.join("\n");
                    if !text.trim().is_empty() {
                        blocks.push(Block::new(BlockKind::Table, text, 1));
                    }
                }
                Event::Start(Tag::TableCell) => cell.clear(),
                Event::End(TagEnd::TableCell) => {
                    row.push(cell.trim().to_string());
                }
                Event::End(TagEnd::TableRow) => {
                    if !row.is_empty() {
                        rows.push(row.join(" | "));
                        row.clear();
                    }
                }
                Event::Start(Tag::Heading { level, .. }) => {
                    buf.clear();
                    kind = Some(BlockKind::Heading(level as u8));
                }
                Event::End(TagEnd::Heading(_)) => {
                    push(&mut blocks, kind.take(), &mut buf);
                }
                Event::Start(Tag::Paragraph) => {
                    if !in_table {
                        buf.clear();
                        kind = Some(BlockKind::Para);
                    }
                }
                Event::End(TagEnd::Paragraph) => {
                    if !in_table {
                        push(&mut blocks, kind.take(), &mut buf);
                    }
                }
                Event::Start(Tag::Item) => {
                    buf.clear();
                    kind = Some(BlockKind::ListItem);
                }
                Event::End(TagEnd::Item) => {
                    push(&mut blocks, kind.take(), &mut buf);
                }
                Event::Start(Tag::CodeBlock(_)) => {
                    buf.clear();
                    kind = Some(BlockKind::Para);
                }
                Event::End(TagEnd::CodeBlock) => {
                    push(&mut blocks, kind.take(), &mut buf);
                }
                Event::Text(t) | Event::Code(t) => {
                    if in_table {
                        cell.push_str(&t);
                    } else {
                        buf.push_str(&t);
                    }
                }
                Event::SoftBreak | Event::HardBreak => {
                    if in_table {
                        cell.push(' ');
                    } else {
                        buf.push(' ');
                    }
                }
                _ => {}
            }
        }

        Ok(ParsedDoc {
            blocks,
            page_count: 1,
            title: None,
        })
    }
}

fn push(blocks: &mut Vec<Block>, kind: Option<BlockKind>, buf: &mut String) {
    let text = buf.split_whitespace().collect::<Vec<_>>().join(" ");
    buf.clear();
    if let Some(kind) = kind {
        if !text.is_empty() {
            blocks.push(Block::new(kind, text, 1));
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reads_headings_paragraphs_and_lists() {
        let src = b"# Hostel Rules\n\nQuiet hours start at ten.\n\n- No visitors after 9 pm\n- Keep corridors clear\n";
        let doc = MarkdownParser.parse(src).unwrap();

        assert_eq!(doc.blocks[0].kind, BlockKind::Heading(1));
        assert_eq!(doc.blocks[0].text, "Hostel Rules");
        assert_eq!(doc.blocks[1].kind, BlockKind::Para);
        assert_eq!(doc.blocks[1].text, "Quiet hours start at ten.");
        assert_eq!(doc.blocks[2].kind, BlockKind::ListItem);
        assert_eq!(doc.blocks[2].text, "No visitors after 9 pm");
    }

    #[test]
    fn reads_pipe_tables() {
        let src = b"| Item | Fee |\n|---|---|\n| Hostel | 48000 |\n| Mess | 3200 |\n";
        let doc = MarkdownParser.parse(src).unwrap();
        let table = doc
            .blocks
            .iter()
            .find(|b| b.kind == BlockKind::Table)
            .expect("table");
        assert!(table.text.contains("Hostel | 48000"), "{}", table.text);
        assert!(table.text.contains("Mess | 3200"), "{}", table.text);
    }
}
