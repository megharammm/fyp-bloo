use crate::{Block, BlockKind, ParsedDoc, Parser};
use anyhow::{Context, Result};
use docx_rs::{
    DocumentChild, Paragraph, ParagraphChild, RunChild, TableCellContent, TableChild, TableRowChild,
};

/// DOCX: paragraphs with `Heading1..6` styles become headings, tables become pipe rows.
pub struct DocxParser;

impl Parser for DocxParser {
    fn parse(&self, bytes: &[u8]) -> Result<ParsedDoc> {
        let docx = docx_rs::read_docx(bytes).context("not a valid .docx file")?;
        let mut blocks = Vec::new();

        for child in &docx.document.children {
            match child {
                DocumentChild::Paragraph(p) => {
                    let text = para_text(p);
                    if text.is_empty() {
                        continue;
                    }
                    let kind = match heading_level(p) {
                        Some(level) => BlockKind::Heading(level),
                        None => BlockKind::Para,
                    };
                    blocks.push(Block::new(kind, text, 1));
                }
                DocumentChild::Table(t) => {
                    let mut rows = Vec::new();
                    for row_child in &t.rows {
                        let TableChild::TableRow(row) = row_child;
                        let mut cells = Vec::new();
                        for cell_child in &row.cells {
                            let TableRowChild::TableCell(cell) = cell_child;
                            let mut text = String::new();
                            for content in &cell.children {
                                if let TableCellContent::Paragraph(p) = content {
                                    if !text.is_empty() {
                                        text.push(' ');
                                    }
                                    text.push_str(&para_text(p));
                                }
                            }
                            cells.push(text.trim().to_string());
                        }
                        if !cells.is_empty() {
                            rows.push(cells.join(" | "));
                        }
                    }
                    if !rows.is_empty() {
                        blocks.push(Block::new(BlockKind::Table, rows.join("\n"), 1));
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

fn para_text(p: &Paragraph) -> String {
    let mut out = String::new();
    for child in &p.children {
        if let ParagraphChild::Run(run) = child {
            for part in &run.children {
                if let RunChild::Text(t) = part {
                    out.push_str(&t.text);
                }
            }
        }
    }
    out.split_whitespace().collect::<Vec<_>>().join(" ")
}

fn heading_level(p: &Paragraph) -> Option<u8> {
    let style = p.property.style.as_ref()?.val.trim().to_ascii_lowercase();
    if style == "title" {
        return Some(1);
    }
    let rest = style.strip_prefix("heading")?.trim();
    rest.parse::<u8>().ok().map(|n| n.clamp(1, 6))
}

#[cfg(test)]
mod tests {
    use super::*;
    use docx_rs::*;

    fn sample_docx() -> Vec<u8> {
        let docx = Docx::new()
            .add_paragraph(
                Paragraph::new()
                    .style("Heading1")
                    .add_run(Run::new().add_text("Hostel Rules")),
            )
            .add_paragraph(
                Paragraph::new().add_run(Run::new().add_text("Quiet hours start at ten.")),
            )
            .add_table(Table::new(vec![
                TableRow::new(vec![
                    TableCell::new()
                        .add_paragraph(Paragraph::new().add_run(Run::new().add_text("Item"))),
                    TableCell::new()
                        .add_paragraph(Paragraph::new().add_run(Run::new().add_text("Fee"))),
                ]),
                TableRow::new(vec![
                    TableCell::new()
                        .add_paragraph(Paragraph::new().add_run(Run::new().add_text("Hostel"))),
                    TableCell::new()
                        .add_paragraph(Paragraph::new().add_run(Run::new().add_text("48000"))),
                ]),
            ]));

        let mut buf = std::io::Cursor::new(Vec::new());
        docx.build().pack(&mut buf).expect("pack docx");
        buf.into_inner()
    }

    #[test]
    fn reads_headings_paragraphs_and_tables() {
        let bytes = sample_docx();
        let doc = DocxParser.parse(&bytes).unwrap();

        assert_eq!(doc.blocks[0].kind, BlockKind::Heading(1));
        assert_eq!(doc.blocks[0].text, "Hostel Rules");
        assert_eq!(doc.blocks[1].kind, BlockKind::Para);
        assert_eq!(doc.blocks[1].text, "Quiet hours start at ten.");

        let table = doc
            .blocks
            .iter()
            .find(|b| b.kind == BlockKind::Table)
            .expect("table");
        assert!(table.text.contains("Item | Fee"), "{}", table.text);
        assert!(table.text.contains("Hostel | 48000"), "{}", table.text);
    }

    #[test]
    fn rejects_garbage() {
        assert!(DocxParser.parse(b"not a zip").is_err());
    }
}
