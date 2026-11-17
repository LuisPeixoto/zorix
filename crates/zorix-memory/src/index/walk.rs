//! Finding the files of a folder that are worth indexing, without ever reading what must stay private.

use std::path::Path;
use std::time::UNIX_EPOCH;

use globset::{Glob, GlobSet, GlobSetBuilder};
use ignore::WalkBuilder;

/// Most files looked at in one folder.
pub const MAX_FILES: usize = 20_000;
/// The default largest file read, in bytes.
pub const DEFAULT_MAX_FILE_BYTES: u64 = 2_000_000;
/// The largest PDF read, in bytes.
pub const MAX_PDF_BYTES: u64 = 25_000_000;

/// What kind of file it is, which decides how it is read and cut.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Kind {
    /// Markdown: cut by headings.
    Markdown,
    /// Plain text.
    Text,
    /// Source code.
    Code,
    /// JSON, YAML, TOML and the like.
    Data,
    /// HTML, read as text.
    Html,
    /// PDF with a text layer.
    Pdf,
}

impl Kind {
    /// The name stored.
    pub fn as_str(&self) -> &'static str {
        match self {
            Kind::Markdown => "markdown",
            Kind::Text => "text",
            Kind::Code => "code",
            Kind::Data => "data",
            Kind::Html => "html",
            Kind::Pdf => "pdf",
        }
    }
}

const MARKDOWN: &[&str] = &["md", "markdown", "mdx"];
const TEXT: &[&str] = &["txt", "rst", "adoc", "org", "text"];
const DATA: &[&str] = &["json", "yaml", "yml", "toml", "ini", "cfg", "conf", "xml", "csv", "tsv"];
const HTML: &[&str] = &["html", "htm"];
const CODE: &[&str] = &[
    "rs", "py", "js", "mjs", "cjs", "ts", "tsx", "jsx", "go", "java", "kt", "kts", "c", "h", "cc", "cpp", "hpp", "cs",
    "rb", "php", "swift", "sh", "bash", "zsh", "sql", "svelte", "vue", "css", "scss", "lua", "dart", "scala", "ex",
    "exs", "hs", "ml", "r", "pl",
];
const KNOWN_NAMES: &[&str] = &[
    "readme",
    "changelog",
    "contributing",
    "license",
    "notice",
    "authors",
    "makefile",
    "dockerfile",
    "justfile",
    "procfile",
];

/// Directories that are never entered, even when nothing ignores them.
const SKIPPED_DIRECTORIES: &[&str] = &[
    ".git",
    "node_modules",
    "target",
    "dist",
    "__pycache__",
    ".venv",
    "venv",
    ".next",
    ".svelte-kit",
    ".idea",
    ".vscode",
];

/// Files that may hold secrets. They are never read, whatever the include patterns say.
pub fn is_private(file_name: &str) -> bool {
    let name = file_name.to_lowercase();
    name == ".env"
        || name.starts_with(".env.")
        || name.ends_with(".env")
        || name.ends_with(".pem")
        || name.ends_with(".key")
        || name.ends_with(".p12")
        || name.ends_with(".pfx")
        || name.ends_with(".kdbx")
        || name.ends_with(".keystore")
        || name.ends_with(".jks")
        || name.starts_with("id_rsa")
        || name.starts_with("id_ed25519")
        || name.starts_with("id_ecdsa")
        || name.starts_with("id_dsa")
        || name == ".npmrc"
        || name == ".netrc"
        || name == ".pypirc"
        || name.starts_with("credentials")
        || name.starts_with("secrets.")
        || name == "secrets"
}

/// The kind of a file by its name, or `None` when it is not something to index.
pub fn kind_of(file_name: &str) -> Option<Kind> {
    let lower = file_name.to_lowercase();
    let (stem, extension) = match lower.rsplit_once('.') {
        Some((stem, extension)) if !stem.is_empty() => (stem, Some(extension)),
        _ => (lower.as_str(), None),
    };
    match extension {
        Some(e) if MARKDOWN.contains(&e) => Some(Kind::Markdown),
        Some(e) if TEXT.contains(&e) => Some(Kind::Text),
        Some(e) if HTML.contains(&e) => Some(Kind::Html),
        Some(e) if DATA.contains(&e) => Some(Kind::Data),
        Some(e) if CODE.contains(&e) => Some(Kind::Code),
        Some("pdf") => Some(Kind::Pdf),
        None if KNOWN_NAMES.contains(&stem) => Some(Kind::Text),
        _ => None,
    }
}

