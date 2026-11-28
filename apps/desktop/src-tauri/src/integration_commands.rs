//! The commands behind the Integrations screens: MCP servers, connections and the sandbox. Like every command they return plain
//! text errors and decide nothing the hub does not decide. Secrets arrive here once, go to the keychain, and are never sent back.

use std::collections::HashMap;

use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use tauri::ipc::Channel;
use tauri::{AppHandle, Emitter, State};
use tokio_util::sync::CancellationToken;
use zorix_core::{
    Clock, ConnectionKind, EnvValue, EnvVar, HttpAuth, IntegrationStore, McpServer, McpTransport, NewMcpServer,
    PinAction, PinStatus, Secret, ServerSandbox, SystemClock, ToolTrust,
};
use zorix_hub::connections::ConnectRequest;
use zorix_mcp::catalog::{CatalogTransport, SecretNeeded};
use zorix_mcp::{McpManager, ServerState, SystemBrowser, Vault, import, secret_base, token_secret_name};

use crate::state::{AppState, Services};
use crate::ui_error::{IntoUi, UiError};

fn services(state: &AppState) -> Result<&Services, UiError> {
    state.services()
}

fn text(error: impl IntoUi) -> UiError {
    error.into_ui()
}

fn clean(value: &str, what: &str) -> Result<Secret, UiError> {
    let value = value.trim();
    if value.is_empty() {
        return Err(UiError::message(format!("The {what} is empty.")));
    }
    if value.chars().any(char::is_whitespace) || value.len() > 4000 {
        return Err(UiError::message(format!("The {what} has spaces or is too long to be kept.")));
    }
    Ok(Secret::new(value))
}

// ---------------------------------------------------------------------------------------------------------------------
// Servers

/// A server as the list shows it.
#[derive(Serialize)]
pub struct ServerRow {
    name: String,
    kind: &'static str,
    enabled: bool,
    /// `off`, `ready`, `starting`, `restarting`, `needs_login`, `failed` or `stopped`.
    state: &'static str,
    reason: Option<String>,
    /// What it runs or where it connects, in a sentence.
    summary: String,
    era: Option<String>,
    server_name: Option<String>,
    server_version: Option<String>,
    tools_listed: usize,
    tools_ready: usize,
    tools_to_review: usize,
    /// `none`, `bearer` or `oauth` for remote servers.
    auth: Option<&'static str>,
    /// A token or a browser sign-in is kept.
    signed_in: bool,
    /// The id of the catalogue entry it came from, when its address or its package still says so.
    catalog: Option<String>,
    /// That entry's mark.
    logo: Option<String>,
}

fn describe(transport: &McpTransport) -> String {
    match transport {
        McpTransport::Stdio { command, args, .. } => format!("{command} {}", args.join(" ")).trim_end().to_string(),
        McpTransport::Http { url, .. } => url.clone(),
    }
}

fn state_parts(enabled: bool, state: &ServerState) -> (&'static str, Option<String>) {
    if !enabled {
        return ("off", None);
    }
    match state {
        ServerState::Stopped => ("stopped", None),
        ServerState::Starting => ("starting", None),
        ServerState::Ready => ("ready", None),
        ServerState::Restarting { .. } => ("restarting", None),
        ServerState::NeedsLogin { reason } => ("needs_login", Some(reason.clone())),
        ServerState::Failed { reason } => ("failed", Some(reason.clone())),
    }
}

async fn signed_in(services: &Services, server: &McpServer) -> bool {
    match &server.transport {
        McpTransport::Http { auth: HttpAuth::OAuth { .. }, .. } => {
            Vault::new(services.secrets.clone(), &secret_base(server.id)).signed_in().await
        }
        McpTransport::Http { auth: HttpAuth::Bearer, .. } => {
            let secrets = services.secrets.clone();
            let name = token_secret_name(server.id);
            tokio::task::spawn_blocking(move || secrets.get(&name).ok().flatten().is_some()).await.unwrap_or(false)
        }
        _ => false,
    }
}

