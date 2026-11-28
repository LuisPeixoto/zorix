//! The git tools on throwaway repositories with a local bare remote: grants, no hooks, no repository-configured program, and a
//! push that never forces (ADR-0054).

use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::Arc;

use serde_json::{Value, json};
use tokio_util::sync::CancellationToken;
use zorix_core::{Action, ExecutionId, Grants, Permission, TenantId, Tool, ToolCallId, ToolContext, ToolError};
use zorix_tools::git::{git_tools, preview};

struct Rig {
    _dir: tempfile::TempDir,
    work: PathBuf,
    remote: PathBuf,
}

fn git(dir: &Path, args: &[&str]) -> String {
    let output = Command::new("git").args(args).current_dir(dir).output().expect("git is installed");
    assert!(output.status.success(), "git {args:?}: {}", String::from_utf8_lossy(&output.stderr));
    String::from_utf8_lossy(&output.stdout).trim().to_string()
}

fn write(path: &Path, text: &str) {
    std::fs::create_dir_all(path.parent().unwrap()).unwrap();
    std::fs::write(path, text).unwrap();
}

fn identity(dir: &Path) {
    for (key, value) in [("user.name", "Ada"), ("user.email", "ada@example.com"), ("commit.gpgsign", "false")] {
        git(dir, &["config", key, value]);
    }
}

fn rig() -> Rig {
    let dir = tempfile::tempdir().unwrap();
    let remote = dir.path().join("remote.git");
    let work = dir.path().join("work");
    std::fs::create_dir_all(&remote).unwrap();
    std::fs::create_dir_all(&work).unwrap();
    git(&remote, &["init", "--bare", "--quiet", "--initial-branch=main"]);
    git(&work, &["init", "--quiet", "--initial-branch=main"]);
    identity(&work);
    write(&work.join("src/lib.rs"), "one\n");
    write(&work.join("secrets.env"), "TOKEN=hidden-value\n");
    git(&work, &["add", "."]);
    git(&work, &["commit", "--quiet", "-m", "Start"]);
    git(&work, &["remote", "add", "origin", remote.to_str().unwrap()]);
    git(&work, &["push", "--quiet", "origin", "main"]);
    Rig { _dir: dir, work, remote }
}

fn grant(entries: &[(Action, &str)]) -> Grants {
    entries.iter().fold(Grants::none(), |g, (action, resource)| g.with(Permission::new(*action, *resource)))
}

fn everything() -> Grants {
    grant(&[
        (Action::Read, "git:*"),
        (Action::Write, "git:*"),
        (Action::Write, "git-push:origin/*"),
        (Action::Read, "workspace:*"),
        (Action::Write, "workspace:*"),
    ])
}

fn tool(id: &str) -> Arc<dyn Tool> {
    git_tools().into_iter().find(|t| t.spec().id == id).unwrap()
}

async fn call(rig: &Rig, id: &str, grants: Grants, args: Value) -> Result<Value, String> {
    let ctx = ToolContext {
        execution_id: ExecutionId::new(),
        tenant: TenantId::local(),
        call_id: ToolCallId("c".into()),
        grants,
        cancel: CancellationToken::new(),
        workspace_root: Some(rig.work.clone()),
        conversation: None,
        agent: None,
        host: None,
    };
    tool(id).call(&ctx, args).await.map_err(|e| match e {
        ToolError::Failed(m) | ToolError::Retryable(m) => m,
    })
}

#[tokio::test]
async fn a_run_sees_only_the_files_it_may_read() {
    let rig = rig();
    write(&rig.work.join("src/lib.rs"), "two\n");
    write(&rig.work.join("secrets.env"), "TOKEN=changed-value\n");
    let code = grant(&[(Action::Read, "git:main"), (Action::Read, "workspace:src/*")]);
    let status = call(&rig, "git.status", code.clone(), json!({})).await.unwrap();
    assert_eq!((status["branch"].as_str(), status["not_shown"].as_u64()), (Some("main"), Some(1)), "{status}");
    assert_eq!(status["changes"], json!([{"state": "M", "path": "src/lib.rs"}]));
    let diff = call(&rig, "git.diff", code.clone(), json!({})).await.unwrap();
    let text = diff["diff"].as_str().unwrap();
    assert!(text.contains("+two") && !text.contains("TOKEN") && !text.contains("changed-value"), "{text}");
    let asked = call(&rig, "git.diff", code.clone(), json!({"paths": ["secrets.env"]})).await.unwrap();
    assert_eq!((asked["diff"].as_str(), asked["not_shown"].as_u64()), (Some(""), Some(1)));
    let log = call(&rig, "git.log", code, json!({})).await.unwrap();
    assert_eq!(log["commits"][0]["message"], "Start");
    assert!(call(&rig, "git.log", Grants::none(), json!({})).await.unwrap_err().contains("read git:main"));
}

