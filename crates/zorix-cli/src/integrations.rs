//! `zorix mcp`, `zorix connect` and `zorix sandbox`: connecting ZORIX to the outside (ADR-0018, ADR-0021, ADR-0022).
//!
//! Secrets never travel on the command line: they are read from an environment variable the person names, from standard input,
//! or from a hidden prompt, and go straight to the keychain.

use std::collections::HashMap;
use std::io::{BufRead, IsTerminal, Write};
use std::path::Path;
use std::sync::Arc;
use std::time::Duration;

use serde_json::{Value, json};
use tokio_util::sync::CancellationToken;
use zorix_core::{
    Clock, ConnectionKind, EnvValue, EnvVar, HttpAuth, IntegrationStore, McpServer, McpTransport, NewMcpServer,
    PinAction, PinStatus, Secret, TenantId, ToolTrust,
};
use zorix_hub::Hub;
use zorix_hub::connections::ConnectRequest;
use zorix_mcp::catalog::SecretNeeded;
use zorix_mcp::{ServerState, SystemBrowser, import};
use zorix_runtime::InMemoryToolRegistry;
use zorix_store::DataDirLock;

use crate::app::{CliError, load_config, secrets};
use crate::cli::{
    ConnectAdd, ConnectCommand, ImportSourceArg, McpAddArgs, McpAuthArg, McpCommand, McpEnableArgs, McpImportArgs,
    McpReviewArgs, McpTrustArgs, SandboxCommand, SandboxInstall, SecretArgs,
};
use crate::follow::{Exit, Printer};
use crate::render::shorten;

/// How long a server gets to start when a command needs it running.
const START_WAIT: Duration = Duration::from_secs(40);

struct Session {
    hub: Hub,
    _lock: Option<DataDirLock>,
}

/// Open the hub. Commands that change anything hold the data-folder lock; the ones that only read do not.
fn open(home: &Path, tenant: &TenantId, write: bool) -> Result<Session, CliError> {
    let lock = if write { Some(DataDirLock::acquire(home)?) } else { None };
    let hub = Hub::open(home, tenant.clone(), secrets(&load_config(home)?), Arc::new(InMemoryToolRegistry::new()))?;
    Ok(Session { hub, _lock: lock })
}

fn out<T: serde::Serialize>(value: &T) -> Result<(), CliError> {
    println!("{}", serde_json::to_string(value)?);
    Ok(())
}

fn say(printer: Printer, text: impl AsRef<str>) {
    if !printer.json {
        eprintln!("{}", text.as_ref());
    }
}

fn ask_yes(question: &str) -> bool {
    eprint!("{question} [y/N] ");
    let _ = std::io::stderr().flush();
    let mut line = String::new();
    std::io::stdin().lock().read_line(&mut line).is_ok()
        && matches!(line.trim().to_ascii_lowercase().as_str(), "y" | "yes")
}

// ---------------------------------------------------------------------------------------------------------------------
// Secrets

fn clean(value: String, what: &str) -> Result<Secret, CliError> {
    let value = value.trim().to_string();
    if value.is_empty() {
        return Err(CliError(format!("The {what} is empty.")));
    }
    if value.chars().any(char::is_whitespace) || value.len() > 4000 {
        return Err(CliError(format!("The {what} has spaces or is too long to be kept.")));
    }
    Ok(Secret::new(value))
}

/// A secret from where the person said: a variable, standard input, or (on a terminal) a hidden prompt. `Ok(None)` when
/// nothing was said and there is no terminal, or when an optional prompt was left empty.
fn read_secret(args: &SecretArgs, what: &str, optional: bool) -> Result<Option<Secret>, CliError> {
    if let Some(var) = &args.secret_env {
        let value = std::env::var(var).map_err(|_| CliError(format!("The environment variable {var} is not set.")))?;
        return clean(value, what).map(Some);
    }
    if args.secret_stdin {
        let mut line = String::new();
        std::io::stdin().lock().read_line(&mut line)?;
        return clean(line, what).map(Some);
    }
    if std::io::stdin().is_terminal() {
        let value = rpassword::prompt_password(format!(
            "{}{} (hidden{}): ",
            what[..1].to_uppercase(),
            &what[1..],
            if optional { ", empty for none" } else { "" }
        ))?;
        if optional && value.trim().is_empty() {
            return Ok(None);
        }
        return clean(value, what).map(Some);
    }
    if optional {
        Ok(None)
    } else {
        Err(CliError(format!("The {what} is needed: use --secret-env VAR or --secret-stdin.")))
    }
}

