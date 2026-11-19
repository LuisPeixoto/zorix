//! The command line, as `clap` types. Every value is checked here, by value parsers, so a bad argument is reported
//! before anything touches the data folder.

use std::path::PathBuf;
use std::str::FromStr;

use clap::{Args, Parser, Subcommand, ValueEnum};
use zorix_core::{ExecutionId, ExecutionState, TenantId};

/// Run AI activities under control: permissions, approvals, limits, and a record of everything.
#[derive(Debug, Parser)]
#[command(name = "zorix", version, long_about = None)]
pub struct Cli {
    /// The folder where ZORIX keeps its data.
    #[arg(long, global = true, env = "ZORIX_HOME", value_name = "DIR")]
    pub home: Option<PathBuf>,

    /// The organisation (or person) the work belongs to.
    #[arg(long, global = true, default_value = "local", value_parser = parse_tenant, value_name = "NAME")]
    pub tenant: TenantId,

    /// Print machine-readable JSON instead of text.
    #[arg(long, global = true)]
    pub json: bool,

    #[command(subcommand)]
    pub command: Command,
}

/// What to do.
#[derive(Debug, Subcommand)]
pub enum Command {
    /// Start an activity and follow it until it finishes, needs you, or you press Ctrl-C.
    Run(RunArgs),
    /// List executions, newest first.
    List(ListArgs),
    /// Show one execution: model, tools, timings, usage and result.
    Show(IdArg),
    /// Show the events of an execution, optionally following it live.
    Events(EventsArgs),
    /// Approve the tool call an execution is waiting for, then follow it.
    Approve(DecideArgs),
    /// Refuse the tool call an execution is waiting for, then follow it.
    Deny(DecideArgs),
    /// Cancel an execution.
    Cancel(CancelArgs),
    /// Continue a paused execution and follow it.
    Resume(ResumeArgs),
    /// Continue executions that a crash or a closed terminal left in the middle of a step.
    Recover(RecoverArgs),
    /// List the built-in tools.
    Tools,
    /// List the models ZORIX knows, or (with --available) ask each provider what it offers.
    Models(ModelsArgs),
    /// Manage AI providers and their keys.
    #[command(subcommand)]
    Provider(ProviderCommand),
    /// Show the routes and aliases from providers.toml.
    #[command(subcommand)]
    Route(RouteCommand),
    /// What ZORIX remembers: facts and preferences you approved.
    #[command(subcommand)]
    Memory(MemoryCommand),
    /// Folders of documents ZORIX can search and cite.
    #[command(subcommand)]
    Knowledge(KnowledgeCommand),
    /// The model that turns text into vectors for search by meaning.
    #[command(subcommand)]
    Embeddings(EmbeddingsCommand),
    /// Connect ZORIX to MCP servers, or serve ZORIX's memory to other tools over MCP.
    #[command(subcommand)]
    Mcp(McpCommand),
    /// Connections to services: Git hosts, Jira, Slack, Discord, databases, web APIs and webhooks.
    #[command(subcommand)]
    Connect(ConnectCommand),
    /// Run code in a WebAssembly sandbox.
    #[command(subcommand)]
    Sandbox(SandboxCommand),
    /// How many real tokens ZORIX's estimates are worth, per model, learned from what providers report.
    Calibration,
    /// Print where ZORIX keeps its data.
    DataDir,
}

/// What `zorix mcp` does.
#[derive(Debug, Subcommand)]
pub enum McpCommand {
    /// The servers ZORIX knows how to add, from a short reviewed list.
    Catalog(McpCatalogArgs),
    /// Add an MCP server (disabled until you enable it): from the catalogue, a command to run, or a web address.
    Add(McpAddArgs),
    /// List the servers you added.
    List,
    /// Show one server: what it runs or where it connects, and what is known about its tools.
    Show(McpNameArg),
    /// Sign in to a server that uses a browser sign-in.
    Login(McpNameArg),
    /// Forget the sign-in of a server.
    Logout(McpNameArg),
    /// Start a server briefly and list the tools it offers and where each stands.
    Tools(McpNameArg),
    /// Approve, or turn off, tools that are new or whose description changed.
    Review(McpReviewArgs),
    /// Say whether calls to a tool ask first.
    Trust(McpTrustArgs),
    /// Turn a server on: it starts when a run is allowed to use it.
    Enable(McpEnableArgs),
    /// Turn a server off.
    Disable(McpNameArg),
    /// Remove a server and every secret kept for it.
    Remove(McpNameArg),
    /// Bring in servers from Claude Desktop, VS Code or Cursor. Shows what would be imported unless you pass --yes.
    Import(McpImportArgs),
    /// Start a server briefly and show what it wrote on its error stream.
    Logs(McpNameArg),
    /// Serve ZORIX over MCP on standard input and output, for Claude Code, Cursor and other MCP clients. Read-only by
    /// default: memory, documents and past work can be searched, never changed.
    Serve(McpServeArgs),
}

/// A server name.
#[derive(Debug, Args)]
pub struct McpNameArg {
    /// The name ZORIX knows the server by.
    pub name: String,
}

/// Options for `mcp catalog`.
#[derive(Debug, Args)]
pub struct McpCatalogArgs {
    /// Only entries whose name, summary or category contain this text.
    pub search: Option<String>,
}

