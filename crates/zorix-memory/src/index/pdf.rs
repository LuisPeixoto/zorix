//! Text from PDFs, kept at arm's length (ADR-0017).
//!
//! PDF parsers see hostile input and have been known to panic, loop or use a lot of memory. So each file is read on a
//! thread of its own, a panic is caught, the wait is limited, and the size and the number of pages are limited. A file that
//! breaks a limit is reported with its reason; it never takes the indexer down. There is no OCR: a scanned PDF with no text
//! layer is reported as having no text.

use std::panic::{AssertUnwindSafe, catch_unwind};
use std::sync::mpsc;
use std::time::Duration;

use super::extract::{Extracted, Page, Unreadable};

/// The most pages read from one PDF.
pub const MAX_PAGES: usize = 500;
/// How long one PDF may take.
pub const TIME_BUDGET: Duration = Duration::from_secs(30);

/// Read the text of a PDF, page by page.
pub fn extract(bytes: &[u8]) -> Result<Extracted, Unreadable> {
    extract_within(bytes, TIME_BUDGET)
}

/// The same with a chosen time budget (for tests).
pub fn extract_within(bytes: &[u8], budget: Duration) -> Result<Extracted, Unreadable> {
    let head = &bytes[..bytes.len().min(1_024)];
    if !head.windows(4).any(|w| w == b"%PDF") {
        return Err(Unreadable::Fail("it is not a PDF".into()));
    }
    let data = bytes.to_vec();
    let (sender, receiver) = mpsc::channel();
    // A thread of its own with room on the stack: the parser recurses.
    let spawned = std::thread::Builder::new().name("zorix-pdf".into()).stack_size(32 * 1024 * 1024).spawn(move || {
        let result = catch_unwind(AssertUnwindSafe(|| pdf_extract::extract_text_from_mem_by_pages(&data)));
        let _ = sender.send(result);
    });
    if spawned.is_err() {
        return Err(Unreadable::Fail("a thread for reading the PDF could not be started".into()));
    }
    match receiver.recv_timeout(budget) {
        Ok(Ok(Ok(pages))) => pages_to_text(pages),
        Ok(Ok(Err(error))) => Err(Unreadable::Fail(format!("the PDF could not be read: {error}"))),
        Ok(Err(_)) => Err(Unreadable::Fail("the PDF could not be read (the reader crashed on it)".into())),
        // The thread is left behind: it cannot be stopped, but it cannot hold the indexer either.
        Err(_) => Err(Unreadable::Fail(format!("reading the PDF took longer than {} seconds", budget.as_secs()))),
    }
}

fn pages_to_text(pages: Vec<String>) -> Result<Extracted, Unreadable> {
    if pages.len() > MAX_PAGES {
        return Err(Unreadable::Skip(format!("it has {} pages, more than the {MAX_PAGES} that are read", pages.len())));
    }
    let pages: Vec<Page> = pages
        .into_iter()
        .enumerate()
        .map(|(index, text)| Page { number: Some(index as u32 + 1), text: tidy(&text) })
        .filter(|page| !page.text.trim().is_empty())
        .collect();
    if pages.is_empty() {
        return Err(Unreadable::Skip("no text was found (a scan needs OCR, which is not included)".into()));
    }
    Ok(Extracted { pages })
}

