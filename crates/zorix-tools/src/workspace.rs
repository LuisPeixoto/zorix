//! `workspace.list`, `workspace.read` and `workspace.write`: file tools confined to one folder.
//!
//! Two checks protect every call. First the grants: the execution must hold `read` or `write` on
//! `workspace:<relative path>`. Then the path: it must be relative, free of `..`, and — after following symbolic links —
//! still inside the workspace root. A tool that runs without a configured workspace refuses every call.

use std::fs;
use std::io::{Read, Write};
use std::path::{Path, PathBuf};

use async_trait::async_trait;
use serde_json::{Value, json};
use zorix_core::{Action, Grants, Permission, Tool, ToolAnnotations, ToolContext, ToolError, ToolSpec};

/// Most bytes `workspace.read` returns; more is cut and marked.
pub const MAX_READ_BYTES: usize = 64 * 1024;
/// Most bytes `workspace.write` accepts.
pub const MAX_WRITE_BYTES: usize = 256 * 1024;
/// Most entries `workspace.list` returns.
pub const MAX_LIST_ENTRIES: usize = 200;

/// Names Windows keeps for devices, with or without an extension (`CON.txt` is still the console). Besides the classic ones: the
/// zero forms, the console input and output handles and the superscript digits, which Windows reads as digits.
const RESERVED: &[&str] = &[
    "CON", "PRN", "AUX", "NUL", "COM0", "COM1", "COM2", "COM3", "COM4", "COM5", "COM6", "COM7", "COM8", "COM9", "COM¹",
    "COM²", "COM³", "LPT0", "LPT1", "LPT2", "LPT3", "LPT4", "LPT5", "LPT6", "LPT7", "LPT8", "LPT9", "LPT¹", "LPT²",
    "LPT³", "CONIN$", "CONOUT$",
];

/// Whether a name is written like the short form Windows invents for a long one (`PROGRA~1`, `ZORIX~1`, `NOTES~2.TXT`). On a
/// Windows volume that keeps short names, `ZORIX~1` is the same folder as `.zorix`, which would walk around the reserved-name check; a
/// name written that way is refused on every system, so the rules do not depend on the volume.
fn looks_like_short_name(name: &str) -> bool {
    let (base, ext) = match name.split_once('.') {
        Some((base, ext)) => (base, Some(ext)),
        None => (name, None),
    };
    if ext.is_some_and(|e| e.len() > 3 || e.contains('.')) {
        return false;
    }
    let Some((prefix, digits)) = base.rsplit_once('~') else { return false };
    !prefix.is_empty()
        && !digits.is_empty()
        && prefix.len() + 1 + digits.len() <= 8
        && digits.bytes().all(|b| b.is_ascii_digit())
}

fn failed(message: impl Into<String>) -> ToolError {
    ToolError::Failed(message.into())
}

/// A path inside the workspace, checked as far as text alone can check it.
#[derive(Debug, PartialEq, Eq)]
pub struct RelativePath {
    parts: Vec<String>,
}

impl RelativePath {
    /// Check the text of a path. Refuses absolute paths, drive letters, `..`, backslashes, control characters and names
    /// Windows reserves, so the same rules hold on every operating system.
    pub fn parse(text: &str) -> Result<Self, ToolError> {
        if text.chars().any(|c| c.is_control()) {
            return Err(failed("The path contains control characters."));
        }
        if text.contains(['<', '>', '"', '|', '?', '*']) {
            return Err(failed("The path contains a character that Windows does not allow in names: < > \" | ? *."));
        }
        if text.contains('\\') || text.contains(':') {
            return Err(failed(
                "Use forward slashes and no drive letters: the path must be relative to the workspace.",
            ));
        }
        if text.starts_with('/') {
            return Err(failed("The path must be relative to the workspace, not absolute."));
        }
        let mut parts = Vec::new();
        for part in text.split('/') {
            match part {
                "" | "." => {}
                ".." => return Err(failed("The path may not go up with “..”.")),
                name => {
                    // Windows ignores spaces before the extension when it decides a name is a device: `CON .txt` is the console.
                    let stem = name.split('.').next().unwrap_or(name).trim_end().to_ascii_uppercase();
                    if RESERVED.contains(&stem.as_str()) {
                        return Err(failed(format!("“{name}” is a reserved name on Windows.")));
                    }
                    if looks_like_short_name(name) {
                        return Err(failed(format!(
                            "“{name}” is written like a short Windows name (PROGRA~1), which can stand for another file. Use the file's own name."
                        )));
                    }
                    if name.ends_with(' ') || name.ends_with('.') {
                        return Err(failed(format!(
                            "“{name}” ends with a space or a dot, which Windows does not allow."
                        )));
                    }
                    parts.push(name.to_string());
                }
            }
        }
        // Whatever the platform, never touch the folder that holds ZORIX's own state.
        // In any case: on macOS and Windows `.ZORIX` is the same folder as `.zorix`.
        if parts.first().is_some_and(|first| first.eq_ignore_ascii_case(".zorix")) {
            return Err(failed("The .zorix folder is reserved."));
        }
        Ok(Self { parts })
    }