/// How a remote server signs people in.
#[derive(Clone, Copy, Debug, PartialEq, Eq, ValueEnum)]
pub enum McpAuthArg {
    /// No sign-in.
    None,
    /// A token you paste.
    Bearer,
    /// OAuth in the browser.
    Oauth,
}

/// Where a secret comes from when it is not typed at a prompt.
#[derive(Debug, Args)]
pub struct SecretArgs {
    /// Read the secret from this environment variable of this command.
    #[arg(long, value_name = "VAR")]
    pub secret_env: Option<String>,
    /// Read the secret from standard input (one line).
    #[arg(long)]
    pub secret_stdin: bool,
}

/// Options for `mcp add`.
#[derive(Debug, Args)]
pub struct McpAddArgs {
    /// The name to give it (lowercase letters, digits, - and _). With --catalog the entry's own name is the default.
    pub name: Option<String>,
    /// Add this entry of the catalogue (see `zorix mcp catalog`).
    #[arg(long, value_name = "ID", conflicts_with_all = ["command", "url"])]
    pub catalog: Option<String>,
    /// A value the catalogue entry asks for, as NAME=VALUE (a folder, for example). Repeat for more.
    #[arg(long = "input", value_name = "NAME=VALUE", requires = "catalog")]
    pub inputs: Vec<String>,
    /// A program to run as the server. It runs as you, without a sandbox.
    #[arg(long, value_name = "PROGRAM", conflicts_with = "url")]
    pub command: Option<String>,
    /// An argument for the program. Repeat for more.
    #[arg(long = "arg", value_name = "ARG", allow_hyphen_values = true, requires = "command")]
    pub args: Vec<String>,
    /// A secret environment variable for the program: its value comes from this command's own environment of the same name,
    /// or is asked for. It goes to the keychain. Repeat for more.
    #[arg(long = "env", value_name = "NAME", requires = "command")]
    pub env: Vec<String>,
    /// A plain environment variable for the program, as NAME=VALUE. Not secret. Repeat for more.
    #[arg(long = "set", value_name = "NAME=VALUE", requires = "command")]
    pub set: Vec<String>,
    /// The folder to start the program in.
    #[arg(long, value_name = "DIR", requires = "command")]
    pub cwd: Option<PathBuf>,
    /// The address of a remote server (https).
    #[arg(long, value_name = "URL")]
    pub url: Option<String>,
    /// How the remote server signs in. Default: none.
    #[arg(long, value_enum, requires = "url")]
    pub auth: Option<McpAuthArg>,
    /// A client id registered in advance, for servers that do not register clients themselves.
    #[arg(long, value_name = "ID")]
    pub client_id: Option<String>,
    /// A scope to ask for at sign-in. Repeat for more.
    #[arg(long = "scope", value_name = "SCOPE")]
    pub scopes: Vec<String>,
    #[command(flatten)]
    pub secret: SecretArgs,
}

/// Options for `mcp review`.
#[derive(Debug, Args)]
pub struct McpReviewArgs {
    /// The server.
    pub name: String,
    /// Approve what the server lists now for this tool.
    #[arg(long, value_name = "TOOL")]
    pub approve: Vec<String>,
    /// Approve every tool that is new or changed.
    #[arg(long)]
    pub approve_all: bool,
    /// Turn this tool off.
    #[arg(long, value_name = "TOOL")]
    pub disable: Vec<String>,
}

/// Options for `mcp trust`.
#[derive(Debug, Args)]
pub struct McpTrustArgs {
    /// The server.
    pub name: String,
    /// The tool.
    #[arg(long, value_name = "TOOL")]
    pub tool: String,
    /// Do not ask before calling it. Only for a tool the server reports as read-only.
    #[arg(long, conflicts_with = "ask")]
    pub read_only_without_asking: bool,
    /// Ask before every call (the default).
    #[arg(long)]
    pub ask: bool,
}

/// Options for `mcp enable`.
#[derive(Debug, Args)]
pub struct McpEnableArgs {
    /// The server.
    pub name: String,
    /// Do not ask for confirmation.
    #[arg(long)]
    pub yes: bool,
}

/// Where `mcp import` looks.
#[derive(Clone, Copy, Debug, PartialEq, Eq, ValueEnum)]
pub enum ImportSourceArg {
    /// Claude Desktop's claude_desktop_config.json.
    Claude,
    /// VS Code's mcp.json.
    Vscode,
    /// Cursor's ~/.cursor/mcp.json.
    Cursor,
}

/// Options for `mcp import`.
#[derive(Debug, Args)]
pub struct McpImportArgs {
    /// Which program's configuration to read. Default: every one that is found.
    #[arg(long, value_enum)]
    pub from: Option<ImportSourceArg>,
    /// A configuration file to read instead of the usual place (needs --from).
    #[arg(long, value_name = "FILE", requires = "from")]
    pub path: Option<PathBuf>,
    /// Import what was found (servers are added disabled).
    #[arg(long)]
    pub yes: bool,
}

