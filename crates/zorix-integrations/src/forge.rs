//! GitHub and GitLab tools (ADR-0021): read issues, pull requests and files, and write issues and comments.
//!
//! A repository is a resource a person grants: `read` or `write` on `github:<owner>/<repo>` (`gitlab:<group>/<project>`), with the
//! usual `*` and `prefix*` patterns. Reads need no approval; every write asks. The connection (a base address and a token) is
//! set up by a person; with none, GitHub's public API is read anonymously.

use std::sync::Arc;

use async_trait::async_trait;
use base64::Engine;
use reqwest::Method;
use serde_json::{Value, json};
use zorix_core::{
    Action, ConnectionKind, Permission, Tool, ToolAnnotations, ToolContext, ToolError, ToolSource, ToolSpec,
};

use crate::api::{Api, Reply};
use crate::connections::{Services, clip, encode, query};

const GITHUB_API: &str = "https://api.github.com";
const GITLAB_API: &str = "https://gitlab.com/api/v4";
/// The most text of an issue or comment body given to the model.
const BODY_CHARS: usize = 8_000;
/// The most bytes of a file given to the model.
const FILE_BYTES: usize = 200_000;
/// The most entries of a folder listed.
const LIST_MAX: usize = 200;

fn failed(message: impl Into<String>) -> ToolError {
    ToolError::Failed(message.into())
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Forge {
    Github,
    Gitlab,
}

impl Forge {
    fn name(self) -> &'static str {
        match self {
            Forge::Github => "github",
            Forge::Gitlab => "gitlab",
        }
    }

    fn kind(self) -> ConnectionKind {
        match self {
            Forge::Github => ConnectionKind::Github,
            Forge::Gitlab => ConnectionKind::Gitlab,
        }
    }

    fn label(self) -> &'static str {
        match self {
            Forge::Github => "GitHub",
            Forge::Gitlab => "GitLab",
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Op {
    SearchIssues,
    GetIssue,
    ListIssues,
    ListComments,
    GetPull,
    ListPulls,
    GetFile,
    CreateIssue,
    Comment,
    UpdateIssue,
}

impl Op {
    const ALL: [Op; 10] = [
        Op::SearchIssues,
        Op::GetIssue,
        Op::ListIssues,
        Op::ListComments,
        Op::GetPull,
        Op::ListPulls,
        Op::GetFile,
        Op::CreateIssue,
        Op::Comment,
        Op::UpdateIssue,
    ];

    fn id(self) -> &'static str {
        match self {
            Op::SearchIssues => "search_issues",
            Op::GetIssue => "get_issue",
            Op::ListIssues => "list_issues",
            Op::ListComments => "list_comments",
            Op::GetPull => "get_pull",
            Op::ListPulls => "list_pulls",
            Op::GetFile => "get_file",
            Op::CreateIssue => "create_issue",
            Op::Comment => "comment",
            Op::UpdateIssue => "update_issue",
        }
    }

    fn writes(self) -> bool {
        matches!(self, Op::CreateIssue | Op::Comment | Op::UpdateIssue)
    }
}

/// A tool of one forge.
pub struct ForgeTool {
    services: Arc<Services>,
    forge: Forge,
    op: Op,
    spec: ToolSpec,
}

fn schema(op: Op, forge: Forge) -> (String, Value) {
    let repo = json!({"type": "string", "minLength": 3, "maxLength": 200, "description": if forge == Forge::Github { "The repository as owner/name." } else { "The project path, such as group/project." }});
    let connection = json!({"type": "string", "maxLength": 32, "description": "Which connection to use, when there is more than one."});
    let number = json!({"type": "integer", "minimum": 1, "description": "The issue or pull request number."});
    let state = json!({"type": "string", "enum": ["open", "closed", "all"], "description": "Which ones to list (default open)."});
    let page = json!({"type": "integer", "minimum": 1, "description": "The page of results (default 1)."});
    let per = json!({"type": "integer", "minimum": 1, "maximum": 50, "description": "How many per page (default 20)."});
    let on = json!({"type": "string", "enum": ["issue", "pull"], "description": "Whether the number is an issue (default) or a pull/merge request."});
    let (describe, mut props, required): (String, Vec<(&str, Value)>, Vec<&str>) = match op {
        Op::SearchIssues => (
            "Search the issues and pull requests of one repository for some words.".into(),
            vec![
                ("query", json!({"type": "string", "minLength": 1, "maxLength": 256})),
                ("state", state),
                ("per_page", per),
                ("page", page),
            ],
            vec!["repo", "query"],
        ),
        Op::GetIssue => (
            "Read one issue: its title, state, labels and text.".into(),
            vec![("number", number)],
            vec!["repo", "number"],
        ),
        Op::ListIssues => (
            "List the issues of a repository.".into(),
            vec![
                ("state", state),
                ("labels", json!({"type": "string", "maxLength": 200, "description": "Comma-separated label names."})),
                ("per_page", per),
                ("page", page),
            ],
            vec!["repo"],
        ),
        Op::ListComments => (
            "Read the comments of an issue or pull request.".into(),
            vec![("number", number), ("on", on), ("per_page", per), ("page", page)],
            vec!["repo", "number"],
        ),
        Op::GetPull => (
            "Read one pull request (merge request): title, branches, state and text.".into(),
            vec![("number", number)],
            vec!["repo", "number"],
        ),
        Op::ListPulls => (
            "List the pull requests (merge requests) of a repository.".into(),
            vec![("state", state), ("per_page", per), ("page", page)],
            vec!["repo"],
        ),
        Op::GetFile => (
            "Read a file of a repository, or list a folder. Text only, up to 200 KB.".into(),
            vec![
                (
                    "path",
                    json!({"type": "string", "maxLength": 512, "description": "The path; empty or ending in / lists a folder."}),
                ),
                (
                    "ref",
                    json!({"type": "string", "maxLength": 200, "description": "A branch, tag or commit (default: the default branch)."}),
                ),
            ],
            vec!["repo", "path"],
        ),
        Op::CreateIssue => (
            "Open a new issue. A person approves it first.".into(),
            vec![
                ("title", json!({"type": "string", "minLength": 1, "maxLength": 256})),
                ("body", json!({"type": "string", "maxLength": 20000})),
                ("labels", json!({"type": "array", "items": {"type": "string", "maxLength": 50}, "maxItems": 10})),
            ],
            vec!["repo", "title"],
        ),
        Op::Comment => (
            "Add a comment to an issue or pull request. A person approves it first.".into(),
            vec![
                ("number", number),
                ("body", json!({"type": "string", "minLength": 1, "maxLength": 20000})),
                ("on", on),
            ],
            vec!["repo", "number", "body"],
        ),
        Op::UpdateIssue => (
            "Change an issue's state, title or text. A person approves it first.".into(),
            vec![
                ("number", number),
                ("state", json!({"type": "string", "enum": ["open", "closed"]})),
                ("title", json!({"type": "string", "minLength": 1, "maxLength": 256})),
                ("body", json!({"type": "string", "maxLength": 20000})),
            ],
            vec!["repo", "number"],
        ),
    };
    props.insert(0, ("repo", repo));
    props.push(("connection", connection));
    let properties: serde_json::Map<String, Value> = props.into_iter().map(|(k, v)| (k.to_string(), v)).collect();
    (describe, json!({"type": "object", "properties": properties, "required": required, "additionalProperties": false}))
}

/// The GitHub and GitLab tools.
pub fn forge_tools(services: Arc<Services>) -> Vec<Arc<dyn Tool>> {
    let mut tools: Vec<Arc<dyn Tool>> = Vec::new();
    for forge in [Forge::Github, Forge::Gitlab] {
        for op in Op::ALL {
            let (describe, input) = schema(op, forge);
            let mut spec = ToolSpec::new(
                format!("{}.{}", forge.name(), op.id()),
                format!("{} {}", forge.label(), op.id().replace('_', " ")),
                format!("{describe} On {}.", forge.label()),
                input,
            );
            spec.source = ToolSource::Integration { connection: None };
            spec.timeout_ms = 40_000;
            spec.max_output_bytes = 128 * 1024;
            spec.requires_approval = op.writes();
            spec.annotations = ToolAnnotations {
                read_only: !op.writes(),
                destructive: op == Op::UpdateIssue,
                idempotent: !op.writes(),
                open_world: true,
            };
            tools.push(Arc::new(ForgeTool { services: services.clone(), forge, op, spec }));
        }
    }
    tools
}

fn text<'a>(args: &'a Value, name: &str) -> Option<&'a str> {
    args.get(name).and_then(Value::as_str).filter(|s| !s.is_empty())
}

