//! Reading the files a person attaches to a run (ADR-0049).
//!
//! A person who picks a file, or drops one on the form, is choosing it: so a file outside the workspace is allowed, and reading it gives the
//! run no permission to read anything else. What is checked is the file itself, and every check is made before the run exists: that it is a
//! regular file, that its name is not one that holds secrets, that it is not under a folder that holds credentials, how large it is, and
//! that it reads as text. The text is read with the same extractors as the indexed documents, has anything that looks like a secret removed,
//! and is stored with the run as data (it reaches the model wrapped, as untrusted).

use std::io::Read;
use std::path::{Component, Path, PathBuf};
use std::sync::Arc;

use zorix_core::{Attachment, MAX_ATTACHMENT_CHARS, MAX_ATTACHMENTS, MAX_ATTACHMENTS_CHARS, redact_text};

use crate::index::PageReader;
use crate::index::extract::Unreadable;
use crate::index::sync::extract_or_scan;
use crate::index::walk::{Kind, MAX_PDF_BYTES, is_private, kind_of};

/// The most bytes read from a file that is not a PDF. A character is at least a byte, so this keeps the text within [`MAX_ATTACHMENT_CHARS`].
pub const MAX_TEXT_BYTES: u64 = MAX_ATTACHMENT_CHARS as u64;

/// Folders that hold credentials. A file anywhere below one is not attached, whatever it is called.
const CREDENTIAL_FOLDERS: &[&str] = &[".ssh", ".aws", ".gnupg", ".kube", ".docker", ".azure"];

/// Why a file cannot be attached, as one sentence that names the file.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct AttachError(pub String);

impl std::fmt::Display for AttachError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.0)
    }
}

impl std::error::Error for AttachError {}

/// A file that passed the checks that do not need to read it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Checked {
    /// The real path, with every link followed.
    pub path: PathBuf,
    /// The name the person knows it by.
    pub name: String,
    /// The size in bytes.
    pub bytes: u64,
    /// How it will be read.
    pub kind: Kind,
}

fn name_of(path: &Path) -> String {
    path.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default()
}

fn refuse(name: &str, why: &str) -> AttachError {
    AttachError(format!("“{name}” cannot be attached: {why}."))
}

/// Everything that can be checked without reading the file: that it exists and is a regular file, its names, where it lives, its kind and its size.
pub fn look(path: &Path) -> Result<Checked, AttachError> {
    let given = name_of(path);
    let shown = if given.is_empty() { path.to_string_lossy().into_owned() } else { given.clone() };
    let real = std::fs::canonicalize(path).map_err(|e| refuse(&shown, &format!("it cannot be found ({e})")))?;
    let meta = std::fs::metadata(&real).map_err(|e| refuse(&shown, &format!("it cannot be read ({e})")))?;
    if !meta.is_file() {
        return Err(refuse(&shown, "it is not a file"));
    }
    // A link called notes.txt that points at `.env` is `.env`: both names are checked.
    let real_name = name_of(&real);
    if is_private(&given) || is_private(&real_name) {
        return Err(refuse(&shown, "its name is one that holds secrets (.env, keys, credentials)"));
    }
    if let Some(folder) = credential_folder(&real) {
        return Err(refuse(&shown, &format!("it is in a folder that holds credentials ({folder})")));
    }
    // A name ZORIX does not know is read as text when it is text: a person chose it, so there is no list of extensions to be on.
    let kind = kind_of(&given).or_else(|| kind_of(&real_name)).unwrap_or(Kind::Text);
    let limit = if kind == Kind::Pdf { MAX_PDF_BYTES } else { MAX_TEXT_BYTES };
    if meta.len() > limit {
        let what = if kind == Kind::Pdf { "a PDF of 25 MB" } else { "1 MB of text" };
        return Err(refuse(
            &shown,
            &format!("it is {:.1} MB and at most {what} can be attached", meta.len() as f64 / 1e6),
        ));
    }
    Ok(Checked { path: real, name: shown, bytes: meta.len(), kind })
}

/// The credential folder a path is under, if any.
fn credential_folder(path: &Path) -> Option<String> {
    let parts: Vec<String> = path
        .components()
        .filter_map(|c| match c {
            Component::Normal(part) => Some(part.to_string_lossy().to_lowercase()),
            _ => None,
        })
        .collect();
    // The last part is the file itself.
    let folders = &parts[..parts.len().saturating_sub(1)];
    if let Some(found) = folders.iter().find(|p| CREDENTIAL_FOLDERS.contains(&p.as_str())) {
        return Some(found.clone());
    }
    folders.windows(2).any(|w| w[0] == ".config" && w[1] == "gcloud").then(|| ".config/gcloud".to_string())
}

