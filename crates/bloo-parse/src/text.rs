use crate::{Block, BlockKind, ParsedDoc, Parser};
use anyhow::Result;

/// Plain text: blank lines separate paragraphs, everything else is one flow.
pub struct TextParser;

impl Parser for TextParser {
    fn parse(&self, bytes: &[u8]) -> Result<ParsedDoc> {
        let raw = String::from_utf8_lossy(bytes)
            .replace("\r\n", "\n")
            .replace('\r', "\n");
        let mut blocks = Vec::new();

        for chunk in raw.split("\n\n") {
            let text = collapse(chunk);
            if !text.is_empty() {
                blocks.push(Block::new(BlockKind::Para, text, 1));
            }
        }

        Ok(ParsedDoc {
            blocks,
            page_count: 1,
            title: None,
        })
    }
}

fn collapse(s: &str) -> String {
    s.split_whitespace().collect::<Vec<_>>().join(" ")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn splits_on_blank_lines() {
        let doc = TextParser
            .parse(b"First para.\nStill first.\n\nSecond para.\n")
            .unwrap();
        assert_eq!(doc.blocks.len(), 2);
        assert_eq!(doc.blocks[0].text, "First para. Still first.");
        assert_eq!(doc.blocks[1].text, "Second para.");
    }

    #[test]
    fn handles_crlf() {
        let doc = TextParser.parse(b"a\r\n\r\nb\r\n").unwrap();
        assert_eq!(doc.blocks.len(), 2);
    }
}