/// A file found in a folder.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Candidate {
    /// The path relative to the folder, with `/`.
    pub rel_path: String,
    /// Size in bytes.
    pub size: u64,
    /// Modification time, in nanoseconds since 1970.
    pub mtime_ns: i64,
    /// How to read it, or `None` when it is not a kind that is indexed.
    pub kind: Option<Kind>,
    /// Why it will not be read, when it will not.
    pub skip: Option<String>,
}

/// What the walk found.
#[derive(Debug, Default)]
pub struct Walked {
    /// The files that are to be looked at.
    pub files: Vec<Candidate>,
    /// True when there were more than [`MAX_FILES`].
    pub truncated: bool,
}

fn globs(patterns: &[String]) -> Result<Option<GlobSet>, String> {
    if patterns.is_empty() {
        return Ok(None);
    }
    let mut builder = GlobSetBuilder::new();
    for pattern in patterns {
        builder.add(Glob::new(pattern).map_err(|e| format!("“{pattern}” is not a valid pattern: {e}"))?);
    }
    builder.build().map(Some).map_err(|e| e.to_string())
}

/// Walk `root` with `.gitignore`, `.ignore`, the global ignore file and `.zorixignore`. Hidden files and folders are
/// skipped, links are not followed, private files are never listed, and the include and exclude globs apply to paths
/// relative to `root`.
pub fn walk(root: &Path, include: &[String], exclude: &[String], max_file_bytes: u64) -> Result<Walked, String> {
    let include = globs(include)?;
    let exclude = globs(exclude)?;
    let mut builder = WalkBuilder::new(root);
    builder
        .hidden(true)
        .git_ignore(true)
        .git_global(true)
        .git_exclude(true)
        .ignore(true)
        .parents(true)
        .require_git(false)
        .follow_links(false)
        .add_custom_ignore_filename(".zorixignore")
        .filter_entry(|entry| {
            !entry.file_type().is_some_and(|t| t.is_dir())
                || !SKIPPED_DIRECTORIES.contains(&entry.file_name().to_string_lossy().as_ref())
        });
    let mut walked = Walked::default();
    for entry in builder.build() {
        let Ok(entry) = entry else { continue };
        if !entry.file_type().is_some_and(|t| t.is_file()) {
            continue;
        }
        let name = entry.file_name().to_string_lossy().into_owned();
        if is_private(&name) {
            continue;
        }
        let Ok(rel) = entry.path().strip_prefix(root) else { continue };
        let rel_path = rel.components().map(|c| c.as_os_str().to_string_lossy()).collect::<Vec<_>>().join("/");
        if exclude.as_ref().is_some_and(|g| g.is_match(&rel_path)) {
            continue;
        }
        let included = include.as_ref().is_some_and(|g| g.is_match(&rel_path));
        let kind = kind_of(&name).or(if included { Some(Kind::Text) } else { None });
        let Some(kind) = kind else { continue };
        let Ok(metadata) = entry.metadata() else { continue };
        if walked.files.len() == MAX_FILES {
            walked.truncated = true;
            break;
        }
        let size = metadata.len();
        let limit = if kind == Kind::Pdf { MAX_PDF_BYTES } else { max_file_bytes };
        let skip = (size > limit).then(|| format!("it is larger than {} MB", limit / 1_000_000));
        let mtime_ns = metadata
            .modified()
            .ok()
            .and_then(|t| t.duration_since(UNIX_EPOCH).ok())
            .map(|d| d.as_nanos() as i64)
            .unwrap_or(0);
        walked.files.push(Candidate { rel_path, size, mtime_ns, kind: Some(kind), skip });
    }
    walked.files.sort_by(|a, b| a.rel_path.cmp(&b.rel_path));
    Ok(walked)
}

#[cfg(test)]
mod tests {
    use std::fs;

    use super::*;