fn number(args: &Value) -> Result<u64, ToolError> {
    args.get("number").and_then(Value::as_u64).filter(|n| *n >= 1).ok_or_else(|| failed("number is missing."))
}

fn repo_of(forge: Forge, args: &Value) -> Result<String, ToolError> {
    let repo = text(args, "repo").ok_or_else(|| failed("repo is missing."))?;
    let parts: Vec<&str> = repo.split('/').collect();
    let part_ok = |p: &&str| {
        !p.is_empty()
            && *p != "."
            && *p != ".."
            && p.chars().all(|c| c.is_ascii_alphanumeric() || matches!(c, '-' | '_' | '.'))
    };
    let shape_ok = match forge {
        Forge::Github => parts.len() == 2,
        Forge::Gitlab => parts.len() >= 2,
    };
    if shape_ok && parts.iter().all(part_ok) {
        Ok(repo.to_string())
    } else {
        Err(failed(match forge {
            Forge::Github => "repo must look like owner/name.",
            Forge::Gitlab => "repo must look like group/project.",
        }))
    }
}

fn need(ctx: &ToolContext, forge: Forge, action: Action, repo: &str) -> Result<(), ToolError> {
    let permission = Permission::new(action, format!("{}:{repo}", forge.name()));
    if ctx.grants.allows(&permission) {
        Ok(())
    } else {
        Err(failed(format!(
            "This execution has no permission to {permission}. A person can grant it with `--allow-repo {repo}{}`.",
            if action == Action::Write { ":write" } else { "" }
        )))
    }
}