/// The value of an environment variable of a server: this command's own variable of the same name, or a prompt.
fn env_secret(name: &str, label: &str) -> Result<Secret, CliError> {
    if let Ok(value) = std::env::var(name) {
        return clean(value, name);
    }
    if std::io::stdin().is_terminal() {
        let value = rpassword::prompt_password(format!("{label} ({name}, hidden): "))?;
        return clean(value, name);
    }
    Err(CliError(format!("{name} is needed: set it in the environment of this command, or run it on a terminal.")))
}

// ---------------------------------------------------------------------------------------------------------------------
// MCP

pub async fn mcp(home: &Path, tenant: TenantId, printer: Printer, command: McpCommand) -> Result<Exit, CliError> {
    match command {
        McpCommand::Serve(_) => unreachable!("handled by mcp_serve"),
        McpCommand::Catalog(args) => catalog(printer, args.search.as_deref()),
        McpCommand::Add(args) => add_server(home, &tenant, printer, args).await,
        McpCommand::List => list_servers(home, &tenant, printer).await,
        McpCommand::Show(arg) => show_server(home, &tenant, printer, &arg.name).await,
        McpCommand::Login(arg) => login(home, &tenant, printer, &arg.name).await,
        McpCommand::Logout(arg) => {
            let session = open(home, &tenant, true)?;
            session.hub.logout(&arg.name).await?;
            say(printer, format!("Forgot the sign-in of {}.", arg.name));
            Ok(Exit::Done)
        }
        McpCommand::Tools(arg) => tools(home, &tenant, printer, &arg.name, false).await,
        McpCommand::Logs(arg) => tools(home, &tenant, printer, &arg.name, true).await,
        McpCommand::Review(args) => review(home, &tenant, printer, args).await,
        McpCommand::Trust(args) => trust(home, &tenant, printer, args).await,
        McpCommand::Enable(args) => enable(home, &tenant, printer, args).await,
        McpCommand::Disable(arg) => {
            let session = open(home, &tenant, true)?;
            session.hub.set_enabled(&arg.name, false).await?;
            say(printer, format!("{} is off. Its tools are no longer offered.", arg.name));
            Ok(Exit::Done)
        }
        McpCommand::Remove(arg) => {
            let session = open(home, &tenant, true)?;
            session.hub.remove_server(&arg.name).await?;
            say(printer, format!("Removed {} and the secrets kept for it.", arg.name));
            Ok(Exit::Done)
        }
        McpCommand::Import(args) => import_servers(home, &tenant, printer, args).await,
    }
}

fn catalog(printer: Printer, search: Option<&str>) -> Result<Exit, CliError> {
    let needle = search.map(str::to_lowercase);
    let entries: Vec<_> = zorix_mcp::catalog()
        .entries
        .iter()
        .filter(|e| {
            needle.as_ref().is_none_or(|n| {
                [&e.id, &e.name, &e.summary, &e.category].iter().any(|text| text.to_lowercase().contains(n.as_str()))
            })
        })
        .collect();
    if printer.json {
        out(&entries)?;
        return Ok(Exit::Done);
    }
    if entries.is_empty() {
        eprintln!("Nothing in the catalogue matches.");
        return Ok(Exit::Done);
    }
    for e in entries {
        let how = match &e.transport {
            zorix_mcp::catalog::CatalogTransport::Stdio { .. } => "runs here",
            zorix_mcp::catalog::CatalogTransport::Http { .. } => "remote",
        };
        println!("{:<14} {:<10} {:<14} {}", e.id, how, e.category, shorten(&e.summary, 80));
    }
    eprintln!("\nAdd one with `zorix mcp add --catalog <id>`.");
    Ok(Exit::Done)
}

fn kv(text: &str, what: &str) -> Result<(String, String), CliError> {
    match text.split_once('=') {
        Some((k, v)) if !k.is_empty() => Ok((k.to_string(), v.to_string())),
        _ => Err(CliError(format!("{what} is NAME=VALUE; got “{text}”."))),
    }
}

fn describe(transport: &McpTransport) -> String {
    match transport {
        McpTransport::Stdio { command, args, .. } => {
            format!("runs `{} {}` on this computer, as you", command, args.join(" ")).trim_end().to_string()
        }
        McpTransport::Http { url, auth } => format!(
            "connects to {url} ({})",
            match auth {
                HttpAuth::None => "no sign-in",
                HttpAuth::Bearer => "token",
                HttpAuth::OAuth { .. } => "browser sign-in",
            }
        ),
    }
}