    /// True for the workspace root itself (`""` or `"."`).
    pub fn is_root(&self) -> bool {
        self.parts.is_empty()
    }

    /// The path with forward slashes, as used in permissions.
    pub fn display(&self) -> String {
        self.parts.join("/")
    }

    /// The permission name for this path.
    pub fn resource(&self) -> String {
        format!("workspace:{}", self.display())
    }

    fn join_onto(&self, root: &Path) -> PathBuf {
        self.parts.iter().fold(root.to_path_buf(), |path, part| path.join(part))
    }
}

fn root_of(ctx: &ToolContext) -> Result<PathBuf, ToolError> {
    let root =
        ctx.workspace_root.as_ref().ok_or_else(|| failed("No workspace folder is configured for this execution."))?;
    fs::canonicalize(root).map_err(|e| failed(format!("The workspace folder cannot be used: {e}")))
}

fn require(grants: &Grants, action: Action, path: &RelativePath) -> Result<(), ToolError> {
    let needed = Permission::new(action, path.resource());
    if grants.allows(&needed) { Ok(()) } else { Err(failed(format!("This execution has no permission to {needed}."))) }
}

/// Where `real` (a path under `root`, links already followed) is, written as the relative path the grants and the reserved names
/// speak of. A link can lead somewhere the text of the request does not say: the grant and the `.zorix` reservation are about
/// the place the bytes are, so they are checked there too.
fn real_relative(root: &Path, real: &Path) -> Result<RelativePath, ToolError> {
    let inside = real.strip_prefix(root).map_err(|_| {
        eprintln!("DIAG relative real={real:?} root={root:?}");
        failed("That path leads outside the workspace.")
    })?;
    let mut text = Vec::new();
    for part in inside.components() {
        let name = part.as_os_str().to_str().ok_or_else(|| failed("That path has a name ZORIX cannot read."))?;
        text.push(name);
    }
    RelativePath::parse(&text.join("/")).map_err(|e| match e {
        ToolError::Failed(why) => {
            failed(format!("That path leads, through a link, to a place that is off limits: {why}"))
        }
        other => other,
    })
}

/// A link inside the workspace may only lead where the same grant reaches: the place it leads to needs its own permission.
fn require_real(grants: &Grants, action: Action, asked: &RelativePath, real: &RelativePath) -> Result<(), ToolError> {
    if real == asked {
        return Ok(());
    }
    let needed = Permission::new(action, real.resource());
    if grants.allows(&needed) {
        Ok(())
    } else {
        Err(failed(format!(
            "{} leads, through a link, to {}, and this execution has no permission to {needed}.",
            asked.display(),
            real.display()
        )))
    }
}

/// Resolve an existing path and prove, after following links, that it is inside the root and that the place it leads to is
/// granted and not reserved.
fn resolve_existing(root: &Path, grants: &Grants, action: Action, path: &RelativePath) -> Result<PathBuf, ToolError> {
    let joined = path.join_onto(root);
    let real = fs::canonicalize(&joined).map_err(|e| match e.kind() {
        std::io::ErrorKind::NotFound => failed(format!("There is no such file or folder: {}.", path.display())),
        _ => failed(format!("Cannot open {}: {e}", path.display())),
    })?;
    if !real.starts_with(root) {
        return Err(failed("That path leads outside the workspace."));
    }
    require_real(grants, action, path, &real_relative(root, &real)?)?;
    Ok(real)
}

