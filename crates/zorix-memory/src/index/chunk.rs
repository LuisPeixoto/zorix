//! Cutting text into pieces of about 800 tokens that keep their place: lines, page and heading.

use zorix_core::{NewChunk, redact_text, text_hash};

use super::extract::Page;
use super::walk::Kind;

/// The size a piece aims for, in characters (about 800 tokens).
pub const TARGET_CHARS: usize = 3_200;
/// A code or data window, in lines.
pub const WINDOW_LINES: usize = 120;
/// Lines shared by neighbouring windows.
pub const OVERLAP_LINES: usize = 10;
/// Most pieces of one file that are kept.
pub const MAX_CHUNKS: usize = 2_000;

/// The pieces of a file and whether some were left out.
#[derive(Debug, PartialEq)]
pub struct Chunked {
    /// The pieces, in order.
    pub chunks: Vec<NewChunk>,
    /// True when the file had more pieces than [`MAX_CHUNKS`].
    pub truncated: bool,
}

fn piece(text: &str, heading: Option<&str>, lines: Option<(u32, u32)>, page: Option<u32>) -> Option<NewChunk> {
    let text = redact_text(text.trim());
    if text.is_empty() {
        return None;
    }
    Some(NewChunk {
        text_hash: text_hash(&text),
        tokens: (text.chars().count() as u32).div_ceil(4) + 4,
        text,
        heading: heading.map(str::to_string),
        line_start: lines.map(|l| l.0),
        line_end: lines.map(|l| l.1),
        page,
    })
}

/// Cut the pages of a file of `kind` into pieces.
pub fn chunk(kind: Kind, pages: &[Page]) -> Chunked {
    let mut chunks = Vec::new();
    for page in pages {
        match (kind, page.number) {
            (Kind::Markdown, _) => chunks.extend(markdown(&page.text)),
            (_, Some(number)) => chunks.extend(by_paragraph(&page.text, None, Some(number), 1)),
            _ => chunks.extend(windows(&page.text)),
        }
        if chunks.len() > MAX_CHUNKS {
            break;
        }
    }
    let truncated = chunks.len() > MAX_CHUNKS;
    chunks.truncate(MAX_CHUNKS);
    Chunked { chunks, truncated }
}

/// Windows of lines with a little overlap, each also kept under a size so a minified line cannot make a huge piece.
fn windows(text: &str) -> Vec<NewChunk> {
    let lines: Vec<&str> = text.lines().collect();
    let mut out = Vec::new();
    let mut start = 0;
    while start < lines.len() {
        let mut end = start;
        let mut chars = 0;
        while end < lines.len()
            && end - start < WINDOW_LINES
            && (chars == 0 || chars + lines[end].len() <= TARGET_CHARS * 2)
        {
            chars += lines[end].len() + 1;
            end += 1;
        }
        // A single line longer than the limit is cut into parts of its own.
        if end == start + 1 && lines[start].len() > TARGET_CHARS * 2 {
            for part in split_long(lines[start]) {
                out.extend(piece(&part, None, Some((start as u32 + 1, start as u32 + 1)), None));
            }
        } else {
            out.extend(piece(&lines[start..end].join("\n"), None, Some((start as u32 + 1, end as u32)), None));
        }
        if end >= lines.len() {
            break;
        }
        start = end.saturating_sub(OVERLAP_LINES).max(start + 1);
    }
    out
}

fn split_long(line: &str) -> Vec<String> {
    let chars: Vec<char> = line.chars().collect();
    chars.chunks(TARGET_CHARS).map(|c| c.iter().collect()).collect()
}

/// Markdown by headings, keeping the path of headings; a long section is cut by paragraphs.
fn markdown(text: &str) -> Vec<NewChunk> {
    struct Section {
        path: Vec<(usize, String)>,
        start: usize,
        lines: Vec<String>,
    }
    let mut sections: Vec<Section> = Vec::new();
    let mut path: Vec<(usize, String)> = Vec::new();
    let mut in_fence = false;
    let mut current = Section { path: Vec::new(), start: 1, lines: Vec::new() };
    for (index, line) in text.lines().enumerate() {
        let trimmed = line.trim_start();
        if trimmed.starts_with("```") || trimmed.starts_with("~~~") {
            in_fence = !in_fence;
        }
        let level = if in_fence { 0 } else { trimmed.chars().take_while(|c| *c == '#').count() };
        if (1..=6).contains(&level) && trimmed.chars().nth(level) == Some(' ') {
            sections.push(std::mem::replace(
                &mut current,
                Section { path: Vec::new(), start: index + 1, lines: Vec::new() },
            ));
            while path.last().is_some_and(|(l, _)| *l >= level) {
                path.pop();
            }
            path.push((level, trimmed[level..].trim().trim_end_matches('#').trim().to_string()));
            current.path = path.clone();
        }
        current.lines.push(line.to_string());
    }
    sections.push(current);
    let mut out = Vec::new();
    for section in sections {
        let heading = (!section.path.is_empty())
            .then(|| section.path.iter().map(|(_, h)| h.as_str()).collect::<Vec<_>>().join(" > "));
        let body = section.lines.join("\n");
        if body.chars().count() <= TARGET_CHARS {
            let end = section.start + section.lines.len().saturating_sub(1);
            out.extend(piece(&body, heading.as_deref(), Some((section.start as u32, end as u32)), None));
        } else {
            out.extend(by_paragraph(&body, heading.as_deref(), None, section.start));
        }
    }
    out
}

