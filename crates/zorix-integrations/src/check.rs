//! Testing a connection: one small read that shows the address, the credential and the permissions work. A test never
//! changes anything and never sends a webhook.

use std::sync::Arc;

use base64::Engine as _;
use reqwest::Method;
use serde_json::{Value, json};
use tokio_util::sync::CancellationToken;
use zorix_core::{
    Action, ConnectionKind, ExecutionId, Grants, Permission, TenantId, Tool, ToolCallId, ToolContext, ToolError,
};

use crate::api::Api;
use crate::connections::{Services, clip};
use crate::{db_tools, http_tools};

/// What a test found.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Checked {
    /// It worked.
    pub ok: bool,
    /// What it found, or why it failed. Never a secret.
    pub note: String,
}

fn good(note: impl Into<String>) -> Checked {
    Checked { ok: true, note: note.into() }
}

fn bad(note: impl Into<String>) -> Checked {
    Checked { ok: false, note: note.into() }
}

fn why(error: ToolError) -> Checked {
    match error {
        ToolError::Retryable(m) | ToolError::Failed(m) => bad(m),
    }
}

fn setting<'a>(connection: &'a zorix_core::Connection, key: &str) -> Option<&'a str> {
    connection.settings[key].as_str().filter(|s| !s.is_empty())
}

/// Test one connection.
pub async fn test_connection(services: Arc<Services>, connection: &zorix_core::Connection) -> Checked {
    let token = match services.secret(connection).await {
        Ok(token) => token,
        Err(e) => return why(e),
    };
    match connection.kind {
        ConnectionKind::Github => github(connection, token).await,
        ConnectionKind::Gitlab => gitlab(connection, token).await,
        ConnectionKind::Jira => jira(connection, token).await,
        ConnectionKind::Slack => slack(connection, token).await,
        ConnectionKind::Discord => discord(connection, token).await,
        ConnectionKind::Postgres | ConnectionKind::Sqlite => database(services, connection).await,
        ConnectionKind::Http => http(services, connection).await,
        ConnectionKind::Webhook => webhook(connection, token),
    }
}

async fn get(api: &Api, path: &str) -> Result<Value, Checked> {
    let reply = api.send(Method::GET, path, None, None).await.map_err(why)?;
    api.check(&reply, "the test request").map_err(why)?;
    reply.json().map_err(why)
}

async fn github(connection: &zorix_core::Connection, token: Option<String>) -> Checked {
    let base = setting(connection, "base_url").unwrap_or("https://api.github.com");
    let mut headers = vec![
        ("Accept".to_string(), "application/vnd.github+json".to_string()),
        ("X-GitHub-Api-Version".to_string(), "2022-11-28".to_string()),
    ];
    let signed_in = token.is_some();
    if let Some(token) = token {
        headers.push(("Authorization".into(), format!("Bearer {token}")));
    }
    let api = match Api::new("GitHub", base, headers) {
        Ok(api) => api,
        Err(e) => return why(e),
    };
    if signed_in {
        match get(&api, "/user").await {
            Ok(user) => good(format!("Signed in to GitHub as {}.", user["login"].as_str().unwrap_or("(unknown)"))),
            Err(c) => c,
        }
    } else {
        match get(&api, "/rate_limit").await {
            Ok(limit) => good(format!(
                "GitHub is reachable. No token is stored, so only public data can be read ({} requests left this hour).",
                limit["resources"]["core"]["remaining"].as_u64().unwrap_or(0)
            )),
            Err(c) => c,
        }
    }
}

async fn gitlab(connection: &zorix_core::Connection, token: Option<String>) -> Checked {
    let mut base =
        setting(connection, "base_url").unwrap_or("https://gitlab.com/api/v4").trim_end_matches('/').to_string();
    if !base.ends_with("/api/v4") {
        base.push_str("/api/v4");
    }
    let signed_in = token.is_some();
    let headers = token.map(|t| vec![("PRIVATE-TOKEN".to_string(), t)]).unwrap_or_default();
    let api = match Api::new("GitLab", &base, headers) {
        Ok(api) => api,
        Err(e) => return why(e),
    };
    if signed_in {
        match get(&api, "/user").await {
            Ok(user) => good(format!("Signed in to GitLab as {}.", user["username"].as_str().unwrap_or("(unknown)"))),
            Err(c) => c,
        }
    } else {
        match get(&api, "/projects?per_page=1&simple=true").await {
            Ok(_) => good("GitLab is reachable. No token is stored, so only public projects can be read."),
            Err(c) => c,
        }
    }
}

