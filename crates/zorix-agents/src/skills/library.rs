//! The skill library: skills kept by the hash of their content under `<home>/skills/<hash>/`, imported from a folder or a zip.
//!
//! Import is bounded (files, bytes, depth), refuses links and unsafe paths, and is atomic: a skill is written to a temporary
//! folder and renamed into place, so a half-imported skill is never visible.

use std::collections::HashSet;
use std::io::Read;
use std::path::{Path, PathBuf};

use sha2::{Digest, Sha256};
use zorix_core::{SkillFile, SkillRecord, Timestamp};

use super::frontmatter::{MAX_SKILL_MD, parse_skill_md};

/// Most files in a skill.
pub const MAX_FILES: usize = 200;
/// Most bytes in a skill, all files together.
pub const MAX_TOTAL_BYTES: u64 = 10 * 1024 * 1024;
/// Most bytes in one file.
pub const MAX_FILE_BYTES: u64 = 2 * 1024 * 1024;
/// Deepest folder nesting inside a skill.
pub const MAX_DEPTH: usize = 8;

/// Why a skill could not be imported or read.
#[derive(Debug, thiserror::Error)]
pub enum SkillError {
    /// The skill breaks a rule; the text says which.
    #[error("{0}")]
    Invalid(String),
    /// The disk said no.
    #[error("{0}")]
    Io(String),
}

fn invalid(text: impl Into<String>) -> SkillError {
    SkillError::Invalid(text.into())
}

fn io(error: impl std::fmt::Display) -> SkillError {
    SkillError::Io(error.to_string())
}

fn hex(bytes: &[u8]) -> String {
    bytes.iter().map(|b| format!("{b:02x}")).collect()
}

/// The hash that names a skill: SHA-256 over the sorted lines `path NUL size NUL sha256(file) LF`.
pub fn content_hash(files: &[SkillFile]) -> String {
    let mut sorted: Vec<&SkillFile> = files.iter().collect();
    sorted.sort_by(|a, b| a.path.cmp(&b.path));
    let mut hasher = Sha256::new();
    for file in sorted {
        hasher.update(format!("{}\0{}\0{}\n", file.path, file.size, file.sha256));
    }
    hex(&hasher.finalize())
}

const RESERVED: &[&str] = &[
    "con", "prn", "aux", "nul", "com1", "com2", "com3", "com4", "com5", "com6", "com7", "com8", "com9", "lpt1", "lpt2",
    "lpt3", "lpt4", "lpt5", "lpt6", "lpt7", "lpt8", "lpt9",
];

/// A path inside a skill: relative, `/`-separated, no `..`, nothing a file system would treat specially.
pub fn check_path(rel: &str) -> Result<(), SkillError> {
    let bad = |why: &str| Err(invalid(format!("The path “{rel}” is not allowed: {why}.")));
    if rel.is_empty() || rel.len() > 260 {
        return bad("it is empty or too long");
    }
    if rel.starts_with('/') || rel.contains('\\') || rel.contains(':') || rel.chars().any(char::is_control) {
        return bad("it must be a relative path with forward slashes");
    }
    for part in rel.split('/') {
        if part.is_empty() || part == "." || part == ".." {
            return bad("it goes up or has an empty part");
        }
        if part.len() > 120 || part.ends_with('.') || part.ends_with(' ') {
            return bad("a part is too long or ends with a dot or a space");
        }
        let stem = part.split('.').next().unwrap_or("").to_ascii_lowercase();
        if RESERVED.contains(&stem.as_str()) {
            return bad("a part is a reserved name on Windows");
        }
    }
    Ok(())
}

fn junk(rel: &str) -> bool {
    rel.split('/').any(|p| matches!(p, ".git" | ".DS_Store" | "__MACOSX" | "Thumbs.db" | "__pycache__"))
}

type Collected = Vec<(String, Vec<u8>)>;

/// Keeps count while files are gathered, so the limits hold whatever the source says about itself.
#[derive(Default)]
struct Budget {
    files: usize,
    bytes: u64,
}