    fn write(root: &Path, rel: &str, content: &str) {
        let path = root.join(rel);
        fs::create_dir_all(path.parent().unwrap()).unwrap();
        fs::write(path, content).unwrap();
    }

    fn names(walked: &Walked) -> Vec<&str> {
        walked.files.iter().map(|f| f.rel_path.as_str()).collect()
    }

    #[test]
    fn kinds_come_from_names_and_private_files_are_recognised() {
        assert_eq!(kind_of("Guide.MD"), Some(Kind::Markdown));
        assert_eq!(kind_of("main.rs"), Some(Kind::Code));
        assert_eq!(kind_of("config.yaml"), Some(Kind::Data));
        assert_eq!(kind_of("index.htm"), Some(Kind::Html));
        assert_eq!(kind_of("paper.pdf"), Some(Kind::Pdf));
        assert_eq!(kind_of("README"), Some(Kind::Text));
        assert_eq!(kind_of("Makefile"), Some(Kind::Text));
        for none in ["photo.png", "archive.zip", "binary.exe", ".md", "noextension", "music.mp3"] {
            assert_eq!(kind_of(none), None, "{none}");
        }
        for private in [
            ".env",
            ".env.production",
            "prod.env",
            "server.pem",
            "api.key",
            "id_rsa",
            "id_ed25519.pub",
            ".npmrc",
            ".netrc",
            "credentials.json",
            "secrets.yaml",
            "store.p12",
            "vault.kdbx",
        ] {
            assert!(is_private(private), "{private}");
        }
        for fine in ["environment.md", "keys.md", "monkey.rs", "secretary.md", "credit.md"] {
            assert!(!is_private(fine), "{fine}");
        }
    }

    #[test]
    fn the_walk_respects_gitignore_hidden_files_private_files_links_and_odd_folders() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path();
        write(root, "README.md", "# Hello");
        write(root, "src/main.rs", "fn main() {}");
        write(root, "docs/guide.md", "guide");
        write(root, ".gitignore", "ignored/\n*.log\n");
        write(root, "ignored/skip.md", "ignored");
        write(root, "debug.log", "log");
        write(root, ".env", "KEY=sk-secret");
        write(root, "config/secrets.yaml", "k: v");
        write(root, ".hidden/notes.md", "hidden");
        write(root, "node_modules/pkg/index.js", "x");
        write(root, "target/debug/build.rs", "x");
        write(root, "image.png", "binary");
        write(root, "own.md", "own");
        write(root, ".zorixignore", "own.md\n");
        #[cfg(unix)]
        std::os::unix::fs::symlink("/etc/hosts", root.join("link.md")).unwrap();
        let walked = walk(root, &[], &[], DEFAULT_MAX_FILE_BYTES).unwrap();
        assert_eq!(names(&walked), ["README.md", "docs/guide.md", "src/main.rs"]);
        assert!(!walked.truncated);
    }

    #[test]
    fn include_and_exclude_patterns_apply_to_relative_paths() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path();
        write(root, "docs/a.md", "a");
        write(root, "docs/drafts/b.md", "b");
        write(root, "data/table.weird", "custom");
        write(root, "data/.env", "secret");
        let walked = walk(root, &["data/*".into()], &["docs/drafts/*".into()], DEFAULT_MAX_FILE_BYTES).unwrap();
        assert_eq!(
            names(&walked),
            ["data/table.weird", "docs/a.md"],
            "an include adds odd extensions, an exclude removes, and .env stays private"
        );
        assert!(walk(root, &["[".into()], &[], 1).unwrap_err().contains("not a valid pattern"));
    }

    #[test]
    fn big_files_are_listed_with_the_reason_they_will_not_be_read() {
        let dir = tempfile::tempdir().unwrap();
        write(dir.path(), "big.md", &"x".repeat(3_000));
        write(dir.path(), "small.md", "x");
        let walked = walk(dir.path(), &[], &[], 1_000).unwrap();
        let big = walked.files.iter().find(|f| f.rel_path == "big.md").unwrap();
        assert_eq!((big.size, big.skip.is_some()), (3_000, true));
        assert!(walked.files.iter().find(|f| f.rel_path == "small.md").unwrap().skip.is_none());
        assert!(big.mtime_ns > 0);
    }
}