/// The folder a new file would go into, proven to be inside the root even if some folders do not exist yet.
fn resolve_for_write(root: &Path, grants: &Grants, path: &RelativePath) -> Result<PathBuf, ToolError> {
    if path.is_root() {
        return Err(failed("Name a file, not the workspace folder."));
    }
    let target = path.join_onto(root);
    // The deepest ancestor that exists must resolve inside the root.
    let mut ancestor = target.parent().unwrap_or(root).to_path_buf();
    while !ancestor.exists() {
        eprintln!("DIAG missing ancestor {ancestor:?} metadata={:?} root={root:?}", fs::metadata(&ancestor).map(|m| m.is_dir()));
        ancestor = ancestor.parent().ok_or_else(|| failed("That path leads outside the workspace."))?.to_path_buf();
    }
    let real = fs::canonicalize(&ancestor).map_err(|e| failed(format!("Cannot use that location: {e}")))?;
    if !real.starts_with(root) {
        eprintln!("DIAG resolve real={real:?} root={root:?} ancestor={ancestor:?}");
        return Err(failed("That path leads outside the workspace."));
    }
    // Where the file would really be: the folder a link leads to, then what is left of the path. That place needs its own grant
    // and cannot be a reserved one.
    let rest = target.strip_prefix(&ancestor).map_err(|_| failed("That path leads outside the workspace."))?;
    let real_target = real_relative(root, &real.join(rest))?;
    require_real(grants, Action::Write, path, &real_target)?;
    // Never write through a link that is already there: it could point anywhere.
    if let Ok(meta) = fs::symlink_metadata(&target) {
        if meta.file_type().is_symlink() {
            return Err(failed("The file is a symbolic link; ZORIX will not write through it."));
        }
        if meta.is_dir() {
            return Err(failed("That path is a folder."));
        }
    }
    Ok(target)
}

/// List a folder of the workspace.
pub fn list(root: &Path, grants: &Grants, path: &str) -> Result<Value, ToolError> {
    let root = fs::canonicalize(root).map_err(|e| failed(format!("The workspace folder cannot be used: {e}")))?;
    let path = RelativePath::parse(path)?;
    require(grants, Action::Read, &path)?;
    let folder = resolve_existing(&root, grants, Action::Read, &path)?;
    if !folder.is_dir() {
        return Err(failed(format!("{} is not a folder.", path.display())));
    }
    let mut entries: Vec<(String, bool, u64)> = fs::read_dir(&folder)
        .map_err(|e| failed(format!("Cannot list {}: {e}", path.display())))?
        .filter_map(Result::ok)
        .filter(|entry| !entry.file_name().to_string_lossy().eq_ignore_ascii_case(".zorix"))
        .map(|entry| {
            let meta = entry.metadata().ok();
            (
                entry.file_name().to_string_lossy().into_owned(),
                meta.as_ref().is_some_and(|m| m.is_dir()),
                meta.map_or(0, |m| m.len()),
            )
        })
        .collect();
    entries.sort();
    let total = entries.len();
    let shown: Vec<Value> = entries
        .into_iter()
        .take(MAX_LIST_ENTRIES)
        .map(|(name, dir, size)| json!({ "name": name, "kind": if dir { "dir" } else { "file" }, "size": size }))
        .collect();
    Ok(json!({ "path": path.display(), "entries": shown, "total": total, "truncated": total > MAX_LIST_ENTRIES }))
}

/// Read a file of the workspace as text.
pub fn read(root: &Path, grants: &Grants, path: &str) -> Result<Value, ToolError> {
    let root = fs::canonicalize(root).map_err(|e| failed(format!("The workspace folder cannot be used: {e}")))?;
    let path = RelativePath::parse(path)?;
    require(grants, Action::Read, &path)?;
    let file = resolve_existing(&root, grants, Action::Read, &path)?;
    if !file.is_file() {
        return Err(failed(format!("{} is not a file.", path.display())));
    }
    let size = fs::metadata(&file).map(|m| m.len()).unwrap_or(0);
    let mut buffer = Vec::new();
    fs::File::open(&file)
        .and_then(|f| f.take(MAX_READ_BYTES as u64).read_to_end(&mut buffer))
        .map_err(|e| failed(format!("Cannot read {}: {e}", path.display())))?;
    Ok(json!({
        "path": path.display(),
        "content": String::from_utf8_lossy(&buffer),
        "bytes": size,
        "truncated": size > MAX_READ_BYTES as u64,
    }))
}