/// Pieces made of whole paragraphs up to the target size. `first_line` is the line the text starts at in its file.
fn by_paragraph(text: &str, heading: Option<&str>, page: Option<u32>, first_line: usize) -> Vec<NewChunk> {
    let mut out = Vec::new();
    let mut buffer = String::new();
    let mut buffer_start = first_line;
    let mut line = first_line;
    let mut paragraph = String::new();
    let mut paragraph_start = first_line;
    let flush = |buffer: &mut String, start: usize, end: usize, out: &mut Vec<NewChunk>| {
        if !buffer.trim().is_empty() {
            out.extend(piece(buffer, heading, page.is_none().then_some((start as u32, end as u32)), page));
        }
        buffer.clear();
    };
    let lines: Vec<&str> = text.lines().collect();
    for (i, current) in lines.iter().enumerate() {
        if paragraph.is_empty() {
            paragraph_start = line;
        }
        if current.trim().is_empty() {
            if !paragraph.is_empty() {
                add(
                    &mut buffer,
                    &mut buffer_start,
                    &mut paragraph,
                    paragraph_start,
                    line.saturating_sub(1),
                    &mut out,
                    heading,
                    page,
                );
            }
        } else {
            paragraph.push_str(current);
            paragraph.push('\n');
        }
        line += 1;
        if i + 1 == lines.len() && !paragraph.is_empty() {
            add(
                &mut buffer,
                &mut buffer_start,
                &mut paragraph,
                paragraph_start,
                line.saturating_sub(1),
                &mut out,
                heading,
                page,
            );
        }
    }
    flush(&mut buffer, buffer_start, line.saturating_sub(1), &mut out);
    return out;

    #[allow(clippy::too_many_arguments)]
    fn add(
        buffer: &mut String,
        buffer_start: &mut usize,
        paragraph: &mut String,
        paragraph_start: usize,
        paragraph_end: usize,
        out: &mut Vec<NewChunk>,
        heading: Option<&str>,
        page: Option<u32>,
    ) {
        if paragraph.chars().count() > TARGET_CHARS {
            // A paragraph that is too long on its own is cut.
            if !buffer.trim().is_empty() {
                out.extend(piece(
                    buffer,
                    heading,
                    page.is_none().then_some((*buffer_start as u32, paragraph_start.saturating_sub(1) as u32)),
                    page,
                ));
                buffer.clear();
            }
            for part in split_long_text(paragraph) {
                out.extend(piece(
                    &part,
                    heading,
                    page.is_none().then_some((paragraph_start as u32, paragraph_end as u32)),
                    page,
                ));
            }
            paragraph.clear();
            *buffer_start = paragraph_end + 1;
            return;
        }
        if buffer.chars().count() + paragraph.chars().count() > TARGET_CHARS && !buffer.trim().is_empty() {
            out.extend(piece(
                buffer,
                heading,
                page.is_none().then_some((*buffer_start as u32, paragraph_start.saturating_sub(1) as u32)),
                page,
            ));
            buffer.clear();
            *buffer_start = paragraph_start;
        }
        if buffer.is_empty() {
            *buffer_start = paragraph_start;
        }
        buffer.push_str(paragraph);
        buffer.push('\n');
        paragraph.clear();
    }
}