/// What `zorix connect` does.
#[derive(Debug, Subcommand)]
pub enum ConnectCommand {
    /// Add a connection to a service.
    #[command(subcommand)]
    Add(ConnectAdd),
    /// List connections.
    List,
    /// Check that a connection works, with one harmless read.
    Test(McpNameArg),
    /// Remove a connection and its secret.
    Remove(McpNameArg),
}

/// Which service a new connection is for.
#[derive(Debug, Subcommand)]
pub enum ConnectAdd {
    /// GitHub (or GitHub Enterprise). A token is only needed to write or to read private repositories.
    Github(ConnectGit),
    /// GitLab (gitlab.com or your own).
    Gitlab(ConnectGit),
    /// Jira Cloud (or Server with a bearer token).
    Jira(ConnectJira),
    /// A Slack workspace, with a bot token.
    Slack(ConnectChat),
    /// A Discord bot.
    Discord(ConnectChat),
    /// A Postgres database. Queries run in read-only transactions.
    Postgres(ConnectPostgres),
    /// A SQLite file, opened read-only.
    Sqlite(ConnectSqlite),
    /// Credentials for a web API, sent only to its host.
    Http(ConnectHttp),
    /// A webhook ZORIX can post signed events to.
    Webhook(ConnectWebhook),
}

/// A GitHub or GitLab connection.
#[derive(Debug, Args)]
pub struct ConnectGit {
    /// The name of the connection.
    pub name: String,
    /// The API address, for GitHub Enterprise or your own GitLab.
    #[arg(long, value_name = "URL")]
    pub base_url: Option<String>,
    /// Keep no token: public data only.
    #[arg(long)]
    pub no_token: bool,
    #[command(flatten)]
    pub secret: SecretArgs,
}

/// A Jira connection.
#[derive(Debug, Args)]
pub struct ConnectJira {
    /// The name of the connection.
    pub name: String,
    /// The site address, for example https://acme.atlassian.net.
    #[arg(long, value_name = "URL")]
    pub site: String,
    /// The account email (Jira Cloud).
    #[arg(long)]
    pub email: Option<String>,
    /// The token is a bearer token (Jira Server), not an API token with an email.
    #[arg(long)]
    pub bearer: bool,
    #[command(flatten)]
    pub secret: SecretArgs,
}

/// A Slack or Discord connection.
#[derive(Debug, Args)]
pub struct ConnectChat {
    /// The name of the connection.
    pub name: String,
    /// The API address, if it is not the usual one.
    #[arg(long, value_name = "URL")]
    pub base_url: Option<String>,
    #[command(flatten)]
    pub secret: SecretArgs,
}

/// A Postgres connection.
#[derive(Debug, Args)]
pub struct ConnectPostgres {
    /// The name of the connection (used in `--allow-db`).
    pub name: String,
    /// The host.
    #[arg(long)]
    pub host: String,
    /// The port.
    #[arg(long, default_value_t = 5432, value_parser = clap::value_parser!(u16).range(1..))]
    pub port: u16,
    /// The database.
    #[arg(long)]
    pub database: String,
    /// The user.
    #[arg(long)]
    pub user: String,
    /// verify-full (the default) or disable (only for a database on this computer).
    #[arg(long, value_name = "MODE")]
    pub sslmode: Option<String>,
    /// A certificate authority file to trust, for a server with its own certificate.
    #[arg(long, value_name = "FILE")]
    pub ca_file: Option<PathBuf>,
    /// The password is not needed (trust or certificate login).
    #[arg(long)]
    pub no_password: bool,
    #[command(flatten)]
    pub secret: SecretArgs,
}

/// A SQLite connection.
#[derive(Debug, Args)]
pub struct ConnectSqlite {
    /// The name of the connection (used in `--allow-db`).
    pub name: String,
    /// The database file.
    pub path: PathBuf,
}

/// An HTTP API connection.
#[derive(Debug, Args)]
pub struct ConnectHttp {
    /// The name of the connection.
    pub name: String,
    /// The host the credential is for, such as api.example.com.
    #[arg(long)]
    pub host: String,
    /// How to send it: bearer (the default), header or basic.
    #[arg(long, default_value = "bearer")]
    pub auth: String,
    /// With header: the header name (default X-Api-Key).
    #[arg(long)]
    pub header: Option<String>,
    /// With basic: the user name.
    #[arg(long)]
    pub username: Option<String>,
    #[command(flatten)]
    pub secret: SecretArgs,
}

/// A webhook connection.
#[derive(Debug, Args)]
pub struct ConnectWebhook {
    /// The name of the connection.
    pub name: String,
    /// The address events are posted to.
    #[arg(long, value_name = "URL")]
    pub url: String,
    #[command(flatten)]
    pub secret: SecretArgs,
}

/// What `zorix sandbox` does.
#[derive(Debug, Subcommand)]
pub enum SandboxCommand {
    /// What the code sandbox can run, and what is installed.
    Status,
    /// Download a language runtime (checked against a fixed checksum).
    #[command(subcommand)]
    Install(SandboxInstall),
}

/// What can be installed.
#[derive(Debug, Subcommand)]
pub enum SandboxInstall {
    /// CPython 3.12 for WebAssembly (about 25 MB). JavaScript needs nothing.
    Python,
}