async fn add_server(home: &Path, tenant: &TenantId, printer: Printer, args: McpAddArgs) -> Result<Exit, CliError> {
    let mut secrets: Vec<(Option<String>, Secret)> = Vec::new();
    let mut notes: Vec<String> = Vec::new();
    let new = if let Some(id) = &args.catalog {
        let entry = zorix_mcp::catalog().entries.iter().find(|e| &e.id == id).ok_or_else(|| {
            CliError(format!("The catalogue has no entry called {id}. `zorix mcp catalog` lists them."))
        })?;
        let mut inputs = HashMap::new();
        for text in &args.inputs {
            let (k, v) = kv(text, "--input")?;
            let is_folder = match &entry.transport {
                zorix_mcp::catalog::CatalogTransport::Stdio { inputs, .. } => {
                    inputs.iter().any(|i| i.name == k && i.kind == "folder")
                }
                _ => false,
            };
            let v = if is_folder {
                std::fs::canonicalize(&v)
                    .map_err(|e| CliError(format!("The folder {v} cannot be used: {e}")))?
                    .to_string_lossy()
                    .into_owned()
            } else {
                v
            };
            inputs.insert(k, v);
        }
        let (new, needed) = entry.to_new_server(args.name.as_deref(), &inputs).map_err(CliError)?;
        if let Some(requires) = &entry.requires {
            notes.push(format!("It needs {requires} on this computer."));
        }
        if let Some(extra) = &entry.notes {
            notes.push(extra.clone());
        }
        for need in needed {
            match need {
                SecretNeeded::Env { name, label } => secrets.push((Some(name.clone()), env_secret(&name, &label)?)),
                SecretNeeded::Token => {
                    if let Some(help) = &entry.token_help {
                        notes.push(format!("Token: {help}"));
                    }
                    secrets.push((None, read_secret(&args.secret, "token", false)?.expect("required")));
                }
                SecretNeeded::SignIn => notes.push(format!("Sign in with `zorix mcp login {}`.", new.name)),
            }
        }
        new
    } else if let Some(command) = &args.command {
        let name = args.name.clone().ok_or_else(|| CliError("Give the server a name.".into()))?;
        let mut env = Vec::new();
        for var in &args.env {
            env.push(EnvVar { name: var.clone(), value: EnvValue::Secret });
            secrets.push((Some(var.clone()), env_secret(var, "Value")?));
        }
        for text in &args.set {
            let (k, v) = kv(text, "--set")?;
            env.push(EnvVar { name: k, value: EnvValue::Literal(v) });
        }
        let cwd = match &args.cwd {
            Some(dir) => Some(
                std::fs::canonicalize(dir)
                    .map_err(|e| CliError(format!("The folder {} cannot be used: {e}", dir.display())))?
                    .to_string_lossy()
                    .into_owned(),
            ),
            None => None,
        };
        NewMcpServer {
            name,
            transport: McpTransport::Stdio { command: command.clone(), args: args.args.clone(), env, cwd },
        }
    } else if let Some(url) = &args.url {
        let name = args.name.clone().ok_or_else(|| CliError("Give the server a name.".into()))?;
        zorix_mcp::http::validate_url(url).map_err(|e| CliError(e.to_string()))?;
        let auth = match args.auth.unwrap_or(McpAuthArg::None) {
            McpAuthArg::None => HttpAuth::None,
            McpAuthArg::Bearer => {
                secrets.push((None, read_secret(&args.secret, "token", false)?.expect("required")));
                HttpAuth::Bearer
            }
            McpAuthArg::Oauth => {
                notes.push(format!("Sign in with `zorix mcp login {name}`."));
                HttpAuth::OAuth { client_id: args.client_id.clone(), scopes: args.scopes.clone() }
            }
        };
        NewMcpServer { name, transport: McpTransport::Http { url: url.clone(), auth } }
    } else {
        return Err(CliError("Say what to add: --catalog <id>, --command <program> or --url <address>.".into()));
    };
    let session = open(home, tenant, true)?;
    let name = new.name.clone();
    let what = describe(&new.transport);
    let server = session.hub.add_server(new, secrets).await?;
    if printer.json {
        out(&server)?;
        return Ok(Exit::Done);
    }
    eprintln!("Added {name}. It is off for now: it {what}.");
    for note in notes {
        eprintln!("  {note}");
    }
    eprintln!("Next: `zorix mcp enable {name}`, then `zorix mcp tools {name}` to see and approve its tools.");
    Ok(Exit::Done)
}