async fn row(services: &Services, server: &McpServer) -> Result<ServerRow, UiError> {
    let status = services.hub.manager.status(server.id).await.map_err(text)?;
    let (state, reason) = match &status {
        Some(status) => state_parts(server.enabled, &status.state),
        None => (if server.enabled { "stopped" } else { "off" }, None),
    };
    let pins = services.hub.store.tool_pins(services.hub.tenant(), server.id).await.map_err(text)?;
    let listed = pins.iter().filter(|p| p.listed).count();
    let ready = pins.iter().filter(|p| p.usable()).count();
    let review = pins.iter().filter(|p| p.listed && matches!(p.status, PinStatus::New | PinStatus::Changed)).count();
    let entry = zorix_mcp::catalog().entry_for(&server.transport);
    Ok(ServerRow {
        name: server.name.clone(),
        kind: match server.transport {
            McpTransport::Stdio { .. } => "stdio",
            McpTransport::Http { .. } => "http",
        },
        enabled: server.enabled,
        state,
        reason,
        summary: describe(&server.transport),
        era: server.era.clone(),
        server_name: status.as_ref().and_then(|s| s.server_name.clone()),
        server_version: status.as_ref().and_then(|s| s.server_version.clone()),
        tools_listed: listed,
        tools_ready: ready,
        tools_to_review: review,
        auth: match &server.transport {
            McpTransport::Http { auth: HttpAuth::None, .. } => Some("none"),
            McpTransport::Http { auth: HttpAuth::Bearer, .. } => Some("bearer"),
            McpTransport::Http { auth: HttpAuth::OAuth { .. }, .. } => Some("oauth"),
            McpTransport::Stdio { .. } => None,
        },
        signed_in: signed_in(services, server).await,
        catalog: entry.map(|e| e.id.clone()),
        logo: entry.map(|e| e.logo.clone()),
    })
}

#[tauri::command]
pub async fn mcp_list(state: State<'_, AppState>) -> Result<Vec<ServerRow>, UiError> {
    let services = services(&state)?;
    let mut rows = Vec::new();
    for server in services.hub.servers().await.map_err(text)? {
        rows.push(row(services, &server).await?);
    }
    Ok(rows)
}

/// One tool of a server, with what a person needs to decide about it.
#[derive(Serialize)]
pub struct ToolRow {
    name: String,
    /// The hash of the definition shown here: approving sends it back, so what is approved is what was on screen.
    hash: String,
    status: &'static str,
    trust: &'static str,
    listed: bool,
    title: Option<String>,
    description: String,
    /// What was approved, when it differs from what is listed now.
    approved_description: Option<String>,
    changed_fields: Vec<String>,
    inputs: Vec<String>,
    schema: Value,
    approved_schema: Option<Value>,
    read_only_hint: bool,
    destructive_hint: bool,
    open_world_hint: bool,
    /// The server reports it as read-only, so a person may let it run without asking.
    can_skip_asking: bool,
}

/// One server in full.
#[derive(Serialize)]
pub struct ServerDetail {
    #[serde(flatten)]
    row: ServerRow,
    tools: Vec<ToolRow>,
    log: Vec<String>,
    command: Option<String>,
    args: Vec<String>,
    /// Variables the program gets: name, and whether it is a secret (a value is never sent).
    env: Vec<(String, bool, Option<String>)>,
    cwd: Option<String>,
    url: Option<String>,
    /// The sandbox of a local server; `None` for a remote one and for one added before sandboxes.
    sandbox: Option<ServerSandbox>,
    /// What the sandbox enforces on this computer.
    enforced: EnforcedHere,
}

/// What native confinement enforces on this computer, so the screen promises nothing more.
#[derive(Serialize)]
pub struct EnforcedHere {
    os: &'static str,
    files: bool,
    network: bool,
    credentials: bool,
    processes: bool,
}

fn enforced_here() -> EnforcedHere {
    let here = zorix_mcp::enforced();
    EnforcedHere {
        os: std::env::consts::OS,
        files: here.files,
        network: here.network,
        credentials: here.credentials,
        processes: here.processes,
    }
}

/// Choose how a local server is confined. A running server is started again with the choice.
#[tauri::command]
pub async fn mcp_set_sandbox(state: State<'_, AppState>, name: String, sandbox: ServerSandbox) -> Result<(), UiError> {
    let services = services(&state)?;
    let mut chosen = sandbox;
    for folder in &mut chosen.folders {
        folder.path = canonical_folder(&folder.path)?;
    }
    services.hub.set_sandbox(&name, chosen).await.map(|_| ()).map_err(text)
}

