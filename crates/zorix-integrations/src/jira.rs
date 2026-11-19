//! Jira tools (ADR-0021): search and read issues, and create, comment and move them.
//!
//! A project is the resource a person grants: `read` or `write` on `jira:<PROJECT>`. Every search is limited to the project by
//! ZORIX, whatever the query says. Every write asks. A connection holds the site's address, the account's email and an API token.

use std::sync::Arc;

use async_trait::async_trait;
use base64::Engine;
use reqwest::Method;
use serde_json::{Value, json};
use zorix_core::{
    Action, ConnectionKind, Permission, Tool, ToolAnnotations, ToolContext, ToolError, ToolSource, ToolSpec,
};

use crate::api::Api;
use crate::connections::{Services, clip, encode};

const BODY_CHARS: usize = 8_000;
const FIELDS: &[&str] =
    &["summary", "status", "assignee", "reporter", "priority", "issuetype", "labels", "created", "updated"];

fn failed(message: impl Into<String>) -> ToolError {
    ToolError::Failed(message.into())
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Op {
    Search,
    GetIssue,
    ListComments,
    ListTransitions,
    CreateIssue,
    Comment,
    Transition,
}

impl Op {
    const ALL: [Op; 7] =
        [Op::Search, Op::GetIssue, Op::ListComments, Op::ListTransitions, Op::CreateIssue, Op::Comment, Op::Transition];

    fn id(self) -> &'static str {
        match self {
            Op::Search => "search",
            Op::GetIssue => "get_issue",
            Op::ListComments => "list_comments",
            Op::ListTransitions => "list_transitions",
            Op::CreateIssue => "create_issue",
            Op::Comment => "comment",
            Op::Transition => "transition",
        }
    }

    fn writes(self) -> bool {
        matches!(self, Op::CreateIssue | Op::Comment | Op::Transition)
    }
}

/// A Jira tool.
pub struct JiraTool {
    services: Arc<Services>,
    op: Op,
    spec: ToolSpec,
}

/// The Jira tools.
pub fn jira_tools(services: Arc<Services>) -> Vec<Arc<dyn Tool>> {
    Op::ALL
        .into_iter()
        .map(|op| {
            let connection = json!({"type": "string", "maxLength": 32, "description": "Which Jira connection, when there is more than one."});
            let key = json!({"type": "string", "pattern": "^[A-Za-z][A-Za-z0-9_]*-[0-9]+$", "description": "The issue key, such as ABC-123."});
            let project = json!({"type": "string", "pattern": "^[A-Za-z][A-Za-z0-9_]{0,31}$", "description": "The project key, such as ABC."});
            let (describe, mut props, required): (&str, Vec<(&str, Value)>, Vec<&str>) = match op {
                Op::Search => (
                    "Search the issues of one project. `jql` is an optional filter that is combined with the project, so it can narrow but never widen the search.",
                    vec![
                        ("project", project),
                        ("jql", json!({"type": "string", "maxLength": 500, "description": "Optional JQL conditions, such as `status = \"In Progress\" ORDER BY updated DESC`."})),
                        ("max_results", json!({"type": "integer", "minimum": 1, "maximum": 50})),
                        ("next_page_token", json!({"type": "string", "maxLength": 500})),
                    ],
                    vec!["project"],
                ),
                Op::GetIssue => ("Read one issue: summary, status, people, description.", vec![("key", key)], vec!["key"]),
                Op::ListComments => ("Read the latest comments of an issue.", vec![("key", key)], vec!["key"]),
                Op::ListTransitions => ("List the moves (status changes) an issue can make now.", vec![("key", key)], vec!["key"]),
                Op::CreateIssue => (
                    "Create an issue. A person approves it first.",
                    vec![
                        ("project", project),
                        ("summary", json!({"type": "string", "minLength": 1, "maxLength": 255})),
                        ("description", json!({"type": "string", "maxLength": 20000})),
                        ("issue_type", json!({"type": "string", "maxLength": 60, "description": "The issue type's name (default Task)."})),
                        ("labels", json!({"type": "array", "items": {"type": "string", "pattern": "^[^ ]{1,255}$"}, "maxItems": 10})),
                    ],
                    vec!["project", "summary"],
                ),
                Op::Comment => ("Add a comment to an issue. A person approves it first.", vec![("key", key), ("body", json!({"type": "string", "minLength": 1, "maxLength": 20000}))], vec!["key", "body"]),
                Op::Transition => (
                    "Move an issue to another status, by the name of the move or of the status it leads to (see list_transitions). A person approves it first.",
                    vec![("key", key), ("to", json!({"type": "string", "minLength": 1, "maxLength": 100}))],
                    vec!["key", "to"],
                ),
            };
            props.push(("connection", connection));
            let properties: serde_json::Map<String, Value> = props.into_iter().map(|(k, v)| (k.to_string(), v)).collect();
            let mut spec = ToolSpec::new(
                format!("jira.{}", op.id()),
                format!("Jira {}", op.id().replace('_', " ")),
                format!("{describe} On Jira."),
                json!({"type": "object", "properties": properties, "required": required, "additionalProperties": false}),
            );
            spec.source = ToolSource::Integration { connection: None };
            spec.timeout_ms = 40_000;
            spec.max_output_bytes = 128 * 1024;
            spec.requires_approval = op.writes();
            spec.annotations = ToolAnnotations { read_only: !op.writes(), destructive: op == Op::Transition, idempotent: !op.writes(), open_world: true };
            Arc::new(JiraTool { services: services.clone(), op, spec }) as Arc<dyn Tool>
        })
        .collect()
}

