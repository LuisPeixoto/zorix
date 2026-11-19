//! Slack tools (ADR-0021): see the channels a run may read, read them, and post. A channel is the resource a person grants:
//! `read` or `write` on `slack:<channel id>` or `slack:<channel name>`. Posting always asks and cannot ping everyone.

use std::sync::Arc;

use async_trait::async_trait;
use reqwest::Method;
use serde_json::{Value, json};
use zorix_core::{
    Action, ConnectionKind, Permission, Tool, ToolAnnotations, ToolContext, ToolError, ToolSource, ToolSpec,
};

use crate::api::{Api, Reply};
use crate::connections::{Services, clip, query};

const DEFAULT_API: &str = "https://slack.com/api";
const TEXT_CHARS: usize = 4_000;

fn failed(message: impl Into<String>) -> ToolError {
    ToolError::Failed(message.into())
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Op {
    ListChannels,
    ReadChannel,
    ReadThread,
    SendMessage,
}

impl Op {
    fn id(self) -> &'static str {
        match self {
            Op::ListChannels => "list_channels",
            Op::ReadChannel => "read_channel",
            Op::ReadThread => "read_thread",
            Op::SendMessage => "send_message",
        }
    }
}

/// A Slack tool.
pub struct SlackTool {
    services: Arc<Services>,
    op: Op,
    spec: ToolSpec,
}

/// The Slack tools.
pub fn slack_tools(services: Arc<Services>) -> Vec<Arc<dyn Tool>> {
    let channel = json!({"type": "string", "minLength": 1, "maxLength": 80, "description": "The channel id (C0123…) or its name (#general)."});
    let connection = json!({"type": "string", "maxLength": 32, "description": "Which Slack connection, when there is more than one."});
    let limit =
        json!({"type": "integer", "minimum": 1, "maximum": 50, "description": "How many messages (default 20)."});
    [Op::ListChannels, Op::ReadChannel, Op::ReadThread, Op::SendMessage]
        .into_iter()
        .map(|op| {
            let (describe, props, required): (&str, Vec<(&str, Value)>, Vec<&str>) = match op {
                Op::ListChannels => ("List the channels this execution may read.", vec![], vec![]),
                Op::ReadChannel => ("Read the latest messages of a channel.", vec![("channel", channel.clone()), ("limit", limit.clone())], vec!["channel"]),
                Op::ReadThread => ("Read the replies of a thread.", vec![("channel", channel.clone()), ("thread_ts", json!({"type": "string", "pattern": "^[0-9]+\\.[0-9]+$", "description": "The thread's timestamp."})), ("limit", limit.clone())], vec!["channel", "thread_ts"]),
                Op::SendMessage => (
                    "Post a message to a channel, or as a reply in a thread. A person approves every message first. It cannot notify the whole channel.",
                    vec![
                        ("channel", channel.clone()),
                        ("text", json!({"type": "string", "minLength": 1, "maxLength": TEXT_CHARS})),
                        ("thread_ts", json!({"type": "string", "pattern": "^[0-9]+\\.[0-9]+$"})),
                    ],
                    vec!["channel", "text"],
                ),
            };
            let mut properties: serde_json::Map<String, Value> = props.into_iter().map(|(k, v)| (k.to_string(), v)).collect();
            properties.insert("connection".into(), connection.clone());
            let mut spec = ToolSpec::new(
                format!("slack.{}", op.id()),
                format!("Slack {}", op.id().replace('_', " ")),
                format!("{describe} On Slack."),
                json!({"type": "object", "properties": properties, "required": required, "additionalProperties": false}),
            );
            spec.source = ToolSource::Integration { connection: None };
            spec.timeout_ms = 40_000;
            spec.max_output_bytes = 128 * 1024;
            spec.requires_approval = op == Op::SendMessage;
            spec.annotations = ToolAnnotations { read_only: op != Op::SendMessage, destructive: false, idempotent: op != Op::SendMessage, open_world: true };
            Arc::new(SlackTool { services: services.clone(), op, spec }) as Arc<dyn Tool>
        })
        .collect()
}

/// Whether `text` would notify a whole channel or workspace.
fn broadcasts(text: &str) -> bool {
    let lower = text.to_ascii_lowercase();
    ["<!channel", "<!here", "<!everyone", "@channel", "@here", "@everyone"].iter().any(|m| lower.contains(m))
}