/// Read the files. At most [`MAX_ATTACHMENTS`] of them (the same file twice counts once), none refused: the first one that is refused is the answer,
/// named, and nothing is attached.
pub fn read_attachments(paths: &[PathBuf], ocr: Option<Arc<dyn PageReader>>) -> Result<Vec<Attachment>, AttachError> {
    let mut seen: Vec<PathBuf> = Vec::new();
    let mut checked = Vec::new();
    for path in paths {
        let file = look(path)?;
        if seen.contains(&file.path) {
            continue;
        }
        seen.push(file.path.clone());
        checked.push(file);
    }
    if checked.len() > MAX_ATTACHMENTS {
        return Err(AttachError(format!("At most {MAX_ATTACHMENTS} files can be attached to a run.")));
    }
    let mut out: Vec<Attachment> = Vec::new();
    let mut total = 0usize;
    for file in checked {
        let attachment = read_one(&file, ocr.clone(), &out)?;
        total += attachment.text.chars().count();
        if total > MAX_ATTACHMENTS_CHARS {
            return Err(AttachError(format!(
                "The attached files are too long together ({} characters at most).",
                MAX_ATTACHMENTS_CHARS
            )));
        }
        out.push(attachment);
    }
    Ok(out)
}

fn read_one(file: &Checked, ocr: Option<Arc<dyn PageReader>>, taken: &[Attachment]) -> Result<Attachment, AttachError> {
    let limit = if file.kind == Kind::Pdf { MAX_PDF_BYTES } else { MAX_TEXT_BYTES };
    let opened =
        std::fs::File::open(&file.path).map_err(|e| refuse(&file.name, &format!("it cannot be read ({e})")))?;
    // The size was checked a moment ago; a file that grew since is cut here, and refused if it is now too large.
    let mut bytes = Vec::new();
    opened
        .take(limit + 1)
        .read_to_end(&mut bytes)
        .map_err(|e| refuse(&file.name, &format!("it cannot be read ({e})")))?;
    if bytes.len() as u64 > limit {
        return Err(refuse(&file.name, "it is larger than the limit"));
    }
    let (read, _) = extract_or_scan(file.kind, &bytes, ocr);
    let extracted = read.map_err(|why| match why {
        Unreadable::Skip(why) | Unreadable::Fail(why) => refuse(&file.name, &why),
    })?;
    // A PDF keeps its page numbers, so an answer can cite one; a text file is one piece.
    let numbered = extracted.pages.iter().any(|p| p.number.is_some());
    let joined = extracted
        .pages
        .iter()
        .map(|p| match p.number {
            Some(n) if numbered => format!("[page {n}]\n{}", p.text.trim()),
            _ => p.text.trim().to_string(),
        })
        .collect::<Vec<_>>()
        .join("\n\n");
    let text = redact_text(joined.trim());
    if text.trim().is_empty() {
        return Err(refuse(&file.name, "there is no text in it"));
    }
    let length = text.chars().count();
    let (text, truncated) = if length > MAX_ATTACHMENT_CHARS {
        (text.chars().take(MAX_ATTACHMENT_CHARS).collect::<String>(), true)
    } else {
        (text, false)
    };
    Ok(Attachment { name: unique(&file.name, taken), text, bytes: file.bytes, truncated })
}

/// A name that no earlier attached file has: `notes.md`, `notes.md (2)`, …
fn unique(name: &str, taken: &[Attachment]) -> String {
    if !taken.iter().any(|a| a.name == name) {
        return name.to_string();
    }
    (2..)
        .map(|n| format!("{name} ({n})"))
        .find(|candidate| !taken.iter().any(|a| &a.name == candidate))
        .expect("an unused number exists")
}

#[cfg(test)]
mod tests {
    use super::*;

    fn dir() -> tempfile::TempDir {
        tempfile::tempdir().unwrap()
    }

    fn put(dir: &Path, name: &str, content: &[u8]) -> PathBuf {
        let path = dir.join(name);
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(&path, content).unwrap();
        path
    }

    fn read(paths: &[PathBuf]) -> Result<Vec<Attachment>, AttachError> {
        read_attachments(paths, None)
    }