fn tool_row(pin: &zorix_core::McpToolPin) -> ToolRow {
    let seen = &pin.seen;
    let annotations = &seen["annotations"];
    let flag = |key: &str, default: bool| annotations[key].as_bool().unwrap_or(default);
    let (read_only, destructive, open_world) =
        (flag("readOnlyHint", false), flag("destructiveHint", true), flag("openWorldHint", true));
    let changed_fields = match &pin.approved {
        Some(old) if pin.status == PinStatus::Changed => {
            let mut keys: Vec<String> = old
                .as_object()
                .into_iter()
                .chain(seen.as_object())
                .flat_map(|o| o.keys().cloned())
                .filter(|k| old[k.as_str()] != seen[k.as_str()])
                .collect();
            keys.sort();
            keys.dedup();
            keys
        }
        _ => Vec::new(),
    };
    ToolRow {
        name: pin.tool.clone(),
        hash: pin.seen_hash.clone(),
        status: pin.status.as_str(),
        trust: pin.trust.as_str(),
        listed: pin.listed,
        title: seen["title"].as_str().map(str::to_string),
        description: seen["description"].as_str().unwrap_or("").to_string(),
        approved_description: pin
            .approved
            .as_ref()
            .filter(|_| pin.status == PinStatus::Changed)
            .and_then(|a| a["description"].as_str().map(str::to_string)),
        changed_fields,
        inputs: seen["inputSchema"]["properties"].as_object().map(|p| p.keys().cloned().collect()).unwrap_or_default(),
        schema: seen["inputSchema"].clone(),
        approved_schema: pin
            .approved
            .as_ref()
            .filter(|_| pin.status == PinStatus::Changed)
            .map(|a| a["inputSchema"].clone()),
        read_only_hint: read_only,
        destructive_hint: destructive,
        open_world_hint: open_world,
        can_skip_asking: read_only && !destructive && !open_world,
    }
}

#[tauri::command]
pub async fn mcp_server(state: State<'_, AppState>, name: String) -> Result<ServerDetail, UiError> {
    let services = services(&state)?;
    let server = services.hub.server(&name).await.map_err(text)?;
    let pins = services.hub.store.tool_pins(services.hub.tenant(), server.id).await.map_err(text)?;
    let (command, args, env, cwd, url) = match &server.transport {
        McpTransport::Stdio { command, args, env, cwd } => (
            Some(command.clone()),
            args.clone(),
            env.iter()
                .map(|v| match &v.value {
                    EnvValue::Secret => (v.name.clone(), true, None),
                    EnvValue::Literal(value) => (v.name.clone(), false, Some(value.clone())),
                })
                .collect(),
            cwd.clone(),
            None,
        ),
        McpTransport::Http { url, .. } => (None, Vec::new(), Vec::new(), None, Some(url.clone())),
    };
    Ok(ServerDetail {
        row: row(services, &server).await?,
        tools: pins.iter().filter(|p| p.listed).map(tool_row).collect(),
        log: services.hub.manager.log(server.id),
        command,
        args,
        env,
        cwd,
        url,
        sandbox: server.sandbox.clone(),
        enforced: enforced_here(),
    })
}

/// The catalogue as the interface needs it.
#[tauri::command]
pub fn mcp_catalog() -> Value {
    serde_json::to_value(zorix_mcp::catalog()).unwrap_or(Value::Null)
}

/// What the Add server form sends. Secrets travel here once, into the keychain.
#[derive(Deserialize)]
pub struct AddServer {
    #[serde(default)]
    name: Option<String>,
    #[serde(default)]
    catalog: Option<String>,
    #[serde(default)]
    inputs: HashMap<String, String>,
    #[serde(default)]
    command: Option<String>,
    #[serde(default)]
    args: Vec<String>,
    #[serde(default)]
    env: Vec<EnvIn>,
    #[serde(default)]
    cwd: Option<String>,
    #[serde(default)]
    url: Option<String>,
    /// `none`, `bearer` or `oauth`.
    #[serde(default)]
    auth: Option<String>,
    #[serde(default)]
    client_id: Option<String>,
    #[serde(default)]
    scopes: Vec<String>,
    /// The bearer token.
    #[serde(default)]
    token: Option<String>,
    /// Values for the secrets a catalogue entry asks for, by variable name.
    #[serde(default)]
    secrets: HashMap<String, String>,
}

#[derive(Deserialize)]
pub struct EnvIn {
    name: String,
    value: String,
    secret: bool,
}

fn canonical_folder(path: &str) -> Result<String, UiError> {
    std::fs::canonicalize(path)
        .map(|p| p.to_string_lossy().into_owned())
        .map_err(|e| UiError::message(format!("The folder {path} cannot be used: {e}")))
}