/// Plain text as an Atlassian document: a paragraph for each blank-line-separated block, a line break for each newline.
pub fn adf(text: &str) -> Value {
    let paragraphs: Vec<Value> = text
        .split("\n\n")
        .filter(|p| !p.trim().is_empty())
        .map(|paragraph| {
            let mut content = Vec::new();
            for (i, line) in paragraph.split('\n').enumerate() {
                if i > 0 {
                    content.push(json!({"type": "hardBreak"}));
                }
                if !line.is_empty() {
                    content.push(json!({"type": "text", "text": line}));
                }
            }
            json!({"type": "paragraph", "content": content})
        })
        .collect();
    json!({"type": "doc", "version": 1, "content": if paragraphs.is_empty() { vec![json!({"type": "paragraph", "content": []})] } else { paragraphs }})
}

/// The text of an Atlassian document (or of plain text).
pub fn adf_text(node: &Value) -> String {
    fn walk(node: &Value, out: &mut String) {
        match node {
            Value::String(s) => out.push_str(s),
            Value::Object(map) => {
                match map.get("type").and_then(Value::as_str) {
                    Some("text") => out.push_str(map.get("text").and_then(Value::as_str).unwrap_or("")),
                    Some("hardBreak") => out.push('\n'),
                    Some("mention") => out.push_str(
                        map.get("attrs").and_then(|a| a.get("text")).and_then(Value::as_str).unwrap_or("@someone"),
                    ),
                    Some("emoji") => out.push_str(
                        map.get("attrs").and_then(|a| a.get("shortName")).and_then(Value::as_str).unwrap_or(""),
                    ),
                    Some("inlineCard") => {
                        out.push_str(map.get("attrs").and_then(|a| a.get("url")).and_then(Value::as_str).unwrap_or(""))
                    }
                    _ => {}
                }
                if let Some(children) = map.get("content").and_then(Value::as_array) {
                    for child in children {
                        walk(child, out);
                    }
                    if matches!(
                        map.get("type").and_then(Value::as_str),
                        Some("paragraph" | "heading" | "listItem" | "codeBlock" | "blockquote")
                    ) {
                        out.push('\n');
                    }
                }
            }
            _ => {}
        }
    }
    let mut out = String::new();
    walk(node, &mut out);
    out.trim().to_string()
}

