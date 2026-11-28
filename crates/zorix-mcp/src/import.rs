//! Bringing in the servers another app already knows (Claude Desktop, VS Code, Cursor), only when a person asks and only
//! after showing them what would be added (ADR-0018).
//!
//! Reading a config is pure: it returns a preview. Nothing is saved until the person confirms, secrets found in the
//! config are moved to the keychain and are not part of what is stored, and entries that cannot be used are listed with
//! the reason instead of being dropped silently.

use std::collections::HashSet;
use std::path::{Path, PathBuf};
use std::sync::Arc;

use serde_json::Value;
use zorix_core::{
    EnvValue, EnvVar, HttpAuth, IntegrationStore, McpServer, McpTransport, NewMcpServer, Secret, TenantId, Timestamp,
    redact_text, valid_name,
};
use zorix_providers::SecretStore;

use crate::http::validate_url;
use crate::manager::{env_secret_name, token_secret_name};

/// The largest config file that is read.
pub const MAX_CONFIG_BYTES: usize = 1024 * 1024;
/// The most entries of one config that are looked at.
pub const MAX_ENTRIES: usize = 200;

/// Where servers can be imported from.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Source {
    /// Claude Desktop's `claude_desktop_config.json`.
    Claude,
    /// VS Code's `mcp.json`.
    Vscode,
    /// Cursor's `~/.cursor/mcp.json`.
    Cursor,
}

impl Source {
    /// The name used on the command line.
    pub fn as_str(&self) -> &'static str {
        match self {
            Source::Claude => "claude",
            Source::Vscode => "vscode",
            Source::Cursor => "cursor",
        }
    }

    /// Read a name.
    pub fn parse(text: &str) -> Option<Self> {
        match text {
            "claude" => Some(Source::Claude),
            "vscode" => Some(Source::Vscode),
            "cursor" => Some(Source::Cursor),
            _ => None,
        }
    }

    /// Where the config usually is on this computer.
    pub fn default_paths(&self) -> Vec<PathBuf> {
        let home = std::env::var_os("HOME").or_else(|| std::env::var_os("USERPROFILE")).map(PathBuf::from);
        let appdata = std::env::var_os("APPDATA").map(PathBuf::from);
        let mut paths = Vec::new();
        match self {
            Source::Cursor => paths.extend(home.map(|h| h.join(".cursor").join("mcp.json"))),
            Source::Claude | Source::Vscode => {
                let folder = if *self == Source::Claude { "Claude" } else { "Code" };
                let file = if *self == Source::Claude { "claude_desktop_config.json" } else { "mcp.json" };
                let tail: PathBuf = if *self == Source::Claude {
                    [folder, file].iter().collect()
                } else {
                    [folder, "User", file].iter().collect()
                };
                if cfg!(target_os = "macos") {
                    paths.extend(home.map(|h| h.join("Library").join("Application Support").join(&tail)));
                } else if cfg!(windows) {
                    paths.extend(appdata.map(|a| a.join(&tail)));
                } else {
                    paths.extend(home.map(|h| h.join(".config").join(&tail)));
                }
            }
        }
        paths
    }
}

/// One server found in a config.
#[derive(Clone, Debug)]
pub struct ImportEntry {
    /// The name it had there.
    pub original_name: String,
    /// The name it would get here (valid and unique).
    pub name: String,
    /// How it would be reached. Holds names of secrets, never their values.
    pub transport: Option<McpTransport>,
    /// Values to move to the keychain: `(environment variable name, value)`. The token of a remote server is under `""`.
    pub secrets: Vec<(String, Secret)>,
    /// Things worth knowing.
    pub warnings: Vec<String>,
    /// Why it cannot be imported, when it cannot.
    pub skipped: Option<String>,
}

/// What would be imported.
#[derive(Clone, Debug)]
pub struct Preview {
    /// Where it came from.
    pub source: Source,
    /// What was found, in the order it was listed.
    pub entries: Vec<ImportEntry>,
}