    /// A PDF of one page with nothing on it: no text layer and no picture, which is how the text path sees a scan.
    fn empty_page_pdf() -> Vec<u8> {
        let mut out = b"%PDF-1.4\n".to_vec();
        let mut offsets = Vec::new();
        for (number, body) in [
            (1, "<< /Type /Catalog /Pages 2 0 R >>"),
            (2, "<< /Type /Pages /Kids [3 0 R] /Count 1 >>"),
            (3, "<< /Type /Page /Parent 2 0 R /MediaBox [0 0 100 100] >>"),
        ] {
            offsets.push(out.len());
            out.extend(format!("{number} 0 obj\n{body}\nendobj\n").into_bytes());
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

    fn why(result: Result<Vec<Attachment>, AttachError>) -> String {
        result.expect_err("it was refused").0
    }

    #[test]
    fn a_text_file_is_read_whole_with_its_name_and_never_its_folder() {
        let d = dir();
        let file = put(d.path(), "notes/plan.md", b"# Plan\nShip on Thursday.\n");
        let out = read(&[file]).unwrap();
        assert_eq!(out.len(), 1);
        assert_eq!(
            (out[0].name.as_str(), out[0].text.as_str(), out[0].bytes),
            ("plan.md", "# Plan\nShip on Thursday.", 25)
        );
        assert!(!out[0].truncated);
        assert!(!out[0].name.contains('/') && !out[0].text.contains(d.path().to_str().unwrap()));
    }

    #[test]
    fn a_file_with_a_name_zorix_does_not_know_is_read_when_it_is_text_and_refused_when_it_is_not() {
        let d = dir();
        let log = put(d.path(), "build.log", b"step 1 ok\nstep 2 failed\n");
        assert_eq!(read(&[log]).unwrap()[0].text, "step 1 ok\nstep 2 failed");
        let binary = put(d.path(), "image.bin", &[0, 1, 2, 3, 0, 255, 0, 9]);
        let message = why(read(&[binary]));
        assert!(message.starts_with("“image.bin” cannot be attached"), "{message}");
    }

    #[test]
    fn files_that_hold_secrets_are_refused_by_name_even_through_a_link() {
        let d = dir();
        for name in
            [".env", ".env.local", "server.pem", "id_rsa", "credentials.json", "secrets.yaml", ".npmrc", "release.KEY"]
        {
            let file = put(d.path(), name, b"TOKEN=abc");
            let message = why(read(&[file]));
            assert!(message.contains("holds secrets"), "{name}: {message}");
        }
        #[cfg(unix)]
        {
            let real = put(d.path(), ".env", b"TOKEN=abc");
            let link = d.path().join("notes.txt");
            std::os::unix::fs::symlink(&real, &link).unwrap();
            let message = why(read(&[link]));
            assert!(
                message.contains("holds secrets") && message.contains("notes.txt"),
                "the real name decides: {message}"
            );
        }
    }

    #[test]
    fn a_file_under_a_folder_of_credentials_is_refused_whatever_it_is_called() {
        let d = dir();
        for folder in [".ssh", ".aws", ".gnupg", ".kube", ".docker", ".azure", ".config/gcloud"] {
            let file = put(d.path(), &format!("home/{folder}/config"), b"x");
            let message = why(read(&[file]));
            assert!(message.contains("holds credentials"), "{folder}: {message}");
        }
        // A folder that only looks like one is fine.
        let ok = put(d.path(), "home/ssh-notes/readme.txt", b"fine");
        assert!(read(&[ok]).is_ok());
    }

    #[test]
    fn a_folder_a_missing_path_and_a_special_file_are_not_files() {
        let d = dir();
        let message = why(read(&[d.path().to_path_buf()]));
        assert!(message.contains("not a file"), "{message}");
        let message = why(read(&[d.path().join("nowhere.txt")]));
        assert!(message.contains("cannot be found"), "{message}");
        assert!(why(read(&[PathBuf::from("")])).contains("cannot be attached"));
    }

    #[cfg(unix)]
    #[test]
    fn a_named_pipe_is_refused_without_being_opened() {
        let d = dir();
        let pipe = d.path().join("feed.txt");
        let made = std::process::Command::new("mkfifo").arg(&pipe).status().map(|s| s.success()).unwrap_or(false);
        if !made {
            return; // no mkfifo here
        }
        // Opening it would wait for a writer for ever; it is a regular-file check that stops it, so this returns.
        assert!(why(read(&[pipe])).contains("not a file"));
    }

    #[test]
    fn size_limits_say_how_large_the_file_is_and_what_is_allowed() {
        let d = dir();
        let words = "lorem ipsum ".repeat(100_000);
        let big = put(d.path(), "big.txt", &words.as_bytes()[..MAX_TEXT_BYTES as usize + 1]);
        let message = why(read(&[big]));
        assert!(message.contains("1.0 MB") && message.contains("at most 1 MB of text"), "{message}");
        let just = put(d.path(), "just.txt", &words.as_bytes()[..MAX_TEXT_BYTES as usize]);
        let out = read(&[just]).unwrap();
        assert!(out[0].text.chars().count() >= MAX_ATTACHMENT_CHARS - 1, "a file of exactly the limit is read whole");
        assert!(!out[0].truncated);
        let pdf = put(d.path(), "huge.pdf", b"%PDF-1.4");
        std::fs::OpenOptions::new().write(true).open(&pdf).unwrap().set_len(MAX_PDF_BYTES + 1).unwrap();
        assert!(why(read(&[pdf])).contains("a PDF of 25 MB"));
    }

    #[test]
    fn at_most_ten_files_and_the_same_file_twice_counts_once() {
        let d = dir();
        let files: Vec<PathBuf> =
            (0..11).map(|i| put(d.path(), &format!("f{i}.txt"), format!("file {i}").as_bytes())).collect();
        assert!(why(read(&files)).contains("At most 10 files"));
        assert_eq!(read(&files[..10]).unwrap().len(), 10);
        let twice = vec![files[0].clone(), files[0].clone(), d.path().join("./f0.txt")];
        assert_eq!(read(&twice).unwrap().len(), 1, "one file however it is spelled");
    }

    #[test]
    fn files_with_the_same_name_in_different_folders_get_different_names() {
        let d = dir();
        let a = put(d.path(), "a/notes.md", b"from a");
        let b = put(d.path(), "b/notes.md", b"from b");
        let c = put(d.path(), "c/notes.md", b"from c");
        let names: Vec<String> = read(&[a, b, c]).unwrap().into_iter().map(|f| f.name).collect();
        assert_eq!(names, ["notes.md", "notes.md (2)", "notes.md (3)"]);
    }

    #[test]
    fn what_looks_like_a_secret_is_removed_before_the_text_is_kept() {
        let d = dir();
        let file = put(
            d.path(),
            "deploy.md",
            b"Use the key sk-abcdefghijklmnopqrstuvwxyz0123456789 to deploy.\npassword=hunter2hunter2\n",
        );
        let text = read(&[file]).unwrap().remove(0).text;
        assert!(
            !text.contains("sk-abcdefghijklmnopqrstuvwxyz0123456789") && !text.contains("hunter2hunter2"),
            "{text}"
        );
        assert!(text.contains("to deploy"));
    }

    #[test]
    fn a_file_with_no_text_and_a_file_of_nothing_but_spaces_are_refused() {
        let d = dir();
        assert!(why(read(&[put(d.path(), "empty.txt", b"")])).contains("no text"));
        assert!(why(read(&[put(d.path(), "blank.md", b"  \n\n \t\n")])).contains("no text"));
    }

    #[test]
    fn html_is_read_as_text_and_the_total_is_limited() {
        let d = dir();
        let html = put(
            d.path(),
            "page.html",
            b"<html><body><h1>Status</h1><p>All green.</p><script>var x = 1;</script></body></html>",
        );
        let text = read(&[html]).unwrap().remove(0).text;
        assert!(text.contains("Status") && text.contains("All green") && !text.contains("var x"), "{text}");

        // Four files of a little under the limit are too much together.
        let each = vec![b'x'; 900_000];
        let files: Vec<PathBuf> = (0..4).map(|i| put(d.path(), &format!("part{i}.txt"), &each)).collect();
        let message = why(read(&files));
        assert!(message.contains("too long together"), "{message}");
    }

    #[test]
    fn the_first_file_that_is_refused_stops_everything_and_is_named() {
        let d = dir();
        let good = put(d.path(), "good.txt", b"fine");
        let bad = put(d.path(), ".env", b"TOKEN=1");
        let message = why(read(&[good, bad]));
        assert!(message.starts_with("“.env” cannot be attached"), "{message}");
    }

    #[test]
    fn a_scanned_pdf_is_read_when_there_is_an_engine_and_refused_with_the_way_out_when_there_is_not() {
        struct Says(&'static str);
        impl PageReader for Says {
            fn read(&self, _: u32, _: u32, _: &[u8]) -> Result<String, String> {
                Ok(self.0.into())
            }
        }
        let d = dir();
        // A PDF with one page and no text layer and no picture: the text path says it is a scan.
        let blank = put(d.path(), "scan.pdf", &empty_page_pdf());
        let message = why(read_attachments(std::slice::from_ref(&blank), None));
        assert!(message.contains("it is a scan") && message.contains("reading scans"), "{message}");
        // With an engine the pictures are read; this PDF has none, so the engine is asked for nothing and the answer is still a reason.
        let engine: Arc<dyn PageReader> = Arc::new(Says("never asked"));
        let message = why(read_attachments(&[blank], Some(engine)));
        assert!(message.starts_with("“scan.pdf” cannot be attached"), "{message}");
    }
}