/// The JQL that limits `user` to `project`. A trailing `ORDER BY` stays at the end; the project is ANDed with everything else, so
/// nothing in the user's part (an `OR`, another `project =`) can reach outside it.
pub fn scoped_jql(project: &str, user: &str) -> String {
    let user = user.trim();
    let lower = user.to_ascii_lowercase();
    let (condition, order) = match lower.rfind("order by") {
        Some(at) => (user[..at].trim(), Some(user[at..].trim())),
        None => (user, None),
    };
    let mut jql = format!("project = \"{project}\"");
    if !condition.is_empty() {
        jql.push_str(&format!(" AND ({condition})"));
    }
    if let Some(order) = order {
        jql.push(' ');
        jql.push_str(order);
    }
    jql
}

fn project_of_key(key: &str) -> Option<String> {
    let (project, number) = key.rsplit_once('-')?;
    (!project.is_empty() && number.chars().all(|c| c.is_ascii_digit()) && !number.is_empty())
        .then(|| project.to_ascii_uppercase())
}

fn person(v: &Value) -> Value {
    v.get("displayName").cloned().unwrap_or(Value::Null)
}

fn issue(v: &Value, with_description: bool) -> Value {
    let f = &v["fields"];
    let mut out = json!({
        "key": v["key"], "summary": f["summary"], "status": f["status"]["name"], "type": f["issuetype"]["name"], "priority": f["priority"]["name"],
        "assignee": person(&f["assignee"]), "reporter": person(&f["reporter"]),
        "labels": f["labels"], "created": f["created"], "updated": f["updated"],
    });
    if with_description {
        out["description"] = json!(clip(&adf_text(&f["description"]), BODY_CHARS));
    }
    out
}

impl JiraTool {
    async fn api(&self, ctx: &ToolContext, args: &Value, writing: bool) -> Result<Api, ToolError> {
        let name = args.get("connection").and_then(Value::as_str);
        let connection = self
            .services
            .pick(&ctx.tenant, ConnectionKind::Jira, name)
            .await?
            .ok_or_else(|| failed("There is no Jira connection. Add one with `zorix connect add jira <name>`."))?;
        let token = self.services.secret(&connection).await?;
        if token.is_none() {
            return Err(failed(format!(
                "The Jira connection {} has no token stored.{}",
                connection.name,
                if writing { " Writing needs one." } else { "" }
            )));
        }
        let token = token.unwrap_or_default();
        let base = connection.settings["base_url"]
            .as_str()
            .ok_or_else(|| failed("The Jira connection has no site address."))?;
        let authorization = match connection.settings["auth"].as_str().unwrap_or("basic") {
            "bearer" => format!("Bearer {token}"),
            _ => {
                let email = connection.settings["email"]
                    .as_str()
                    .ok_or_else(|| failed("The Jira connection has no account email."))?;
                format!("Basic {}", base64::engine::general_purpose::STANDARD.encode(format!("{email}:{token}")))
            }
        };
        Api::new(
            "Jira",
            base,
            vec![("Authorization".into(), authorization), ("Accept".into(), "application/json".into())],
        )
    }

    fn need(&self, ctx: &ToolContext, action: Action, project: &str) -> Result<(), ToolError> {
        let permission = Permission::new(action, format!("jira:{}", project.to_ascii_uppercase()));
        if ctx.grants.allows(&permission) {
            Ok(())
        } else {
            Err(failed(format!(
                "This execution has no permission to {permission}. A person can grant it with `--allow-jira {}{}`.",
                project.to_ascii_uppercase(),
                if action == Action::Write { ":write" } else { "" }
            )))
        }
    }