impl Preview {
    /// The entries that can be imported.
    pub fn importable(&self) -> impl Iterator<Item = &ImportEntry> {
        self.entries.iter().filter(|e| e.skipped.is_none())
    }
}

fn looks_secret(name: &str, value: &str) -> bool {
    let upper = name.to_ascii_uppercase();
    ["KEY", "TOKEN", "SECRET", "PASSWORD", "PASSWD", "CREDENTIAL", "AUTH", "COOKIE"].iter().any(|w| upper.contains(w))
        || redact_text(value) != value
}

fn sanitise(original: &str, taken: &mut HashSet<String>) -> String {
    let mut name: String = original
        .to_lowercase()
        .chars()
        .map(|c| if c.is_ascii_lowercase() || c.is_ascii_digit() || c == '-' || c == '_' { c } else { '-' })
        .collect();
    name = name.trim_matches(['-', '_']).to_string();
    if name.is_empty() {
        name = "server".into();
    }
    name.truncate(32);
    let base = name.clone();
    let mut n = 2;
    while !valid_name(&name) || taken.contains(&name) {
        let suffix = format!("-{n}");
        name = format!("{}{suffix}", &base[..base.len().min(32 - suffix.len())]);
        n += 1;
    }
    taken.insert(name.clone());
    name
}

fn strings(value: Option<&Value>) -> Vec<String> {
    value
        .and_then(Value::as_array)
        .map(|a| a.iter().filter_map(|v| v.as_str().map(str::to_string)).collect())
        .unwrap_or_default()
}

fn entry(original: &str, raw: &Value, taken: &mut HashSet<String>) -> ImportEntry {
    let mut found = ImportEntry {
        original_name: original.to_string(),
        name: sanitise(original, taken),
        transport: None,
        secrets: Vec::new(),
        warnings: Vec::new(),
        skipped: None,
    };
    let Some(object) = raw.as_object() else {
        found.skipped = Some("it is not a server definition".into());
        return found;
    };
    let kind =
        object.get("type").and_then(Value::as_str).unwrap_or(if object.contains_key("url") { "http" } else { "stdio" });
    if object.get("disabled") == Some(&Value::Bool(true)) {
        found.warnings.push("it was turned off there; it will be added turned off here too".into());
    }
    match kind {
        "stdio" => {
            let Some(command) = object.get("command").and_then(Value::as_str).filter(|c| !c.trim().is_empty()) else {
                found.skipped = Some("it has no command".into());
                return found;
            };
            // A command written as one line with its arguments ("npx -y pkg") is not a program name.
            let (command, mut args) = if command.contains(' ') && object.get("args").is_none() {
                let mut parts = command.split_whitespace().map(str::to_string);
                (parts.next().unwrap_or_default(), parts.collect::<Vec<_>>())
            } else {
                (command.to_string(), Vec::new())
            };
            args.extend(strings(object.get("args")));
            if args.iter().chain(std::iter::once(&command)).any(|a| a.contains("${")) {
                found.skipped = Some("it refers to a variable (${…}) that ZORIX cannot fill in; add it by hand".into());
                return found;
            }
            let mut env = Vec::new();
            if let Some(vars) = object.get("env").and_then(Value::as_object) {
                for (name, value) in vars {
                    let Some(value) = value.as_str() else { continue };
                    if value.contains("${") {
                        found.warnings.push(format!("{name} refers to a variable (${{…}}); it was left out"));
                    } else if looks_secret(name, value) && !value.is_empty() {
                        env.push(EnvVar { name: name.clone(), value: EnvValue::Secret });
                        found.secrets.push((name.clone(), Secret::new(value)));
                    } else {
                        env.push(EnvVar { name: name.clone(), value: EnvValue::Literal(value.to_string()) });
                    }
                }
            }
            found.transport = Some(McpTransport::Stdio {
                command,
                args,
                env,
                cwd: object.get("cwd").and_then(Value::as_str).map(str::to_string),
            });
        }
        "http" | "streamable-http" | "streamableHttp" => {
            let url = object.get("url").and_then(Value::as_str).unwrap_or("");
            if url.contains("${") {
                found.skipped = Some("its address refers to a variable (${…}); add it by hand".into());
                return found;
            }
            if url.trim_end_matches('/').ends_with("/sse") {
                found.skipped = Some("it uses the older HTTP+SSE transport, which ZORIX does not support".into());
                return found;
            }
            if let Err(e) = validate_url(url) {
                found.skipped = Some(e.to_string());
                return found;
            }
            let mut auth = HttpAuth::OAuth { client_id: None, scopes: vec![] };
            if let Some(headers) = object.get("headers").and_then(Value::as_object) {
                for (header, value) in headers {
                    let value = value.as_str().unwrap_or("");
                    if header.eq_ignore_ascii_case("authorization")
                        && value.to_ascii_lowercase().starts_with("bearer ")
                        && !value.contains("${")
                    {
                        auth = HttpAuth::Bearer;
                        found.secrets.push((String::new(), Secret::new(value[7..].trim())));
                    } else {
                        found.warnings.push(format!("the header {header} is not supported and was left out"));
                    }
                }
            }
            // A server with no token is signed in to with the browser when it asks for it.
            found.transport = Some(McpTransport::Http { url: url.to_string(), auth });
        }
        "sse" => found.skipped = Some("it uses the older HTTP+SSE transport, which ZORIX does not support".into()),
        other => found.skipped = Some(format!("its type “{other}” is not supported")),
    }
    found
}