/// Options for `mcp serve`.
#[derive(Debug, Args)]
pub struct McpServeArgs {
    /// Also let the client start activities (`zorix.run`, `zorix.status`, `zorix.result`). This takes the data folder
    /// for as long as the server runs, so the desktop app cannot be used meanwhile.
    #[arg(long)]
    pub allow_runs: bool,

    /// The project whose memories are visible and whose folder runs work in.
    #[arg(long, value_name = "DIR")]
    pub project: Option<PathBuf>,

    /// With --allow-runs: the model runs use (see `zorix run --model`).
    #[arg(long, value_name = "MODEL", requires = "allow_runs")]
    pub model: Option<ModelChoice>,

    /// With --allow-runs: let runs use this tool (an id, or a prefix ending in *). Repeat for more.
    #[arg(long = "allow", value_name = "TOOL", value_parser = parse_tool_pattern, requires = "allow_runs")]
    pub allow: Vec<String>,

    /// With --allow-runs: let file tools read paths matching this pattern, relative to the project.
    #[arg(long, value_name = "PATTERN", value_parser = parse_path_pattern, requires = "allow_runs")]
    pub allow_read: Vec<String>,

    /// With --allow-runs: let file tools write paths matching this pattern, relative to the project.
    #[arg(long, value_name = "PATTERN", value_parser = parse_path_pattern, requires = "allow_runs")]
    pub allow_write: Vec<String>,
}

/// Which model answers.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ModelChoice {
    /// The built-in rule-based demo model.
    Demo,
    /// A scripted model read from a JSON file. Its position in the script lives in this process only.
    Script(PathBuf),
    /// Anything the router resolves: `provider/model`, `route:name`, an alias or `default`.
    Reference(String),
}

impl FromStr for ModelChoice {
    type Err = String;

    fn from_str(text: &str) -> Result<Self, String> {
        if text == "demo" {
            return Ok(ModelChoice::Demo);
        }
        if let Some(path) = text.strip_prefix("script:") {
            return if path.is_empty() {
                Err("use script:<file.json>".into())
            } else {
                Ok(ModelChoice::Script(PathBuf::from(path)))
            };
        }
        zorix_providers::Selector::parse(text).map(|_| ModelChoice::Reference(text.to_string()))
    }
}

impl ModelChoice {
    /// The reference recorded in the execution and given to the router.
    pub fn label(&self) -> String {
        match self {
            ModelChoice::Demo => "demo".into(),
            ModelChoice::Script(_) => "script".into(),
            ModelChoice::Reference(text) => text.clone(),
        }
    }
}

/// How to answer approval requests.
#[derive(Clone, Copy, Debug, PartialEq, Eq, ValueEnum)]
pub enum ApproveMode {
    /// Ask on the terminal.
    Ask,
    /// Leave the execution waiting and exit; answer later with `zorix approve` or `zorix deny`.
    Never,
    /// Approve every request without asking. Only for demos and tests.
    Yes,
    /// Refuse every request without asking.
    No,
}

/// Options that choose the model.
#[derive(Debug, Args)]
pub struct ModelArgs {
    /// The model: `provider/model` (for example `anthropic/claude-opus-5-5`), `route:<name>`, an alias, `demo`, or
    /// `script:<file.json>`. Default: the `default` alias when one is set or Anthropic has a key, otherwise `demo`.
    #[arg(long, value_name = "MODEL")]
    pub model: Option<ModelChoice>,

    /// Use only providers on this computer. Nothing is sent to the cloud.
    #[arg(long)]
    pub local_only: bool,
}

impl ModelArgs {
    /// Options that leave the choice to the defaults.
    pub fn default_demo() -> Self {
        Self { model: None, local_only: false }
    }
}

/// Options for `run`.
#[derive(Debug, Args)]
pub struct RunArgs {
    /// What to do, in plain words.
    #[arg(value_parser = parse_prompt)]
    pub prompt: String,

    #[command(flatten)]
    pub model: ModelArgs,

    /// Standing instructions for the model.
    #[arg(long, value_name = "TEXT")]
    pub instructions: Option<String>,

    /// Let the execution use this tool (a tool id, or a prefix ending in *). Repeat for more tools.
    #[arg(long = "allow", value_name = "TOOL", value_parser = parse_tool_pattern)]
    pub allow: Vec<String>,

    /// Let the execution use every built-in tool. Tools that change things still ask first.
    #[arg(long)]
    pub allow_all_tools: bool,

    /// Let file tools read paths matching this pattern, relative to the workspace (for example `*` or `docs/*`).
    #[arg(long, value_name = "PATTERN", value_parser = parse_path_pattern)]
    pub allow_read: Vec<String>,

    /// Let file tools write paths matching this pattern, relative to the workspace.
    #[arg(long, value_name = "PATTERN", value_parser = parse_path_pattern)]
    pub allow_write: Vec<String>,

    /// Let the execution reach this host over HTTPS with `http.get`, `http.request` and `webhook.send`.
    #[arg(long, value_name = "HOST", value_parser = parse_http_grant)]
    pub allow_http: Vec<GrantFlag>,

    /// Let the execution use a repository: `[github:|gitlab:]OWNER/REPO[:write]`, or `OWNER/*`.
    #[arg(long, value_name = "REPO", value_parser = parse_repo_grant)]
    pub allow_repo: Vec<GrantFlag>,