/// Write (create or replace) a file of the workspace.
pub fn write(root: &Path, grants: &Grants, path: &str, content: &str) -> Result<Value, ToolError> {
    let root = fs::canonicalize(root).map_err(|e| failed(format!("The workspace folder cannot be used: {e}")))?;
    let path = RelativePath::parse(path)?;
    require(grants, Action::Write, &path)?;
    if content.len() > MAX_WRITE_BYTES {
        return Err(failed(format!("The content is larger than {MAX_WRITE_BYTES} bytes.")));
    }
    let target = resolve_for_write(&root, grants, &path)?;
    if let Some(parent) = target.parent() {
        fs::create_dir_all(parent).map_err(|e| failed(format!("Cannot create the folder: {e}")))?;
        // Created folders could not be links, but check again: another process may have raced us.
        let real = fs::canonicalize(parent).map_err(|e| failed(format!("Cannot use that location: {e}")))?;
        if !real.starts_with(&root) {
            eprintln!("DIAG after-create real={real:?} root={root:?} parent={parent:?}");
            return Err(failed("That path leads outside the workspace."));
        }
    }
    // Write a temporary file next to the target and rename it, so a crash never leaves half a file. The file is created, never
    // opened: it must not exist (a link left at that name is not followed), and its name is the target's own plus a number nobody
    // can guess, so two files with the same stem never share one.
    let (temporary, mut file) =
        create_temporary(&target).map_err(|e| failed(format!("Cannot write {}: {e}", path.display())))?;
    if let Err(e) = file.write_all(content.as_bytes()).and_then(|()| file.sync_all()) {
        drop(file);
        let _ = fs::remove_file(&temporary);
        return Err(failed(format!("Cannot write {}: {e}", path.display())));
    }
    drop(file);
    fs::rename(&temporary, &target).map_err(|e| {
        let _ = fs::remove_file(&temporary);
        failed(format!("Cannot write {}: {e}", path.display()))
    })?;
    Ok(json!({ "path": path.display(), "bytes": content.len() }))
}

/// Create a new, empty file next to `target` for the content to be written to before the rename. `create_new` fails on anything
/// already at that name, a symbolic link included, so it can never write through one.
fn create_temporary(target: &Path) -> std::io::Result<(PathBuf, fs::File)> {
    use std::sync::atomic::{AtomicU64, Ordering};
    static COUNTER: AtomicU64 = AtomicU64::new(0);
    let name = target.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
    let mut last = None;
    for _ in 0..8 {
        let nanos = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map_or(0, |d| d.as_nanos());
        let unique =
            (nanos as u64) ^ COUNTER.fetch_add(1, Ordering::Relaxed).rotate_left(32) ^ u64::from(std::process::id());
        let temporary = target.with_file_name(format!(".{name}.zorix-{unique:016x}.tmp"));
        match fs::OpenOptions::new().write(true).create_new(true).open(&temporary) {
            Ok(file) => return Ok((temporary, file)),
            Err(e) if e.kind() == std::io::ErrorKind::AlreadyExists => last = Some(e),
            Err(e) => return Err(e),
        }
    }
    Err(last.unwrap_or_else(|| std::io::Error::other("no free name for a temporary file")))
}

macro_rules! file_tool {
    ($name:ident, $doc:literal) => {
        #[doc = $doc]
        pub struct $name {
            spec: ToolSpec,
        }
    };
}

file_tool!(ListTool, "The `workspace.list` tool.");
file_tool!(ReadTool, "The `workspace.read` tool.");
file_tool!(WriteTool, "The `workspace.write` tool.");

fn path_schema(extra: Option<(&str, Value)>) -> Value {
    let mut properties = json!({ "path": {"type": "string", "maxLength": 512, "description": "Relative to the workspace, with forward slashes"} });
    let mut required = vec!["path"];
    if let Some((name, schema)) = extra {
        properties[name] = schema;
        required.push(name);
    }
    json!({ "type": "object", "properties": properties, "required": required, "additionalProperties": false })
}

impl ListTool {
    /// A new tool.
    pub fn new() -> Self {
        let mut spec = ToolSpec::new(
            "workspace.list",
            "List files",
            "Lists the files and folders inside a folder of the workspace.",
            path_schema(None),
        );
        spec.annotations = ToolAnnotations::safe_read();
        Self { spec }
    }
}

impl ReadTool {
    /// A new tool.
    pub fn new() -> Self {
        let mut spec = ToolSpec::new(
            "workspace.read",
            "Read file",
            "Reads a text file of the workspace (the first 64 KiB).",
            path_schema(None),
        );
        spec.annotations = ToolAnnotations::safe_read();
        Self { spec }
    }
}

impl WriteTool {
    /// A new tool. Every call asks a person first.
    pub fn new() -> Self {
        let schema = path_schema(Some(("content", json!({"type": "string", "maxLength": MAX_WRITE_BYTES}))));
        let mut spec =
            ToolSpec::new("workspace.write", "Write file", "Creates or replaces a text file of the workspace.", schema);
        spec.annotations = ToolAnnotations { read_only: false, destructive: true, idempotent: true, open_world: false };
        spec.requires_approval = true;
        Self { spec }
    }
}

macro_rules! default_new {
    ($($name:ident),+) => { $(impl Default for $name { fn default() -> Self { Self::new() } })+ };
}
default_new!(ListTool, ReadTool, WriteTool);