/// Read a config and say what it holds.
pub fn parse(source: Source, text: &str) -> Result<Preview, String> {
    if text.len() > MAX_CONFIG_BYTES {
        return Err("The file is larger than 1 MB, so it was not read.".into());
    }
    let value: Value = serde_json::from_str(text).map_err(|e| format!("The file is not valid JSON ({e})."))?;
    let key = if source == Source::Vscode { "servers" } else { "mcpServers" };
    let servers = value
        .get(key)
        .or_else(|| value.get("mcpServers"))
        .or_else(|| value.get("servers"))
        .and_then(Value::as_object)
        .ok_or_else(|| format!("No “{key}” section was found in the file."))?;
    let mut taken = HashSet::new();
    let mut entries = Vec::new();
    for (name, raw) in servers.iter().take(MAX_ENTRIES) {
        entries.push(entry(name, raw, &mut taken));
    }
    if servers.len() > MAX_ENTRIES {
        return Err(format!("The file lists {} servers; ZORIX reads at most {MAX_ENTRIES}.", servers.len()));
    }
    Ok(Preview { source, entries })
}

/// Read a config file.
pub fn read(source: Source, path: &Path) -> Result<Preview, String> {
    let meta = std::fs::metadata(path).map_err(|e| format!("{} could not be read: {e}", path.display()))?;
    if meta.len() as usize > MAX_CONFIG_BYTES {
        return Err("The file is larger than 1 MB, so it was not read.".into());
    }
    let text = std::fs::read_to_string(path).map_err(|e| format!("{} could not be read: {e}", path.display()))?;
    parse(source, &text)
}

/// What importing did to one entry.
#[derive(Clone, Debug)]
pub struct Imported {
    /// The server that was added (disabled).
    pub server: McpServer,
    /// Environment variables or tokens that went to the keychain.
    pub secrets_moved: usize,
}