impl Budget {
    fn take(&mut self, rel: &str, len: u64) -> Result<(), SkillError> {
        if len > MAX_FILE_BYTES {
            return Err(invalid(format!("“{rel}” is larger than {} MB.", MAX_FILE_BYTES / 1024 / 1024)));
        }
        self.files += 1;
        self.bytes += len;
        if self.files > MAX_FILES {
            return Err(invalid(format!("The skill has more than {MAX_FILES} files.")));
        }
        if self.bytes > MAX_TOTAL_BYTES {
            return Err(invalid(format!("The skill is larger than {} MB.", MAX_TOTAL_BYTES / 1024 / 1024)));
        }
        Ok(())
    }
}

fn collect_dir(root: &Path) -> Result<Collected, SkillError> {
    let mut out: Collected = Vec::new();
    let mut budget = Budget::default();
    let mut pending = vec![(root.to_path_buf(), String::new(), 0usize)];
    while let Some((dir, prefix, depth)) = pending.pop() {
        if depth > MAX_DEPTH {
            return Err(invalid(format!("The skill nests folders deeper than {MAX_DEPTH} levels.")));
        }
        let mut entries: Vec<_> = std::fs::read_dir(&dir).map_err(io)?.collect::<Result<_, _>>().map_err(io)?;
        entries.sort_by_key(|e| e.file_name());
        for entry in entries {
            let name = entry.file_name().to_string_lossy().into_owned();
            let rel = if prefix.is_empty() { name.clone() } else { format!("{prefix}/{name}") };
            if junk(&rel) {
                continue;
            }
            let meta = std::fs::symlink_metadata(entry.path()).map_err(io)?;
            if meta.file_type().is_symlink() {
                return Err(invalid(format!("“{rel}” is a link. Links are not followed; copy the file in.")));
            }
            check_path(&rel)?;
            if meta.is_dir() {
                pending.push((entry.path(), rel, depth + 1));
            } else if meta.is_file() {
                budget.take(&rel, meta.len())?;
                let mut bytes = Vec::new();
                std::fs::File::open(entry.path())
                    .map_err(io)?
                    .take(MAX_FILE_BYTES + 1)
                    .read_to_end(&mut bytes)
                    .map_err(io)?;
                if bytes.len() as u64 > MAX_FILE_BYTES {
                    return Err(invalid(format!("“{rel}” is larger than {} MB.", MAX_FILE_BYTES / 1024 / 1024)));
                }
                out.push((rel, bytes));
            } else {
                return Err(invalid(format!("“{rel}” is not a plain file.")));
            }
        }
    }
    Ok(out)
}

fn collect_zip(path: &Path) -> Result<(Collected, Option<String>), SkillError> {
    let file = std::fs::File::open(path).map_err(io)?;
    let mut archive = zip::ZipArchive::new(std::io::BufReader::new(file))
        .map_err(|e| invalid(format!("That is not a readable zip file: {e}")))?;
    if archive.len() > MAX_FILES * 2 {
        return Err(invalid("The zip holds too many entries to be a skill."));
    }
    let mut out: Collected = Vec::new();
    let mut budget = Budget::default();
    for index in 0..archive.len() {
        let mut entry = archive.by_index(index).map_err(|e| invalid(format!("The zip is damaged: {e}")))?;
        let Some(enclosed) = entry.enclosed_name() else {
            return Err(invalid("The zip has an entry with an unsafe path."));
        };
        let rel =
            enclosed.components().map(|c| c.as_os_str().to_string_lossy().into_owned()).collect::<Vec<_>>().join("/");
        if entry.is_dir() || rel.is_empty() || junk(&rel) {
            continue;
        }
        if entry.unix_mode().is_some_and(|mode| mode & 0o170000 == 0o120000) {
            return Err(invalid(format!("“{rel}” is a link inside the zip. Links are not followed.")));
        }
        check_path(&rel)?;
        budget.take(&rel, entry.size())?;
        // The header may lie about the size, so the read itself is bounded.
        let mut bytes = Vec::new();
        (&mut entry).take(MAX_FILE_BYTES + 1).read_to_end(&mut bytes).map_err(io)?;
        if bytes.len() as u64 > MAX_FILE_BYTES {
            return Err(invalid(format!("“{rel}” is larger than {} MB once unpacked.", MAX_FILE_BYTES / 1024 / 1024)));
        }
        out.push((rel, bytes));
    }
    // One top-level folder around everything is the usual way to zip a skill: step into it.
    let top: HashSet<&str> = out.iter().filter_map(|(rel, _)| rel.split('/').next()).collect();
    if top.len() == 1 {
        let name = top.into_iter().next().unwrap_or("").to_string();
        let inside = format!("{name}/");
        if out.iter().all(|(rel, _)| rel.starts_with(&inside)) {
            for (rel, _) in &mut out {
                *rel = rel[inside.len()..].to_string();
            }
            return Ok((out, Some(name)));
        }
    }
    Ok((out, None))
}

