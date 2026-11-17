//! Reading a file into text, or saying why it cannot be read.

use super::walk::Kind;

/// How many bytes at the start are searched for a NUL, which marks a binary file.
const BINARY_SNIFF: usize = 8_192;
/// The share of characters that may be replacement characters before the text is refused.
const MAX_LOSSY: f64 = 0.01;
/// The width html2text wraps at (wide, so lines are not broken for no reason).
#[cfg(feature = "html")]
const HTML_WIDTH: usize = 200;

/// One page of a document. Text files have a single page with no number.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Page {
    /// The page number, from 1, for PDFs.
    pub number: Option<u32>,
    /// Its text.
    pub text: String,
}

/// What was read from a file.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Extracted {
    /// The pages.
    pub pages: Vec<Page>,
}

/// Why a file was not read. `Skip` is on purpose and `Fail` is a problem to show.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Unreadable {
    /// Left out on purpose (binary, not text, no text found).
    Skip(String),
    /// Could not be read.
    Fail(String),
}

/// Read `bytes` as the text of a file of `kind`.
pub fn extract(kind: Kind, bytes: &[u8]) -> Result<Extracted, Unreadable> {
    match kind {
        Kind::Pdf => extract_pdf(bytes),
        Kind::Html => {
            let text = decode(bytes)?;
            #[cfg(feature = "html")]
            {
                let rendered = html2text::from_read(text.as_bytes(), HTML_WIDTH)
                    .map_err(|e| Unreadable::Fail(format!("the HTML could not be read: {e}")))?;
                single(rendered)
            }
            #[cfg(not(feature = "html"))]
            {
                single(text)
            }
        }
        _ => single(decode(bytes)?),
    }
}

fn single(text: String) -> Result<Extracted, Unreadable> {
    if text.trim().is_empty() {
        return Err(Unreadable::Skip("it has no text".into()));
    }
    Ok(Extracted { pages: vec![Page { number: None, text }] })
}

/// UTF-8 text. A NUL byte early on means binary; a little invalid text is tolerated, a lot is not.
fn decode(bytes: &[u8]) -> Result<String, Unreadable> {
    if bytes.iter().take(BINARY_SNIFF).any(|b| *b == 0) {
        return Err(Unreadable::Skip("it is a binary file".into()));
    }
    let bytes = bytes.strip_prefix(&[0xEF, 0xBB, 0xBF]).unwrap_or(bytes);
    match std::str::from_utf8(bytes) {
        Ok(text) => Ok(text.to_string()),
        Err(_) => {
            let text = String::from_utf8_lossy(bytes).into_owned();
            let bad = text.chars().filter(|c| *c == '\u{FFFD}').count();
            let total = text.chars().count().max(1);
            if (bad as f64 / total as f64) <= MAX_LOSSY {
                Ok(text)
            } else {
                Err(Unreadable::Skip("it is not UTF-8 text".into()))
            }
        }
    }
}

#[cfg(not(feature = "pdf"))]
fn extract_pdf(_bytes: &[u8]) -> Result<Extracted, Unreadable> {
    Err(Unreadable::Skip("PDF support is not included in this build".into()))
}

#[cfg(feature = "pdf")]
fn extract_pdf(bytes: &[u8]) -> Result<Extracted, Unreadable> {
    crate::index::pdf::extract(bytes)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn text_of(extracted: Extracted) -> String {
        extracted.pages.into_iter().map(|p| p.text).collect::<Vec<_>>().join("\n")
    }

    #[test]
    fn text_is_read_with_or_without_a_byte_order_mark() {
        assert_eq!(text_of(extract(Kind::Text, "Olá, mundo".as_bytes()).unwrap()), "Olá, mundo");
        assert_eq!(text_of(extract(Kind::Markdown, b"\xEF\xBB\xBF# Title").unwrap()), "# Title");
    }

    #[test]
    fn binaries_empty_files_and_other_encodings_are_skipped_with_a_reason() {
        let reason = |r: Result<Extracted, Unreadable>| match r {
            Err(Unreadable::Skip(why)) => why,
            other => panic!("{other:?}"),
        };
        assert_eq!(reason(extract(Kind::Code, b"MZ\x90\x00\x03\x00\x00")), "it is a binary file");
        assert_eq!(reason(extract(Kind::Text, b"   \n\n ")), "it has no text");
        // Latin-1 text with accents is mostly invalid UTF-8.
        let latin1: Vec<u8> = "ação é coração ".repeat(20).chars().map(|c| c as u32 as u8).collect();
        assert_eq!(reason(extract(Kind::Text, &latin1)), "it is not UTF-8 text");
        // One bad byte in a long text is tolerated.
        let mut mostly = "word ".repeat(400).into_bytes();
        mostly.push(0xFF);
        assert!(extract(Kind::Text, &mostly).is_ok());
    }

    #[cfg(feature = "html")]
    #[test]
    fn html_becomes_readable_text_without_scripts_markup_or_entities() {
        let html = "<html><head><title>T</title><script>var x = 'secret-js';</script></head><body><h1>Deploys</h1><p>Staging &amp; production.</p><ul><li>one</li><li>two</li></ul></body></html>";
        let text = text_of(extract(Kind::Html, html.as_bytes()).unwrap());
        assert!(
            text.contains("Deploys")
                && text.contains("Staging & production.")
                && text.contains("one")
                && text.contains("two"),
            "{text}"
        );
        assert!(!text.contains("<p>") && !text.contains("&amp;"));
    }

    #[cfg(not(feature = "pdf"))]
    #[test]
    fn a_pdf_is_skipped_when_the_build_has_no_pdf_support() {
        assert!(matches!(extract(Kind::Pdf, b"%PDF-1.4"), Err(Unreadable::Skip(_))));
    }
}