/// A search may not widen itself to other repositories with qualifiers of its own.
fn safe_query(query: &str) -> Result<(), ToolError> {
    let lower = query.to_ascii_lowercase();
    for qualifier in ["repo:", "org:", "user:", "owner:", "project:", "group:"] {
        if lower.contains(qualifier) {
            return Err(failed(format!("The search cannot contain {qualifier} : the repository is set by `repo`.")));
        }
    }
    Ok(())
}

fn has_next(forge: Forge, reply: &Reply) -> bool {
    match forge {
        Forge::Github => reply.header("link").is_some_and(|l| l.contains("rel=\"next\"")),
        Forge::Gitlab => reply.header("x-next-page").is_some_and(|p| !p.is_empty()),
    }
}

fn names(value: Option<&Value>, key: &str) -> Vec<String> {
    value
        .and_then(Value::as_array)
        .map(|a| {
            a.iter()
                .filter_map(|v| {
                    v.as_str().map(str::to_string).or_else(|| v.get(key).and_then(Value::as_str).map(str::to_string))
                })
                .collect()
        })
        .unwrap_or_default()
}

fn issue(forge: Forge, v: &Value) -> Value {
    match forge {
        Forge::Github => json!({
            "number": v["number"], "title": v["title"], "state": v["state"], "is_pull_request": v.get("pull_request").is_some(),
            "author": v["user"]["login"], "labels": names(v.get("labels"), "name"), "assignees": names(v.get("assignees"), "login"),
            "comments": v["comments"], "created_at": v["created_at"], "updated_at": v["updated_at"], "url": v["html_url"],
            "body": v["body"].as_str().map(|b| clip(b, BODY_CHARS)),
        }),
        Forge::Gitlab => json!({
            "number": v["iid"], "title": v["title"], "state": if v["state"] == "opened" { json!("open") } else { v["state"].clone() }, "is_pull_request": false,
            "author": v["author"]["username"], "labels": names(v.get("labels"), "name"), "assignees": names(v.get("assignees"), "username"),
            "comments": v["user_notes_count"], "created_at": v["created_at"], "updated_at": v["updated_at"], "url": v["web_url"],
            "body": v["description"].as_str().map(|b| clip(b, BODY_CHARS)),
        }),
    }
}