    async fn run(&self, ctx: &ToolContext, args: &Value) -> Result<Value, ToolError> {
        let action = if self.op.writes() { Action::Write } else { Action::Read };
        let text = |name: &str| args.get(name).and_then(Value::as_str).filter(|s| !s.is_empty());
        let project = match text("project") {
            Some(p) => p.to_ascii_uppercase(),
            None => project_of_key(text("key").ok_or_else(|| failed("key is missing."))?)
                .ok_or_else(|| failed("That is not an issue key (such as ABC-123)."))?,
        };
        self.need(ctx, action, &project)?;
        let api = self.api(ctx, args, self.op.writes()).await?;
        match self.op {
            Op::Search => {
                let mut body = json!({
                    "jql": scoped_jql(&project, text("jql").unwrap_or("")),
                    "maxResults": args.get("max_results").and_then(Value::as_u64).unwrap_or(20).clamp(1, 50),
                    "fields": FIELDS,
                });
                if let Some(token) = text("next_page_token") {
                    body["nextPageToken"] = json!(token);
                }
                let reply = api.send(Method::POST, "/rest/api/3/search/jql", None, Some(&body)).await?;
                api.check(&reply, "that search")?;
                let found = reply.json()?;
                let issues: Vec<Value> =
                    found["issues"].as_array().map(|a| a.iter().map(|v| issue(v, false)).collect()).unwrap_or_default();
                Ok(
                    json!({"issues": issues, "next_page_token": found.get("nextPageToken").cloned().unwrap_or(Value::Null)}),
                )
            }
            Op::GetIssue => {
                let key = text("key").unwrap_or("");
                let reply = api
                    .send(
                        Method::GET,
                        &format!("/rest/api/3/issue/{}?fields={}", encode(key), FIELDS.join(",") + ",description"),
                        None,
                        None,
                    )
                    .await?;
                api.check(&reply, "that issue")?;
                Ok(issue(&reply.json()?, true))
            }
            Op::ListComments => {
                let key = text("key").unwrap_or("");
                let reply = api
                    .send(
                        Method::GET,
                        &format!("/rest/api/3/issue/{}/comment?maxResults=20&orderBy=-created", encode(key)),
                        None,
                        None,
                    )
                    .await?;
                api.check(&reply, "those comments")?;
                let all = reply.json()?;
                let comments: Vec<Value> = all["comments"]
                    .as_array()
                    .map(|a| a.iter().map(|c| json!({"author": person(&c["author"]), "created": c["created"], "body": clip(&adf_text(&c["body"]), BODY_CHARS)})).collect())
                    .unwrap_or_default();
                Ok(json!({"comments": comments, "total": all["total"]}))
            }
            Op::ListTransitions => {
                let key = text("key").unwrap_or("");
                let reply = api
                    .send(Method::GET, &format!("/rest/api/3/issue/{}/transitions", encode(key)), None, None)
                    .await?;
                api.check(&reply, "those moves")?;
                let all = reply.json()?;
                let moves: Vec<Value> = all["transitions"]
                    .as_array()
                    .map(|a| {
                        a.iter().map(|t| json!({"id": t["id"], "name": t["name"], "to": t["to"]["name"]})).collect()
                    })
                    .unwrap_or_default();
                Ok(json!({"transitions": moves}))
            }
            Op::CreateIssue => {
                let mut fields = json!({
                    "project": {"key": project},
                    "summary": text("summary").ok_or_else(|| failed("summary is missing."))?,
                    "issuetype": {"name": text("issue_type").unwrap_or("Task")},
                });
                if let Some(description) = text("description") {
                    fields["description"] = adf(description);
                }
                if let Some(labels) = args.get("labels") {
                    fields["labels"] = labels.clone();
                }
                let reply = api.send(Method::POST, "/rest/api/3/issue", None, Some(&json!({"fields": fields}))).await?;
                api.check(&reply, "creating the issue")?;
                let created = reply.json()?;
                Ok(json!({"key": created["key"], "id": created["id"]}))
            }
            Op::Comment => {
                let key = text("key").unwrap_or("");
                let body = json!({"body": adf(text("body").ok_or_else(|| failed("body is missing."))?)});
                let reply = api
                    .send(Method::POST, &format!("/rest/api/3/issue/{}/comment", encode(key)), None, Some(&body))
                    .await?;
                api.check(&reply, "adding the comment")?;
                Ok(json!({"id": reply.json()?["id"], "key": key}))
            }
            Op::Transition => {
                let key = text("key").unwrap_or("");
                let want = text("to").ok_or_else(|| failed("to is missing."))?.to_ascii_lowercase();
                let reply = api
                    .send(Method::GET, &format!("/rest/api/3/issue/{}/transitions", encode(key)), None, None)
                    .await?;
                api.check(&reply, "those moves")?;
                let all = reply.json()?;
                let found = all["transitions"].as_array().and_then(|a| {
                    a.iter().find(|t| {
                        t["id"].as_str() == Some(want.as_str())
                            || t["name"].as_str().is_some_and(|n| n.to_ascii_lowercase() == want)
                            || t["to"]["name"].as_str().is_some_and(|n| n.to_ascii_lowercase() == want)
                    })
                });
                let Some(found) = found else {
                    let options: Vec<&str> = all["transitions"]
                        .as_array()
                        .map(|a| a.iter().filter_map(|t| t["name"].as_str()).collect())
                        .unwrap_or_default();
                    return Err(failed(format!("{key} cannot move to “{want}” now. It can: {}.", options.join(", "))));
                };
                let moved = api
                    .send(
                        Method::POST,
                        &format!("/rest/api/3/issue/{}/transitions", encode(key)),
                        None,
                        Some(&json!({"transition": {"id": found["id"]}})),
                    )
                    .await?;
                api.check(&moved, "moving the issue")?;
                Ok(json!({"key": key, "moved": found["name"], "status": found["to"]["name"]}))
            }
        }
    }
}