/// Add the chosen entries. Servers are added **disabled**; a person enables each one after seeing what it is. Secrets go to
/// the keychain; if one cannot be kept, that server is not added and the reason is returned.
pub async fn apply(
    preview: &Preview,
    names: &[String],
    store: &Arc<dyn IntegrationStore>,
    secrets: &Arc<dyn SecretStore>,
    tenant: &TenantId,
    now: Timestamp,
) -> Vec<Result<Imported, (String, String)>> {
    let mut existing: HashSet<String> =
        store.mcp_servers(tenant).await.map(|s| s.into_iter().map(|s| s.name).collect()).unwrap_or_default();
    let mut results = Vec::new();
    for found in preview.importable().filter(|e| names.iter().any(|n| n == &e.name || n == &e.original_name)) {
        let Some(transport) = found.transport.clone() else { continue };
        let mut name = found.name.clone();
        let mut n = 2;
        while existing.contains(&name) {
            name = format!("{}-{n}", &found.name[..found.name.len().min(28)]);
            n += 1;
        }
        let added = match store.add_mcp_server(tenant, NewMcpServer { name: name.clone(), transport }, now).await {
            Ok(server) => {
                // The next entry of this same import must not take the name just given out.
                existing.insert(server.name.clone());
                server
            }
            Err(e) => {
                results.push(Err((found.original_name.clone(), e.to_string())));
                continue;
            }
        };
        let mut moved = 0;
        let mut failure = None;
        for (var, value) in &found.secrets {
            let key = if var.is_empty() { token_secret_name(added.id) } else { env_secret_name(added.id, var) };
            let kept = {
                let secrets = secrets.clone();
                let value = value.clone();
                tokio::task::spawn_blocking(move || secrets.set(&key, &value)).await
            };
            match kept {
                Ok(Ok(())) => moved += 1,
                Ok(Err(e)) => {
                    failure = Some(format!("{}: {e}", if var.is_empty() { "the token" } else { var.as_str() }))
                }
                Err(e) => failure = Some(e.to_string()),
            }
            if failure.is_some() {
                break;
            }
        }
        match failure {
            None => results.push(Ok(Imported { server: added, secrets_moved: moved })),
            Some(why) => {
                // Nothing half-made is left behind.
                for (var, _) in &found.secrets {
                    let key = if var.is_empty() { token_secret_name(added.id) } else { env_secret_name(added.id, var) };
                    let _ = secrets.remove(&key);
                }
                let _ = store.remove_mcp_server(tenant, added.id).await;
                results.push(Err((
                    found.original_name.clone(),
                    format!("a secret could not be kept in the keychain ({why})"),
                )));
            }
        }
    }
    results
}

#[cfg(test)]
mod tests {
    use super::*;

