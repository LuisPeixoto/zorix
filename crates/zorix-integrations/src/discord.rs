//! Discord tools (ADR-0021): see a server's channels a run may read, read them, and post. A channel is the resource a person
//! grants: `read` or `write` on `discord:<channel id>`. Posting always asks and never mentions anyone.

use std::sync::Arc;

use async_trait::async_trait;
use reqwest::Method;
use serde_json::{Value, json};
use zorix_core::{
    Action, ConnectionKind, Permission, Tool, ToolAnnotations, ToolContext, ToolError, ToolSource, ToolSpec,
};

use crate::api::Api;
use crate::connections::{Services, clip, query};

const DEFAULT_API: &str = "https://discord.com/api/v10";
const TEXT_CHARS: usize = 2_000;

fn failed(message: impl Into<String>) -> ToolError {
    ToolError::Failed(message.into())
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Op {
    ListChannels,
    ReadChannel,
    SendMessage,
}

impl Op {
    fn id(self) -> &'static str {
        match self {
            Op::ListChannels => "list_channels",
            Op::ReadChannel => "read_channel",
            Op::SendMessage => "send_message",
        }
    }
}

/// A Discord tool.
pub struct DiscordTool {
    services: Arc<Services>,
    op: Op,
    spec: ToolSpec,
}

/// The Discord tools.
pub fn discord_tools(services: Arc<Services>) -> Vec<Arc<dyn Tool>> {
    let snowflake = |what: &str| json!({"type": "string", "pattern": "^[0-9]{5,25}$", "description": what});
    let connection = json!({"type": "string", "maxLength": 32, "description": "Which Discord connection, when there is more than one."});
    [Op::ListChannels, Op::ReadChannel, Op::SendMessage]
        .into_iter()
        .map(|op| {
            let (describe, props, required): (&str, Vec<(&str, Value)>, Vec<&str>) = match op {
                Op::ListChannels => ("List the text channels of a server that this execution may read.", vec![("guild_id", snowflake("The server's id."))], vec!["guild_id"]),
                Op::ReadChannel => (
                    "Read the latest messages of a channel.",
                    vec![("channel_id", snowflake("The channel's id.")), ("limit", json!({"type": "integer", "minimum": 1, "maximum": 50})), ("before", snowflake("Read messages older than this message id."))],
                    vec!["channel_id"],
                ),
                Op::SendMessage => (
                    "Post a message to a channel. A person approves every message first. It never mentions anyone.",
                    vec![("channel_id", snowflake("The channel's id.")), ("content", json!({"type": "string", "minLength": 1, "maxLength": TEXT_CHARS})), ("reply_to", snowflake("A message id to reply to."))],
                    vec!["channel_id", "content"],
                ),
            };
            let mut properties: serde_json::Map<String, Value> = props.into_iter().map(|(k, v)| (k.to_string(), v)).collect();
            properties.insert("connection".into(), connection.clone());
            let mut spec = ToolSpec::new(
                format!("discord.{}", op.id()),
                format!("Discord {}", op.id().replace('_', " ")),
                format!("{describe} On Discord."),
                json!({"type": "object", "properties": properties, "required": required, "additionalProperties": false}),
            );
            spec.source = ToolSource::Integration { connection: None };
            spec.timeout_ms = 40_000;
            spec.max_output_bytes = 128 * 1024;
            spec.requires_approval = op == Op::SendMessage;
            spec.annotations = ToolAnnotations { read_only: op != Op::SendMessage, destructive: false, idempotent: op != Op::SendMessage, open_world: true };
            Arc::new(DiscordTool { services: services.clone(), op, spec }) as Arc<dyn Tool>
        })
        .collect()
}

impl DiscordTool {
    async fn api(&self, ctx: &ToolContext, args: &Value) -> Result<Api, ToolError> {
        let connection = self
            .services
            .pick(&ctx.tenant, ConnectionKind::Discord, args.get("connection").and_then(Value::as_str))
            .await?
            .ok_or_else(|| {
                failed("There is no Discord connection. Add one with `zorix connect add discord <name>`.")
            })?;
        let token =
            self.services.secret(&connection).await?.ok_or_else(|| {
                failed(format!("The Discord connection {} has no bot token stored.", connection.name))
            })?;
        let base = connection.settings["base_url"].as_str().unwrap_or(DEFAULT_API);
        Api::new("Discord", base, vec![("Authorization".into(), format!("Bot {token}"))])
    }

    fn allowed(ctx: &ToolContext, action: Action, channel: &str) -> bool {
        ctx.grants.allows(&Permission::new(action, format!("discord:{channel}")))
    }

