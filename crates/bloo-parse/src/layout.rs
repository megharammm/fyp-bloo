//! Layout reconstruction from positioned text runs.
//!
//! PDF gives us glyphs with coordinates. This module turns them into lines, detects columns,
//! classifies headings, merges paragraphs and spots tables. It is deliberately pure so the
//! heuristics can be tested with synthetic runs â€” no PDFium, no fixtures, no native library.

use crate::{Block, BlockKind};

/// One positioned piece of text, as extracted from a PDF page.
#[derive(Debug, Clone, PartialEq)]
pub struct TextRun {
    pub text: String,
    /// Left edge, PDF points.
    pub x: f32,
    /// Baseline. PDF coordinates grow upwards, so larger y is higher on the page.
    pub y: f32,
    pub width: f32,
    pub size: f32,
    pub bold: bool,
    /// 1-based page number.
    pub page: u32,
}

impl TextRun {
    pub fn new(text: impl Into<String>, x: f32, y: f32, size: f32, page: u32) -> Self {
        let text = text.into();
        // A cheap width estimate; real runs carry a measured width.
        let width = text.chars().count() as f32 * size * 0.5;
        Self {
            text,
            x,
            y,
            width,
            size,
            bold: false,
            page,
        }
    }

    pub fn bold(mut self, bold: bool) -> Self {
        self.bold = bold;
        self
    }

    fn x1(&self) -> f32 {
        self.x + self.width
    }
}

#[derive(Debug, Clone)]
struct Line {
    text: String,
    cells: Vec<String>,
    x0: f32,
    x1: f32,
    y: f32,
    size: f32,
    bold: bool,
    page: u32,
}

impl Line {
    fn bbox(&self) -> [f32; 4] {
        [self.x0, self.y, self.x1, self.y + self.size]
    }
}

/// Turn positioned runs into ordered blocks.
pub fn build_blocks(runs: &[TextRun]) -> Vec<Block> {
    let mut pages: Vec<u32> = runs.iter().map(|r| r.page).collect();
    pages.sort_unstable();
    pages.dedup();

    let mut blocks = Vec::new();
    for page in pages {
        let page_runs: Vec<&TextRun> = runs.iter().filter(|r| r.page == page).collect();

        // A table is a property of the whole page: never split it into columns, or the
        // gutter between cells would be mistaken for a column gutter.
        let whole_page = build_lines(&page_runs);
        if looks_like_table(&whole_page) {
            blocks.extend(lines_to_blocks(&whole_page));
            continue;
        }

        for column in split_columns(&page_runs) {
            let lines = build_lines(&column);
            blocks.extend(lines_to_blocks(&lines));
        }
    }
    blocks
}

/// Two consecutive lines with three or more wide-gap segments: treat the page as tabular.
fn looks_like_table(lines: &[Line]) -> bool {
    lines
        .windows(2)
        .any(|w| w[0].cells.len() >= 3 && w[1].cells.len() >= 3)
}

/// Split a page into column groups when there is a wide vertical gutter.
///
/// Single-column documents must not be split: the gap has to be both absolutely wide
/// (>= 28 pt) and a meaningful fraction (>= 12%) of the content width, with at least 20% of
/// the runs on each side.
fn split_columns<'a>(runs: &[&'a TextRun]) -> Vec<Vec<&'a TextRun>> {
    if runs.len() < 8 {
        return vec![runs.to_vec()];
    }

    let mut spans: Vec<(f32, f32)> = runs.iter().map(|r| (r.x, r.x1())).collect();
    spans.sort_by(|a, b| a.0.partial_cmp(&b.0).unwrap_or(std::cmp::Ordering::Equal));

    let min_x = spans.first().map(|s| s.0).unwrap_or(0.0);
    let max_x = spans.iter().map(|s| s.1).fold(f32::MIN, f32::max);
    let content_width = (max_x - min_x).max(1.0);

    // Walk left to right, tracking the furthest right edge seen so far. A gutter is a place
    // where the next span starts well beyond everything before it.
    let mut max_right = spans[0].1;
    let mut best: Option<(f32, f32)> = None;
    for (start, end) in spans.iter().skip(1) {
        let gap = *start - max_right;
        if gap > 28.0 && gap > content_width * 0.12 && best.map(|(g, _)| gap > g).unwrap_or(true) {
            best = Some((gap, (max_right + *start) / 2.0));
        }
        if *end > max_right {
            max_right = *end;
        }
    }

    let Some((_, split_at)) = best else {
        return vec![runs.to_vec()];
    };

    let left: Vec<&TextRun> = runs.iter().copied().filter(|r| r.x < split_at).collect();
    let right: Vec<&TextRun> = runs.iter().copied().filter(|r| r.x >= split_at).collect();

    let min_each = (runs.len() as f32 * 0.2) as usize;
    if left.len() < min_each || right.len() < min_each {
        return vec![runs.to_vec()];
    }

    vec![left, right]
}