/// Slack answers HTTP 200 with `ok: false` and an error code. Say what it means.
fn slack_check(reply: &Reply) -> Result<Value, ToolError> {
    if reply.status == 429 {
        return Err(ToolError::Retryable(format!(
            "Slack is limiting requests; try again in {} seconds.",
            reply.header("retry-after").unwrap_or("a little")
        )));
    }
    if reply.status >= 500 {
        return Err(ToolError::Retryable(format!("Slack had a problem ({}). Try again later.", reply.status)));
    }
    let body = reply.json()?;
    if body["ok"] == true {
        return Ok(body);
    }
    let code = body["error"].as_str().unwrap_or("unknown_error");
    Err(failed(match code {
        "not_in_channel" => {
            "The Slack app has not been invited to that channel. A person has to add it there.".to_string()
        }
        "channel_not_found" => "Slack cannot find that channel (or the app cannot see it).".to_string(),
        "invalid_auth" | "token_revoked" | "token_expired" | "account_inactive" | "not_authed" => {
            "Slack did not accept the connection's token. A person needs to replace it.".to_string()
        }
        "missing_scope" => format!(
            "The Slack token lacks a permission it needs ({}).",
            body["needed"].as_str().unwrap_or("see the app's scopes")
        ),
        "ratelimited" => return Err(ToolError::Retryable("Slack is limiting requests; try again shortly.".into())),
        "is_archived" => "That channel is archived.".to_string(),
        "msg_too_long" | "no_text" => format!("Slack rejected the message ({code})."),
        other => format!("Slack refused the request ({other})."),
    }))
}

impl SlackTool {
    async fn api(&self, ctx: &ToolContext, args: &Value) -> Result<Api, ToolError> {
        let connection = self
            .services
            .pick(&ctx.tenant, ConnectionKind::Slack, args.get("connection").and_then(Value::as_str))
            .await?
            .ok_or_else(|| failed("There is no Slack connection. Add one with `zorix connect add slack <name>`."))?;
        let token = self
            .services
            .secret(&connection)
            .await?
            .ok_or_else(|| failed(format!("The Slack connection {} has no token stored.", connection.name)))?;
        let base = connection.settings["base_url"].as_str().unwrap_or(DEFAULT_API);
        Api::new("Slack", base, vec![("Authorization".into(), format!("Bearer {token}"))])
    }

    /// May this execution use the channel? A grant names its id or its name.
    fn allowed(ctx: &ToolContext, action: Action, id: &str, name: Option<&str>) -> bool {
        let check = |r: &str| ctx.grants.allows(&Permission::new(action, format!("slack:{r}")));
        check(id)
            || name
                .is_some_and(|n| check(n.trim_start_matches('#')) || check(&format!("#{}", n.trim_start_matches('#'))))
    }

    /// The channel's id and name from what was asked for: an id, or a name found in the workspace's channel list.
    async fn resolve(&self, api: &Api, asked: &str) -> Result<(String, Option<String>), ToolError> {
        let is_id = asked.len() >= 9
            && asked.chars().all(|c| c.is_ascii_uppercase() || c.is_ascii_digit())
            && asked.starts_with(['C', 'G', 'D']);
        if is_id {
            let reply = api
                .send(
                    Method::GET,
                    &format!("/conversations.info{}", query(&[("channel", Some(asked.to_string()))])),
                    None,
                    None,
                )
                .await?;
            let name = slack_check(&reply).ok().and_then(|b| b["channel"]["name"].as_str().map(str::to_string));
            return Ok((asked.to_string(), name));
        }
        let wanted = asked.trim_start_matches('#').to_ascii_lowercase();
        let mut cursor: Option<String> = None;
        for _ in 0..10 {
            let reply = api
                .send(
                    Method::GET,
                    &format!(
                        "/conversations.list{}",
                        query(&[
                            ("types", Some("public_channel,private_channel".into())),
                            ("exclude_archived", Some("true".into())),
                            ("limit", Some("200".into())),
                            ("cursor", cursor.clone())
                        ])
                    ),
                    None,
                    None,
                )
                .await?;
            let body = slack_check(&reply)?;
            if let Some(found) = body["channels"]
                .as_array()
                .and_then(|a| a.iter().find(|c| c["name"].as_str().is_some_and(|n| n.to_ascii_lowercase() == wanted)))
            {
                return Ok((
                    found["id"].as_str().unwrap_or("").to_string(),
                    found["name"].as_str().map(str::to_string),
                ));
            }
            cursor = body["response_metadata"]["next_cursor"].as_str().filter(|c| !c.is_empty()).map(str::to_string);
            if cursor.is_none() {
                break;
            }
        }
        Err(failed(format!("Slack has no channel called {asked} that the app can see.")))
    }