    /// Let the execution use a Jira project: `KEY[:write]`.
    #[arg(long, value_name = "KEY", value_parser = parse_jira_grant)]
    pub allow_jira: Vec<GrantFlag>,

    /// Let the execution use a chat channel: `slack:#channel[:write]` or `discord:<channel id>[:write]`.
    #[arg(long, value_name = "CHANNEL", value_parser = parse_chat_grant)]
    pub allow_chat: Vec<GrantFlag>,

    /// Let the execution use a database connection: `NAME[:write]`. Writes always ask first.
    #[arg(long, value_name = "NAME", value_parser = parse_db_grant)]
    pub allow_db: Vec<GrantFlag>,

    /// Let the execution run code in the sandbox: `javascript` or `python`.
    #[arg(long, value_name = "LANGUAGE", value_parser = parse_code_grant)]
    pub allow_code: Vec<GrantFlag>,

    /// The folder file tools are confined to.
    #[arg(long, value_name = "DIR")]
    pub workspace: Option<PathBuf>,

    /// What to do when a tool call needs approval. Default: ask when on a terminal, otherwise never.
    #[arg(long, value_enum)]
    pub approve: Option<ApproveMode>,

    /// Most model steps.
    #[arg(long, value_parser = clap::value_parser!(u32).range(1..=1000), value_name = "N")]
    pub max_iterations: Option<u32>,

    /// Most tool calls.
    #[arg(long, value_parser = clap::value_parser!(u32).range(0..=10000), value_name = "N")]
    pub max_tool_calls: Option<u32>,

    /// Most seconds of work.
    #[arg(long, value_parser = clap::value_parser!(u64).range(1..=86400), value_name = "SECONDS")]
    pub max_seconds: Option<u64>,

    /// Group this execution with others from the same conversation.
    #[arg(long, value_parser = parse_conversation, value_name = "ID")]
    pub conversation: Option<zorix_core::ConversationId>,

    /// Running again with the same key returns the same execution instead of making a new one.
    #[arg(long, value_name = "KEY", value_parser = parse_key)]
    pub idempotency_key: Option<String>,

    /// Do not use long-term memory or records of past work in this run.
    #[arg(long)]
    pub no_memory: bool,

    /// Do not use indexed documents in this run.
    #[arg(long)]
    pub no_knowledge: bool,

    /// Do not use earlier executions of the conversation in this run.
    #[arg(long)]
    pub no_conversation: bool,
}

/// A scope: where a memory or a folder applies.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ScopeArg(pub zorix_core::Scope);

impl FromStr for ScopeArg {
    type Err = String;

    fn from_str(text: &str) -> Result<Self, String> {
        match text.split_once(':') {
            None if text == "global" => Ok(ScopeArg(zorix_core::Scope::Global)),
            Some(("project", dir)) if !dir.is_empty() => {
                let real =
                    std::fs::canonicalize(dir).map_err(|e| format!("The project folder {dir} cannot be used: {e}"))?;
                Ok(ScopeArg(zorix_core::Scope::Project(real.to_string_lossy().into_owned())))
            }
            Some(("conversation", id)) => zorix_core::ConversationId::new(id)
                .map(|c| ScopeArg(zorix_core::Scope::Conversation(c)))
                .map_err(|e| e.to_string()),
            _ => Err("use global, project:<folder> or conversation:<id>".into()),
        }
    }
}

/// What `zorix memory` does.
#[derive(Debug, Subcommand)]
pub enum MemoryCommand {
    /// List what is remembered.
    List(MemoryListArgs),
    /// Remember something yourself.
    Add(MemoryAddArgs),
    /// Change a memory.
    Edit(MemoryEditArgs),
    /// Say that a memory is still true.
    Confirm(MemoryIdArg),
    /// Delete a memory everywhere ZORIX keeps it.
    Forget(MemoryForgetArgs),
    /// Search memory, documents or past work.
    Search(MemorySearchArgs),
    /// Write every memory to a JSON file (or standard output).
    Export(MemoryExportArgs),
}

/// Options for `memory list`.
#[derive(Debug, Args)]
pub struct MemoryListArgs {
    /// Only this scope: global, project:<folder> or conversation:<id>. Default: every scope.
    #[arg(long, value_name = "SCOPE")]
    pub scope: Option<ScopeArg>,
    /// Also list replaced and expired memories.
    #[arg(long)]
    pub all: bool,
}

/// What a memory is.
#[derive(Clone, Copy, Debug, PartialEq, Eq, ValueEnum)]
pub enum KindArg {
    /// Something true.
    Fact,
    /// How you want things done.
    Preference,
}

/// Options for `memory add`.
#[derive(Debug, Args)]
pub struct MemoryAddArgs {
    /// What to remember, in one short sentence (500 characters at most).
    pub text: String,
    /// Fact or preference.
    #[arg(long, value_enum, default_value = "fact")]
    pub kind: KindArg,
    /// Where it applies: global, project:<folder> or conversation:<id>.
    #[arg(long, default_value = "global", value_name = "SCOPE")]
    pub scope: ScopeArg,
    /// What it is about. A newer memory about the same subject replaces this one.
    #[arg(long)]
    pub subject: Option<String>,
    /// Forget it after this date (YYYY-MM-DD).
    #[arg(long, value_name = "DATE")]
    pub until: Option<String>,
}