/// Group runs into lines by vertical proximity, then order each line left to right.
fn build_lines(runs: &[&TextRun]) -> Vec<Line> {
    let mut sorted: Vec<&TextRun> = runs.to_vec();
    sorted.sort_by(|a, b| {
        b.y.partial_cmp(&a.y)
            .unwrap_or(std::cmp::Ordering::Equal)
            .then(a.x.partial_cmp(&b.x).unwrap_or(std::cmp::Ordering::Equal))
    });

    let mut lines: Vec<Line> = Vec::new();
    let mut current: Vec<&TextRun> = Vec::new();
    let mut current_y = f32::MIN;

    for run in sorted {
        let tolerance = run.size.max(8.0) * 0.55;
        if !current.is_empty() && (current_y - run.y).abs() > tolerance {
            lines.push(merge_line(&mut current));
            current = Vec::new();
        }
        if current.is_empty() {
            current_y = run.y;
        }
        current.push(run);
    }
    if !current.is_empty() {
        lines.push(merge_line(&mut current));
    }

    // Columns are read top to bottom. build_lines already sorted by descending y, so the
    // line order is correct within a column.
    lines
}

fn merge_line(runs: &mut Vec<&TextRun>) -> Line {
    runs.sort_by(|a, b| a.x.partial_cmp(&b.x).unwrap_or(std::cmp::Ordering::Equal));

    let mut text = String::new();
    let mut cells: Vec<String> = Vec::new();
    let mut cell = String::new();
    let mut prev_end: Option<f32> = None;
    let mut weighted_size = 0.0f32;
    let mut chars = 0usize;
    let mut bold_chars = 0usize;

    for run in runs.iter() {
        let t = run.text.trim();
        if t.is_empty() {
            continue;
        }
        let gap = prev_end.map(|e| run.x - e).unwrap_or(0.0);
        // Words arrive as atomic runs from the PDF adapter, so any real gap is a word
        // boundary. A tight threshold avoids "DummyPDFfile" without splitting words,
        // because intra-word glyph gaps were already merged upstream.
        let space = if prev_end.is_some() && gap > (run.size * 0.10).max(0.6) {
            " "
        } else {
            ""
        };

        // A wide gap starts a new table cell.
        if prev_end.is_some() && gap > run.size * 1.6 {
            if !cell.trim().is_empty() {
                cells.push(cell.trim().to_string());
            }
            cell.clear();
        }

        text.push_str(space);
        text.push_str(t);
        if !cell.is_empty() {
            cell.push(' ');
        }
        cell.push_str(t);

        let n = t.chars().count();
        weighted_size += run.size * n as f32;
        chars += n;
        if run.bold {
            bold_chars += n;
        }
        prev_end = Some(run.x1());
    }

    if !cell.trim().is_empty() {
        cells.push(cell.trim().to_string());
    }

    let size = if chars > 0 {
        weighted_size / chars as f32
    } else {
        10.0
    };
    let x0 = runs.iter().map(|r| r.x).fold(f32::MAX, f32::min);
    let x1 = runs.iter().map(|r| r.x1()).fold(f32::MIN, f32::max);
    let page = runs.first().map(|r| r.page).unwrap_or(1);

    Line {
        text: collapse_spaces(&text),
        cells,
        x0,
        x1,
        y: runs.first().map(|r| r.y).unwrap_or(0.0),
        size,
        bold: chars > 0 && bold_chars * 2 >= chars,
        page,
    }
}

fn collapse_spaces(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    let mut prev_space = false;
    for c in s.chars() {
        if c.is_whitespace() {
            if !prev_space && !out.is_empty() {
                out.push(' ');
            }
            prev_space = true;
        } else {
            out.push(c);
            prev_space = false;
        }
    }
    out.trim().to_string()
}