#[tokio::test]
async fn a_commit_takes_only_the_named_files_it_may_write_and_runs_no_hook() {
    let rig = rig();
    let marker = rig.work.join("hook-ran");
    write(&rig.work.join(".git/hooks/pre-commit"), &format!("#!/bin/sh\ntouch '{}'\n", marker.display()));
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(rig.work.join(".git/hooks/pre-commit"), std::fs::Permissions::from_mode(0o755))
            .unwrap();
    }
    write(&rig.work.join("src/lib.rs"), "two\n");
    write(&rig.work.join("src/new.rs"), "fresh\n");
    write(&rig.work.join("secrets.env"), "TOKEN=changed\n");
    let code = grant(&[(Action::Write, "git:main"), (Action::Write, "workspace:src/*")]);
    let args = |paths: &[&str]| json!({"message": "Change the code", "paths": paths});
    let refused = call(&rig, "git.commit", code.clone(), args(&["src/lib.rs", "secrets.env"])).await.unwrap_err();
    assert!(refused.contains("write workspace:secrets.env"), "{refused}");
    for bad in [".git/config", "../outside", "-A", ":(top)secrets.env"] {
        assert!(call(&rig, "git.commit", everything(), args(&[bad])).await.is_err(), "{bad}");
    }
    let done = call(&rig, "git.commit", code, args(&["src/lib.rs", "src/new.rs"])).await.unwrap();
    assert_eq!(done["commit"].as_str().unwrap(), git(&rig.work, &["rev-parse", "HEAD"]));
    assert_eq!(git(&rig.work, &["show", "--name-only", "--format=", "HEAD"]), "src/lib.rs\nsrc/new.rs");
    assert_eq!(git(&rig.work, &["status", "--porcelain"]).lines().count(), 1, "secrets.env is left as it was");
    assert!(!marker.exists(), "no hook ran");
}

#[tokio::test]
async fn a_push_sends_exactly_the_approved_commit_and_never_forces() {
    let rig = rig();
    call(&rig, "git.branch", everything(), json!({"name": "fix/limits", "create": true})).await.unwrap();
    write(&rig.work.join("src/lib.rs"), "limited\n");
    let first =
        call(&rig, "git.commit", everything(), json!({"message": "Limit", "paths": ["src/lib.rs"]})).await.unwrap();
    let first = first["commit"].as_str().unwrap().to_string();
    let push = |commit: &str| json!({"remote": "origin", "branch": "fix/limits", "commit": commit});

    let seen = preview(&rig.work, &everything(), "git.push", &push(&first)).await.unwrap();
    assert_eq!((seen["new_on_remote"].as_bool(), seen["subject"].as_str()), (Some(true), Some("Limit")));
    assert!(seen["diff"].as_str().unwrap().contains("+limited"), "{seen}");

    let no_grant = grant(&[(Action::Write, "git:*"), (Action::Write, "git-push:origin/main")]);
    assert!(call(&rig, "git.push", no_grant, push(&first)).await.unwrap_err().contains("git-push:origin/fix/limits"));
    call(&rig, "git.push", everything(), push(&first)).await.unwrap();
    assert_eq!(git(&rig.remote, &["rev-parse", "fix/limits"]), first);

    // The branch moved after the approval was given for the first commit: the old approval sends nothing.
    write(&rig.work.join("src/lib.rs"), "more\n");
    call(&rig, "git.commit", everything(), json!({"message": "More", "paths": ["src/lib.rs"]})).await.unwrap();
    assert!(call(&rig, "git.push", everything(), push(&first)).await.unwrap_err().contains("not at"));

    // Somebody else pushed meanwhile: the push is refused, never forced.
    let other = rig.work.parent().unwrap().join("other");
    git(
        rig.work.parent().unwrap(),
        &["clone", "--quiet", "--branch", "fix/limits", rig.remote.to_str().unwrap(), "other"],
    );
    identity(&other);
    write(&other.join("src/lib.rs"), "theirs\n");
    git(&other, &["commit", "--quiet", "-am", "Theirs"]);
    git(&other, &["push", "--quiet", "origin", "fix/limits"]);
    let theirs = git(&rig.remote, &["rev-parse", "fix/limits"]);
    let head = git(&rig.work, &["rev-parse", "HEAD"]);
    assert!(call(&rig, "git.push", everything(), push(&head)).await.unwrap_err().contains("refused"));
    assert_eq!(git(&rig.remote, &["rev-parse", "fix/limits"]), theirs, "their work is still there");

    for bad in [
        json!({"remote": "--all", "branch": "main", "commit": head}),
        json!({"remote": "origin", "branch": "+main", "commit": head}),
        json!({"remote": "origin", "branch": "main", "commit": "HEAD"}),
    ] {
        assert!(call(&rig, "git.push", everything(), bad.clone()).await.is_err(), "{bad}");
    }
    assert!(tool("git.push").spec().approval_by_person_only, "no automatic mode approves a push");
}