fn text<'a>(args: &'a Value, key: &str) -> &'a str {
    args[key].as_str().unwrap_or_default()
}

/// Run blocking file work off the async threads.
async fn blocking<T: Send + 'static>(
    work: impl FnOnce() -> Result<T, ToolError> + Send + 'static,
) -> Result<T, ToolError> {
    tokio::task::spawn_blocking(work).await.map_err(|e| failed(format!("The file task failed: {e}")))?
}

#[async_trait]
impl Tool for ListTool {
    fn spec(&self) -> &ToolSpec {
        &self.spec
    }

    async fn call(&self, ctx: &ToolContext, args: Value) -> Result<Value, ToolError> {
        let (root, grants, path) = (root_of(ctx)?, ctx.grants.clone(), text(&args, "path").to_string());
        blocking(move || list(&root, &grants, &path)).await
    }
}

#[async_trait]
impl Tool for ReadTool {
    fn spec(&self) -> &ToolSpec {
        &self.spec
    }

    async fn call(&self, ctx: &ToolContext, args: Value) -> Result<Value, ToolError> {
        let (root, grants, path) = (root_of(ctx)?, ctx.grants.clone(), text(&args, "path").to_string());
        blocking(move || read(&root, &grants, &path)).await
    }
}

#[async_trait]
impl Tool for WriteTool {
    fn spec(&self) -> &ToolSpec {
        &self.spec
    }

    async fn call(&self, ctx: &ToolContext, args: Value) -> Result<Value, ToolError> {
        let (root, grants) = (root_of(ctx)?, ctx.grants.clone());
        let (path, content) = (text(&args, "path").to_string(), text(&args, "content").to_string());
        blocking(move || write(&root, &grants, &path, &content)).await
    }
}

#[cfg(test)]
mod tests {
    use zorix_core::{ExecutionId, TenantId, ToolCallId};

    use super::*;

    fn grants() -> Grants {
        Grants::none()
            .with(Permission::new(Action::Read, "workspace:*"))
            .with(Permission::new(Action::Write, "workspace:*"))
    }

    fn message(result: Result<Value, ToolError>) -> String {
        match result.unwrap_err() {
            ToolError::Failed(text) | ToolError::Retryable(text) => text,
        }
    }

    #[test]
    fn paths_that_could_escape_are_refused_before_touching_the_disk() {
        for bad in [
            "/etc/passwd",
            "../secret",
            "a/../../secret",
            "a/..",
            "C:/Windows",
            "c:\\x",
            "a\\b",
            "//server/share",
            "dir/CON",
            "nul.txt",
            "trailing.",
            "name ",
            ".zorix/state",
            ".ZORIX/state",
            ".Zorix",
            "./.zOrIx/x",
            "a\0b",
            "line\nbreak",
            // Ways of writing "up" or "root" that other systems read as one.
            "a/./../../x",
            "..//x",
            "a/b/../../..",
            "x:stream",
            "COM1.txt",
            "Prn",
            "aux.tar.gz",
            "\\\\server\\share\\f",
            "/",
            "\u{7f}",
            "tab\there",
            "...",
            // The device names and forms an earlier version let through.
            "COM0",
            "lpt0.txt",
            "CONIN$",
            "conout$.log",
            "COM\u{b9}",
            "lpt\u{b2}.txt",
            "CON .txt",
            "nul  ",
            // Characters Windows refuses in a name, and short names that stand for another file.
            "a<b",
            "a>b",
            "a\"b",
            "a|b",
            "a?b",
            "a*b",
            "ZORIX~1",
            "zorix~1",
            "docs/ZORIX~1/state",
            "PROGRA~1",
            "NOTES~2.TXT",
            "a~12345",
        ] {
            assert!(RelativePath::parse(bad).is_err(), "{bad:?} should be refused");
        }
        for good in [
            "",
            ".",
            "a.txt",
            "./a/./b.txt",
            "docs/ünïcode.md",
            "a//b",
            ".hidden",
            "data.v1/x",
            "a/.zorix",
            "%2e%2e/x",
            "console.txt",
            "a..b/c",
            "COM10",
            "com.txt",
            "conin",
            "report~final.md",
            "backup~12345678.txt",
            "a~1.tar.gz",
            "~1",
            "x~y",
            "ünï~1d",
        ] {
            assert!(RelativePath::parse(good).is_ok(), "{good:?} should be accepted");
        }
    }

    #[test]
    fn paths_are_normalised_for_permissions() {
        assert_eq!(RelativePath::parse("./a//b/./c.txt").unwrap().resource(), "workspace:a/b/c.txt");
        assert!(RelativePath::parse(".").unwrap().is_root());
    }