fn pull(forge: Forge, v: &Value) -> Value {
    match forge {
        Forge::Github => json!({
            "number": v["number"], "title": v["title"], "state": v["state"], "draft": v["draft"], "merged": v["merged"],
            "author": v["user"]["login"], "source_branch": v["head"]["ref"], "target_branch": v["base"]["ref"], "url": v["html_url"],
            "changed_files": v["changed_files"], "additions": v["additions"], "deletions": v["deletions"],
            "body": v["body"].as_str().map(|b| clip(b, BODY_CHARS)),
        }),
        Forge::Gitlab => json!({
            "number": v["iid"], "title": v["title"], "state": if v["state"] == "opened" { json!("open") } else { v["state"].clone() },
            "draft": v["draft"], "merged": v["state"] == "merged", "author": v["author"]["username"],
            "source_branch": v["source_branch"], "target_branch": v["target_branch"], "url": v["web_url"], "changes": v["changes_count"],
            "body": v["description"].as_str().map(|b| clip(b, BODY_CHARS)),
        }),
    }
}

fn comment(forge: Forge, v: &Value) -> Value {
    let author = if forge == Forge::Github { &v["user"]["login"] } else { &v["author"]["username"] };
    json!({"author": author, "created_at": v["created_at"], "body": v["body"].as_str().map(|b| clip(b, BODY_CHARS))})
}

impl ForgeTool {
    async fn api(&self, ctx: &ToolContext, args: &Value, writing: bool) -> Result<Api, ToolError> {
        let connection = self.services.pick(&ctx.tenant, self.forge.kind(), text(args, "connection")).await?;
        let token = match &connection {
            Some(c) => self.services.secret(c).await?,
            None => None,
        };
        if writing && token.is_none() {
            return Err(failed(format!(
                "Writing to {} needs a connection with a token. Add one with `zorix connect add {} <name>`.",
                self.forge.label(),
                self.forge.name()
            )));
        }
        let configured = connection.as_ref().and_then(|c| c.settings["base_url"].as_str()).map(str::to_string);
        let (base, mut headers) = match self.forge {
            Forge::Github => (
                configured.unwrap_or_else(|| GITHUB_API.to_string()),
                vec![
                    ("Accept".to_string(), "application/vnd.github+json".to_string()),
                    ("X-GitHub-Api-Version".to_string(), "2022-11-28".to_string()),
                ],
            ),
            Forge::Gitlab => {
                if connection.is_none() {
                    return Err(failed(
                        "There is no GitLab connection. Add one with `zorix connect add gitlab <name>`.",
                    ));
                }
                let mut base = configured.unwrap_or_else(|| GITLAB_API.to_string());
                if !base.trim_end_matches('/').ends_with("/api/v4") {
                    base = format!("{}/api/v4", base.trim_end_matches('/'));
                }
                (base, Vec::new())
            }
        };
        if let Some(token) = token {
            match self.forge {
                Forge::Github => headers.push(("Authorization".into(), format!("Bearer {token}"))),
                // A token from signing in is a bearer token; GitLab takes it only that way.
                Forge::Gitlab if connection.as_ref().is_some_and(crate::connections::signed_in) => {
                    headers.push(("Authorization".into(), format!("Bearer {token}")));
                }
                Forge::Gitlab => headers.push(("PRIVATE-TOKEN".into(), token)),
            }
        }
        Api::new(self.forge.label(), &base, headers)
    }

    fn project(&self, repo: &str) -> String {
        match self.forge {
            Forge::Github => format!("/repos/{repo}"),
            Forge::Gitlab => format!("/projects/{}", encode(repo)),
        }
    }

