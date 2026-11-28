//! A short, reviewed list of MCP servers people commonly add (ADR-0018). It is data, not code: adding one is a change to
//! `catalog.json`, and the tests check that every entry is usable (valid addresses, names, inputs).

use std::collections::HashMap;
use std::sync::OnceLock;

use serde::{Deserialize, Serialize};
use zorix_core::{EnvValue, EnvVar, HttpAuth, McpTransport, NewMcpServer, valid_name};

/// How an entry's server is reached.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum CatalogTransport {
    /// A program ZORIX starts.
    Stdio {
        /// The program.
        command: String,
        /// Its arguments; `{name}` is replaced by the input of that name.
        args: Vec<String>,
        /// Environment variables it needs.
        #[serde(default)]
        env: Vec<CatalogEnv>,
        /// What a person has to fill in.
        #[serde(default)]
        inputs: Vec<CatalogInput>,
    },
    /// A remote server.
    Http {
        /// Its address.
        url: String,
        /// How to sign in: `none`, `bearer` or `oauth`.
        auth: CatalogAuth,
    },
}

/// How to sign in to a remote catalogue server.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum CatalogAuth {
    /// No sign-in.
    None,
    /// A token the person pastes.
    Bearer,
    /// OAuth in the browser.
    Oauth,
}

/// An environment variable a catalogue server needs.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct CatalogEnv {
    /// Its name.
    pub name: String,
    /// What it is, for the person.
    pub label: String,
    /// Kept in the keychain.
    #[serde(default)]
    pub secret: bool,
}

/// Something a person fills in before the server can be added.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct CatalogInput {
    /// Its name, as it appears in `{braces}` in the arguments.
    pub name: String,
    /// What to ask.
    pub label: String,
    /// `folder` or `text`.
    pub kind: String,
}

/// One server of the catalogue.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct CatalogEntry {
    /// A stable id.
    pub id: String,
    /// What people call it.
    pub name: String,
    /// What it does, in a sentence.
    pub summary: String,
    /// A group for the interface.
    pub category: String,
    /// Where to read more.
    pub homepage: String,
    /// The name of its logo.
    pub logo: String,
    /// The name it gets in ZORIX unless the person chooses another.
    pub default_name: String,
    /// How to reach it.
    pub transport: CatalogTransport,
    /// What a person needs on this computer.
    #[serde(default)]
    pub requires: Option<String>,
    /// How to get a token, for `bearer` servers.
    #[serde(default)]
    pub token_help: Option<String>,
    /// Anything a person should know first.
    #[serde(default)]
    pub notes: Option<String>,
}

/// The whole catalogue.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Catalog {
    /// The layout version.
    pub version: u32,
    /// When the entries were last checked against the real servers.
    pub checked: String,
    /// The entries.
    pub entries: Vec<CatalogEntry>,
}

/// The package a launcher command runs, without its version: the first argument that is not a flag. `npx -y @playwright/mcp@latest`
/// and `npx -y @playwright/mcp@0.0.86` both run `@playwright/mcp`.
fn package_of(args: &[String]) -> Option<&str> {
    let spec = args.iter().find(|a| !a.starts_with('-'))?;
    // The version follows the last `@`; the `@` that starts a scoped name is part of the name.
    match spec.rfind('@') {
        Some(at) if at > 0 => Some(&spec[..at]),
        _ => Some(spec),
    }
}

impl Catalog {
    /// The entry a server was made from, when its address, or its command and package, still say so. A version, a folder a person
    /// chose or a flag do not change which entry it is; another package does.
    pub fn entry_for(&self, transport: &McpTransport) -> Option<&CatalogEntry> {
        self.entries.iter().find(|entry| match (&entry.transport, transport) {
            (CatalogTransport::Http { url, .. }, McpTransport::Http { url: mine, .. }) => {
                url.trim_end_matches('/') == mine.trim_end_matches('/')
            }
            (
                CatalogTransport::Stdio { command, args, .. },
                McpTransport::Stdio { command: mine, args: my_args, .. },
            ) => command == mine && package_of(args).is_some_and(|package| package_of(my_args) == Some(package)),
            _ => false,
        })
    }
}