    fn check(&self, reply: &crate::api::Reply) -> Result<Value, ToolError> {
        if reply.status == 429 {
            let wait = reply
                .json()
                .ok()
                .and_then(|b| b["retry_after"].as_f64())
                .map(|s| format!("{s:.0} seconds"))
                .unwrap_or_else(|| "a little".into());
            return Err(ToolError::Retryable(format!("Discord is limiting requests; try again in {wait}.")));
        }
        if (200..300).contains(&reply.status) {
            return reply.json();
        }
        let body = reply.json().unwrap_or(Value::Null);
        let message = body["message"].as_str().unwrap_or("");
        Err(match reply.status {
            401 => failed("Discord did not accept the bot token. A person needs to replace it."),
            403 => failed(
                "The bot is not allowed to do that in this channel (it may be missing a permission, or not be in the server).",
            ),
            404 => failed("Discord cannot find that channel or message, or the bot cannot see it."),
            500..=599 => ToolError::Retryable(format!("Discord had a problem ({}). Try again later.", reply.status)),
            other => failed(format!("Discord answered {other}: {message}")),
        })
    }

    async fn run(&self, ctx: &ToolContext, args: &Value) -> Result<Value, ToolError> {
        let text = |name: &str| args.get(name).and_then(Value::as_str).filter(|s| !s.is_empty());
        // The ids go into the path of a request. The schema says what they look like, and it is checked before a call; this holds
        // for a caller that did not check it, so an id can never carry a `/`, a `..` or a `?` into the path.
        for name in ["guild_id", "channel_id", "reply_to", "before"] {
            if let Some(id) = text(name)
                && !is_snowflake(id)
            {
                return Err(failed(format!("{name} is not an id (it has to be 5 to 25 digits).")));
            }
        }
        let api = self.api(ctx, args).await?;
        match self.op {
            Op::ListChannels => {
                let guild = text("guild_id").ok_or_else(|| failed("guild_id is missing."))?;
                let body =
                    self.check(&api.send(Method::GET, &format!("/guilds/{guild}/channels"), None, None).await?)?;
                // Text-like channels only, and only the ones this execution may read.
                let channels: Vec<Value> = body
                    .as_array()
                    .map(|a| {
                        a.iter()
                            .filter(|c| matches!(c["type"].as_u64(), Some(0 | 5)))
                            .filter(|c| Self::allowed(ctx, Action::Read, c["id"].as_str().unwrap_or("")))
                            .map(|c| json!({"id": c["id"], "name": c["name"], "topic": c["topic"]}))
                            .collect()
                    })
                    .unwrap_or_default();
                Ok(json!({"channels": channels}))
            }
            Op::ReadChannel | Op::SendMessage => {
                let channel = text("channel_id").ok_or_else(|| failed("channel_id is missing."))?;
                let action = if self.op == Op::SendMessage { Action::Write } else { Action::Read };
                if !Self::allowed(ctx, action, channel) {
                    return Err(failed(format!(
                        "This execution has no permission to {action} discord:{channel}. A person can grant it with `--allow-chat discord:{channel}{}`.",
                        if action == Action::Write { ":write" } else { "" }
                    )));
                }
                if self.op == Op::ReadChannel {
                    let limit = args.get("limit").and_then(Value::as_u64).unwrap_or(20).clamp(1, 50).to_string();
                    let path = format!(
                        "/channels/{channel}/messages{}",
                        query(&[("limit", Some(limit)), ("before", text("before").map(str::to_string))])
                    );
                    let body = self.check(&api.send(Method::GET, &path, None, None).await?)?;
                    let messages: Vec<Value> = body
                        .as_array()
                        .map(|a| a.iter().map(|m| json!({"id": m["id"], "author": m["author"]["username"], "bot": m["author"]["bot"], "timestamp": m["timestamp"], "content": clip(m["content"].as_str().unwrap_or(""), TEXT_CHARS)})).collect())
                        .unwrap_or_default();
                    return Ok(json!({"channel": channel, "messages": messages}));
                }
                let content = text("content").ok_or_else(|| failed("content is missing."))?;
                // Nobody is mentioned, whatever the text says.
                let mut body = json!({"content": content, "allowed_mentions": {"parse": []}});
                if let Some(reply_to) = text("reply_to") {
                    body["message_reference"] = json!({"message_id": reply_to, "fail_if_not_exists": false});
                }
                let sent = self.check(
                    &api.send(Method::POST, &format!("/channels/{channel}/messages"), None, Some(&body)).await?,
                )?;
                Ok(json!({"ok": true, "channel": channel, "id": sent["id"]}))
            }
        }
    }
}

/// A Discord id: 5 to 25 digits, nothing else.
fn is_snowflake(id: &str) -> bool {
    (5..=25).contains(&id.len()) && id.bytes().all(|b| b.is_ascii_digit())
}

#[async_trait]
impl Tool for DiscordTool {
    fn spec(&self) -> &ToolSpec {
        &self.spec
    }

    async fn call(&self, ctx: &ToolContext, args: Value) -> Result<Value, ToolError> {
        self.run(ctx, &args).await
    }

    fn describe_call(&self, args: &Value) -> Option<String> {
        let channel =
            args.get("channel_id").and_then(Value::as_str).or_else(|| args.get("guild_id").and_then(Value::as_str))?;
        Some(match self.op {
            Op::SendMessage => format!(
                "Post a message to Discord channel {channel}: “{}”",
                clip(args.get("content").and_then(Value::as_str).unwrap_or(""), 120)
            ),
            other => format!("{} {channel} on Discord", other.id().replace('_', " ")),
        })
    }
}