#[tauri::command]
pub async fn mcp_add(state: State<'_, AppState>, request: AddServer) -> Result<ServerRow, UiError> {
    let services = services(&state)?;
    let mut secrets: Vec<(Option<String>, Secret)> = Vec::new();
    let new = if let Some(id) = &request.catalog {
        let entry = zorix_mcp::catalog()
            .entries
            .iter()
            .find(|e| &e.id == id)
            .ok_or_else(|| UiError::message(format!("The catalogue has no entry called {id}.")))?;
        let mut inputs = request.inputs.clone();
        if let CatalogTransport::Stdio { inputs: wanted, .. } = &entry.transport {
            for input in wanted.iter().filter(|i| i.kind == "folder") {
                if let Some(path) = inputs.get(&input.name).cloned() {
                    inputs.insert(input.name.clone(), canonical_folder(&path)?);
                }
            }
        }
        let (new, needed) = entry.to_new_server(request.name.as_deref(), &inputs)?;
        for need in needed {
            match need {
                SecretNeeded::Env { name, .. } => {
                    let value =
                        request.secrets.get(&name).ok_or_else(|| UiError::message(format!("{name} is needed.")))?;
                    secrets.push((Some(name.clone()), clean(value, &name)?));
                }
                SecretNeeded::Token => {
                    let token = request.token.as_deref().ok_or("A token is needed.")?;
                    secrets.push((None, clean(token, "token")?));
                }
                SecretNeeded::SignIn => {}
            }
        }
        new
    } else if let Some(command) = &request.command {
        let name = request.name.clone().ok_or("Give the server a name.")?;
        let mut env = Vec::new();
        for var in &request.env {
            if var.secret {
                env.push(EnvVar { name: var.name.clone(), value: EnvValue::Secret });
                secrets.push((Some(var.name.clone()), clean(&var.value, &var.name)?));
            } else {
                env.push(EnvVar { name: var.name.clone(), value: EnvValue::Literal(var.value.clone()) });
            }
        }
        let cwd = match request.cwd.as_deref().map(str::trim).filter(|c| !c.is_empty()) {
            Some(path) => Some(canonical_folder(path)?),
            None => None,
        };
        NewMcpServer {
            name,
            transport: McpTransport::Stdio {
                command: command.trim().to_string(),
                args: request.args.clone(),
                env,
                cwd,
            },
        }
    } else if let Some(url) = &request.url {
        let name = request.name.clone().ok_or("Give the server a name.")?;
        zorix_mcp::http::validate_url(url).map_err(text)?;
        let auth = match request.auth.as_deref().unwrap_or("none") {
            "none" => HttpAuth::None,
            "bearer" => {
                let token = request.token.as_deref().ok_or("A token is needed.")?;
                secrets.push((None, clean(token, "token")?));
                HttpAuth::Bearer
            }
            "oauth" => HttpAuth::OAuth {
                client_id: request.client_id.clone().filter(|c| !c.trim().is_empty()),
                scopes: request.scopes.clone(),
            },
            other => return Err(UiError::message(format!("The sign-in kind “{other}” is not supported."))),
        };
        NewMcpServer { name, transport: McpTransport::Http { url: url.trim().to_string(), auth } }
    } else {
        return Err("Say what to add: a catalogue entry, a command or an address.".into());
    };
    let server = services.hub.add_server(new, secrets).await.map_err(text)?;
    row(services, &server).await
}

#[tauri::command]
pub async fn mcp_set_enabled(state: State<'_, AppState>, name: String, enabled: bool) -> Result<ServerRow, UiError> {
    let services = services(&state)?;
    let server = services.hub.set_enabled(&name, enabled).await.map_err(text)?;
    if enabled {
        services.hub.manager.start(server.id).await.map_err(text)?;
    }
    row(services, &server).await
}

#[tauri::command]
pub async fn mcp_remove(state: State<'_, AppState>, name: String) -> Result<(), UiError> {
    services(&state)?.hub.remove_server(&name).await.map_err(text)
}

#[tauri::command]
pub async fn mcp_restart(state: State<'_, AppState>, name: String) -> Result<ServerRow, UiError> {
    let services = services(&state)?;
    let server = services.hub.server(&name).await.map_err(text)?;
    services.hub.manager.stop(server.id).await;
    if server.enabled {
        services.hub.manager.start(server.id).await.map_err(text)?;
    }
    row(services, &server).await
}

/// Ask a running server for its tools again.
#[tauri::command]
pub async fn mcp_refresh(state: State<'_, AppState>, name: String) -> Result<(), UiError> {
    let services = services(&state)?;
    let server = services.hub.server(&name).await.map_err(text)?;
    services.hub.manager.refresh(server.id).await.map_err(text)
}