/// The catalogue that ships with ZORIX.
pub fn catalog() -> &'static Catalog {
    static CATALOG: OnceLock<Catalog> = OnceLock::new();
    CATALOG.get_or_init(|| {
        serde_json::from_str(include_str!("../catalog.json")).expect("catalog.json is valid (a test checks it)")
    })
}

/// A secret the person still has to provide for a server that was just made from an entry.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum SecretNeeded {
    /// An environment variable of a stdio server.
    Env {
        /// Its name.
        name: String,
        /// What it is.
        label: String,
    },
    /// A bearer token for a remote server.
    Token,
    /// A browser sign-in.
    SignIn,
}

impl CatalogEntry {
    /// The server this entry describes, with the person's inputs filled in, and what is still needed from them.
    pub fn to_new_server(
        &self,
        name: Option<&str>,
        inputs: &HashMap<String, String>,
    ) -> Result<(NewMcpServer, Vec<SecretNeeded>), String> {
        let name = name.unwrap_or(&self.default_name).to_string();
        if !valid_name(&name) {
            return Err(format!("“{name}” is not a valid name: use 1 to 32 lowercase letters, digits, '-' or '_'."));
        }
        match &self.transport {
            CatalogTransport::Http { url, auth } => {
                let (auth, needed) = match auth {
                    CatalogAuth::None => (HttpAuth::None, vec![]),
                    CatalogAuth::Bearer => (HttpAuth::Bearer, vec![SecretNeeded::Token]),
                    CatalogAuth::Oauth => {
                        (HttpAuth::OAuth { client_id: None, scopes: vec![] }, vec![SecretNeeded::SignIn])
                    }
                };
                Ok((NewMcpServer { name, transport: McpTransport::Http { url: url.clone(), auth } }, needed))
            }
            CatalogTransport::Stdio { command, args, env, inputs: wanted } => {
                for input in wanted {
                    if inputs.get(&input.name).is_none_or(|v| v.trim().is_empty()) {
                        return Err(format!("{} is needed: {}.", input.name, input.label));
                    }
                }
                let args = args
                    .iter()
                    .map(|arg| {
                        let mut arg = arg.clone();
                        for (key, value) in inputs {
                            arg = arg.replace(&format!("{{{key}}}"), value.trim());
                        }
                        arg
                    })
                    .collect();
                let mut needed = Vec::new();
                let env = env
                    .iter()
                    .map(|var| {
                        if var.secret {
                            needed.push(SecretNeeded::Env { name: var.name.clone(), label: var.label.clone() });
                        }
                        EnvVar { name: var.name.clone(), value: EnvValue::Secret }
                    })
                    .collect();
                Ok((
                    NewMcpServer {
                        name,
                        transport: McpTransport::Stdio { command: command.clone(), args, env, cwd: None },
                    },
                    needed,
                ))
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::http::validate_url;

    #[test]
    fn the_catalogue_that_ships_is_valid_and_every_entry_is_usable() {
        let catalog = catalog();
        assert_eq!(catalog.version, 1);
        assert!(catalog.entries.len() >= 8);
        let mut ids = std::collections::HashSet::new();
        for entry in &catalog.entries {
            assert!(ids.insert(entry.id.clone()), "{} appears twice", entry.id);
            assert!(valid_name(&entry.default_name), "{}: {}", entry.id, entry.default_name);
            for text in [&entry.name, &entry.summary, &entry.category, &entry.logo] {
                assert!(!text.trim().is_empty(), "{}", entry.id);
            }
            assert!(entry.homepage.starts_with("https://"), "{}", entry.id);
            match &entry.transport {
                CatalogTransport::Http { url, auth } => {
                    validate_url(url).unwrap_or_else(|e| panic!("{}: {e}", entry.id));
                    assert!(url.starts_with("https://"), "{}: remote servers use HTTPS", entry.id);
                    if *auth == CatalogAuth::Bearer {
                        assert!(entry.token_help.is_some(), "{} needs to say where to get a token", entry.id);
                    }
                }
                CatalogTransport::Stdio { command, args, inputs, .. } => {
                    assert!(!command.is_empty() && !command.contains(' '), "{}: a command is one program", entry.id);
                    assert!(entry.requires.is_some(), "{} should say what it needs on this computer", entry.id);
                    for arg in args.iter().filter(|a| a.starts_with('{')) {
                        let name = arg.trim_matches(['{', '}']);
                        assert!(inputs.iter().any(|i| i.name == name), "{}: {arg} has no input", entry.id);
                    }
                }
            }
        }
    }

    fn stdio(command: &str, args: &[&str]) -> McpTransport {
        McpTransport::Stdio {
            command: command.into(),
            args: args.iter().map(|a| a.to_string()).collect(),
            env: vec![],
            cwd: None,
        }
    }

    #[test]
    fn a_server_made_from_an_entry_is_recognised_as_that_entry_and_no_other() {
        for entry in &catalog().entries {
            let inputs: HashMap<String, String> = match &entry.transport {
                CatalogTransport::Stdio { inputs, .. } => {
                    inputs.iter().map(|i| (i.name.clone(), "/tmp/chosen".to_string())).collect()
                }
                CatalogTransport::Http { .. } => HashMap::new(),
            };
            let (server, _) = entry.to_new_server(None, &inputs).unwrap_or_else(|e| panic!("{}: {e}", entry.id));
            assert_eq!(
                catalog().entry_for(&server.transport).map(|e| e.id.as_str()),
                Some(entry.id.as_str()),
                "{}",
                entry.id
            );
        }
    }

    #[test]
    fn the_package_decides_which_entry_a_command_is_not_its_flags_or_its_version() {
        let found = |t: McpTransport| catalog().entry_for(&t).map(|e| e.id.clone());
        // `-y` is shared by every npx entry: it used to make any of them look like the first one.
        assert_eq!(found(stdio("npx", &["-y", "some-other-server"])), None);
        assert_eq!(found(stdio("npx", &["-y"])), None);
        assert_eq!(found(stdio("npx", &["-y", "@playwright/mcp@latest"])).as_deref(), Some("playwright"));
        assert_eq!(found(stdio("npx", &["--yes", "@playwright/mcp@9.9.9"])).as_deref(), Some("playwright"));
        assert_eq!(
            found(stdio("npx", &["-y", "@modelcontextprotocol/server-filesystem", "/x"])).as_deref(),
            Some("filesystem")
        );
        assert_eq!(found(stdio("uvx", &["mcp-server-fetch"])).as_deref(), Some("fetch"));
        // Another launcher running a name that looks the same is not the entry.
        assert_eq!(found(stdio("sh", &["mcp-server-fetch"])), None);
        let remote = McpTransport::Http { url: "https://mcp.example.com/mcp".into(), auth: HttpAuth::None };
        assert_eq!(found(remote), None);
    }

    #[test]
    fn an_entry_becomes_a_server_with_its_inputs_and_says_what_else_it_needs() {
        let filesystem = catalog().entries.iter().find(|e| e.id == "filesystem").unwrap();
        assert!(filesystem.to_new_server(None, &HashMap::new()).is_err(), "the folder is required");
        let (server, needed) = filesystem
            .to_new_server(None, &HashMap::from([("folder".to_string(), " /Users/me/notes ".to_string())]))
            .unwrap();
        assert_eq!(server.name, "files");
        assert!(needed.is_empty());
        match server.transport {
            McpTransport::Stdio { command, args, .. } => {
                assert_eq!((command.as_str(), args.last().map(String::as_str)), ("npx", Some("/Users/me/notes")))
            }
            other => panic!("{other:?}"),
        }
        let github = catalog().entries.iter().find(|e| e.id == "github").unwrap();
        let (server, needed) = github.to_new_server(Some("work-github"), &HashMap::new()).unwrap();
        assert_eq!((server.name.as_str(), needed), ("work-github", vec![SecretNeeded::Token]));
        let linear = catalog().entries.iter().find(|e| e.id == "linear").unwrap();
        assert_eq!(linear.to_new_server(None, &HashMap::new()).unwrap().1, vec![SecretNeeded::SignIn]);
        assert!(linear.to_new_server(Some("Bad Name"), &HashMap::new()).is_err());
        let open = catalog().entries.iter().find(|e| e.id == "deepwiki").unwrap();
        assert!(open.to_new_server(None, &HashMap::new()).unwrap().1.is_empty());
    }
}