    #[test]
    fn write_then_read_then_list_round_trip() {
        let dir = tempfile::tempdir().unwrap();
        let written = write(dir.path(), &grants(), "notes/today.txt", "héllo").unwrap();
        assert_eq!(written, json!({"path": "notes/today.txt", "bytes": 6}));
        let read = read(dir.path(), &grants(), "notes/today.txt").unwrap();
        assert_eq!((read["content"].as_str(), read["truncated"].as_bool()), (Some("héllo"), Some(false)));
        let listed = list(dir.path(), &grants(), "notes").unwrap();
        assert_eq!(listed["entries"], json!([{"name": "today.txt", "kind": "file", "size": 6}]));
        let root = list(dir.path(), &grants(), ".").unwrap();
        assert_eq!(root["entries"], json!([{"name": "notes", "kind": "dir", "size": root["entries"][0]["size"]}]));
        assert!(fs::read_dir(dir.path().join("notes")).unwrap().count() == 1, "no temporary file is left behind");
    }

    #[test]
    fn overwriting_replaces_the_content() {
        let dir = tempfile::tempdir().unwrap();
        write(dir.path(), &grants(), "a.txt", "one").unwrap();
        write(dir.path(), &grants(), "a.txt", "two").unwrap();
        assert_eq!(read(dir.path(), &grants(), "a.txt").unwrap()["content"], "two");
    }

    #[test]
    fn nothing_works_without_a_grant_for_that_exact_path() {
        let dir = tempfile::tempdir().unwrap();
        fs::write(dir.path().join("a.txt"), "x").unwrap();
        assert!(message(read(dir.path(), &Grants::none(), "a.txt")).contains("no permission"));
        let only_docs = Grants::none().with(Permission::new(Action::Read, "workspace:docs/*"));
        assert!(message(read(dir.path(), &only_docs, "a.txt")).contains("no permission to read workspace:a.txt"));
        let read_only = Grants::none().with(Permission::new(Action::Read, "workspace:*"));
        assert!(message(write(dir.path(), &read_only, "a.txt", "y")).contains("no permission to write"));
        assert_eq!(fs::read_to_string(dir.path().join("a.txt")).unwrap(), "x", "untouched");
    }

    #[test]
    fn a_grant_for_a_subfolder_is_enough_inside_it_and_not_outside() {
        let dir = tempfile::tempdir().unwrap();
        let docs = Grants::none().with(Permission::new(Action::Write, "workspace:docs/*"));
        write(dir.path(), &docs, "docs/a.md", "ok").unwrap();
        assert!(write(dir.path(), &docs, "other/a.md", "no").is_err());
        assert!(!dir.path().join("other").exists());
    }

    #[test]
    fn missing_files_and_wrong_kinds_get_clear_messages() {
        let dir = tempfile::tempdir().unwrap();
        fs::create_dir(dir.path().join("folder")).unwrap();
        fs::write(dir.path().join("file.txt"), "x").unwrap();
        assert!(message(read(dir.path(), &grants(), "nope.txt")).contains("There is no such file"));
        assert!(message(read(dir.path(), &grants(), "folder")).contains("is not a file"));
        assert!(message(list(dir.path(), &grants(), "file.txt")).contains("is not a folder"));
        assert!(message(write(dir.path(), &grants(), "folder", "x")).contains("folder"));
        assert!(message(write(dir.path(), &grants(), "", "x")).contains("Name a file"));
    }

    #[test]
    fn size_limits_hold() {
        let dir = tempfile::tempdir().unwrap();
        let big = "x".repeat(MAX_WRITE_BYTES + 1);
        assert!(message(write(dir.path(), &grants(), "big.txt", &big)).contains("larger than"));
        fs::write(dir.path().join("long.txt"), "y".repeat(MAX_READ_BYTES + 500)).unwrap();
        let read = read(dir.path(), &grants(), "long.txt").unwrap();
        assert_eq!(read["content"].as_str().unwrap().len(), MAX_READ_BYTES);
        assert_eq!(read["truncated"], true);
        for n in 0..(MAX_LIST_ENTRIES + 5) {
            fs::write(dir.path().join(format!("f{n:04}.txt")), "").unwrap();
        }
        let listed = list(dir.path(), &grants(), ".").unwrap();
        assert_eq!(listed["entries"].as_array().unwrap().len(), MAX_LIST_ENTRIES);
        assert_eq!(listed["truncated"], true);
    }