fn kind_of(transport: &McpTransport) -> &'static str {
    match transport {
        McpTransport::Stdio { .. } => "stdio",
        McpTransport::Http { .. } => "http",
    }
}

async fn list_servers(home: &Path, tenant: &TenantId, printer: Printer) -> Result<Exit, CliError> {
    let session = open(home, tenant, false)?;
    let servers = session.hub.servers().await?;
    if printer.json {
        out(&servers)?;
        return Ok(Exit::Done);
    }
    if servers.is_empty() {
        eprintln!("No MCP servers yet. `zorix mcp catalog` shows some to add.");
        return Ok(Exit::Done);
    }
    for server in servers {
        let pins = session.hub.store.tool_pins(tenant, server.id).await?;
        let ready = pins.iter().filter(|p| p.usable()).count();
        let review =
            pins.iter().filter(|p| p.listed && matches!(p.status, PinStatus::New | PinStatus::Changed)).count();
        println!(
            "{:<16} {:<6} {:<4} {:>3} tools ready{}",
            server.name,
            kind_of(&server.transport),
            if server.enabled { "on" } else { "off" },
            ready,
            if review > 0 { format!(", {review} to review") } else { String::new() }
        );
    }
    Ok(Exit::Done)
}

async fn show_server(home: &Path, tenant: &TenantId, printer: Printer, name: &str) -> Result<Exit, CliError> {
    let session = open(home, tenant, false)?;
    let server = session.hub.server(name).await?;
    let pins = session.hub.store.tool_pins(tenant, server.id).await?;
    if printer.json {
        out(
            &json!({"server": server, "tools": pins.iter().map(|p| json!({"tool": p.tool, "status": p.status.as_str(), "trust": p.trust.as_str(), "listed": p.listed})).collect::<Vec<_>>()}),
        )?;
        return Ok(Exit::Done);
    }
    println!("Name       {}", server.name);
    println!("State      {}", if server.enabled { "on" } else { "off" });
    println!("Protocol   {}", server.era.as_deref().unwrap_or("not seen yet"));
    println!("It         {}", describe(&server.transport));
    if let McpTransport::Stdio { env, cwd, .. } = &server.transport {
        for var in env {
            match &var.value {
                EnvValue::Secret => println!("Variable   {} (secret, in the keychain)", var.name),
                EnvValue::Literal(v) => println!("Variable   {}={}", var.name, shorten(v, 60)),
            }
        }
        if let Some(cwd) = cwd {
            println!("Folder     {cwd}");
        }
    }
    for pin in pins.iter().filter(|p| p.listed) {
        println!(
            "Tool       {:<28} {:<9} {}",
            pin.tool,
            pin.status.as_str(),
            if pin.trust == ToolTrust::Ask { "asks" } else { "no ask" }
        );
    }
    Ok(Exit::Done)
}

async fn login(home: &Path, tenant: &TenantId, printer: Printer, name: &str) -> Result<Exit, CliError> {
    let session = open(home, tenant, true)?;
    let cancel = CancellationToken::new();
    let on_signal = cancel.clone();
    tokio::spawn(async move {
        let _ = tokio::signal::ctrl_c().await;
        on_signal.cancel();
    });
    say(printer, "Opening your browser to sign in. Press Ctrl-C to give up.");
    let summary = session.hub.login(name, &SystemBrowser, &cancel).await?;
    if printer.json {
        out(&json!({"issuer": summary.issuer, "scope": summary.scope, "refreshable": summary.refreshable}))?;
    } else {
        eprintln!(
            "Signed in to {name} through {}{}.",
            summary.issuer,
            if summary.refreshable { "; the sign-in renews itself" } else { "; it will need renewing" }
        );
    }
    Ok(Exit::Done)
}

fn state_text(state: &ServerState) -> String {
    match state {
        ServerState::Stopped => "stopped".into(),
        ServerState::Starting => "still starting".into(),
        ServerState::Ready => "ready".into(),
        ServerState::Restarting { attempt } => format!("restarting (attempt {attempt})"),
        ServerState::NeedsLogin { reason } => format!("needs sign-in: {reason}"),
        ServerState::Failed { reason } => format!("failed: {reason}"),
    }
}