#[async_trait]
impl Tool for JiraTool {
    fn spec(&self) -> &ToolSpec {
        &self.spec
    }

    async fn call(&self, ctx: &ToolContext, args: Value) -> Result<Value, ToolError> {
        self.run(ctx, &args).await
    }

    fn describe_call(&self, args: &Value) -> Option<String> {
        let text = |n: &str| args.get(n).and_then(Value::as_str).unwrap_or("");
        Some(match self.op {
            Op::CreateIssue => format!(
                "Create a Jira issue “{}” in {}",
                clip(text("summary"), 80),
                text("project").to_ascii_uppercase()
            ),
            Op::Comment => format!("Comment on Jira issue {}", text("key")),
            Op::Transition => format!("Move Jira issue {} to “{}”", text("key"), text("to")),
            other => format!("{} on Jira", other.id().replace('_', " ")),
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_search_is_always_inside_its_project_whatever_the_user_part_says() {
        assert_eq!(scoped_jql("ABC", ""), "project = \"ABC\"");
        assert_eq!(scoped_jql("ABC", "status = Open"), "project = \"ABC\" AND (status = Open)");
        assert_eq!(
            scoped_jql("ABC", "status = Open ORDER BY updated DESC"),
            "project = \"ABC\" AND (status = Open) ORDER BY updated DESC"
        );
        assert_eq!(scoped_jql("ABC", "order by created"), "project = \"ABC\" order by created");
        // An OR or another project inside cannot leave the outer condition.
        assert_eq!(scoped_jql("ABC", "x = 1 OR project = OTHER"), "project = \"ABC\" AND (x = 1 OR project = OTHER)");
    }

    #[test]
    fn text_and_atlassian_documents_convert_both_ways() {
        let doc = adf("First line\nsecond line\n\nNew paragraph");
        assert_eq!(doc["content"].as_array().unwrap().len(), 2);
        assert_eq!(adf_text(&doc), "First line\nsecond line\nNew paragraph", "paragraphs end with a newline");
        let rich = json!({"type": "doc", "content": [
            {"type": "heading", "content": [{"type": "text", "text": "Title"}]},
            {"type": "paragraph", "content": [{"type": "text", "text": "Hi "}, {"type": "mention", "attrs": {"text": "@ana"}}]},
            {"type": "bulletList", "content": [{"type": "listItem", "content": [{"type": "paragraph", "content": [{"type": "text", "text": "one"}]}]}]}]});
        let text = adf_text(&rich);
        assert!(text.contains("Title") && text.contains("Hi @ana") && text.contains("one"), "{text}");
        assert_eq!(adf_text(&Value::Null), "");
        assert_eq!(adf_text(&json!("plain")), "plain");
    }

    #[test]
    fn the_project_comes_from_the_key() {
        assert_eq!(project_of_key("abc-12").as_deref(), Some("ABC"));
        assert_eq!(project_of_key("A_B-1").as_deref(), Some("A_B"));
        for bad in ["ABC", "ABC-", "-1", "ABC-x", ""] {
            assert!(project_of_key(bad).is_none(), "{bad}");
        }
    }
}