    #[cfg(unix)]
    #[test]
    fn a_link_that_points_outside_is_not_followed() {
        use std::os::unix::fs::symlink;
        let outside = tempfile::tempdir().unwrap();
        fs::write(outside.path().join("secret.txt"), "top secret").unwrap();
        let dir = tempfile::tempdir().unwrap();
        symlink(outside.path().join("secret.txt"), dir.path().join("link.txt")).unwrap();
        symlink(outside.path(), dir.path().join("folder-link")).unwrap();

        assert!(message(read(dir.path(), &grants(), "link.txt")).contains("outside the workspace"));
        assert!(message(read(dir.path(), &grants(), "folder-link/secret.txt")).contains("outside the workspace"));
        assert!(message(list(dir.path(), &grants(), "folder-link")).contains("outside the workspace"));
        assert!(message(write(dir.path(), &grants(), "folder-link/new.txt", "x")).contains("outside the workspace"));
        assert!(message(write(dir.path(), &grants(), "link.txt", "overwritten")).contains("symbolic link"));
        assert_eq!(fs::read_to_string(outside.path().join("secret.txt")).unwrap(), "top secret");
        assert!(!outside.path().join("new.txt").exists());
    }

    #[cfg(unix)]
    #[test]
    fn a_link_that_stays_inside_is_fine_to_read() {
        use std::os::unix::fs::symlink;
        let dir = tempfile::tempdir().unwrap();
        fs::write(dir.path().join("real.txt"), "inside").unwrap();
        symlink(dir.path().join("real.txt"), dir.path().join("alias.txt")).unwrap();
        assert_eq!(read(dir.path(), &grants(), "alias.txt").unwrap()["content"], "inside");
    }

    #[cfg(unix)]
    #[test]
    fn a_link_inside_the_workspace_cannot_lead_past_the_grant() {
        use std::os::unix::fs::symlink;
        let dir = tempfile::tempdir().unwrap();
        fs::create_dir_all(dir.path().join("docs")).unwrap();
        fs::create_dir_all(dir.path().join("private")).unwrap();
        fs::write(dir.path().join("private/notes.txt"), "classified").unwrap();
        symlink("../private/notes.txt", dir.path().join("docs/l")).unwrap();
        symlink("../private", dir.path().join("docs/dir")).unwrap();

        let docs = Grants::none()
            .with(Permission::new(Action::Read, "workspace:docs/*"))
            .with(Permission::new(Action::Write, "workspace:docs/*"));
        // The grant is for docs/*; the link leads to private/notes.txt, which it does not cover.
        let error = message(read(dir.path(), &docs, "docs/l"));
        assert!(error.contains("leads, through a link, to private/notes.txt"), "{error}");
        assert!(message(read(dir.path(), &docs, "docs/dir/notes.txt")).contains("through a link"));
        assert!(message(list(dir.path(), &docs, "docs/dir")).contains("through a link"));
        assert!(message(write(dir.path(), &docs, "docs/dir/new.txt", "x")).contains("through a link"));
        assert!(message(write(dir.path(), &docs, "docs/dir/notes.txt", "x")).contains("through a link"));
        assert!(!dir.path().join("private/new.txt").exists());
        assert_eq!(fs::read_to_string(dir.path().join("private/notes.txt")).unwrap(), "classified");

        // A grant for the place the link leads to is a grant for the link.
        let both = docs
            .clone()
            .with(Permission::new(Action::Read, "workspace:private/*"))
            .with(Permission::new(Action::Write, "workspace:private/*"));
        assert_eq!(read(dir.path(), &both, "docs/l").unwrap()["content"], "classified");
        write(dir.path(), &both, "docs/dir/new.txt", "ok").unwrap();
        assert_eq!(fs::read_to_string(dir.path().join("private/new.txt")).unwrap(), "ok");
    }

    #[cfg(unix)]
    #[test]
    fn a_link_cannot_reach_the_reserved_folder() {
        use std::os::unix::fs::symlink;
        let dir = tempfile::tempdir().unwrap();
        fs::create_dir_all(dir.path().join(".zorix")).unwrap();
        fs::write(dir.path().join(".zorix/state"), "internal").unwrap();
        symlink(".zorix", dir.path().join("x")).unwrap();
        symlink(".zorix/state", dir.path().join("y")).unwrap();
        // Even with a grant for everything: the folder is reserved wherever the request says it is.
        for path in ["x/state", "y"] {
            let error = message(read(dir.path(), &grants(), path));
            assert!(error.contains("off limits") && error.contains("reserved"), "{path}: {error}");
        }
        assert!(message(list(dir.path(), &grants(), "x")).contains("off limits"));
        assert!(message(write(dir.path(), &grants(), "x/new", "x")).contains("off limits"));
        assert!(!dir.path().join(".zorix/new").exists());
    }