/// Body text size, weighted by character count so a heading cannot drag the mode.
fn body_size(lines: &[Line]) -> f32 {
    let mut buckets: Vec<(i32, usize)> = Vec::new();
    for line in lines {
        let key = (line.size * 2.0).round() as i32; // 0.5 pt buckets
        let n = line.text.chars().count();
        match buckets.iter_mut().find(|(k, _)| *k == key) {
            Some((_, count)) => *count += n,
            None => buckets.push((key, n)),
        }
    }
    buckets
        .into_iter()
        .max_by_key(|(_, count)| *count)
        .map(|(k, _)| k as f32 / 2.0)
        .unwrap_or(10.0)
}

fn lines_to_blocks(lines: &[Line]) -> Vec<Block> {
    if lines.is_empty() {
        return Vec::new();
    }
    let body = body_size(lines);

    let mut blocks: Vec<Block> = Vec::new();
    let mut para = String::new();
    let mut para_bbox: Option<[f32; 4]> = None;
    let mut para_page = lines[0].page;
    let mut prev: Option<&Line> = None;

    let flush_para =
        |para: &mut String, bbox: &mut Option<[f32; 4]>, page: u32, blocks: &mut Vec<Block>| {
            let text = collapse_spaces(para);
            if !text.is_empty() {
                let mut b = Block::new(BlockKind::Para, text, page);
                b.bbox = *bbox;
                blocks.push(b);
            }
            para.clear();
            *bbox = None;
        };

    let mut i = 0;
    while i < lines.len() {
        let line = &lines[i];

        // Tables: a run of lines that each have >= 3 aligned-ish cells.
        if line.cells.len() >= 3 {
            let mut j = i;
            let mut rows: Vec<String> = Vec::new();
            while j < lines.len() && lines[j].cells.len() >= 3 {
                rows.push(lines[j].cells.join(" | "));
                j += 1;
            }
            if rows.len() >= 2 {
                flush_para(&mut para, &mut para_bbox, para_page, &mut blocks);
                let mut b = Block::new(BlockKind::Table, rows.join("\n"), line.page);
                b.bbox = Some(line.bbox());
                blocks.push(b);
                prev = Some(&lines[j - 1]);
                i = j;
                continue;
            }
        }

        let is_h1 = line.size >= body * 1.18;
        let is_h2 = !is_h1 && line.size >= body * 1.06 && line.bold;
        let is_h3 = !is_h1 && !is_h2 && line.bold && line.text.chars().count() <= 80;

        if is_h1 || is_h2 || is_h3 {
            flush_para(&mut para, &mut para_bbox, para_page, &mut blocks);
            let level = if is_h1 {
                1
            } else if is_h2 {
                2
            } else {
                3
            };
            let mut b = Block::new(BlockKind::Heading(level), line.text.clone(), line.page);
            b.bbox = Some(line.bbox());
            blocks.push(b);
            prev = Some(line);
            i += 1;
            continue;
        }

        // Paragraph continuation: same page, normal size, small vertical gap.
        let continues = match prev {
            Some(p) => {
                let gap = (p.y - line.y).abs();
                gap <= p.size * 1.9 && (line.size - body).abs() <= body * 0.08
            }
            None => false,
        };

        if continues {
            if !para.is_empty() && !para.ends_with('-') {
                para.push(' ');
            }
            if para.ends_with('-') {
                para.pop();
            }
            para.push_str(&line.text);
            if let Some(bb) = para_bbox.as_mut() {
                bb[0] = bb[0].min(line.x0);
                bb[1] = bb[1].min(line.y);
                bb[2] = bb[2].max(line.x1);
                bb[3] = bb[3].max(line.y + line.size);
            }
        } else {
            flush_para(&mut para, &mut para_bbox, para_page, &mut blocks);
            para_page = line.page;
            para.push_str(&line.text);
            para_bbox = Some(line.bbox());
        }

        prev = Some(line);
        i += 1;
    }

    flush_para(&mut para, &mut para_bbox, para_page, &mut blocks);
    blocks
}

#[cfg(test)]
mod tests {
    use super::*;

    fn runs_of(lines: &[(&str, f32, f32, f32)]) -> Vec<TextRun> {
        // (text, x, y, size)
        lines
            .iter()
            .map(|(t, x, y, s)| TextRun::new(*t, *x, *y, *s, 1))
            .collect()
    }

