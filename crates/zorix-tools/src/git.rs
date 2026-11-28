//! `git.status`, `git.diff`, `git.log`, `git.branch`, `git.commit` and `git.push` in the workspace, with the system's git made
//! deaf to everything in the repository that could run a program (ADR-0054).
//!
//! Grants: `read git:<branch>` to look, `write git:<branch>` to create, switch or commit on it, and `write git-push:<remote>/<branch>`
//! to push. Files also need the workspace grants: what a run may not read it is not shown, and what it may not write it cannot commit.

use std::collections::BTreeSet;
use std::ffi::OsStr;
use std::path::{Path, PathBuf};
use std::process::Stdio;
use std::sync::Arc;
use std::time::Duration;

use async_trait::async_trait;
use serde_json::{Value, json};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::process::Command;
use tokio_util::sync::CancellationToken;
use zorix_core::{Action, Grants, Permission, Tool, ToolAnnotations, ToolContext, ToolError, ToolSpec};

use crate::workspace::RelativePath;

/// The most of a diff or a log that is returned.
pub const MAX_OUTPUT: usize = 64 * 1024;
/// The most entries `git.status` lists.
pub const MAX_ENTRIES: usize = 200;
/// The most commits `git.log` returns.
pub const MAX_COMMITS: usize = 50;
/// The most paths one call names.
const MAX_PATHS: usize = 100;
const LOCAL_TIMEOUT: Duration = Duration::from_secs(30);
const PUSH_TIMEOUT: Duration = Duration::from_secs(120);

/// What of the environment git is not given: variables that point it elsewhere or make it run something.
const REMOVED_ENV: [&str; 14] = [
    "GIT_DIR",
    "GIT_WORK_TREE",
    "GIT_INDEX_FILE",
    "GIT_OBJECT_DIRECTORY",
    "GIT_ALTERNATE_OBJECT_DIRECTORIES",
    "GIT_COMMON_DIR",
    "GIT_NAMESPACE",
    "GIT_CONFIG",
    "GIT_CONFIG_GLOBAL",
    "GIT_CONFIG_COUNT",
    "GIT_EXEC_PATH",
    "GIT_EXTERNAL_DIFF",
    "GIT_ASKPASS",
    "GIT_EDITOR",
];

fn failed(message: impl Into<String>) -> ToolError {
    ToolError::Failed(message.into())
}

/// Whether a key of the repository's own configuration can run a program or send git somewhere else.
fn dangerous(key: &str) -> bool {
    let key = key.to_ascii_lowercase();
    let (section, rest) = key.split_once('.').unwrap_or((key.as_str(), ""));
    let last = key.rsplit('.').next().unwrap_or("");
    match section {
        "filter" => matches!(last, "clean" | "smudge" | "process"),
        "diff" => key == "diff.external" || matches!(last, "command" | "textconv"),
        "merge" => last == "driver",
        "credential" => last == "helper",
        "gpg" => last == "program",
        "remote" => matches!(last, "receivepack" | "uploadpack" | "vcs"),
        "url" => matches!(last, "insteadof" | "pushinsteadof"),
        "include" | "includeif" | "protocol" => true,
        "core" => matches!(
            rest,
            "sshcommand" | "gitproxy" | "fsmonitor" | "hookspath" | "askpass" | "editor" | "pager" | "worktree"
        ),
        "sequence" => rest == "editor",
        _ => false,
    }
}

/// What a run of git came to.
struct Output {
    ok: bool,
    stdout: String,
    stderr: String,
    cut: bool,
}

async fn capped(reader: Option<impl AsyncReadExt + Unpin>, cap: usize) -> (Vec<u8>, bool) {
    let (mut kept, mut cut) = (Vec::new(), false);
    let Some(mut reader) = reader else { return (kept, cut) };
    let mut buffer = [0u8; 8192];
    while let Ok(n) = reader.read(&mut buffer).await {
        if n == 0 {
            break;
        }
        let room = cap.saturating_sub(kept.len());
        kept.extend_from_slice(&buffer[..n.min(room)]);
        // The rest is still read, so git never blocks writing it.
        cut |= n > room;
    }
    (kept, cut)
}