fn split_long_text(text: &str) -> Vec<String> {
    let chars: Vec<char> = text.chars().collect();
    chars.chunks(TARGET_CHARS).map(|c| c.iter().collect()).collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn text(t: &str) -> Vec<Page> {
        vec![Page { number: None, text: t.into() }]
    }

    #[test]
    fn markdown_is_cut_by_headings_that_keep_their_path_and_lines() {
        let md = "# Guide\nIntro line.\n\n## Deploys\nDeploys go to staging.\nThen production.\n\n### Rollbacks\nUse the previous image.\n\n## Office\nLunch at noon.\n";
        let chunked = chunk(Kind::Markdown, &text(md));
        assert!(!chunked.truncated);
        let by: Vec<Option<&str>> = chunked.chunks.iter().map(|c| c.heading.as_deref()).collect();
        assert_eq!(by[0], Some("Guide"));
        assert!(chunked.chunks.iter().any(|c| c.heading.as_deref() == Some("Guide > Deploys")
            && c.text.contains("staging")
            && c.line_start == Some(4)));
        assert!(
            chunked
                .chunks
                .iter()
                .any(|c| c.heading.as_deref() == Some("Guide > Deploys > Rollbacks")
                    && c.text.contains("previous image"))
        );
        assert!(
            chunked.chunks.iter().any(|c| c.heading.as_deref() == Some("Guide > Office") && c.text.contains("Lunch")),
            "a heading of the same level replaces the sibling: {by:?}"
        );
        assert!(chunked.chunks.iter().all(|c| c.text_hash == text_hash(&c.text) && c.tokens > 4));
    }

    #[test]
    fn a_hash_inside_a_code_fence_is_not_a_heading() {
        let md = "# Real\n```bash\n# just a comment\necho hi\n```\nafter\n";
        let chunked = chunk(Kind::Markdown, &text(md));
        assert_eq!(chunked.chunks.len(), 1);
        assert!(chunked.chunks[0].text.contains("# just a comment"));
    }

    #[test]
    fn a_long_section_is_cut_by_paragraphs_with_their_own_line_ranges() {
        let paragraph = |n: usize| format!("Paragraph {n}: {}", "word ".repeat(120));
        let md = format!("# Long\n{}", (0..12).map(paragraph).collect::<Vec<_>>().join("\n\n"));
        let chunked = chunk(Kind::Markdown, &text(&md));
        assert!(chunked.chunks.len() > 1);
        assert!(
            chunked
                .chunks
                .iter()
                .all(|c| c.text.chars().count() <= TARGET_CHARS + 600 && c.heading.as_deref() == Some("Long"))
        );
        let all: String = chunked.chunks.iter().map(|c| c.text.as_str()).collect::<Vec<_>>().join("\n");
        assert!((0..12).all(|n| all.contains(&format!("Paragraph {n}:"))), "nothing is lost");
        let first = &chunked.chunks[0];
        assert!(first.line_start.unwrap() <= first.line_end.unwrap());
    }

    #[test]
    fn code_is_cut_in_windows_with_overlap_and_line_numbers() {
        let code: String = (1..=300).map(|i| format!("let line_{i} = {i};\n")).collect();
        let chunked = chunk(Kind::Code, &text(&code));
        let ranges: Vec<(u32, u32)> =
            chunked.chunks.iter().map(|c| (c.line_start.unwrap(), c.line_end.unwrap())).collect();
        assert_eq!(ranges[0], (1, 120));
        assert_eq!(ranges[1].0, 111, "ten lines of overlap");
        assert_eq!(ranges.last().unwrap().1, 300);
        assert!(chunked.chunks[1].text.starts_with("let line_111 = 111;"));
    }

    #[test]
    fn a_minified_line_cannot_make_a_huge_piece() {
        let line = "x".repeat(50_000);
        let chunked = chunk(Kind::Code, &text(&format!("short\n{line}\nshort again")));
        assert!(
            chunked.chunks.iter().all(|c| c.text.chars().count() <= TARGET_CHARS * 2),
            "{:?}",
            chunked.chunks.iter().map(|c| c.text.len()).collect::<Vec<_>>()
        );
        assert!(chunked.chunks.len() >= 3);
    }

    #[test]
    fn pages_keep_their_number_and_have_no_line_range() {
        let pages = vec![
            Page { number: Some(1), text: "First page text.\n\nSecond paragraph.".into() },
            Page { number: Some(2), text: "Page two.".into() },
        ];
        let chunked = chunk(Kind::Pdf, &pages);
        assert_eq!(chunked.chunks.iter().map(|c| c.page).collect::<Vec<_>>(), [Some(1), Some(2)]);
        assert!(chunked.chunks.iter().all(|c| c.line_start.is_none()));
    }

    #[test]
    fn secrets_are_removed_from_the_text_before_it_is_hashed_and_stored() {
        let chunked = chunk(Kind::Text, &text("Deploy with sk-abcdefghijklmnopqrstuvwxyz0123456789 today"));
        assert!(!chunked.chunks[0].text.contains("sk-abcdef"));
        assert_eq!(
            chunked.chunks[0].text_hash,
            text_hash(&chunked.chunks[0].text),
            "the hash is of what is stored and embedded"
        );
    }

    #[test]
    fn empty_text_has_no_pieces_and_a_huge_file_is_truncated() {
        assert!(chunk(Kind::Text, &text("  \n\n")).chunks.is_empty());
        let many: String = (0..(MAX_CHUNKS * WINDOW_LINES + 5_000)).map(|i| format!("l{i}\n")).collect();
        let chunked = chunk(Kind::Text, &text(&many));
        assert!(chunked.truncated);
        assert_eq!(chunked.chunks.len(), MAX_CHUNKS);
    }
}