/// Options for `memory edit`.
#[derive(Debug, Args)]
pub struct MemoryEditArgs {
    /// The memory id (m:… or the bare id).
    pub id: String,
    /// New text.
    #[arg(long)]
    pub text: Option<String>,
    /// New subject.
    #[arg(long)]
    pub subject: Option<String>,
    /// New expiry date (YYYY-MM-DD).
    #[arg(long, value_name = "DATE", conflicts_with = "no_until")]
    pub until: Option<String>,
    /// Remove the expiry.
    #[arg(long)]
    pub no_until: bool,
    /// Move it to another scope.
    #[arg(long, value_name = "SCOPE")]
    pub scope: Option<ScopeArg>,
}

/// A memory id.
#[derive(Debug, Args)]
pub struct MemoryIdArg {
    /// The memory id (m:… or the bare id).
    pub id: String,
}

/// Options for `memory forget`.
#[derive(Debug, Args)]
pub struct MemoryForgetArgs {
    /// The memory id (m:… or the bare id).
    pub id: String,
    /// Do not ask.
    #[arg(long)]
    pub yes: bool,
}

/// Where `memory search` looks.
#[derive(Clone, Copy, Debug, PartialEq, Eq, ValueEnum)]
pub enum CorpusArg {
    /// Facts and preferences.
    Memory,
    /// Indexed documents.
    Knowledge,
    /// Records of past work.
    History,
}

/// Options for `memory search`.
#[derive(Debug, Args)]
pub struct MemorySearchArgs {
    /// What to look for.
    pub query: String,
    /// Where to look.
    #[arg(long = "in", value_enum, default_value = "memory")]
    pub corpus: CorpusArg,
    /// Look as this project (a folder).
    #[arg(long, value_name = "FOLDER")]
    pub project: Option<PathBuf>,
}

/// Options for `memory export`.
#[derive(Debug, Args)]
pub struct MemoryExportArgs {
    /// The file to write. Default: standard output.
    #[arg(long, value_name = "FILE")]
    pub out: Option<PathBuf>,
}

/// What `zorix knowledge` does.
#[derive(Debug, Subcommand)]
pub enum KnowledgeCommand {
    /// Add a folder and index it.
    Add(KnowledgeAddArgs),
    /// List the folders.
    List,
    /// Bring the index of a folder (or all folders) up to date.
    Sync(KnowledgeSyncArgs),
    /// List the files of a folder, or only the ones that could not be indexed.
    Files(KnowledgeFilesArgs),
    /// Remove a folder from the index. The files are not touched.
    Remove(KnowledgeRootArg),
    /// Search the indexed documents.
    Search(KnowledgeSearchArgs),
}

/// Options for `knowledge add`.
#[derive(Debug, Args)]
pub struct KnowledgeAddArgs {
    /// The folder.
    pub dir: PathBuf,
    /// Only for runs in this project folder. Default: for every run.
    #[arg(long, value_name = "FOLDER", conflicts_with = "global")]
    pub project: Option<PathBuf>,
    /// For every run (the default).
    #[arg(long)]
    pub global: bool,
    /// Also index files matching this pattern (relative to the folder). Repeat for more.
    #[arg(long, value_name = "GLOB")]
    pub include: Vec<String>,
    /// Leave out files matching this pattern. Repeat for more.
    #[arg(long, value_name = "GLOB")]
    pub exclude: Vec<String>,
    /// Look for changes whenever the desktop app starts.
    #[arg(long)]
    pub auto_sync: bool,
    /// Add it without indexing it now.
    #[arg(long)]
    pub no_sync: bool,
}

/// A folder, by id or path.
#[derive(Debug, Args)]
pub struct KnowledgeRootArg {
    /// The folder: its id (or the start of it) or its path.
    pub folder: String,
}

/// Options for `knowledge sync`.
#[derive(Debug, Args)]
pub struct KnowledgeSyncArgs {
    /// The folder. Default: all of them.
    pub folder: Option<String>,
}

/// Options for `knowledge files`.
#[derive(Debug, Args)]
pub struct KnowledgeFilesArgs {
    /// The folder: its id (or the start of it) or its path.
    pub folder: String,
    /// Only the files that failed or were left out, with the reason.
    #[arg(long)]
    pub failed: bool,
}

/// Options for `knowledge search`.
#[derive(Debug, Args)]
pub struct KnowledgeSearchArgs {
    /// What to look for.
    pub query: String,
    /// Look as this project (a folder).
    #[arg(long, value_name = "FOLDER")]
    pub project: Option<PathBuf>,
}

/// What `zorix embeddings` does.
#[derive(Debug, Subcommand)]
pub enum EmbeddingsCommand {
    /// Show the model in use and how much is embedded.
    Show,
    /// List the models you can choose.
    Models,
    /// Choose the model, as provider/model (for example ollama/all-minilm).
    Set(EmbeddingsSetArgs),
    /// Go back to keyword search only.
    Off,
    /// Embed a short text with the chosen model.
    Test,
    /// Embed everything again with the chosen model and delete the vectors of any other.
    Reembed,
}