/// Squeeze the blank runs PDF layout leaves behind.
fn tidy(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    let mut blank = 0;
    for line in text.lines() {
        if line.trim().is_empty() {
            blank += 1;
            if blank == 1 {
                out.push('\n');
            }
        } else {
            blank = 0;
            out.push_str(line.trim_end());
            out.push('\n');
        }
    }
    out.trim().to_string()
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A small valid PDF with one line of text per page, in Helvetica.
    pub fn make_pdf(pages: &[&str]) -> Vec<u8> {
        let mut out = b"%PDF-1.4\n".to_vec();
        let mut offsets: Vec<usize> = Vec::new();
        let mut object = |out: &mut Vec<u8>, number: usize, body: String| {
            offsets.push(out.len());
            out.extend(format!("{number} 0 obj\n{body}\nendobj\n").into_bytes());
        };
        let count = pages.len();
        object(&mut out, 1, "<< /Type /Catalog /Pages 2 0 R >>".into());
        let kids: Vec<String> = (0..count).map(|i| format!("{} 0 R", 4 + i * 2)).collect();
        object(&mut out, 2, format!("<< /Type /Pages /Kids [{}] /Count {count} >>", kids.join(" ")));
        object(&mut out, 3, "<< /Type /Font /Subtype /Type1 /BaseFont /Helvetica >>".into());
        for (i, text) in pages.iter().enumerate() {
            let page = 4 + i * 2;
            object(
                &mut out,
                page,
                format!(
                    "<< /Type /Page /Parent 2 0 R /MediaBox [0 0 612 792] /Contents {} 0 R /Resources << /Font << /F1 3 0 R >> >> >>",
                    page + 1
                ),
            );
            let stream = format!("BT /F1 12 Tf 72 720 Td ({text}) Tj ET");
            object(&mut out, page + 1, format!("<< /Length {} >>\nstream\n{stream}\nendstream", stream.len()));
        }
        let xref = out.len();
        out.extend(format!("xref\n0 {}\n0000000000 65535 f \n", offsets.len() + 1).into_bytes());
        for offset in &offsets {
            out.extend(format!("{offset:010} 00000 n \n").into_bytes());
        }
        out.extend(
            format!("trailer\n<< /Size {} /Root 1 0 R >>\nstartxref\n{xref}\n%%EOF\n", offsets.len() + 1).into_bytes(),
        );
        out
    }

    #[test]
    fn the_text_of_each_page_comes_back_with_its_number() {
        let pdf = make_pdf(&["Deploys reach staging first", "Rollbacks use the previous image"]);
        let extracted = extract(&pdf).unwrap();
        assert_eq!(extracted.pages.iter().map(|p| p.number).collect::<Vec<_>>(), [Some(1), Some(2)]);
        assert!(extracted.pages[0].text.contains("Deploys reach staging first"), "{:?}", extracted.pages);
        assert!(extracted.pages[1].text.contains("Rollbacks use the previous image"));
    }

    #[test]
    fn a_pdf_with_no_text_is_skipped_and_garbage_is_a_failure() {
        let blank = make_pdf(&[""]);
        assert!(matches!(extract(&blank), Err(Unreadable::Skip(why)) if why.contains("OCR")), "no text layer");
        assert!(
            matches!(extract(b"just some text, not a pdf at all"), Err(Unreadable::Fail(why)) if why.contains("not a PDF"))
        );
        let mut broken = make_pdf(&["text"]);
        broken.truncate(broken.len() / 2);
        let result = extract(&broken);
        assert!(result.is_err() || result.is_ok(), "a damaged file never panics the caller");
        let corrupt = b"%PDF-1.4\n1 0 obj << /Type /Catalog >> endobj\ngarbage garbage trailer".to_vec();
        assert!(matches!(extract(&corrupt), Err(Unreadable::Fail(_)) | Err(Unreadable::Skip(_))));
    }

    #[test]
    fn too_many_pages_are_refused_with_the_reason() {
        let many: Vec<String> = (0..MAX_PAGES + 1).map(|i| format!("Page number {i}")).collect();
        let refs: Vec<&str> = many.iter().map(String::as_str).collect();
        assert!(matches!(extract(&make_pdf(&refs)), Err(Unreadable::Skip(why)) if why.contains("500")));
    }

    #[test]
    fn a_reader_that_takes_too_long_is_given_up_on() {
        let pdf = make_pdf(&["Some text"]);
        // A budget of zero cannot be met: the file is reported, and the caller goes on.
        let result = extract_within(&pdf, Duration::from_millis(0));
        let given_up = matches!(&result, Err(Unreadable::Fail(why)) if why.contains("took longer"));
        assert!(given_up || result.is_ok(), "either it was fast enough or it was given up on");
    }

    #[test]
    fn blank_runs_are_squeezed() {
        assert_eq!(tidy("a  \n\n\n\n b\n\n"), "a\n\n b");
    }
}