    const CLAUDE: &str = r#"{
      "mcpServers": {
        "GitHub": {"command": "npx", "args": ["-y", "@modelcontextprotocol/server-github"], "env": {"GITHUB_PERSONAL_ACCESS_TOKEN": "ghp_abcdefghijklmnopqrstuvwxyz0123456789", "LOG_LEVEL": "info"}},
        "My Files!": {"command": "npx -y @modelcontextprotocol/server-filesystem /Users/me/notes"},
        "github": {"command": "node", "args": ["server.js"]},
        "remote": {"url": "https://mcp.example.com/mcp", "headers": {"Authorization": "Bearer abc.def.ghi", "X-Team": "42"}},
        "oauth-one": {"type": "http", "url": "https://mcp.linear.app/mcp"},
        "old": {"type": "sse", "url": "https://old.example.com/sse"},
        "plain": {"url": "http://mcp.example.com/mcp"},
        "templated": {"command": "node", "args": ["${workspaceFolder}/x.js"]},
        "empty": {},
        "broken": "nope"
      }
    }"#;

    #[test]
    fn a_config_becomes_a_preview_that_says_what_each_entry_would_be_and_why_some_cannot() {
        let preview = parse(Source::Claude, CLAUDE).unwrap();
        let by = |name: &str| preview.entries.iter().find(|e| e.original_name == name).unwrap();
        // Names are made valid and unique, and the original is kept for display.
        assert_eq!(
            (by("GitHub").name.as_str(), by("github").name.as_str(), by("My Files!").name.as_str()),
            ("github", "github-2", "my-files")
        );
        // A secret in the environment is named in the transport and carried separately.
        let github = by("GitHub");
        match github.transport.as_ref().unwrap() {
            McpTransport::Stdio { env, .. } => {
                assert!(env.contains(&EnvVar { name: "GITHUB_PERSONAL_ACCESS_TOKEN".into(), value: EnvValue::Secret }));
                assert!(env.contains(&EnvVar { name: "LOG_LEVEL".into(), value: EnvValue::Literal("info".into()) }));
            }
            other => panic!("{other:?}"),
        }
        assert!(!format!("{:?}", github.transport).contains("ghp_"), "the transport holds no secret");
        assert_eq!(github.secrets.len(), 1);
        assert!(!format!("{github:?}").contains("ghp_"), "and a preview never prints one");
        // A one-line command is split.
        match by("My Files!").transport.as_ref().unwrap() {
            McpTransport::Stdio { command, args, .. } => assert_eq!((command.as_str(), args.len()), ("npx", 3)),
            other => panic!("{other:?}"),
        }
        // A bearer header becomes a token; other headers are named and left out.
        let remote = by("remote");
        assert!(matches!(remote.transport, Some(McpTransport::Http { auth: HttpAuth::Bearer, .. })));
        assert!(remote.warnings.iter().any(|w| w.contains("X-Team")));
        assert!(matches!(by("oauth-one").transport, Some(McpTransport::Http { auth: HttpAuth::OAuth { .. }, .. })));
        // What cannot be imported says why.
        for (name, why) in [
            ("old", "HTTP+SSE"),
            ("plain", "plain HTTP"),
            ("templated", "variable"),
            ("empty", "no command"),
            ("broken", "not a server definition"),
        ] {
            assert!(by(name).skipped.as_deref().is_some_and(|s| s.contains(why)), "{name}: {:?}", by(name).skipped);
        }
        assert_eq!(preview.importable().count(), 5);
    }

    #[test]
    fn the_other_apps_use_their_own_layout_and_bad_files_are_refused_with_a_reason() {
        let vscode = r#"{"servers": {"a": {"type": "stdio", "command": "uvx", "args": ["x"]}, "b": {"type": "http", "url": "https://x.example/mcp"}}, "inputs": []}"#;
        assert_eq!(parse(Source::Vscode, vscode).unwrap().importable().count(), 2);
        assert_eq!(
            parse(Source::Cursor, r#"{"mcpServers": {"a": {"url": "https://x.example/mcp"}}}"#)
                .unwrap()
                .importable()
                .count(),
            1
        );
        assert!(parse(Source::Claude, "not json").unwrap_err().contains("not valid JSON"));
        assert!(parse(Source::Claude, "{}").unwrap_err().contains("No “mcpServers”"));
        assert!(parse(Source::Claude, &" ".repeat(MAX_CONFIG_BYTES + 1)).unwrap_err().contains("larger than 1 MB"));
        let many: String =
            (0..=MAX_ENTRIES).map(|i| format!("\"s{i}\": {{\"command\": \"x\"}}")).collect::<Vec<_>>().join(",");
        assert!(parse(Source::Claude, &format!("{{\"mcpServers\": {{{many}}}}}")).unwrap_err().contains("at most"));
    }

    #[test]
    fn default_places_are_named_for_each_app_and_every_source_has_a_name() {
        for source in [Source::Claude, Source::Vscode, Source::Cursor] {
            assert_eq!(Source::parse(source.as_str()), Some(source));
            assert!(source.default_paths().iter().all(|p| p.is_absolute()));
        }
        assert!(
            Source::Cursor
                .default_paths()
                .iter()
                .any(|p| p.ends_with(".cursor/mcp.json") || p.ends_with(".cursor\\mcp.json"))
                || Source::Cursor.default_paths().is_empty()
        );
        assert!(
            Source::Claude.default_paths().iter().all(|p| p.to_string_lossy().contains("claude_desktop_config.json"))
        );
    }
}