/// `tools` and `logs`: start the server, look, and stop it again.
async fn tools(home: &Path, tenant: &TenantId, printer: Printer, name: &str, logs: bool) -> Result<Exit, CliError> {
    let session = open(home, tenant, true)?;
    let server = session.hub.server(name).await?;
    if !server.enabled {
        return Err(CliError(format!("{name} is off. Turn it on first: `zorix mcp enable {name}`.")));
    }
    say(printer, format!("Starting {name}…"));
    let status = session.hub.start_and_wait(name, START_WAIT).await?;
    let pins = session.hub.store.tool_pins(tenant, server.id).await?;
    let log = session.hub.manager.log(server.id);
    session.hub.manager.shutdown().await;
    if logs {
        if printer.json {
            out(&json!({"state": state_text(&status.state), "log": log}))?;
        } else if log.is_empty() {
            eprintln!("{name} wrote nothing on its error stream ({}).", state_text(&status.state));
        } else {
            for line in log {
                println!("{line}");
            }
        }
        return Ok(Exit::Done);
    }
    if printer.json {
        out(&json!({
            "state": state_text(&status.state),
            "tools": pins.iter().filter(|p| p.listed).map(|p| json!({"tool": p.tool, "status": p.status.as_str(), "trust": p.trust.as_str(), "description": p.seen["description"]})).collect::<Vec<_>>(),
        }))?;
        return Ok(if matches!(status.state, ServerState::Ready) { Exit::Done } else { Exit::Failed });
    }
    if !matches!(status.state, ServerState::Ready) {
        // A reason is often a sentence of its own: one full stop, not two.
        eprintln!("{name} is {}.", state_text(&status.state).trim_end_matches('.'));
        return Ok(Exit::Failed);
    }
    let mut pending = 0;
    for pin in pins.iter().filter(|p| p.listed) {
        if matches!(pin.status, PinStatus::New | PinStatus::Changed) {
            pending += 1;
        }
        println!(
            "{:<30} {:<9} {:<7} {}",
            pin.tool,
            pin.status.as_str(),
            if pin.trust == ToolTrust::Ask { "asks" } else { "no ask" },
            shorten(pin.seen["description"].as_str().unwrap_or(""), 70)
        );
    }
    if pending > 0 {
        eprintln!("\n{pending} tool(s) are not offered to runs until you approve them: `zorix mcp review {name}`.");
    }
    Ok(Exit::Done)
}

fn short_json(value: &Value) -> String {
    match value {
        Value::String(s) => shorten(s, 160),
        other => shorten(&other.to_string(), 160),
    }
}

async fn review(home: &Path, tenant: &TenantId, printer: Printer, args: McpReviewArgs) -> Result<Exit, CliError> {
    let session = open(home, tenant, true)?;
    let server = session.hub.server(&args.name).await?;
    let by = crate::follow::decider();
    let acting = args.approve_all || !args.approve.is_empty() || !args.disable.is_empty();
    if acting {
        for tool in &args.approve {
            session.hub.manager.review(server.id, tool, PinAction::approve(), &by).await?;
            say(printer, format!("Approved {tool}."));
        }
        for tool in &args.disable {
            session.hub.manager.review(server.id, tool, PinAction::Disable, &by).await?;
            say(printer, format!("Turned off {tool}."));
        }
        if args.approve_all {
            let n = session.hub.manager.approve_all(server.id, &by).await?;
            say(printer, format!("Approved {n} tool(s)."));
        }
        return Ok(Exit::Done);
    }
    let pins = session.hub.store.tool_pins(tenant, server.id).await?;
    let pending: Vec<_> =
        pins.iter().filter(|p| p.listed && matches!(p.status, PinStatus::New | PinStatus::Changed)).collect();
    if printer.json {
        out(&pending
            .iter()
            .map(|p| json!({"tool": p.tool, "status": p.status.as_str(), "was": p.approved, "now": p.seen}))
            .collect::<Vec<_>>())?;
        return Ok(Exit::Done);
    }
    if pending.is_empty() {
        eprintln!(
            "Nothing to review for {}. (`zorix mcp tools {}` asks the server what it lists now.)",
            args.name, args.name
        );
        return Ok(Exit::Done);
    }
    for pin in pending {
        println!(
            "{} — {}",
            pin.tool,
            if pin.status == PinStatus::New { "new" } else { "CHANGED since you approved it" }
        );
        match (&pin.status, &pin.approved) {
            (PinStatus::Changed, Some(old)) => {
                let keys: std::collections::BTreeSet<&String> =
                    old.as_object().into_iter().chain(pin.seen.as_object()).flat_map(|o| o.keys()).collect();
                for key in keys {
                    let (was, now) = (&old[key.as_str()], &pin.seen[key.as_str()]);
                    if was != now {
                        println!("  {key}\n    was: {}\n    now: {}", short_json(was), short_json(now));
                    }
                }
            }
            _ => {
                println!("  {}", short_json(&pin.seen["description"]));
                if let Some(props) = pin.seen["inputSchema"]["properties"].as_object() {
                    println!("  takes: {}", props.keys().cloned().collect::<Vec<_>>().join(", "));
                }
            }
        }
        println!();
    }
    eprintln!(
        "Approve with `zorix mcp review {0} --approve <tool>` or `--approve-all`; turn one off with `--disable <tool>`.",
        args.name
    );
    Ok(Exit::Done)
}