/// Approve what a server lists for one tool, or turn it off.
#[tauri::command]
pub async fn mcp_review(
    state: State<'_, AppState>,
    name: String,
    tool: String,
    action: String,
    hash: Option<String>,
) -> Result<(), UiError> {
    let services = services(&state)?;
    let server = services.hub.server(&name).await.map_err(text)?;
    let action = match action.as_str() {
        "approve" => match hash {
            Some(shown) => PinAction::approve_seen(shown),
            None => return Err(UiError::message("Approving needs the definition that was shown.")),
        },
        "disable" => PinAction::Disable,
        other => return Err(UiError::message(format!("“{other}” is not an action."))),
    };
    services.hub.manager.review(server.id, &tool, action, "desktop").await.map(|_| ()).map_err(text)
}

#[tauri::command]
pub async fn mcp_approve_all(
    state: State<'_, AppState>,
    name: String,
    shown: Vec<(String, String)>,
) -> Result<usize, UiError> {
    let services = services(&state)?;
    let server = services.hub.server(&name).await.map_err(text)?;
    services.hub.manager.approve_shown(server.id, "desktop", &shown).await.map_err(text)
}

#[tauri::command]
pub async fn mcp_trust(
    state: State<'_, AppState>,
    name: String,
    tool: String,
    skip_asking: bool,
) -> Result<(), UiError> {
    let services = services(&state)?;
    let server = services.hub.server(&name).await.map_err(text)?;
    let level = if skip_asking { ToolTrust::ReadOnlyNoAsk } else { ToolTrust::Ask };
    services.hub.manager.set_trust(server.id, &tool, level).await.map(|_| ()).map_err(text)
}

/// Sign in to a remote server in the system browser. Resolves when the person is done, or `mcp_login_cancel` is called.
#[tauri::command]
pub async fn mcp_login(state: State<'_, AppState>, name: String) -> Result<String, UiError> {
    let services = services(&state)?;
    let cancel = CancellationToken::new();
    services.logins.lock().insert(name.clone(), cancel.clone());
    let outcome = services.hub.login(&name, &SystemBrowser, &cancel).await;
    services.logins.lock().remove(&name);
    let summary = outcome.map_err(text)?;
    // With a sign-in in hand, the server can start (or start again).
    let server = services.hub.server(&name).await.map_err(text)?;
    services.hub.manager.stop(server.id).await;
    if server.enabled {
        services.hub.manager.start(server.id).await.map_err(text)?;
    }
    Ok(summary.issuer)
}

#[tauri::command]
pub fn mcp_login_cancel(state: State<'_, AppState>, name: String) -> Result<(), UiError> {
    if let Some(token) = services(&state)?.logins.lock().remove(&name) {
        token.cancel();
    }
    Ok(())
}

#[tauri::command]
pub async fn mcp_logout(state: State<'_, AppState>, name: String) -> Result<(), UiError> {
    services(&state)?.hub.logout(&name).await.map_err(text)
}

/// One server found in another program's configuration.
#[derive(Serialize)]
pub struct ImportRow {
    original_name: String,
    name: String,
    summary: Option<String>,
    warnings: Vec<String>,
    skipped: Option<String>,
    secrets: usize,
}

#[derive(Serialize)]
pub struct ImportPreview {
    source: String,
    path: String,
    rows: Vec<ImportRow>,
}

fn source_of(text: &str) -> Result<import::Source, UiError> {
    import::Source::parse(text).ok_or_else(|| UiError::message(format!("“{text}” is not claude, vscode or cursor.")))
}

/// What would be imported from the usual places, or from one file.
#[tauri::command]
pub fn mcp_import_preview(source: String, path: Option<String>) -> Result<Vec<ImportPreview>, UiError> {
    let source = source_of(&source)?;
    let paths = match path.as_deref().map(str::trim).filter(|p| !p.is_empty()) {
        Some(path) => vec![std::path::PathBuf::from(path)],
        None => source.default_paths().into_iter().filter(|p| p.is_file()).collect(),
    };
    let mut previews = Vec::new();
    for path in paths {
        let preview = import::read(source, &path)?;
        previews.push(ImportPreview {
            source: source.as_str().to_string(),
            path: path.display().to_string(),
            rows: preview
                .entries
                .iter()
                .map(|e| ImportRow {
                    original_name: e.original_name.clone(),
                    name: e.name.clone(),
                    summary: e.transport.as_ref().map(describe),
                    warnings: e.warnings.clone(),
                    skipped: e.skipped.clone(),
                    secrets: e.secrets.len(),
                })
                .collect(),
        });
    }
    Ok(previews)
}