/// What cleaning up removed.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct GcReport {
    /// Skill folders deleted.
    pub removed: usize,
    /// Bytes they held.
    pub bytes: u64,
}

fn folder_size(path: &Path) -> u64 {
    let mut total = 0;
    let mut pending = vec![path.to_path_buf()];
    while let Some(dir) = pending.pop() {
        let Ok(entries) = std::fs::read_dir(&dir) else { continue };
        for entry in entries.flatten() {
            match entry.metadata() {
                Ok(m) if m.is_dir() => pending.push(entry.path()),
                Ok(m) => total += m.len(),
                Err(_) => {}
            }
        }
    }
    total
}

/// The skills on disk.
#[derive(Clone, Debug)]
pub struct SkillLibrary {
    root: PathBuf,
}

impl SkillLibrary {
    /// The library of a data folder (`<home>/skills`).
    pub fn new(home: &Path) -> Self {
        Self { root: home.join("skills") }
    }

    /// Where the skills are kept.
    pub fn root(&self) -> &Path {
        &self.root
    }

    /// The folder of a skill, by its content hash.
    pub fn path(&self, hash: &str) -> Result<PathBuf, SkillError> {
        if hash.len() != 64 || !hash.chars().all(|c| c.is_ascii_hexdigit()) {
            return Err(invalid("A skill is named by a SHA-256 hash."));
        }
        Ok(self.root.join(hash))
    }

    /// Whether a skill's files are here.
    pub fn has(&self, hash: &str) -> bool {
        self.path(hash).is_ok_and(|p| p.join("SKILL.md").is_file())
    }

    /// Import a skill from a folder (the folder's name must match the skill's).
    pub fn import_folder(
        &self,
        folder: &Path,
        source: Option<String>,
        now: Timestamp,
    ) -> Result<SkillRecord, SkillError> {
        let real = std::fs::canonicalize(folder)
            .map_err(|e| invalid(format!("The folder {} cannot be read: {e}", folder.display())))?;
        if !real.is_dir() {
            return Err(invalid(format!("{} is not a folder.", folder.display())));
        }
        let name = real.file_name().map(|n| n.to_string_lossy().into_owned());
        let files = collect_dir(&real)?;
        self.keep(files, name.as_deref(), source.or_else(|| Some(format!("folder {}", folder.display()))), now)
    }

    /// Import a skill from a zip file (zipped with or without a folder around it).
    pub fn import_zip(&self, zip: &Path, source: Option<String>, now: Timestamp) -> Result<SkillRecord, SkillError> {
        let (files, folder) = collect_zip(zip)?;
        self.keep(files, folder.as_deref(), source.or_else(|| Some(format!("zip {}", zip.display()))), now)
    }

    /// Import files that are already in memory (a built-in template).
    pub fn import_files(
        &self,
        files: Vec<(String, Vec<u8>)>,
        folder: Option<&str>,
        source: Option<String>,
        now: Timestamp,
    ) -> Result<SkillRecord, SkillError> {
        let mut budget = Budget::default();
        for (rel, bytes) in &files {
            check_path(rel)?;
            budget.take(rel, bytes.len() as u64)?;
        }
        self.keep(files, folder, source, now)
    }