    fn page(&self, args: &Value) -> [(&'static str, Option<String>); 2] {
        let per = args.get("per_page").and_then(Value::as_u64).unwrap_or(20).clamp(1, 50);
        let page = args.get("page").and_then(Value::as_u64).unwrap_or(1).max(1);
        [("per_page", Some(per.to_string())), ("page", Some(page.to_string()))]
    }

    fn state(&self, args: &Value, pulls: bool) -> Option<String> {
        let state = text(args, "state").unwrap_or("open");
        Some(match (self.forge, state, pulls) {
            (Forge::Github, s, _) => s.to_string(),
            (Forge::Gitlab, "open", _) => "opened".into(),
            (Forge::Gitlab, "closed", _) => "closed".into(),
            (Forge::Gitlab, _, _) => "all".into(),
        })
    }

    async fn listing(
        &self,
        api: &Api,
        path: &str,
        args: &Value,
        map: impl Fn(&Value) -> Value,
        key: &str,
    ) -> Result<Value, ToolError> {
        let reply = api.send(Method::GET, path, None, None).await?;
        api.check(&reply, "that list")?;
        let items = reply.json()?;
        let rows: Vec<Value> = items.as_array().map(|a| a.iter().map(&map).collect()).unwrap_or_default();
        Ok(
            json!({key: rows, "page": args.get("page").and_then(Value::as_u64).unwrap_or(1), "has_more": has_next(self.forge, &reply)}),
        )
    }

    async fn run(&self, ctx: &ToolContext, args: &Value) -> Result<Value, ToolError> {
        let forge = self.forge;
        let repo = repo_of(forge, args)?;
        need(ctx, forge, if self.op.writes() { Action::Write } else { Action::Read }, &repo)?;
        let api = self.api(ctx, args, self.op.writes()).await?;
        let base = self.project(&repo);
        match self.op {
            Op::SearchIssues => {
                let words = text(args, "query").ok_or_else(|| failed("query is missing."))?;
                safe_query(words)?;
                let path = match forge {
                    Forge::Github => {
                        let state = text(args, "state")
                            .filter(|s| *s != "all")
                            .map(|s| format!(" state:{s}"))
                            .unwrap_or_else(|| {
                                if text(args, "state") == Some("all") { String::new() } else { " state:open".into() }
                            });
                        let [per, page] = self.page(args);
                        format!(
                            "/search/issues{}",
                            query(&[("q", Some(format!("{words} repo:{repo}{state}"))), per, page])
                        )
                    }
                    Forge::Gitlab => {
                        let [per, page] = self.page(args);
                        format!(
                            "{base}/issues{}",
                            query(&[
                                ("search", Some(words.to_string())),
                                ("state", self.state(args, false)),
                                per,
                                page
                            ])
                        )
                    }
                };
                let reply = api.send(Method::GET, &path, None, None).await?;
                api.check(&reply, "that search")?;
                let found = reply.json()?;
                let items = if forge == Forge::Github { found["items"].clone() } else { found };
                let rows: Vec<Value> =
                    items.as_array().map(|a| a.iter().map(|v| issue(forge, v)).collect()).unwrap_or_default();
                Ok(
                    json!({"issues": rows, "page": args.get("page").and_then(Value::as_u64).unwrap_or(1), "has_more": has_next(forge, &reply)}),
                )
            }
            Op::GetIssue | Op::GetPull => {
                let n = number(args)?;
                let (path, what) = match (forge, self.op) {
                    (Forge::Github, Op::GetIssue) => (format!("{base}/issues/{n}"), "that issue"),
                    (Forge::Github, _) => (format!("{base}/pulls/{n}"), "that pull request"),
                    (Forge::Gitlab, Op::GetIssue) => (format!("{base}/issues/{n}"), "that issue"),
                    (Forge::Gitlab, _) => (format!("{base}/merge_requests/{n}"), "that merge request"),
                };
                let reply = api.send(Method::GET, &path, None, None).await?;
                api.check(&reply, what)?;
                let v = reply.json()?;
                Ok(if self.op == Op::GetIssue { issue(forge, &v) } else { pull(forge, &v) })
            }
            Op::ListIssues => {
                let [per, page] = self.page(args);
                let labels = text(args, "labels").map(str::to_string);
                let path = match forge {
                    Forge::Github => format!(
                        "{base}/issues{}",
                        query(&[("state", self.state(args, false)), ("labels", labels), per, page])
                    ),
                    Forge::Gitlab => format!(
                        "{base}/issues{}",
                        query(&[("state", self.state(args, false)), ("labels", labels), per, page])
                    ),
                };
                self.listing(&api, &path, args, |v| issue(forge, v), "issues").await
            }
            Op::ListPulls => {
                let [per, page] = self.page(args);
                let path = match forge {
                    Forge::Github => format!("{base}/pulls{}", query(&[("state", self.state(args, true)), per, page])),
                    Forge::Gitlab => format!(
                        "{base}/merge_requests{}",
                        query(&[
                            (
                                "state",
                                self.state(args, true).map(|s| if s == "open" { "opened".to_string() } else { s })
                            ),
                            per,
                            page
                        ])
                    ),
                };
                self.listing(&api, &path, args, |v| pull(forge, v), "pulls").await
            }
            Op::ListComments => {
                let n = number(args)?;
                let [per, page] = self.page(args);
                let on_pull = text(args, "on") == Some("pull");
                let path = match (forge, on_pull) {
                    (Forge::Github, _) => format!("{base}/issues/{n}/comments{}", query(&[per, page])),
                    (Forge::Gitlab, false) => {
                        format!("{base}/issues/{n}/notes{}", query(&[("sort", Some("asc".into())), per, page]))
                    }
                    (Forge::Gitlab, true) => {
                        format!("{base}/merge_requests/{n}/notes{}", query(&[("sort", Some("asc".into())), per, page]))
                    }
                };
                let reply = api.send(Method::GET, &path, None, None).await?;
                api.check(&reply, "those comments")?;
                let all = reply.json()?;
                // GitLab mixes system messages ("changed the title") in with comments.
                let rows: Vec<Value> = all
                    .as_array()
                    .map(|a| a.iter().filter(|v| v["system"] != true).map(|v| comment(forge, v)).collect())
                    .unwrap_or_default();
                Ok(
                    json!({"comments": rows, "page": args.get("page").and_then(Value::as_u64).unwrap_or(1), "has_more": has_next(forge, &reply)}),
                )
            }
            Op::GetFile => self.file(&api, &base, args).await,
            Op::CreateIssue => {
                let title = text(args, "title").ok_or_else(|| failed("title is missing."))?;
                let labels: Vec<String> = args
                    .get("labels")
                    .and_then(Value::as_array)
                    .map(|a| a.iter().filter_map(|v| v.as_str().map(str::to_string)).collect())
                    .unwrap_or_default();
                let (path, body) = match forge {
                    Forge::Github => {
                        (format!("{base}/issues"), json!({"title": title, "body": args.get("body"), "labels": labels}))
                    }
                    Forge::Gitlab => (
                        format!("{base}/issues"),
                        json!({"title": title, "description": args.get("body"), "labels": labels.join(",")}),
                    ),
                };
                let reply = api.send(Method::POST, &path, None, Some(&body)).await?;
                api.check(&reply, "creating the issue")?;
                Ok(issue(forge, &reply.json()?))
            }
            Op::Comment => {
                let n = number(args)?;
                let body = text(args, "body").ok_or_else(|| failed("body is missing."))?;
                let on_pull = text(args, "on") == Some("pull");
                let path = match (forge, on_pull) {
                    (Forge::Github, _) => format!("{base}/issues/{n}/comments"),
                    (Forge::Gitlab, false) => format!("{base}/issues/{n}/notes"),
                    (Forge::Gitlab, true) => format!("{base}/merge_requests/{n}/notes"),
                };
                let reply = api.send(Method::POST, &path, None, Some(&json!({"body": body}))).await?;
                api.check(&reply, "adding the comment")?;
                let v = reply.json()?;
                Ok(
                    json!({"id": v["id"], "url": v.get("html_url").cloned().unwrap_or(Value::Null), "created_at": v["created_at"]}),
                )
            }
            Op::UpdateIssue => {
                let n = number(args)?;
                let mut body = serde_json::Map::new();
                let (title_key, body_key) =
                    if forge == Forge::Github { ("title", "body") } else { ("title", "description") };
                if let Some(t) = text(args, "title") {
                    body.insert(title_key.into(), json!(t));
                }
                if let Some(b) = args.get("body").and_then(Value::as_str) {
                    body.insert(body_key.into(), json!(b));
                }
                if let Some(state) = text(args, "state") {
                    match forge {
                        Forge::Github => body.insert("state".into(), json!(state)),
                        Forge::Gitlab => {
                            body.insert("state_event".into(), json!(if state == "closed" { "close" } else { "reopen" }))
                        }
                    };
                }
                if body.is_empty() {
                    return Err(failed("Say what to change: state, title or body."));
                }
                let method = if forge == Forge::Github { Method::PATCH } else { Method::PUT };
                let reply = api.send(method, &format!("{base}/issues/{n}"), None, Some(&Value::Object(body))).await?;
                api.check(&reply, "updating the issue")?;
                Ok(issue(forge, &reply.json()?))
            }
        }
    }

    async fn file(&self, api: &Api, base: &str, args: &Value) -> Result<Value, ToolError> {
        let path = args.get("path").and_then(Value::as_str).unwrap_or("").trim_matches('/').to_string();
        if path.split('/').any(|p| p == "..") {
            return Err(failed("The path may not contain `..`."));
        }
        let reference = text(args, "ref").map(str::to_string);
        let folder = path.is_empty() || args.get("path").and_then(Value::as_str).is_some_and(|p| p.ends_with('/'));
        let (url, accept) = match self.forge {
            Forge::Github => (
                format!(
                    "{base}/contents/{}{}",
                    path.split('/').map(encode).collect::<Vec<_>>().join("/"),
                    query(&[("ref", reference.clone())])
                ),
                None,
            ),
            Forge::Gitlab if folder => (
                format!(
                    "{base}/repository/tree{}",
                    query(&[
                        ("path", Some(path.clone())),
                        ("ref", reference.clone()),
                        ("per_page", Some("100".into()))
                    ])
                ),
                None,
            ),
            Forge::Gitlab => (
                format!(
                    "{base}/repository/files/{}{}",
                    encode(&path),
                    query(&[("ref", Some(reference.clone().unwrap_or_else(|| "HEAD".into())))])
                ),
                None::<&str>,
            ),
        };
        let reply = api.send(Method::GET, &url, accept, None).await?;
        api.check(&reply, &format!("“{path}”"))?;
        let v = reply.json()?;
        if let Some(entries) = v.as_array() {
            let rows: Vec<Value> = entries
                .iter()
                .take(LIST_MAX)
                .map(|e| json!({"name": e["name"], "path": e["path"], "type": match e["type"].as_str() { Some("tree") => "dir", Some("blob") => "file", Some(other) => other, None => "unknown" }, "size": e.get("size").cloned().unwrap_or(Value::Null)}))
                .collect();
            return Ok(json!({"path": path, "type": "dir", "entries": rows, "truncated": entries.len() > LIST_MAX}));
        }
        let encoded = v["content"].as_str().ok_or_else(|| failed("That is not a file ZORIX can read."))?;
        let bytes = base64::engine::general_purpose::STANDARD
            .decode(encoded.replace(['\n', '\r'], ""))
            .map_err(|_| failed("The file's content could not be decoded."))?;
        if bytes.contains(&0) {
            return Ok(
                json!({"path": path, "type": "file", "size": bytes.len(), "content": null, "note": "A binary file; its content is not shown."}),
            );
        }
        let truncated = bytes.len() > FILE_BYTES;
        let kept = &bytes[..bytes.len().min(FILE_BYTES)];
        Ok(
            json!({"path": path, "type": "file", "size": bytes.len(), "content": String::from_utf8_lossy(kept), "truncated": truncated}),
        )
    }
}

#[async_trait]
impl Tool for ForgeTool {
    fn spec(&self) -> &ToolSpec {
        &self.spec
    }

    async fn call(&self, ctx: &ToolContext, args: Value) -> Result<Value, ToolError> {
        self.run(ctx, &args).await
    }

    fn describe_call(&self, args: &Value) -> Option<String> {
        let repo = text(args, "repo")?;
        let on = self.forge.label();
        Some(match self.op {
            Op::CreateIssue => {
                format!("Open an issue “{}” in {repo} on {on}", clip(text(args, "title").unwrap_or(""), 80))
            }
            Op::Comment => {
                format!("Comment on #{} in {repo} on {on}", args.get("number").and_then(Value::as_u64).unwrap_or(0))
            }
            Op::UpdateIssue => {
                let what = match text(args, "state") {
                    Some("closed") => "Close",
                    Some(_) => "Reopen",
                    None => "Edit",
                };
                format!("{what} #{} in {repo} on {on}", args.get("number").and_then(Value::as_u64).unwrap_or(0))
            }
            other => format!("{} in {repo} on {on}", other.id().replace('_', " ")),
        })
    }
}