/// Windows' canonical paths start with `\\?\`, which git and `CreateProcess` do not take as a working folder.
fn for_git(path: &Path) -> PathBuf {
    match path.to_str().and_then(|text| text.strip_prefix(r"\\?\")) {
        // A network path written that way (`\\?\UNC\…`) is left alone.
        Some(rest) if !rest.starts_with("UNC") => PathBuf::from(rest),
        _ => path.to_path_buf(),
    }
}

/// The repository whose top folder is the workspace.
pub struct Repo {
    root: PathBuf,
}

impl Repo {
    fn command(root: &Path) -> Command {
        let mut command = Command::new("git");
        for name in REMOVED_ENV {
            command.env_remove(name);
        }
        command
            .env("GIT_TERMINAL_PROMPT", "0")
            .env("GIT_OPTIONAL_LOCKS", "0")
            .env("GIT_LITERAL_PATHSPECS", "1")
            .env("LC_ALL", "C")
            .current_dir(for_git(root));
        if let Some(parent) = for_git(root).parent() {
            command.env("GIT_CEILING_DIRECTORIES", parent);
        }
        // Said on the command line, which wins over every configuration file.
        for setting in [
            "core.hooksPath=/dev/null",
            "core.fsmonitor=false",
            "core.pager=cat",
            "core.quotePath=false",
            "protocol.ext.allow=never",
            "submodule.recurse=false",
        ] {
            command.arg("-c").arg(setting);
        }
        command
    }

    async fn run<I, S>(
        &self,
        args: I,
        input: Option<&str>,
        cancel: &CancellationToken,
        timeout: Duration,
    ) -> Result<Output, ToolError>
    where
        I: IntoIterator<Item = S>,
        S: AsRef<OsStr>,
    {
        let mut command = Self::command(&self.root);
        command
            .args(args)
            .stdin(if input.is_some() { Stdio::piped() } else { Stdio::null() })
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .kill_on_drop(true);
        let mut child = command.spawn().map_err(|e| match e.kind() {
            std::io::ErrorKind::NotFound => failed("Git is not installed on this computer."),
            _ => failed(format!("Git could not be started: {e}")),
        })?;
        let work = async {
            let writer = child.stdin.take();
            let write = async {
                if let (Some(mut writer), Some(input)) = (writer, input) {
                    let _ = writer.write_all(input.as_bytes()).await;
                }
            };
            let (out, err) = (child.stdout.take(), child.stderr.take());
            let (_, (stdout, cut), (stderr, _)) = tokio::join!(write, capped(out, MAX_OUTPUT), capped(err, 4096));
            let status = child.wait().await;
            (status, stdout, stderr, cut)
        };
        let (status, stdout, stderr, cut) = tokio::select! {
            _ = cancel.cancelled() => return Err(failed("Cancelled.")),
            done = tokio::time::timeout(timeout, work) => done
                .map_err(|_| ToolError::Retryable(format!("Git took longer than {} seconds.", timeout.as_secs())))?,
        };
        let status = status.map_err(|e| failed(format!("Git failed: {e}")))?;
        Ok(Output {
            ok: status.success(),
            stdout: String::from_utf8_lossy(&stdout).into_owned(),
            stderr: zorix_core::redact_text(String::from_utf8_lossy(&stderr).trim()),
            cut,
        })
    }

    /// Run and expect success.
    async fn ok<I, S>(&self, args: I, cancel: &CancellationToken) -> Result<Output, ToolError>
    where
        I: IntoIterator<Item = S>,
        S: AsRef<OsStr>,
    {
        let output = self.run(args, None, cancel, LOCAL_TIMEOUT).await?;
        if output.ok { Ok(output) } else { Err(failed(format!("Git said: {}", output.stderr))) }
    }

    /// The repository of the workspace, refused unless its top folder is the workspace, its git folder is inside it and its own
    /// configuration cannot run anything.
    pub async fn open(root: Option<&Path>, cancel: &CancellationToken) -> Result<Self, ToolError> {
        let root = root.ok_or_else(|| failed("No workspace folder is configured for this execution."))?;
        let root =
            std::fs::canonicalize(root).map_err(|e| failed(format!("The workspace folder cannot be used: {e}")))?;
        let repo = Self { root };
        let found = repo
            .run(
                ["rev-parse", "--show-toplevel", "--absolute-git-dir", "--git-common-dir"],
                None,
                cancel,
                LOCAL_TIMEOUT,
            )
            .await?;
        if !found.ok {
            return Err(failed("The workspace folder is not the top of a git repository."));
        }
        let mut lines = found.stdout.lines();
        let real = |line: Option<&str>| line.and_then(|l| std::fs::canonicalize(repo.root.join(l)).ok());
        if real(lines.next()).as_deref() != Some(repo.root.as_path()) {
            return Err(failed("The workspace folder is not the top of a git repository."));
        }
        for _ in 0..2 {
            // A `.git` file can point anywhere on the computer: only a repository kept inside the workspace is used.
            if !real(lines.next()).is_some_and(|dir| dir.starts_with(&repo.root)) {
                return Err(failed(
                    "This repository keeps its git data outside the workspace (a worktree or a submodule), so ZORIX does not use it.",
                ));
            }
        }
        repo.check_config(cancel).await?;
        Ok(repo)
    }

    async fn check_config(&self, cancel: &CancellationToken) -> Result<(), ToolError> {
        let listed = self.ok(["config", "--show-scope", "--name-only", "--list", "-z"], cancel).await?;
        if listed.cut {
            return Err(failed("The repository's configuration is too long to check, so ZORIX does not use it."));
        }
        let mut fields = listed.stdout.split('\0');
        while let (Some(scope), Some(key)) = (fields.next(), fields.next()) {
            if matches!(scope, "local" | "worktree") && dangerous(key) {
                return Err(failed(format!(
                    "The repository's own configuration sets {key}, which can run a program or send git elsewhere, so ZORIX does not \
                     use this repository. Remove it from .git/config, or move it to your global git configuration."
                )));
            }
        }
        Ok(())
    }

    /// The branch checked out, or `HEAD` when none is.
    async fn branch(&self, cancel: &CancellationToken) -> Result<String, ToolError> {
        let found = self.run(["symbolic-ref", "--short", "-q", "HEAD"], None, cancel, LOCAL_TIMEOUT).await?;
        let name = found.stdout.trim();
        Ok(if found.ok && !name.is_empty() { name.to_string() } else { "HEAD".to_string() })
    }

    /// Every path that differs between the working folder (or the index) and `HEAD`, untracked ones included when asked.
    async fn changed(&self, staged: bool, cancel: &CancellationToken) -> Result<Vec<String>, ToolError> {
        let mut args = vec!["diff", "--no-ext-diff", "--no-textconv", "--name-only", "-z", "--no-renames"];
        if staged {
            args.push("--cached");
        }
        let has_head = self.run(["rev-parse", "-q", "--verify", "HEAD"], None, cancel, LOCAL_TIMEOUT).await?.ok;
        if has_head && !staged {
            args.push("HEAD");
        }
        let output = self.ok(args, cancel).await?;
        Ok(output.stdout.split('\0').filter(|p| !p.is_empty()).map(str::to_string).collect())
    }
}

fn require(grants: &Grants, action: Action, resource: String) -> Result<(), ToolError> {
    let needed = Permission::new(action, resource);
    if grants.allows(&needed) { Ok(()) } else { Err(failed(format!("This execution has no permission to {needed}."))) }
}

/// Whether the run may read the file at `path` (as git names it, from the top).
fn readable(grants: &Grants, path: &str) -> bool {
    RelativePath::parse(path).is_ok_and(|p| grants.allows(&Permission::new(Action::Read, p.resource())))
}

/// A branch or remote name a model gave: nothing git could take for an option or a refspec.
fn plain_name(what: &str, name: &str) -> Result<(), ToolError> {
    let ok = !name.is_empty()
        && name.len() <= 200
        && !name.starts_with(['-', '/', '.', '+'])
        && !name.ends_with(['/', '.'])
        && !name.contains("..")
        && !name.contains("@{")
        && !name.ends_with(".lock")
        && name.chars().all(|c| c.is_ascii_alphanumeric() || matches!(c, '/' | '-' | '_' | '.'));
    if ok { Ok(()) } else { Err(failed(format!("“{name}” is not a usable {what} name."))) }
}

fn paths_of(args: &Value) -> Result<Vec<RelativePath>, ToolError> {
    let list = args["paths"].as_array().cloned().unwrap_or_default();
    if list.len() > MAX_PATHS {
        return Err(failed(format!("Name at most {MAX_PATHS} paths.")));
    }
    list.iter()
        .map(|p| {
            let path = RelativePath::parse(p.as_str().unwrap_or(""))?;
            if path.is_root() || path.in_git_folder() {
                return Err(failed("Name files, not the whole folder or anything in .git."));
            }
            Ok(path)
        })
        .collect()
}

// ---------------------------------------------------------------------------------------------------------------------
// What each tool does

async fn status(repo: &Repo, grants: &Grants, cancel: &CancellationToken) -> Result<Value, ToolError> {
    let branch = repo.branch(cancel).await?;
    require(grants, Action::Read, format!("git:{branch}"))?;
    let output = repo.ok(["status", "--porcelain=v1", "-z", "--untracked-files=all", "--no-renames"], cancel).await?;
    let (mut entries, mut hidden) = (Vec::new(), 0u64);
    for entry in output.stdout.split('\0').filter(|e| e.len() > 3) {
        let (state, path) = entry.split_at(3);
        if !readable(grants, path) {
            hidden += 1;
        } else if entries.len() < MAX_ENTRIES {
            entries.push(json!({"state": state.trim(), "path": path}));
        }
    }
    let head = repo.run(["rev-parse", "-q", "--verify", "HEAD"], None, cancel, LOCAL_TIMEOUT).await?;
    Ok(json!({
        "branch": branch,
        "head": head.ok.then(|| head.stdout.trim().to_string()),
        "changes": entries,
        "not_shown": hidden,
        "more": output.cut || entries.len() == MAX_ENTRIES,
    }))
}

/// The text of what changed in `paths` (all that differs when empty), only files the run may read.
async fn diff_of(
    repo: &Repo,
    grants: &Grants,
    paths: &[String],
    staged: bool,
    cancel: &CancellationToken,
) -> Result<Value, ToolError> {
    let changed = repo.changed(staged, cancel).await?;
    let wanted: Vec<&String> = changed.iter().filter(|p| paths.is_empty() || paths.contains(p)).collect();
    let shown: Vec<&String> = wanted.iter().copied().filter(|p| readable(grants, p)).take(MAX_ENTRIES).collect();
    let hidden = wanted.len() - shown.len();
    if shown.is_empty() {
        return Ok(json!({"diff": "", "files": [], "not_shown": hidden}));
    }
    let mut args = vec!["diff", "--no-ext-diff", "--no-textconv", "--no-color", "--no-renames"];
    if staged {
        args.push("--cached");
    } else if repo.run(["rev-parse", "-q", "--verify", "HEAD"], None, cancel, LOCAL_TIMEOUT).await?.ok {
        args.push("HEAD");
    }
    args.push("--");
    args.extend(shown.iter().map(|p| p.as_str()));
    let output = repo.ok(args, cancel).await?;
    Ok(json!({"diff": output.stdout, "files": shown, "not_shown": hidden, "cut": output.cut}))
}

async fn log(repo: &Repo, grants: &Grants, limit: usize, cancel: &CancellationToken) -> Result<Value, ToolError> {
    let branch = repo.branch(cancel).await?;
    require(grants, Action::Read, format!("git:{branch}"))?;
    let count = format!("-n{}", limit.clamp(1, MAX_COMMITS));
    let output = repo
        .ok(["log", count.as_str(), "--no-color", "--no-show-signature", "--format=%H%x00%an%x00%aI%x00%B%x00"], cancel)
        .await?;
    let mut fields = output.stdout.split('\0');
    let mut commits = Vec::new();
    while let (Some(hash), Some(author), Some(date), Some(message)) =
        (fields.next(), fields.next(), fields.next(), fields.next())
    {
        let message: String = message.trim().chars().take(2000).collect();
        commits.push(json!({"commit": hash.trim(), "author": author, "date": date, "message": message}));
    }
    Ok(json!({"branch": branch, "commits": commits, "cut": output.cut}))
}

async fn switch(
    repo: &Repo,
    grants: &Grants,
    name: &str,
    create: bool,
    cancel: &CancellationToken,
) -> Result<Value, ToolError> {
    plain_name("branch", name)?;
    require(grants, Action::Write, format!("git:{name}"))?;
    if create {
        repo.ok(["switch", "--create", name], cancel).await?;
    } else {
        // Switching rewrites files: each one it would change needs the same grant as writing it.
        let target = format!("refs/heads/{name}");
        let output = repo
            .ok(
                [
                    "diff",
                    "--no-ext-diff",
                    "--no-textconv",
                    "--name-only",
                    "-z",
                    "--no-renames",
                    "HEAD",
                    target.as_str(),
                    "--",
                ],
                cancel,
            )
            .await?;
        for path in output.stdout.split('\0').filter(|p| !p.is_empty()) {
            require(grants, Action::Write, RelativePath::parse(path)?.resource())?;
        }
        if output.cut {
            return Err(failed("The branches differ in too many files to check them all."));
        }
        repo.ok(["switch", "--no-guess", name], cancel).await?;
    }
    Ok(json!({"branch": name, "created": create}))
}

async fn commit(
    repo: &Repo,
    grants: &Grants,
    message: &str,
    paths: &[RelativePath],
    cancel: &CancellationToken,
) -> Result<Value, ToolError> {
    let branch = repo.branch(cancel).await?;
    if branch == "HEAD" {
        return Err(failed("No branch is checked out; create or switch to one first."));
    }
    require(grants, Action::Write, format!("git:{branch}"))?;
    if message.trim().is_empty() {
        return Err(failed("A commit needs a message."));
    }
    if paths.is_empty() {
        return Err(failed("Name the files to commit."));
    }
    for path in paths {
        require(grants, Action::Write, path.resource())?;
    }
    let names: Vec<String> = paths.iter().map(RelativePath::display).collect();
    let mut add = vec!["add".to_string(), "--".to_string()];
    add.extend(names.iter().cloned());
    repo.ok(add, cancel).await?;
    let mut args = vec!["commit", "--no-verify", "--quiet", "--file=-", "--"];
    args.extend(names.iter().map(String::as_str));
    let output = repo.run(args, Some(message), cancel, LOCAL_TIMEOUT).await?;
    if !output.ok {
        return Err(failed(format!("Git did not commit: {} {}", output.stderr, output.stdout.trim())));
    }
    let head = repo.ok(["rev-parse", "HEAD"], cancel).await?;
    Ok(json!({"branch": branch, "commit": head.stdout.trim(), "files": names}))
}

/// Where a push to `remote` goes, as git would send it.
async fn push_url(repo: &Repo, remote: &str, cancel: &CancellationToken) -> Result<String, ToolError> {
    plain_name("remote", remote)?;
    let output = repo.run(["remote", "get-url", "--push", "--", remote], None, cancel, LOCAL_TIMEOUT).await?;
    if !output.ok {
        return Err(failed(format!("There is no remote called {remote}.")));
    }
    Ok(zorix_core::redact_text(output.stdout.trim()))
}

fn full_hash(text: &str) -> bool {
    matches!(text.len(), 40 | 64) && text.bytes().all(|b| b.is_ascii_hexdigit())
}

async fn push(
    repo: &Repo,
    grants: &Grants,
    remote: &str,
    branch: &str,
    commit: &str,
    cancel: &CancellationToken,
) -> Result<Value, ToolError> {
    plain_name("branch", branch)?;
    plain_name("remote", remote)?;
    require(grants, Action::Write, format!("git-push:{remote}/{branch}"))?;
    if !full_hash(commit) {
        return Err(failed("Name the commit with its full hash, as git.commit and git.log give it."));
    }
    let url = push_url(repo, remote, cancel).await?;
    // The approval was for this commit: if the branch has moved since, nothing is sent.
    let head =
        repo.run(["rev-parse", "-q", "--verify", &format!("refs/heads/{branch}")], None, cancel, LOCAL_TIMEOUT).await?;
    if head.stdout.trim() != commit {
        return Err(failed(format!("The branch {branch} is not at {commit} any more, so nothing was pushed.")));
    }
    // Never forced: a refspec without “+” and no force option, so a branch that moved elsewhere is refused by the server.
    let refspec = format!("{commit}:refs/heads/{branch}");
    let output =
        repo.run(["push", "--porcelain", "--no-verify", "--", remote, &refspec], None, cancel, PUSH_TIMEOUT).await?;
    if !output.ok {
        return Err(failed(format!("The push was refused: {} {}", output.stderr, output.stdout.trim())));
    }
    Ok(json!({"remote": remote, "url": url, "branch": branch, "commit": commit}))
}

/// What an approval of `git.commit` or `git.push` is about, for the person to read: where it goes and what changes.
pub async fn preview(root: &Path, grants: &Grants, tool: &str, args: &Value) -> Result<Value, ToolError> {
    let cancel = CancellationToken::new();
    let repo = Repo::open(Some(root), &cancel).await?;
    let text = |key: &str| args[key].as_str().unwrap_or_default().to_string();
    match tool {
        "git.commit" => {
            let names: Vec<String> = paths_of(args)?.iter().map(RelativePath::display).collect();
            let mut shown = diff_of(&repo, grants, &names, false, &cancel).await?;
            // Files git does not know yet are not in a diff: say so rather than leave them out silently.
            let known: BTreeSet<String> = shown["files"]
                .as_array()
                .map(|f| f.iter().filter_map(Value::as_str).map(str::to_string).collect())
                .unwrap_or_default();
            shown["new_files"] = json!(names.iter().filter(|n| !known.contains(*n)).collect::<Vec<_>>());
            shown["branch"] = json!(repo.branch(&cancel).await?);
            Ok(shown)
        }
        "git.push" => {
            let (remote, branch, commit) = (text("remote"), text("branch"), text("commit"));
            plain_name("branch", &branch)?;
            if !full_hash(&commit) {
                return Err(failed("The commit is not a full hash."));
            }
            let url = push_url(&repo, &remote, &cancel).await?;
            let tracking = format!("refs/remotes/{remote}/{branch}");
            let known = repo.run(["rev-parse", "-q", "--verify", &tracking], None, &cancel, LOCAL_TIMEOUT).await?.ok;
            let subject =
                repo.ok(["log", "-n1", "--no-show-signature", "--format=%s", commit.as_str(), "--"], &cancel).await?;
            let base = if known { tracking.clone() } else { format!("{commit}^") };
            let names = repo
                .run(
                    [
                        "diff",
                        "--no-ext-diff",
                        "--no-textconv",
                        "--name-only",
                        "-z",
                        "--no-renames",
                        &base,
                        &commit,
                        "--",
                    ],
                    None,
                    &cancel,
                    LOCAL_TIMEOUT,
                )
                .await?;
            let files: Vec<&str> =
                names.stdout.split('\0').filter(|p| !p.is_empty() && readable(grants, p)).take(MAX_ENTRIES).collect();
            let mut diff = String::new();
            if names.ok && !files.is_empty() {
                let mut show = vec![
                    "diff",
                    "--no-ext-diff",
                    "--no-textconv",
                    "--no-color",
                    "--no-renames",
                    base.as_str(),
                    commit.as_str(),
                    "--",
                ];
                show.extend(files.iter().copied());
                diff = repo.ok(show, &cancel).await?.stdout;
            }
            Ok(json!({
                "url": url,
                "branch": branch,
                "new_on_remote": !known,
                "commit": commit,
                "subject": subject.stdout.trim(),
                "files": files,
                "diff": diff,
            }))
        }
        _ => Err(failed("There is no preview for that tool.")),
    }
}

// ---------------------------------------------------------------------------------------------------------------------
// The tools

#[derive(Clone, Copy)]
enum Kind {
    Status,
    Diff,
    Log,
    Branch,
    Commit,
    Push,
}

struct GitTool {
    kind: Kind,
    spec: ToolSpec,
}

fn spec(kind: Kind) -> ToolSpec {
    let object = |properties: Value, required: &[&str]| json!({"type": "object", "properties": properties, "required": required, "additionalProperties": false});
    let paths = json!({"type": "array", "maxItems": MAX_PATHS, "items": {"type": "string", "maxLength": 500}});
    let name = json!({"type": "string", "minLength": 1, "maxLength": 200});
    let (id, title, description, schema) = match kind {
        Kind::Status => (
            "git.status",
            "Git status",
            "The branch checked out in the workspace and the files that changed.",
            object(json!({}), &[]),
        ),
        Kind::Diff => (
            "git.diff",
            "Git diff",
            "What changed since the last commit (or what is staged), for the files the run may read.",
            object(json!({"paths": paths, "staged": {"type": "boolean"}}), &[]),
        ),
        Kind::Log => (
            "git.log",
            "Git log",
            "The latest commits of the current branch: hash, author, date and message.",
            object(json!({"limit": {"type": "integer", "minimum": 1, "maximum": MAX_COMMITS}}), &[]),
        ),
        Kind::Branch => (
            "git.branch",
            "Git branch",
            "Create a branch from the current commit and switch to it, or switch to an existing one.",
            object(json!({"name": name, "create": {"type": "boolean"}}), &["name"]),
        ),
        Kind::Commit => (
            "git.commit",
            "Git commit",
            "Commit the named files on the current branch, with the person's git identity. Hooks do not run.",
            object(
                json!({"message": {"type": "string", "minLength": 1, "maxLength": 10000}, "paths": paths}),
                &["message", "paths"],
            ),
        ),
        Kind::Push => (
            "git.push",
            "Git push",
            "Send one commit of a branch to a configured remote. Never forces. Give the full commit hash.",
            object(
                json!({"remote": name, "branch": name, "commit": {"type": "string", "minLength": 40, "maxLength": 64}}),
                &["remote", "branch", "commit"],
            ),
        ),
    };
    let mut spec = ToolSpec::new(id, title, description, schema);
    match kind {
        Kind::Status | Kind::Diff | Kind::Log => spec.annotations = ToolAnnotations::safe_read(),
        Kind::Branch | Kind::Commit => {
            spec.annotations =
                ToolAnnotations { read_only: false, destructive: false, idempotent: false, open_world: false };
            spec.requires_approval = true;
        }
        Kind::Push => {
            spec.annotations =
                ToolAnnotations { read_only: false, destructive: true, idempotent: true, open_world: true };
            // Leaving the computer is decided by a person who saw it, whatever mode the run is in.
            spec.requires_approval = true;
            spec.approval_by_person_only = true;
            spec.timeout_ms = (PUSH_TIMEOUT.as_secs() + 10) * 1000;
        }
    }
    spec
}

/// The git tools.
pub fn git_tools() -> Vec<Arc<dyn Tool>> {
    [Kind::Status, Kind::Diff, Kind::Log, Kind::Branch, Kind::Commit, Kind::Push]
        .into_iter()
        .map(|kind| Arc::new(GitTool { kind, spec: spec(kind) }) as Arc<dyn Tool>)
        .collect()
}

#[async_trait]
impl Tool for GitTool {
    fn spec(&self) -> &ToolSpec {
        &self.spec
    }

    async fn call(&self, ctx: &ToolContext, args: Value) -> Result<Value, ToolError> {
        let repo = Repo::open(ctx.workspace_root.as_deref(), &ctx.cancel).await?;
        let (grants, cancel) = (&ctx.grants, &ctx.cancel);
        let text = |key: &str| args[key].as_str().unwrap_or_default().to_string();
        match self.kind {
            Kind::Status => status(&repo, grants, cancel).await,
            Kind::Diff => {
                require(grants, Action::Read, format!("git:{}", repo.branch(cancel).await?))?;
                let names: Vec<String> = paths_of(&args)?.iter().map(RelativePath::display).collect();
                diff_of(&repo, grants, &names, args["staged"].as_bool().unwrap_or(false), cancel).await
            }
            Kind::Log => log(&repo, grants, args["limit"].as_u64().unwrap_or(20) as usize, cancel).await,
            Kind::Branch => {
                switch(&repo, grants, &text("name"), args["create"].as_bool().unwrap_or(false), cancel).await
            }
            Kind::Commit => commit(&repo, grants, &text("message"), &paths_of(&args)?, cancel).await,
            Kind::Push => push(&repo, grants, &text("remote"), &text("branch"), &text("commit"), cancel).await,
        }
    }

    fn describe_call(&self, args: &Value) -> Option<String> {
        let text = |key: &str| args[key].as_str().unwrap_or_default().to_string();
        let short = |hash: &str| hash.chars().take(7).collect::<String>();
        Some(match self.kind {
            Kind::Push => format!("Push {} to {}/{}", short(&text("commit")), text("remote"), text("branch")),
            Kind::Commit => format!(
                "Commit {} file(s): {}",
                args["paths"].as_array().map_or(0, Vec::len),
                text("message").lines().next().unwrap_or("").chars().take(80).collect::<String>()
            ),
            Kind::Branch if args["create"].as_bool().unwrap_or(false) => format!("Create the branch {}", text("name")),
            Kind::Branch => format!("Switch to the branch {}", text("name")),
            _ => return None,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn what_can_run_or_redirect_is_dangerous_and_ordinary_settings_are_not() {
        for bad in [
            "filter.lfs.clean",
            "filter.x.process",
            "core.sshCommand",
            "core.hookspath",
            "diff.x.textconv",
            "diff.external",
            "merge.x.driver",
            "credential.helper",
            "credential.https://x.helper",
            "gpg.ssh.program",
            "include.path",
            "includeIf.gitdir:/x.path",
            "remote.origin.receivepack",
            "url.ssh://evil/.insteadOf",
            "protocol.file.allow",
            "core.worktree",
        ] {
            assert!(dangerous(bad), "{bad}");
        }
        for fine in [
            "user.name",
            "core.autocrlf",
            "remote.origin.url",
            "branch.main.remote",
            "filter.lfs.required",
            "push.default",
        ] {
            assert!(!dangerous(fine), "{fine}");
        }
    }

    #[test]
    fn windows_canonical_paths_are_written_the_way_git_takes_them() {
        assert_eq!(for_git(Path::new(r"\\?\C:\work")), Path::new(r"C:\work"));
        assert_eq!(for_git(Path::new(r"\\?\UNC\server\share")), Path::new(r"\\?\UNC\server\share"));
        assert_eq!(for_git(Path::new("/home/x")), Path::new("/home/x"));
    }

    #[test]
    fn names_that_git_could_read_as_options_or_refspecs_are_refused() {
        for bad in ["-f", "--force", "+main", "main:other", "a..b", "x@{1}", "a b", ".x", "x.lock", "x/", ""] {
            assert!(plain_name("branch", bad).is_err(), "{bad}");
        }
        for fine in ["main", "feat/git-tool", "v1.2_x"] {
            assert!(plain_name("branch", fine).is_ok(), "{fine}");
        }
    }
}