async fn jira(connection: &zorix_core::Connection, token: Option<String>) -> Checked {
    let Some(base) = setting(connection, "base_url") else { return bad("The connection has no site address.") };
    let Some(token) = token else { return bad("No token is stored for this connection.") };
    let authorization = if connection.settings["auth"].as_str() == Some("bearer") {
        format!("Bearer {token}")
    } else {
        let Some(email) = setting(connection, "email") else { return bad("The connection has no account email.") };
        format!("Basic {}", base64::engine::general_purpose::STANDARD.encode(format!("{email}:{token}")))
    };
    let api = match Api::new(
        "Jira",
        base,
        vec![("Authorization".into(), authorization), ("Accept".into(), "application/json".into())],
    ) {
        Ok(api) => api,
        Err(e) => return why(e),
    };
    match get(&api, "/rest/api/3/myself").await {
        Ok(me) => good(format!("Signed in to Jira as {}.", me["displayName"].as_str().unwrap_or("(unknown)"))),
        Err(c) => c,
    }
}

async fn slack(connection: &zorix_core::Connection, token: Option<String>) -> Checked {
    let Some(token) = token else { return bad("No token is stored for this connection.") };
    let base = setting(connection, "base_url").unwrap_or("https://slack.com/api");
    let api = match Api::new("Slack", base, vec![("Authorization".into(), format!("Bearer {token}"))]) {
        Ok(api) => api,
        Err(e) => return why(e),
    };
    match get(&api, "/auth.test").await {
        Ok(answer) if answer["ok"] == true => good(format!(
            "Signed in to Slack workspace {} as {}.",
            answer["team"].as_str().unwrap_or("(unknown)"),
            answer["user"].as_str().unwrap_or("(unknown)")
        )),
        Ok(answer) => bad(format!("Slack said: {}.", clip(answer["error"].as_str().unwrap_or("not ok"), 80))),
        Err(c) => c,
    }
}

async fn discord(connection: &zorix_core::Connection, token: Option<String>) -> Checked {
    let Some(token) = token else { return bad("No bot token is stored for this connection.") };
    let base = setting(connection, "base_url").unwrap_or("https://discord.com/api/v10");
    let api = match Api::new("Discord", base, vec![("Authorization".into(), format!("Bot {token}"))]) {
        Ok(api) => api,
        Err(e) => return why(e),
    };
    match get(&api, "/users/@me").await {
        Ok(me) => good(format!("The bot is {}.", me["username"].as_str().unwrap_or("(unknown)"))),
        Err(c) => c,
    }
}

fn context(tenant: &TenantId, grants: Grants) -> ToolContext {
    ToolContext {
        execution_id: ExecutionId::new(),
        tenant: tenant.clone(),
        call_id: ToolCallId("test".into()),
        grants,
        cancel: CancellationToken::new(),
        workspace_root: None,
        conversation: None,
    }
}

async fn call(tools: Vec<Arc<dyn Tool>>, id: &str, ctx: &ToolContext, args: Value) -> Result<Value, ToolError> {
    let tool =
        tools.into_iter().find(|t| t.spec().id == id).ok_or_else(|| ToolError::Failed(format!("{id} is missing")))?;
    tool.call(ctx, args).await
}

async fn database(services: Arc<Services>, connection: &zorix_core::Connection) -> Checked {
    let grants = Grants::none().with(Permission::new(Action::Read, format!("db:{}", connection.name)));
    let ctx = context(&connection.tenant, grants);
    let tools = db_tools(services);
    let outcome = if connection.kind == ConnectionKind::Sqlite {
        call(tools, "db.schema", &ctx, json!({"connection": connection.name})).await.map(|schema| {
            format!(
                "The SQLite file opens read-only and has {} table(s).",
                schema["tables"].as_array().map_or(0, Vec::len)
            )
        })
    } else {
        call(tools, "db.query", &ctx, json!({"connection": connection.name, "sql": "SELECT version() AS version"}))
            .await
            .map(|rows| {
                format!(
                    "Connected to Postgres ({}). Queries run in read-only transactions.",
                    clip(rows["rows"][0][0].as_str().unwrap_or("version unknown"), 60)
                )
            })
    };
    match outcome {
        Ok(note) => good(note),
        Err(e) => why(e),
    }
}

async fn http(services: Arc<Services>, connection: &zorix_core::Connection) -> Checked {
    let Some(host) = setting(connection, "host") else { return bad("The connection has no host.") };
    let grants = Grants::none().with(Permission::new(Action::Network, format!("http:{host}")));
    let ctx = context(&connection.tenant, grants);
    let tools = http_tools(services.store.clone(), services.secrets.clone());
    match call(tools, "http.get", &ctx, json!({"url": format!("https://{host}/"), "max_chars": 200})).await {
        Ok(reply) => good(format!(
            "{host} is reachable (HTTP {}). The stored credential is sent only to this host.",
            reply["status"].as_u64().unwrap_or(0)
        )),
        Err(e) => why(e),
    }
}

fn webhook(connection: &zorix_core::Connection, token: Option<String>) -> Checked {
    let Some(url) = setting(connection, "url") else { return bad("The connection has no address.") };
    if token.is_none() {
        return bad("No signing key is stored for this webhook.");
    }
    good(format!("{url} is set up with a signing key. Nothing was sent: a test would trigger whatever listens there."))
}