/// Import the chosen servers. They are added off; a person turns each on.
#[tauri::command]
pub async fn mcp_import_apply(
    state: State<'_, AppState>,
    source: String,
    path: String,
    names: Vec<String>,
) -> Result<Vec<String>, UiError> {
    let services = services(&state)?;
    let preview = import::read(source_of(&source)?, std::path::Path::new(&path))?;
    let store: std::sync::Arc<dyn IntegrationStore> = services.hub.store.clone();
    let results =
        import::apply(&preview, &names, &store, &services.hub.secrets, services.hub.tenant(), SystemClock.now()).await;
    Ok(results
        .into_iter()
        .map(|r| match r {
            Ok(done) => format!("Added {} (off).", done.server.name),
            Err((name, why)) => format!("{name}: {why}"),
        })
        .collect())
}

// ---------------------------------------------------------------------------------------------------------------------
// Connections

#[tauri::command]
pub async fn connections_list(state: State<'_, AppState>) -> Result<Vec<zorix_core::Connection>, UiError> {
    services(&state)?.hub.connections().await.map_err(text)
}

#[derive(Deserialize)]
pub struct AddConnection {
    kind: String,
    name: String,
    #[serde(default)]
    settings: Value,
    #[serde(default)]
    secret: Option<String>,
}

#[tauri::command]
pub async fn connection_add(
    state: State<'_, AppState>,
    request: AddConnection,
) -> Result<zorix_core::Connection, UiError> {
    let services = services(&state)?;
    let kind = ConnectionKind::parse(&request.kind)
        .ok_or_else(|| UiError::message(format!("“{}” is not a kind of connection.", request.kind)))?;
    let secret = match request.secret.as_deref().map(str::trim).filter(|s| !s.is_empty()) {
        Some(value) => Some(clean(value, "secret")?),
        None => None,
    };
    let settings = if request.settings.is_object() { request.settings } else { json!({}) };
    services.hub.add_connection(ConnectRequest { kind, name: request.name, settings, secret }).await.map_err(text)
}

#[derive(Serialize)]
pub struct TestResult {
    ok: bool,
    note: String,
}

#[tauri::command]
pub async fn connection_test(state: State<'_, AppState>, name: String) -> Result<TestResult, UiError> {
    let test = services(&state)?.hub.test_connection(&name).await.map_err(text)?;
    Ok(TestResult { ok: test.ok, note: test.note })
}

#[tauri::command]
pub async fn connection_remove(state: State<'_, AppState>, name: String) -> Result<(), UiError> {
    services(&state)?.hub.remove_connection(&name).await.map_err(text)
}

// ---------------------------------------------------------------------------------------------------------------------
// Sandbox

#[derive(Serialize)]
pub struct SandboxStatus {
    javascript: bool,
    python: bool,
    python_megabytes: u64,
}

#[tauri::command]
pub fn sandbox_status(state: State<'_, AppState>) -> Result<SandboxStatus, UiError> {
    let services = services(&state)?;
    Ok(SandboxStatus {
        javascript: true,
        python: services.hub.sandbox.python_installed(),
        python_megabytes: zorix_sandbox::runner::PYTHON_BYTES / 1_000_000,
    })
}

/// Download Python for the sandbox, reporting `[received, total]` bytes on the channel.
#[tauri::command]
pub async fn sandbox_install_python(state: State<'_, AppState>, progress: Channel<(u64, u64)>) -> Result<(), UiError> {
    let services = services(&state)?;
    services
        .hub
        .sandbox
        .install_python(move |done, total| {
            let _ = progress.send((done, total));
        })
        .await
        .map_err(text)
}

// ---------------------------------------------------------------------------------------------------------------------

/// Tell the interface when a server changes state, so cards update without polling.
pub fn forward_status(app: AppHandle, manager: McpManager) {
    tauri::async_runtime::spawn(async move {
        let mut changes = manager.subscribe();
        loop {
            match changes.recv().await {
                Ok(change) => {
                    let _ = app.emit("mcp-status", change.server.to_string());
                }
                Err(tokio::sync::broadcast::error::RecvError::Lagged(_)) => {
                    let _ = app.emit("mcp-status", "*");
                }
                Err(_) => break,
            }
        }
    });
}