/// Options for `embeddings set`.
#[derive(Debug, Args)]
pub struct EmbeddingsSetArgs {
    /// The model: provider/model.
    pub model: String,
    /// The size of the vectors, for models that let you choose.
    #[arg(long, value_name = "N", value_parser = clap::value_parser!(u32).range(16..=8192))]
    pub dims: Option<u32>,
    /// Allow a cloud provider. Your documents' text is sent to it to be embedded.
    #[arg(long)]
    pub cloud: bool,
}

/// Options for `models`.
#[derive(Debug, Args)]
pub struct ModelsArgs {
    /// Ask each provider that has a key (or runs here) which models it offers. Needs the network.
    #[arg(long)]
    pub available: bool,
}

/// What `zorix provider` does.
#[derive(Debug, Subcommand)]
pub enum ProviderCommand {
    /// List providers, where they are and whether they have a key.
    List,
    /// Store a key in the system keychain. The key is read from a hidden prompt, never from the command line.
    SetKey(KeyArgs),
    /// Delete a stored key.
    RemoveKey(NameArg),
    /// Check a provider: key, connection and the models it lists. Sends no conversation and costs nothing.
    Test(NameArg),
}

/// What `zorix route` does.
#[derive(Debug, Subcommand)]
pub enum RouteCommand {
    /// List routes and aliases.
    List,
}

/// A provider name.
#[derive(Debug, Args)]
pub struct NameArg {
    /// The provider, as `zorix provider list` shows it.
    pub name: String,
}

/// Options for `provider set-key`.
#[derive(Debug, Args)]
pub struct KeyArgs {
    /// The provider, as `zorix provider list` shows it.
    pub name: String,

    /// Read the key from standard input (one line) instead of asking on the terminal. For scripts.
    #[arg(long)]
    pub stdin: bool,
}

/// Options for `list`.
#[derive(Debug, Args)]
pub struct ListArgs {
    /// Only executions in this state.
    #[arg(long, value_parser = parse_state, value_name = "STATE")]
    pub state: Option<ExecutionState>,

    /// Most rows.
    #[arg(long, default_value_t = 20, value_parser = clap::value_parser!(u32).range(1..=1000), value_name = "N")]
    pub limit: u32,
}

/// One execution id.
#[derive(Debug, Args)]
pub struct IdArg {
    /// The execution id.
    #[arg(value_parser = parse_execution_id)]
    pub id: ExecutionId,

    /// Also show what memory put into the context: facts, earlier exchanges, documents with their sources, and what each part cost.
    #[arg(long)]
    pub context: bool,
}

/// Options for `events`.
#[derive(Debug, Args)]
pub struct EventsArgs {
    /// The execution id.
    #[arg(value_parser = parse_execution_id)]
    pub id: ExecutionId,

    /// Keep going and print new events until the execution finishes or waits for a person.
    #[arg(long, short)]
    pub follow: bool,

    /// Only events after this sequence number.
    #[arg(long, default_value_t = 0, value_name = "SEQ")]
    pub after: u64,
}

/// Options for `approve` and `deny`.
#[derive(Debug, Args)]
pub struct DecideArgs {
    /// The execution id.
    #[arg(value_parser = parse_execution_id)]
    pub id: ExecutionId,

    /// The approval id. Optional when the execution is waiting for exactly one.
    #[arg(value_parser = parse_approval_id)]
    pub approval: Option<zorix_core::ApprovalId>,

    /// A note, kept with the decision. When refusing, the model reads it.
    #[arg(long)]
    pub note: Option<String>,

    #[command(flatten)]
    pub model: ModelArgs,

    /// What to do if the execution asks for something else afterwards.
    #[arg(long, value_enum, default_value = "never")]
    pub then: ApproveMode,
}

/// Options for `cancel`.
#[derive(Debug, Args)]
pub struct CancelArgs {
    /// The execution id.
    #[arg(value_parser = parse_execution_id)]
    pub id: ExecutionId,

    /// Why. Kept in the record.
    #[arg(long)]
    pub reason: Option<String>,
}

/// Options for `resume`.
#[derive(Debug, Args)]
pub struct ResumeArgs {
    /// The execution id.
    #[arg(value_parser = parse_execution_id)]
    pub id: ExecutionId,

    #[command(flatten)]
    pub model: ModelArgs,

    /// What to do if the execution asks for approval.
    #[arg(long, value_enum, default_value = "never")]
    pub then: ApproveMode,
}

/// Options for `recover`.
#[derive(Debug, Args)]
pub struct RecoverArgs {
    #[command(flatten)]
    pub model: ModelArgs,

    /// Print what was found and exit, without waiting for the continued executions to finish.
    #[arg(long)]
    pub no_wait: bool,

    /// What to do if a recovered execution asks for approval.
    #[arg(long, value_enum, default_value = "never")]
    pub then: ApproveMode,
}

fn parse_tenant(text: &str) -> Result<TenantId, String> {
    TenantId::new(text).map_err(|e| e.to_string())
}

fn parse_conversation(text: &str) -> Result<zorix_core::ConversationId, String> {
    zorix_core::ConversationId::new(text).map_err(|e| e.to_string())
}