async fn trust(home: &Path, tenant: &TenantId, printer: Printer, args: McpTrustArgs) -> Result<Exit, CliError> {
    if !args.ask && !args.read_only_without_asking {
        return Err(CliError("Say --read-only-without-asking or --ask.".into()));
    }
    let session = open(home, tenant, true)?;
    let server = session.hub.server(&args.name).await?;
    let level = if args.read_only_without_asking { ToolTrust::ReadOnlyNoAsk } else { ToolTrust::Ask };
    session.hub.manager.set_trust(server.id, &args.tool, level).await?;
    say(
        printer,
        if args.ask {
            format!("{} now asks before every call.", args.tool)
        } else {
            format!("{} will run without asking, as long as the server keeps reporting it as read-only.", args.tool)
        },
    );
    Ok(Exit::Done)
}

async fn enable(home: &Path, tenant: &TenantId, printer: Printer, args: McpEnableArgs) -> Result<Exit, CliError> {
    let session = open(home, tenant, true)?;
    let server: McpServer = session.hub.server(&args.name).await?;
    if server.enabled {
        say(printer, format!("{} is already on.", args.name));
        return Ok(Exit::Done);
    }
    eprintln!("{} {}.", args.name, describe(&server.transport));
    if matches!(server.transport, McpTransport::Stdio { .. }) {
        eprintln!(
            "A program started this way can do anything you can do. Only turn it on if you trust where it came from."
        );
    } else {
        eprintln!(
            "Its tool descriptions are written by whoever runs it; ZORIX shows them to the model and asks you before using any tool."
        );
    }
    if !args.yes {
        if !std::io::stdin().is_terminal() {
            return Err(CliError(
                "Turning a server on needs your confirmation: run this on a terminal, or add --yes.".into(),
            ));
        }
        if !ask_yes("Turn it on?") {
            return Ok(Exit::Cancelled);
        }
    }
    session.hub.set_enabled(&args.name, true).await?;
    say(
        printer,
        format!(
            "{} is on. `zorix mcp tools {}` shows its tools; approve the ones you want with `zorix mcp review {}`.",
            args.name, args.name, args.name
        ),
    );
    Ok(Exit::Done)
}

async fn import_servers(
    home: &Path,
    tenant: &TenantId,
    printer: Printer,
    args: McpImportArgs,
) -> Result<Exit, CliError> {
    let sources: Vec<import::Source> = match args.from {
        Some(ImportSourceArg::Claude) => vec![import::Source::Claude],
        Some(ImportSourceArg::Vscode) => vec![import::Source::Vscode],
        Some(ImportSourceArg::Cursor) => vec![import::Source::Cursor],
        None => vec![import::Source::Claude, import::Source::Vscode, import::Source::Cursor],
    };
    let mut previews = Vec::new();
    for source in sources {
        let paths = match &args.path {
            Some(path) => vec![path.clone()],
            None => source.default_paths().into_iter().filter(|p| p.is_file()).collect(),
        };
        for path in paths {
            match import::read(source, &path) {
                Ok(preview) => previews.push((path, preview)),
                Err(e) => eprintln!("{}: {e}", path.display()),
            }
        }
    }
    if previews.is_empty() {
        say(printer, "No configuration found to import.");
        return Ok(Exit::Done);
    }
    let session = open(home, tenant, args.yes)?;
    let mut total = 0;
    for (path, preview) in &previews {
        println!("{} ({})", path.display(), preview.source.as_str());
        for entry in &preview.entries {
            match (&entry.skipped, &entry.transport) {
                (Some(why), _) => println!("  skip  {:<20} {why}", entry.original_name),
                (None, Some(transport)) => {
                    println!("  add   {:<20} {}", entry.name, describe(transport));
                    for warning in &entry.warnings {
                        println!("        note: {warning}");
                    }
                    if !entry.secrets.is_empty() {
                        println!("        {} secret(s) would move to the keychain", entry.secrets.len());
                    }
                }
                _ => {}
            }
        }
        total += preview.importable().count();
        if args.yes {
            let names: Vec<String> = preview.importable().map(|e| e.name.clone()).collect();
            let store: Arc<dyn zorix_core::IntegrationStore> = session.hub.store.clone();
            let results =
                import::apply(preview, &names, &store, &session.hub.secrets, tenant, zorix_core::SystemClock.now())
                    .await;
            for result in results {
                match result {
                    Ok(done) => println!("  added {} (off), {} secret(s) kept", done.server.name, done.secrets_moved),
                    Err((name, why)) => println!("  FAILED {name}: {why}"),
                }
            }
        }
    }
    if !args.yes && total > 0 {
        eprintln!("\nNothing was changed. Add `--yes` to import these; they start off, and you turn each on yourself.");
    }
    Ok(Exit::Done)
}