    async fn run(&self, ctx: &ToolContext, args: &Value) -> Result<Value, ToolError> {
        let api = self.api(ctx, args).await?;
        if self.op == Op::ListChannels {
            let reply = api
                .send(
                    Method::GET,
                    &format!(
                        "/conversations.list{}",
                        query(&[
                            ("types", Some("public_channel,private_channel".into())),
                            ("exclude_archived", Some("true".into())),
                            ("limit", Some("200".into()))
                        ])
                    ),
                    None,
                    None,
                )
                .await?;
            let body = slack_check(&reply)?;
            // Only what this execution may read is shown: a run does not learn the names of channels it cannot use.
            let channels: Vec<Value> = body["channels"]
                .as_array()
                .map(|a| {
                    a.iter()
                        .filter(|c| Self::allowed(ctx, Action::Read, c["id"].as_str().unwrap_or(""), c["name"].as_str()))
                        .map(|c| json!({"id": c["id"], "name": c["name"], "private": c["is_private"], "member": c["is_member"], "topic": c["topic"]["value"], "members": c["num_members"]}))
                        .collect()
                })
                .unwrap_or_default();
            return Ok(json!({"channels": channels}));
        }
        let asked = args
            .get("channel")
            .and_then(Value::as_str)
            .filter(|c| !c.is_empty())
            .ok_or_else(|| failed("channel is missing."))?;
        let action = if self.op == Op::SendMessage { Action::Write } else { Action::Read };
        let (id, name) = self.resolve(&api, asked).await?;
        if !Self::allowed(ctx, action, &id, name.as_deref()) {
            return Err(failed(format!(
                "This execution has no permission to {action} slack:{}. A person can grant it with `--allow-chat slack:{}{}`.",
                name.as_deref().unwrap_or(&id),
                name.as_deref().unwrap_or(&id),
                if action == Action::Write { ":write" } else { "" }
            )));
        }
        let limit = args.get("limit").and_then(Value::as_u64).unwrap_or(20).clamp(1, 50).to_string();
        match self.op {
            Op::ReadChannel | Op::ReadThread => {
                let path = if self.op == Op::ReadChannel {
                    format!("/conversations.history{}", query(&[("channel", Some(id.clone())), ("limit", Some(limit))]))
                } else {
                    format!(
                        "/conversations.replies{}",
                        query(&[
                            ("channel", Some(id.clone())),
                            ("ts", args.get("thread_ts").and_then(Value::as_str).map(str::to_string)),
                            ("limit", Some(limit))
                        ])
                    )
                };
                let body = slack_check(&api.send(Method::GET, &path, None, None).await?)?;
                let messages: Vec<Value> = body["messages"]
                    .as_array()
                    .map(|a| a.iter().map(|m| json!({"user": m.get("user").or_else(|| m.get("bot_id")), "ts": m["ts"], "thread_ts": m.get("thread_ts"), "replies": m.get("reply_count"), "text": clip(m["text"].as_str().unwrap_or(""), TEXT_CHARS)})).collect())
                    .unwrap_or_default();
                Ok(json!({"channel": id, "messages": messages, "has_more": body["has_more"]}))
            }
            Op::SendMessage => {
                let text = args
                    .get("text")
                    .and_then(Value::as_str)
                    .filter(|t| !t.is_empty())
                    .ok_or_else(|| failed("text is missing."))?;
                if broadcasts(text) {
                    return Err(failed(
                        "A message may not notify the whole channel (@channel, @here, @everyone). Rewrite it without.",
                    ));
                }
                let mut body = json!({"channel": id, "text": text, "unfurl_links": false, "unfurl_media": false, "link_names": false});
                if let Some(ts) = args.get("thread_ts").and_then(Value::as_str) {
                    body["thread_ts"] = json!(ts);
                }
                let sent = slack_check(&api.send(Method::POST, "/chat.postMessage", None, Some(&body)).await?)?;
                Ok(json!({"ok": true, "channel": id, "ts": sent["ts"]}))
            }
            Op::ListChannels => unreachable!("handled above"),
        }
    }
}

#[async_trait]
impl Tool for SlackTool {
    fn spec(&self) -> &ToolSpec {
        &self.spec
    }

    async fn call(&self, ctx: &ToolContext, args: Value) -> Result<Value, ToolError> {
        self.run(ctx, &args).await
    }

    fn describe_call(&self, args: &Value) -> Option<String> {
        let channel = args.get("channel").and_then(Value::as_str)?;
        Some(match self.op {
            Op::SendMessage => format!(
                "Post a message to {channel} on Slack: “{}”",
                clip(args.get("text").and_then(Value::as_str).unwrap_or(""), 120)
            ),
            other => format!("{} {channel} on Slack", other.id().replace('_', " ")),
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn anything_that_would_notify_everyone_is_caught() {
        for yes in ["<!channel> hi", "hello @channel", "@HERE now", "<!everyone>", "x <!here|here>"] {
            assert!(broadcasts(yes), "{yes}");
        }
        for no in ["hello team", "email ana@example.com", "#general is busy", "a channel is a place"] {
            assert!(!broadcasts(no), "{no}");
        }
    }
}