    #[test]
    fn a_listing_hides_the_reserved_folder_in_any_case() {
        let dir = tempfile::tempdir().unwrap();
        fs::write(dir.path().join(".Zorix"), "x").unwrap();
        fs::write(dir.path().join("a.txt"), "x").unwrap();
        let listed = list(dir.path(), &grants(), ".").unwrap();
        let names: Vec<&str> =
            listed["entries"].as_array().unwrap().iter().map(|e| e["name"].as_str().unwrap()).collect();
        assert_eq!(names, ["a.txt"]);
    }

    #[cfg(unix)]
    #[test]
    fn a_write_never_goes_through_a_link_left_at_the_temporary_name() {
        use std::os::unix::fs::symlink;
        let outside = tempfile::tempdir().unwrap();
        let victim = outside.path().join("victim.txt");
        fs::write(&victim, "untouched").unwrap();
        let dir = tempfile::tempdir().unwrap();
        // The names an older version used for its temporary file: a link there pointed the write at the victim.
        let old_name = format!("a.zorix-{}.tmp", std::process::id());
        symlink(&victim, dir.path().join(old_name)).unwrap();
        write(dir.path(), &grants(), "a.txt", "mine").unwrap();
        assert_eq!(fs::read_to_string(dir.path().join("a.txt")).unwrap(), "mine");
        assert_eq!(fs::read_to_string(&victim).unwrap(), "untouched");
    }

    #[test]
    fn files_that_share_a_stem_never_share_a_temporary_file() {
        let dir = tempfile::tempdir().unwrap();
        let root = std::sync::Arc::new(dir.path().to_path_buf());
        let writers: Vec<_> = (0..8)
            .map(|n| {
                let root = root.clone();
                std::thread::spawn(move || {
                    let (name, body) = if n % 2 == 0 { ("a.txt", "text") } else { ("a.md", "markdown") };
                    for _ in 0..40 {
                        write(&root, &grants(), name, body).unwrap();
                        assert_eq!(read(&root, &grants(), name).unwrap()["content"], body);
                    }
                })
            })
            .collect();
        for writer in writers {
            writer.join().unwrap();
        }
        let mut left: Vec<String> =
            fs::read_dir(dir.path()).unwrap().map(|e| e.unwrap().file_name().to_string_lossy().into_owned()).collect();
        left.sort();
        assert_eq!(left, ["a.md", "a.txt"], "no temporary file is left behind");
    }

    #[test]
    fn a_temporary_file_is_new_and_has_a_name_of_its_own() {
        let dir = tempfile::tempdir().unwrap();
        let target = dir.path().join("a.txt");
        let (first, _f1) = create_temporary(&target).unwrap();
        let (second, _f2) = create_temporary(&target).unwrap();
        assert_ne!(first, second);
        assert_eq!(first.parent(), Some(dir.path()));
        assert!(first.file_name().unwrap().to_string_lossy().starts_with(".a.txt.zorix-"));
    }

    #[tokio::test]
    async fn without_a_workspace_every_call_is_refused() {
        let ctx = ToolContext {
            execution_id: ExecutionId::new(),
            tenant: TenantId::local(),
            call_id: ToolCallId("c".into()),
            grants: grants(),
            cancel: tokio_util::sync::CancellationToken::new(),
            workspace_root: None,
            conversation: None,
            agent: None,
        };
        let error = ReadTool::new().call(&ctx, json!({"path": "a.txt"})).await.unwrap_err();
        assert!(matches!(error, ToolError::Failed(text) if text.contains("No workspace folder")));
    }

    #[tokio::test]
    async fn the_tools_work_through_the_trait() {
        let dir = tempfile::tempdir().unwrap();
        let ctx = ToolContext {
            execution_id: ExecutionId::new(),
            tenant: TenantId::local(),
            call_id: ToolCallId("c".into()),
            grants: grants(),
            cancel: tokio_util::sync::CancellationToken::new(),
            workspace_root: Some(dir.path().to_path_buf()),
            conversation: None,
            agent: None,
        };
        WriteTool::new().call(&ctx, json!({"path": "x.txt", "content": "hi"})).await.unwrap();
        assert_eq!(ReadTool::new().call(&ctx, json!({"path": "x.txt"})).await.unwrap()["content"], "hi");
        assert_eq!(ListTool::new().call(&ctx, json!({"path": "."})).await.unwrap()["total"], 1);
        assert!(WriteTool::new().spec().requires_approval);
    }
}