// ---------------------------------------------------------------------------------------------------------------------
// Connections

pub async fn connect(
    home: &Path,
    tenant: TenantId,
    printer: Printer,
    command: ConnectCommand,
) -> Result<Exit, CliError> {
    match command {
        ConnectCommand::List => {
            let session = open(home, &tenant, false)?;
            let all = session.hub.connections().await?;
            if printer.json {
                out(&all)?;
                return Ok(Exit::Done);
            }
            if all.is_empty() {
                eprintln!("No connections yet. `zorix connect add --help` shows what can be added.");
            }
            for c in all {
                let where_ = ["base_url", "host", "url", "path", "site"]
                    .iter()
                    .find_map(|k| c.settings[k].as_str())
                    .unwrap_or("");
                let tested = match &c.last_test {
                    Some(t) if t.ok => "tested ok",
                    Some(_) => "last test failed",
                    None => "not tested",
                };
                println!("{:<16} {:<9} {:<18} {}", c.name, c.kind.as_str(), tested, shorten(where_, 50));
            }
            Ok(Exit::Done)
        }
        ConnectCommand::Test(arg) => {
            let session = open(home, &tenant, true)?;
            let test = session.hub.test_connection(&arg.name).await?;
            if printer.json {
                out(&json!({"ok": test.ok, "note": test.note}))?;
            } else {
                eprintln!("{}: {}", if test.ok { "ok" } else { "failed" }, test.note);
            }
            Ok(if test.ok { Exit::Done } else { Exit::Failed })
        }
        ConnectCommand::Remove(arg) => {
            let session = open(home, &tenant, true)?;
            session.hub.remove_connection(&arg.name).await?;
            say(printer, format!("Removed {} and its secret.", arg.name));
            Ok(Exit::Done)
        }
        ConnectCommand::Add(add) => {
            let (name, request, note) = build_connection(add)?;
            let session = open(home, &tenant, true)?;
            let connection = session.hub.add_connection(request).await?;
            if printer.json {
                out(&connection)?;
            } else {
                eprintln!("Added {name}.");
                if let Some(note) = note {
                    eprintln!("{note}");
                }
                eprintln!("Check it with `zorix connect test {name}`.");
            }
            Ok(Exit::Done)
        }
    }
}

fn request(kind: ConnectionKind, name: &str, settings: Value, secret: Option<Secret>) -> ConnectRequest {
    ConnectRequest { kind, name: name.to_string(), settings, secret }
}