    #[test]
    fn merges_lines_into_one_paragraph() {
        let runs = runs_of(&[
            ("The hostel fee is payable", 72.0, 700.0, 10.0),
            ("before the fifteenth of August.", 72.0, 686.0, 10.0),
        ]);
        let blocks = build_blocks(&runs);
        assert_eq!(blocks.len(), 1);
        assert_eq!(
            blocks[0].text,
            "The hostel fee is payable before the fifteenth of August."
        );
        assert_eq!(blocks[0].kind, BlockKind::Para);
    }

    #[test]
    fn detects_a_large_heading() {
        let runs = runs_of(&[
            ("HOSTEL RULES", 72.0, 760.0, 20.0),
            (
                "Residents must keep the corridors clear.",
                72.0,
                720.0,
                10.0,
            ),
            (
                "Quiet hours start at ten in the evening.",
                72.0,
                706.0,
                10.0,
            ),
        ]);
        let blocks = build_blocks(&runs);
        assert_eq!(blocks[0].kind, BlockKind::Heading(1));
        assert_eq!(blocks[0].text, "HOSTEL RULES");
        assert_eq!(blocks[1].kind, BlockKind::Para);
    }

    #[test]
    fn splits_paragraphs_on_a_large_gap() {
        let runs = runs_of(&[
            ("First paragraph line one.", 72.0, 700.0, 10.0),
            ("First paragraph line two.", 72.0, 686.0, 10.0),
            ("Second paragraph after a gap.", 72.0, 600.0, 10.0),
        ]);
        let blocks = build_blocks(&runs);
        assert_eq!(blocks.len(), 2);
        assert!(blocks[0].text.starts_with("First paragraph"));
        assert!(blocks[1].text.starts_with("Second paragraph"));
    }

    #[test]
    fn detects_a_table() {
        let cells = [
            ("Category", 72.0, 700.0),
            ("Fee", 220.0, 700.0),
            ("Deadline", 340.0, 700.0),
            ("Hostel", 72.0, 684.0),
            ("48000", 220.0, 684.0),
            ("15 Aug", 340.0, 684.0),
            ("Mess", 72.0, 668.0),
            ("3200", 220.0, 668.0),
            ("Monthly", 340.0, 668.0),
        ];
        let runs: Vec<TextRun> = cells
            .iter()
            .map(|(t, x, y)| TextRun::new(*t, *x, *y, 10.0, 1))
            .collect();

        let blocks = build_blocks(&runs);
        let table = blocks
            .iter()
            .find(|b| b.kind == BlockKind::Table)
            .expect("table block");
        assert!(
            table.text.contains("Hostel | 48000 | 15 Aug"),
            "{}",
            table.text
        );
        assert!(
            table.text.contains("Mess | 3200 | Monthly"),
            "{}",
            table.text
        );
    }

    #[test]
    fn reads_columns_in_order() {
        // Two columns: left x=72, right x=320, same rows.
        let mut runs = Vec::new();
        for i in 0..6 {
            let y = 700.0 - i as f32 * 14.0;
            runs.push(TextRun::new(format!("left line {i}"), 72.0, y, 10.0, 1));
            runs.push(TextRun::new(format!("right line {i}"), 320.0, y, 10.0, 1));
        }
        let blocks = build_blocks(&runs);
        let text = blocks
            .iter()
            .map(|b| b.text.as_str())
            .collect::<Vec<_>>()
            .join("\n");
        let left = text.find("left line 0").unwrap();
        let right = text.find("right line 0").unwrap();
        assert!(left < right, "left column must come first:\n{text}");
        assert!(blocks.iter().filter(|b| b.kind == BlockKind::Para).count() >= 2);
    }

    #[test]
    fn page_numbers_are_preserved() {
        let mut a = TextRun::new("page one text", 72.0, 700.0, 10.0, 1);
        a.page = 1;
        let mut b = TextRun::new("page two text", 72.0, 700.0, 10.0, 2);
        b.page = 2;
        let blocks = build_blocks(&[a, b]);
        assert_eq!(blocks.len(), 2);
        assert_eq!(blocks[0].page, 1);
        assert_eq!(blocks[1].page, 2);
    }
}