#[tokio::test]
async fn a_repository_that_could_run_something_is_not_used_and_nothing_runs() {
    let rig = rig();
    let marker = rig.work.parent().unwrap().join("ran");
    let command = format!("touch '{}'", marker.display());
    write(&rig.work.join(".gitattributes"), "*.rs filter=x\n");
    write(&rig.work.join("src/lib.rs"), "two\n");
    for (key, value) in [
        ("filter.x.clean", command.as_str()),
        ("core.sshCommand", command.as_str()),
        ("diff.external", command.as_str()),
    ] {
        git(&rig.work, &["config", key, value]);
        for (id, args) in [
            ("git.status", json!({})),
            ("git.diff", json!({})),
            ("git.commit", json!({"message": "m", "paths": ["src/lib.rs"]})),
        ] {
            let error = call(&rig, id, everything(), args).await.unwrap_err();
            assert!(error.to_lowercase().contains(&key.to_lowercase()), "{id} {key}: {error}");
        }
        git(&rig.work, &["config", "--unset", key]);
    }
    // The same through an included file.
    write(&rig.work.join(".git/extra"), &format!("[core]\n\tsshCommand = {command}\n"));
    git(&rig.work, &["config", "include.path", "extra"]);
    assert!(call(&rig, "git.status", everything(), json!({})).await.unwrap_err().contains("include.path"));
    assert!(!marker.exists(), "nothing the repository configured ran");
}

#[tokio::test]
async fn only_the_repository_kept_in_the_workspace_is_used() {
    let rig = rig();
    // A subfolder is not the repository's top.
    let inside = Rig { _dir: tempfile::tempdir().unwrap(), work: rig.work.join("src"), remote: rig.remote.clone() };
    assert!(
        call(&inside, "git.status", everything(), json!({})).await.unwrap_err().contains("top of a git repository")
    );
    // A `.git` file that points at another repository on the computer is refused.
    let folder = tempfile::tempdir().unwrap();
    let stolen = folder.path().join("stolen");
    std::fs::create_dir_all(&stolen).unwrap();
    write(&stolen.join(".git"), &format!("gitdir: {}\n", rig.work.join(".git").display()));
    let pointed = Rig { _dir: folder, work: stolen, remote: rig.remote.clone() };
    let error = call(&pointed, "git.log", everything(), json!({})).await.unwrap_err();
    assert!(error.contains("outside the workspace") || error.contains("top of a git repository"), "{error}");
}

#[tokio::test]
async fn switching_needs_the_grant_for_every_file_it_would_change() {
    let rig = rig();
    git(&rig.work, &["switch", "--quiet", "-c", "other"]);
    write(&rig.work.join("secrets.env"), "TOKEN=other\n");
    git(&rig.work, &["commit", "--quiet", "-am", "Other"]);
    git(&rig.work, &["switch", "--quiet", "main"]);
    let narrow = grant(&[(Action::Write, "git:other"), (Action::Write, "workspace:src/*")]);
    let refused = call(&rig, "git.branch", narrow, json!({"name": "other"})).await.unwrap_err();
    assert!(refused.contains("write workspace:secrets.env"), "{refused}");
    assert_eq!(git(&rig.work, &["branch", "--show-current"]), "main");
    call(&rig, "git.branch", everything(), json!({"name": "other"})).await.unwrap();
    assert_eq!(git(&rig.work, &["branch", "--show-current"]), "other");
}