fn build_connection(add: ConnectAdd) -> Result<(String, ConnectRequest, Option<String>), CliError> {
    Ok(match add {
        ConnectAdd::Github(a) => {
            let secret = if a.no_token { None } else { read_secret(&a.secret, "token", true)? };
            let note = secret.is_none().then(|| "No token: only public repositories can be read.".to_string());
            (a.name.clone(), request(ConnectionKind::Github, &a.name, json!({"base_url": a.base_url}), secret), note)
        }
        ConnectAdd::Gitlab(a) => {
            let secret = if a.no_token { None } else { read_secret(&a.secret, "token", true)? };
            (a.name.clone(), request(ConnectionKind::Gitlab, &a.name, json!({"base_url": a.base_url}), secret), None)
        }
        ConnectAdd::Jira(a) => {
            let secret = read_secret(&a.secret, "API token", false)?;
            let settings =
                json!({"base_url": a.site, "email": a.email, "auth": if a.bearer { "bearer" } else { "basic" }});
            (a.name.clone(), request(ConnectionKind::Jira, &a.name, settings, secret), None)
        }
        ConnectAdd::Slack(a) => {
            let secret = read_secret(&a.secret, "bot token", false)?;
            (a.name.clone(), request(ConnectionKind::Slack, &a.name, json!({"base_url": a.base_url}), secret), None)
        }
        ConnectAdd::Discord(a) => {
            let secret = read_secret(&a.secret, "bot token", false)?;
            (a.name.clone(), request(ConnectionKind::Discord, &a.name, json!({"base_url": a.base_url}), secret), None)
        }
        ConnectAdd::Postgres(a) => {
            let secret = if a.no_password { None } else { read_secret(&a.secret, "password", true)? };
            let ca = match &a.ca_file {
                Some(file) => Some(
                    std::fs::canonicalize(file)
                        .map_err(|e| CliError(format!("The certificate file {} cannot be used: {e}", file.display())))?
                        .to_string_lossy()
                        .into_owned(),
                ),
                None => None,
            };
            let settings = json!({"host": a.host, "port": a.port, "database": a.database, "user": a.user, "sslmode": a.sslmode, "ca_file": ca});
            (
                a.name.clone(),
                request(ConnectionKind::Postgres, &a.name, settings, secret),
                Some(
                    "Queries run in read-only transactions; writes need `--allow-db <name>:write` and always ask."
                        .into(),
                ),
            )
        }
        ConnectAdd::Sqlite(a) => (
            a.name.clone(),
            request(ConnectionKind::Sqlite, &a.name, json!({"path": a.path.to_string_lossy()}), None),
            Some("The file is opened read-only.".into()),
        ),
        ConnectAdd::Http(a) => {
            let secret = read_secret(&a.secret, "credential", false)?;
            let settings = json!({"host": a.host, "auth": a.auth, "header": a.header, "username": a.username});
            (
                a.name.clone(),
                request(ConnectionKind::Http, &a.name, settings, secret),
                Some(format!(
                    "The credential is sent only to {}, over HTTPS. Runs still need `--allow-http {}`.",
                    a.host, a.host
                )),
            )
        }
        ConnectAdd::Webhook(a) => {
            let (secret, note) = match read_secret(&a.secret, "signing key", true)? {
                Some(secret) => (secret, None),
                None => {
                    let mut bytes = [0u8; 32];
                    getrandom::fill(&mut bytes).map_err(|e| CliError(format!("Could not make a signing key: {e}")))?;
                    let key: String = bytes.iter().map(|b| format!("{b:02x}")).collect();
                    let note = format!(
                        "Signing key (shown once, give it to the receiver so it can check the X-Zorix-Signature header): {key}"
                    );
                    (Secret::new(key), Some(note))
                }
            };
            (a.name.clone(), request(ConnectionKind::Webhook, &a.name, json!({"url": a.url}), Some(secret)), note)
        }
    })
}

// ---------------------------------------------------------------------------------------------------------------------
// Sandbox

pub async fn sandbox(
    home: &Path,
    tenant: TenantId,
    printer: Printer,
    command: SandboxCommand,
) -> Result<Exit, CliError> {
    let session = open(home, &tenant, matches!(command, SandboxCommand::Install(_)))?;
    let sandbox = &session.hub.sandbox;
    match command {
        SandboxCommand::Status => {
            let python = sandbox.python_installed();
            if printer.json {
                out(&json!({"javascript": true, "python": python, "folder": sandbox.folder()}))?;
                return Ok(Exit::Done);
            }
            println!("javascript   ready (QuickJS, built in)");
            if python {
                println!("python       ready (CPython 3.12, {})", sandbox.python_file().display());
            } else {
                println!("python       not installed: `zorix sandbox install python` downloads about 25 MB");
            }
            println!("Limits       256 MB of memory, 10 s of time, 1 MB of output, 50 MB of scratch files, no network");
            Ok(Exit::Done)
        }
        SandboxCommand::Install(SandboxInstall::Python) => {
            if sandbox.python_installed() {
                say(printer, "Python is already installed.");
                return Ok(Exit::Done);
            }
            eprintln!("Downloading CPython for WebAssembly from {} …", zorix_sandbox::runner::PYTHON_URL);
            let last = std::sync::atomic::AtomicU64::new(0);
            sandbox
                .install_python(|done, total| {
                    let step = done * 10 / total.max(1);
                    if last.swap(step, std::sync::atomic::Ordering::Relaxed) != step {
                        eprintln!("  {}%", step * 10);
                    }
                })
                .await
                .map_err(|e| CliError(e.to_string()))?;
            say(printer, "Installed. The file matched its expected checksum.");
            Ok(Exit::Done)
        }
    }
}