fn parse_execution_id(text: &str) -> Result<ExecutionId, String> {
    text.parse::<ExecutionId>().map_err(|e| e.to_string())
}

fn parse_approval_id(text: &str) -> Result<zorix_core::ApprovalId, String> {
    text.parse::<zorix_core::ApprovalId>().map_err(|e| e.to_string())
}

fn parse_state(text: &str) -> Result<ExecutionState, String> {
    text.parse::<ExecutionState>().map_err(|_| {
        let all: Vec<&str> = ExecutionState::ALL.iter().map(ExecutionState::as_str).collect();
        format!("“{text}” is not a state. Use one of: {}", all.join(", "))
    })
}

fn parse_prompt(text: &str) -> Result<String, String> {
    if text.trim().is_empty() {
        return Err("the prompt is empty".into());
    }
    Ok(text.to_string())
}

fn parse_key(text: &str) -> Result<String, String> {
    if text.trim().is_empty() || text.len() > 128 {
        return Err("a key has 1 to 128 characters".into());
    }
    Ok(text.to_string())
}

/// Permissions a flag expands to.
#[derive(Clone, Debug)]
pub struct GrantFlag(pub Vec<zorix_core::Permission>);

fn parse_http_grant(text: &str) -> Result<GrantFlag, String> {
    zorix_hub::grants::http(text).map(GrantFlag)
}

fn parse_repo_grant(text: &str) -> Result<GrantFlag, String> {
    zorix_hub::grants::repo(text).map(GrantFlag)
}

fn parse_jira_grant(text: &str) -> Result<GrantFlag, String> {
    zorix_hub::grants::jira(text).map(GrantFlag)
}

fn parse_chat_grant(text: &str) -> Result<GrantFlag, String> {
    zorix_hub::grants::chat(text).map(GrantFlag)
}

fn parse_db_grant(text: &str) -> Result<GrantFlag, String> {
    zorix_hub::grants::db(text).map(GrantFlag)
}

fn parse_code_grant(text: &str) -> Result<GrantFlag, String> {
    zorix_hub::grants::code(text).map(GrantFlag)
}

fn parse_tool_pattern(text: &str) -> Result<String, String> {
    zorix_core::validate_tool_pattern(text).map(|()| text.to_string())
}

fn parse_path_pattern(text: &str) -> Result<String, String> {
    zorix_core::validate_workspace_pattern(text).map(|()| text.to_string())
}

#[cfg(test)]
mod tests {
    use clap::CommandFactory;

    use super::*;

    #[test]
    fn the_command_line_definition_is_consistent() {
        Cli::command().debug_assert();
    }

    #[test]
    fn models_are_demo_or_a_script_file() {
        assert_eq!("demo".parse::<ModelChoice>().unwrap(), ModelChoice::Demo);
        assert_eq!("script:a/b.json".parse::<ModelChoice>().unwrap(), ModelChoice::Script("a/b.json".into()));
        for good in ["anthropic/claude-opus-5-5", "route:fast", "default", "ollama/qwen3:8b"] {
            assert_eq!(good.parse::<ModelChoice>().unwrap(), ModelChoice::Reference(good.into()), "{good}");
        }
        for bad in ["", "script:", "Demo", "a b", "route:"] {
            assert!(bad.parse::<ModelChoice>().is_err(), "{bad}");
        }
    }

    #[test]
    fn bad_values_are_refused_with_a_reason() {
        assert!(parse_tenant("a/b").is_err());
        assert!(parse_execution_id("nope").is_err());
        assert!(parse_state("sleeping").unwrap_err().contains("waiting_for_approval"));
        assert!(parse_prompt("  ").is_err());
        assert!(parse_tool_pattern("math evaluate").is_err());
        assert!(parse_tool_pattern("files.*").is_ok());
        for bad in ["/etc/*", "../x", "a\\b", "C:/x", ""] {
            assert!(parse_path_pattern(bad).is_err(), "{bad}");
        }
        assert!(parse_path_pattern("docs/*").is_ok());
    }

    #[test]
    fn run_parses_its_options() {
        let cli = Cli::try_parse_from([
            "zorix",
            "run",
            "What is 2+2?",
            "--allow",
            "math.evaluate",
            "--allow",
            "notes.*",
            "--approve",
            "never",
            "--max-iterations",
            "5",
            "--allow-read",
            "docs/*",
        ])
        .unwrap();
        let Command::Run(run) = cli.command else { panic!("run") };
        assert_eq!(run.allow, vec!["math.evaluate", "notes.*"]);
        assert_eq!((run.approve, run.max_iterations), (Some(ApproveMode::Never), Some(5)));
        assert_eq!(run.allow_read, vec!["docs/*"]);
        assert_eq!(cli.tenant, TenantId::local());
    }

    #[test]
    fn out_of_range_numbers_are_refused_by_the_parser() {
        assert!(Cli::try_parse_from(["zorix", "run", "x", "--max-iterations", "0"]).is_err());
        assert!(Cli::try_parse_from(["zorix", "run", "x", "--max-iterations", "100000"]).is_err());
        assert!(Cli::try_parse_from(["zorix", "list", "--limit", "0"]).is_err());
    }
}