    fn keep(
        &self,
        files: Collected,
        folder: Option<&str>,
        source: Option<String>,
        now: Timestamp,
    ) -> Result<SkillRecord, SkillError> {
        let md = files
            .iter()
            .find(|(rel, _)| rel == "SKILL.md")
            .ok_or_else(|| invalid("A skill has a SKILL.md at its top. This folder does not."))?;
        let text = std::str::from_utf8(&md.1).map_err(|_| invalid("SKILL.md is not UTF-8 text."))?;
        if md.1.len() > MAX_SKILL_MD {
            return Err(invalid(format!("SKILL.md is larger than {} KB.", MAX_SKILL_MD / 1024)));
        }
        let (meta, _) = parse_skill_md(text, folder).map_err(invalid)?;
        let mut listed: Vec<SkillFile> = files
            .iter()
            .map(|(rel, bytes)| SkillFile {
                path: rel.clone(),
                size: bytes.len() as u64,
                sha256: hex(&Sha256::digest(bytes)),
            })
            .collect();
        listed.sort_by(|a, b| a.path.cmp(&b.path));
        let hash = content_hash(&listed);

        if !self.has(&hash) {
            std::fs::create_dir_all(&self.root).map_err(io)?;
            let staging = tempfile::Builder::new().prefix(".tmp-").tempdir_in(&self.root).map_err(io)?;
            for (rel, bytes) in &files {
                let target = staging.path().join(rel);
                if let Some(parent) = target.parent() {
                    std::fs::create_dir_all(parent).map_err(io)?;
                }
                std::fs::write(&target, bytes).map_err(io)?;
            }
            let staged = staging.keep();
            let target = self.path(&hash)?;
            if let Err(error) = std::fs::rename(&staged, &target) {
                let _ = std::fs::remove_dir_all(&staged);
                // Another import of the same content won the race: that is fine.
                if !self.has(&hash) {
                    return Err(io(error));
                }
            }
        }
        Ok(SkillRecord {
            name: meta.name,
            hash,
            description: meta.description,
            files: listed,
            license: meta.license,
            compatibility: meta.compatibility,
            metadata: meta.metadata,
            allowed_tools: meta.allowed_tools,
            source,
            imported_at: now,
        })
    }

    /// Read one file of a skill, at most `max` bytes. The path must be one the skill lists; nothing outside the skill's own
    /// folder can be reached.
    pub fn read(&self, hash: &str, rel: &str, max: usize) -> Result<Vec<u8>, SkillError> {
        check_path(rel)?;
        let full = self.path(hash)?.join(rel);
        let meta = std::fs::symlink_metadata(&full).map_err(|_| invalid(format!("The skill has no file {rel}.")))?;
        if !meta.is_file() {
            return Err(invalid(format!("{rel} is not a plain file.")));
        }
        if meta.len() as usize > max {
            return Err(invalid(format!(
                "{rel} is {} bytes, more than the {max} that can be read at once.",
                meta.len()
            )));
        }
        std::fs::read(&full).map_err(io)
    }

    /// Delete the skills nothing refers to. `keep` holds every hash that must stay. Half-made imports older than ten minutes go
    /// too.
    pub fn gc(&self, keep: &HashSet<String>) -> Result<GcReport, SkillError> {
        let mut report = GcReport::default();
        let Ok(entries) = std::fs::read_dir(&self.root) else { return Ok(report) };
        for entry in entries.flatten() {
            let name = entry.file_name().to_string_lossy().into_owned();
            let stale_staging = name.starts_with(".tmp-")
                && entry
                    .metadata()
                    .and_then(|m| m.modified())
                    .ok()
                    .and_then(|t| t.elapsed().ok())
                    .is_some_and(|age| age > std::time::Duration::from_secs(600));
            let unreferenced = name.len() == 64 && name.chars().all(|c| c.is_ascii_hexdigit()) && !keep.contains(&name);
            if stale_staging || unreferenced {
                report.bytes += folder_size(&entry.path());
                std::fs::remove_dir_all(entry.path()).map_err(io)?;
                report.removed += 1;
            }
        }
        Ok(report)
    }
}
