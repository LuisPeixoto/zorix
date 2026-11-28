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
    Run(Box<RunArgs>),
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
    /// The settings of the program: long conversations, how long runs are kept, telemetry and scanned pages.
    #[command(subcommand)]
    Settings(SettingsCommand),
    /// Keep finished runs for a while, then compact or delete them (see `settings set retention.days`).
    #[command(subcommand)]
    Retention(RetentionCommand),
    /// Connect ZORIX to MCP servers, or serve ZORIX's memory to other tools over MCP.
    #[command(subcommand)]
    Mcp(McpCommand),
    /// Agents: reusable AI workers with their own instructions, skills, tools, memory and versions.
    #[command(subcommand)]
    Agent(AgentCommand),
    /// Teams: a lead and the agents it may ask to do part of the work.
    #[command(subcommand)]
    Team(TeamCommand),
    /// Workflows: triggers, agents, conditions and people joined into steps that run on their own.
    #[command(subcommand)]
    Workflow(WorkflowCommand),
    /// Projects: tasks for you and your agents, with a manager that plans through tools.
    #[command(subcommand)]
    Project(ProjectCommand),
    /// Tasks: add, move, assign and review the tasks of a project. Moving an agent's task to in-progress starts its run.
    #[command(subcommand)]
    Task(TaskCommand),
    /// The workforce: workers (agents with a role, a place in the organisation and a level of autonomy), departments and settings.
    #[command(subcommand)]
    Org(OrgCommand),
    /// Workers: hire, edit and watch the agents of your organisation, and ask a manager to plan the week or write the report.
    #[command(subcommand)]
    Worker(WorkerCommand),
    /// Show an execution and everything it started, as a tree.
    Tree(IdArg),
    /// Skills: folders of instructions and scripts an agent loads when it needs them.
    #[command(subcommand)]
    Skill(SkillCommand),
    /// Evaluations: cases that check what an agent does, and comparisons between versions and models.
    #[command(subcommand)]
    Eval(EvalCommand),
    /// Connections to services: Git hosts, Jira, Slack, Discord, databases, web APIs and webhooks.
    #[command(subcommand)]
    Connect(ConnectCommand),
    /// Run code in a WebAssembly sandbox.
    #[command(subcommand)]
    Sandbox(SandboxCommand),
    /// How many real tokens ZORIX's estimates are worth, per model, learned from what providers report.
    Calibration,
    /// Run schedules, events and (when asked) webhooks for your workflows, until you stop it. Keeps the data folder to itself,
    /// so the desktop app cannot run at the same time.
    Serve(ServeArgs),
    /// Print where ZORIX keeps its data.
    DataDir,
}

/// Options for `serve`.
#[derive(Debug, Args)]
pub struct ServeArgs {
    #[command(flatten)]
    pub model: ModelArgs,

    /// Receive webhooks on 127.0.0.1:7788 (this computer only).
    #[arg(long)]
    pub webhooks: bool,

    /// Receive webhooks on this address, such as `0.0.0.0:7788` behind a proxy you trust. Implies `--webhooks`.
    #[arg(long, value_name = "ADDRESS")]
    pub bind: Option<String>,
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
    /// Show or choose how a local server is sandboxed: what it may read and write, and whether it has the network.
    Sandbox(McpSandboxArgs),
    /// What the sandbox of a local server enforces on this computer.
    Doctor,
    /// Serve ZORIX over MCP on standard input and output, for Claude Code, Cursor and other MCP clients. Read-only by
    /// default: memory, documents and past work can be searched, never changed.
    Serve(McpServeArgs),
}

/// Options for `mcp sandbox`.
#[derive(Debug, Args)]
pub struct McpSandboxArgs {
    /// The server.
    pub name: String,
    /// Confine the server (`native`) or not (`off`).
    #[arg(long, value_enum)]
    pub mode: Option<McpSandboxMode>,
    /// Whether it may open network connections (`on` or `off`).
    #[arg(long, value_name = "on|off", value_parser = parse_on_off)]
    pub network: Option<bool>,
    /// A folder it may read. Repeat for more.
    #[arg(long = "folder", value_name = "DIR")]
    pub folders: Vec<PathBuf>,
    /// A folder it may read and write. Repeat for more.
    #[arg(long = "folder-write", value_name = "DIR")]
    pub folders_write: Vec<PathBuf>,
    /// Take away the folders it was given before adding these.
    #[arg(long)]
    pub clear_folders: bool,
}

/// How a local server is confined.
#[derive(Clone, Copy, Debug, PartialEq, Eq, ValueEnum)]
pub enum McpSandboxMode {
    /// With what this operating system can enforce.
    Native,
    /// Not confined: it runs as you.
    Off,
}

fn parse_on_off(text: &str) -> Result<bool, String> {
    match text {
        "on" => Ok(true),
        "off" => Ok(false),
        other => Err(format!("write on or off, not {other}")),
    }
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
    /// A program to run as the server. It starts sandboxed: see `zorix mcp sandbox`.
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

    /// With --allow-runs: let `zorix.status` and `zorix.result` follow any run of this data folder, not only the ones this server
    /// started. Off by default: other runs hold what you asked and were answered in ZORIX.
    #[arg(long, requires = "allow_runs")]
    pub allow_read_runs: bool,

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

    /// With --allow-runs: let runs reach a resource, written as an agent's access entry: `http:HOST`,
    /// `repo:OWNER/REPO[:write]`, `jira:KEY`, `chat:slack:#channel`, `db:NAME[:write]`, `code:python`. Repeat for more.
    /// Runs and agents only ever get what this and --allow give them, whatever an agent's own access says.
    #[arg(long, value_name = "ENTRY", value_parser = parse_access_entry, requires = "allow_runs")]
    pub allow_access: Vec<String>,
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

    /// Most money the execution may spend, in dollars. A model with no known price cannot keep this limit, and the run is refused.
    #[arg(long, value_name = "USD", value_parser = parse_usd)]
    pub max_cost: Option<u64>,

    /// Say what the execution would probably cost and how long it would take, from what similar runs took, and do not start it.
    #[arg(long)]
    pub estimate: bool,

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

    /// Ask a team instead: its lead runs, and may ask the members to do part of the work at the same time. The other flags only
    /// narrow what the lead may do.
    #[arg(long, value_name = "TEAM", value_parser = parse_slug, conflicts_with = "agent")]
    pub team: Option<String>,

    /// Ask an agent instead: it brings its own instructions, skills, tools and memory. The other flags only narrow
    /// what it may do (`--allow…` flags keep only what both the agent and the flag allow).
    #[arg(long, value_name = "AGENT", value_parser = parse_slug)]
    pub agent: Option<String>,

    /// With --agent: the version to use (default: the newest).
    #[arg(long, value_name = "N", requires = "agent")]
    pub agent_version: Option<u32>,

    /// With --agent or --team: data that goes with the prompt, as JSON.
    #[arg(long, value_name = "JSON", value_parser = parse_json)]
    pub input: Option<serde_json::Value>,

    /// Give the run a file to read: text, code, HTML or a PDF. Repeat for more, up to ten. A file is given as data and adds no permission; one with a
    /// private name (.env, keys), in a folder of credentials, or over 1 MB of text (25 MB for a PDF) is refused.
    #[arg(long = "attach", value_name = "FILE")]
    pub attach: Vec<PathBuf>,
}

/// What `zorix agent` does.
#[derive(Debug, Subcommand)]
pub enum AgentCommand {
    /// List agents.
    List(AgentListArgs),
    /// Show an agent's definition.
    Show(AgentShowArgs),
    /// Make an agent from a file or from a few options.
    Create(AgentCreateArgs),
    /// Edit an agent as a Markdown file in $VISUAL or $EDITOR; saving makes a new version.
    Edit(AgentSlugArg),
    /// Read an agent file (Markdown with a header). A new name makes an agent; an existing one gets a new version.
    Import(AgentImportArgs),
    /// Write an agent as a Markdown file.
    Export(AgentExportArgs),
    /// List an agent's versions.
    Versions(AgentSlugArg),
    /// Show what changed between two versions.
    Diff(AgentDiffArgs),
    /// Go back to an older version by writing a new version that copies it.
    Rollback(AgentRollbackArgs),
    /// Hide an agent from lists (or show it again with --undo). Nothing is deleted.
    Archive(AgentArchiveArgs),
    /// Delete an agent with its versions and evaluation cases.
    Remove(AgentRemoveArgs),
    /// Add a ready-made agent with its skills and evaluation cases.
    Add(AgentAddArgs),
    /// List the ready-made agents.
    Templates,
}

/// An agent's name (slug).
#[derive(Debug, Args)]
pub struct AgentSlugArg {
    /// The agent's name.
    #[arg(value_parser = parse_slug)]
    pub agent: String,
}

/// Options for `agent list`.
#[derive(Debug, Args)]
pub struct AgentListArgs {
    /// Also list archived agents.
    #[arg(long)]
    pub all: bool,
}

/// Options for `agent show`.
#[derive(Debug, Args)]
pub struct AgentShowArgs {
    /// The agent's name.
    #[arg(value_parser = parse_slug)]
    pub agent: String,
    /// A version other than the newest.
    #[arg(long, value_name = "N")]
    pub version: Option<u32>,
}

/// Options for `agent create`.
#[derive(Debug, Args)]
pub struct AgentCreateArgs {
    /// The agent's name: lowercase letters, digits and dashes.
    #[arg(value_parser = parse_slug)]
    pub agent: String,
    /// Take the definition from this agent file (its name is replaced by the one above).
    #[arg(long, value_name = "FILE")]
    pub from_file: Option<PathBuf>,
    /// The display name.
    #[arg(long)]
    pub name: Option<String>,
    /// What it is for, in a sentence.
    #[arg(long)]
    pub description: Option<String>,
    /// The model it uses by default.
    #[arg(long, value_name = "MODEL", default_value = "default")]
    pub model: String,
    /// What it is told to do and how.
    #[arg(long, value_name = "TEXT")]
    pub instructions: Option<String>,
    /// A tool it may use (an id, or a prefix ending in *). Repeat for more.
    #[arg(long = "tool", value_name = "TOOL", value_parser = parse_tool_pattern)]
    pub tools: Vec<String>,
    /// A resource it may reach: `http:HOST`, `repo:OWNER/REPO`, `db:NAME`, `code:python`, `read:PATTERN`… Repeat for more.
    #[arg(long = "access", value_name = "ENTRY", value_parser = parse_access_entry)]
    pub access: Vec<String>,
    /// A skill in the library it has. Repeat for more.
    #[arg(long = "skill", value_name = "NAME")]
    pub skills: Vec<String>,
}

/// Options for `agent import`.
#[derive(Debug, Args)]
pub struct AgentImportArgs {
    /// The agent file.
    pub file: PathBuf,
    /// Import it under this name instead of the one in the file.
    #[arg(long = "as", value_name = "NAME", value_parser = parse_slug)]
    pub rename: Option<String>,
    /// Import the skills the file names from `DIR/<skill name>` first.
    #[arg(long, value_name = "DIR")]
    pub skills: Option<PathBuf>,
    /// A note for the version history.
    #[arg(long)]
    pub note: Option<String>,
}

/// Options for `agent export`.
#[derive(Debug, Args)]
pub struct AgentExportArgs {
    /// The agent's name.
    #[arg(value_parser = parse_slug)]
    pub agent: String,
    /// A version other than the newest.
    #[arg(long, value_name = "N")]
    pub version: Option<u32>,
    /// Write to this file instead of standard output.
    #[arg(long, short, value_name = "FILE")]
    pub output: Option<PathBuf>,
    /// Also copy the agent's skills into this folder, one folder each, so the file can travel with them.
    #[arg(long, value_name = "DIR")]
    pub with_skills: Option<PathBuf>,
}

/// Options for `agent diff`.
#[derive(Debug, Args)]
pub struct AgentDiffArgs {
    /// The agent's name.
    #[arg(value_parser = parse_slug)]
    pub agent: String,
    /// The older version.
    pub from: u32,
    /// The newer version.
    pub to: u32,
}

/// Options for `agent rollback`.
#[derive(Debug, Args)]
pub struct AgentRollbackArgs {
    /// The agent's name.
    #[arg(value_parser = parse_slug)]
    pub agent: String,
    /// The version to copy.
    pub version: u32,
}

/// Options for `agent archive`.
#[derive(Debug, Args)]
pub struct AgentArchiveArgs {
    /// The agent's name.
    #[arg(value_parser = parse_slug)]
    pub agent: String,
    /// Show it again.
    #[arg(long)]
    pub undo: bool,
}

/// Options for `agent remove`.
#[derive(Debug, Args)]
pub struct AgentRemoveArgs {
    /// The agent's name.
    #[arg(value_parser = parse_slug)]
    pub agent: String,
    /// Say yes without being asked: this cannot be undone.
    #[arg(long)]
    pub yes: bool,
}

/// Options for `agent add`.
#[derive(Debug, Args)]
pub struct AgentAddArgs {
    /// The ready-made agent (see `zorix agent templates`).
    #[arg(long, value_name = "NAME")]
    pub template: String,
    /// Add it under another name.
    #[arg(long = "as", value_name = "NAME", value_parser = parse_slug)]
    pub rename: Option<String>,
}

/// What `zorix team` does.
#[derive(Debug, Subcommand)]
pub enum TeamCommand {
    /// List teams.
    List,
    /// Show a team: its lead, its members and whether they all exist.
    Show(TeamNameArg),
    /// Make a team from a lead and members that are agents.
    Create(TeamCreateArgs),
    /// Change a team's lead, members, name or description.
    Edit(TeamEditArgs),
    /// Delete a team. The agents in it stay.
    Remove(TeamRemoveArgs),
    /// Add a ready-made team, with the agents it needs that are not here yet.
    Add(TeamAddArgs),
    /// List the ready-made teams.
    Templates,
}

/// A team's name (slug).
#[derive(Debug, Args)]
pub struct TeamNameArg {
    /// The team's name.
    #[arg(value_parser = parse_slug)]
    pub team: String,
}

/// Options for `team create`.
#[derive(Debug, Args)]
pub struct TeamCreateArgs {
    /// The team's name: lowercase letters, digits and dashes.
    #[arg(value_parser = parse_slug)]
    pub team: String,
    /// The agent that receives the work and may ask the others.
    #[arg(long, value_parser = parse_slug)]
    pub lead: String,
    /// An agent the lead may ask. Repeat for more.
    #[arg(long = "member", value_name = "AGENT", value_parser = parse_slug)]
    pub members: Vec<String>,
    /// The display name.
    #[arg(long)]
    pub name: Option<String>,
    /// What it is for, in a sentence.
    #[arg(long)]
    pub description: Option<String>,
}

/// Options for `team edit`.
#[derive(Debug, Args)]
pub struct TeamEditArgs {
    /// The team's name.
    #[arg(value_parser = parse_slug)]
    pub team: String,
    /// A new lead.
    #[arg(long, value_parser = parse_slug)]
    pub lead: Option<String>,
    /// Add a member. Repeat for more.
    #[arg(long = "add-member", value_name = "AGENT", value_parser = parse_slug)]
    pub add: Vec<String>,
    /// Take a member out. Repeat for more.
    #[arg(long = "remove-member", value_name = "AGENT", value_parser = parse_slug)]
    pub remove: Vec<String>,
    /// A new display name.
    #[arg(long)]
    pub name: Option<String>,
    /// A new description.
    #[arg(long)]
    pub description: Option<String>,
}

/// Options for `team remove`.
#[derive(Debug, Args)]
pub struct TeamRemoveArgs {
    /// The team's name.
    #[arg(value_parser = parse_slug)]
    pub team: String,
    /// Say yes without being asked.
    #[arg(long)]
    pub yes: bool,
}

/// Options for `team add`.
#[derive(Debug, Args)]
pub struct TeamAddArgs {
    /// The ready-made team (see `zorix team templates`).
    #[arg(long, value_name = "NAME")]
    pub template: String,
}

/// What `zorix skill` does.
#[derive(Debug, Subcommand)]
pub enum SkillCommand {
    /// List the skills in the library.
    List,
    /// Show a skill: what it is for, its files and where it came from.
    Show(SkillNameArg),
    /// Add a skill from a folder (with a SKILL.md) or a zip file.
    Import(SkillImportArgs),
    /// Forget a skill. Agents that pin it keep working until `skill gc`.
    Remove(SkillNameArg),
    /// Delete the files of skills no agent version refers to.
    Gc,
}

/// A skill's name.
#[derive(Debug, Args)]
pub struct SkillNameArg {
    /// The skill's name.
    pub name: String,
}

/// Options for `skill import`.
#[derive(Debug, Args)]
pub struct SkillImportArgs {
    /// A folder or a .zip file.
    pub path: PathBuf,
}

/// What `zorix eval` does.
#[derive(Debug, Subcommand)]
pub enum EvalCommand {
    /// Add a case (or a list of cases) from a JSON file, or from standard input with `-`.
    Add(EvalAddArgs),
    /// List an agent's cases.
    List(AgentSlugArg),
    /// Delete a case.
    Remove(EvalRemoveArgs),
    /// Run an agent's cases and say what passed.
    Run(EvalRunArgs),
    /// List the runs of an agent's evaluations.
    Runs(AgentSlugArg),
    /// Show one evaluation run.
    Show(EvalRunArg),
    /// Compare two runs case by case.
    Compare(EvalCompareArgs),
}

/// Options for `eval add`.
#[derive(Debug, Args)]
pub struct EvalAddArgs {
    /// The agent's name.
    #[arg(value_parser = parse_slug)]
    pub agent: String,
    /// A JSON file with a case or a list of cases, or `-` for standard input.
    pub file: PathBuf,
}

/// Options for `eval remove`.
#[derive(Debug, Args)]
pub struct EvalRemoveArgs {
    /// The agent's name.
    #[arg(value_parser = parse_slug)]
    pub agent: String,
    /// The case's name.
    pub case: String,
}

/// Options for `eval run`.
#[derive(Debug, Args)]
pub struct EvalRunArgs {
    /// The agent's name.
    #[arg(value_parser = parse_slug)]
    pub agent: String,
    /// A version other than the newest.
    #[arg(long, value_name = "N")]
    pub version: Option<u32>,
    /// A model to try instead of the agent's default. Repeat to try several: each one is its own run.
    #[arg(long = "model", value_name = "MODEL")]
    pub models: Vec<String>,
    /// Only cases with this tag. Repeat for more.
    #[arg(long = "tag", value_name = "TAG")]
    pub tags: Vec<String>,
    /// Take away network access, whatever the agent has.
    #[arg(long)]
    pub offline: bool,
    /// How many cases run at once (1 to 8).
    #[arg(long, default_value_t = 2, value_parser = clap::value_parser!(u8).range(1..=8))]
    pub concurrency: u8,
    /// Use only providers on this computer.
    #[arg(long)]
    pub local_only: bool,
}

/// An evaluation run's id.
#[derive(Debug, Args)]
pub struct EvalRunArg {
    /// The run's id.
    pub run: String,
}

/// Options for `eval compare`.
#[derive(Debug, Args)]
pub struct EvalCompareArgs {
    /// The first run's id.
    pub before: String,
    /// The second run's id.
    pub after: String,
}

/// A scope: where a memory or a folder applies.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ScopeArg(pub ScopeSpec);

/// A scope as typed: most are complete, an agent is named by its slug and looked up when the command runs.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ScopeSpec {
    /// A scope that needs nothing looked up.
    Fixed(zorix_core::Scope),
    /// The memory of the agent with this slug.
    Agent(String),
}

impl FromStr for ScopeArg {
    type Err = String;

    fn from_str(text: &str) -> Result<Self, String> {
        match text.split_once(':') {
            None if text == "global" => Ok(ScopeArg(ScopeSpec::Fixed(zorix_core::Scope::Global))),
            Some(("project", dir)) if !dir.is_empty() => {
                let real =
                    std::fs::canonicalize(dir).map_err(|e| format!("The project folder {dir} cannot be used: {e}"))?;
                Ok(ScopeArg(ScopeSpec::Fixed(zorix_core::Scope::Project(real.to_string_lossy().into_owned()))))
            }
            Some(("conversation", id)) => zorix_core::ConversationId::new(id)
                .map(|c| ScopeArg(ScopeSpec::Fixed(zorix_core::Scope::Conversation(c))))
                .map_err(|e| e.to_string()),
            Some(("agent", slug)) => {
                zorix_core::AgentSlug::new(slug).map(|s| ScopeArg(ScopeSpec::Agent(s.to_string())))
            }
            _ => Err("use global, project:<folder>, conversation:<id> or agent:<name>".into()),
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
    /// Look as this agent, which also sees what it remembers for itself.
    #[arg(long, value_name = "NAME", value_parser = parse_agent_slug)]
    pub agent: Option<String>,
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
    /// Keep the folders that are marked to be watched up to date until Ctrl-C: a change is read again a few seconds after it happens.
    Watch(KnowledgeWatchArgs),
}

/// Options for `knowledge watch`.
#[derive(Debug, Args)]
pub struct KnowledgeWatchArgs {
    /// Watch only this folder (by id or path), whether or not it is marked to be watched.
    pub folder: Option<String>,
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
    /// Mark it to be kept up to date while a program that watches runs (the desktop app, or `zorix knowledge watch`).
    #[arg(long)]
    pub watch: bool,
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

    /// Also list the sub-runs that agents started (by default only the runs somebody asked for are listed).
    #[arg(long)]
    pub all: bool,
    /// Only executions made by this agent.
    #[arg(long, value_name = "AGENT", value_parser = parse_slug)]
    pub agent: Option<String>,
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

/// Dollars as millionths of a dollar.
fn parse_usd(text: &str) -> Result<u64, String> {
    let value: f64 =
        text.trim().trim_start_matches('$').parse().map_err(|_| format!("“{text}” is not an amount of dollars."))?;
    if !value.is_finite() || value <= 0.0 || value > 100_000.0 {
        return Err("Give an amount of dollars above zero and at most 100000.".into());
    }
    Ok((value * 1_000_000.0).round() as u64)
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

fn parse_agent_slug(text: &str) -> Result<String, String> {
    zorix_core::AgentSlug::new(text).map(|s| s.to_string())
}

fn parse_tool_pattern(text: &str) -> Result<String, String> {
    zorix_core::validate_tool_pattern(text).map(|()| text.to_string())
}

fn parse_slug(text: &str) -> Result<String, String> {
    zorix_core::AgentSlug::new(text).map(|s| s.to_string())
}

fn parse_json(text: &str) -> Result<serde_json::Value, String> {
    serde_json::from_str(text).map_err(|e| format!("that is not JSON: {e}"))
}

fn parse_access_entry(text: &str) -> Result<String, String> {
    zorix_core::access::compile_entry(text).map(|_| text.to_string())
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

/// What `zorix workflow` does.
#[derive(Debug, Subcommand)]
pub enum WorkflowCommand {
    /// List workflows with what starts them, their last runs and how they are doing.
    List,
    /// Show a workflow: what it does, step by step, and what it needs that is missing.
    Show(WorkflowShowArgs),
    /// Make a new workflow with only a manual start, ready to edit.
    Create(WorkflowNameArg),
    /// Edit a workflow's draft as a file in $EDITOR. Nothing is published.
    Edit(WorkflowNameArg),
    /// Read a `.workflow.json` file into a workflow's draft (making the workflow when it is new).
    Import(WorkflowImportArgs),
    /// Print a workflow as a file (the draft, or a published version).
    Export(WorkflowExportArgs),
    /// List the published versions of a workflow.
    Versions(WorkflowNameArg),
    /// Show what changed between two versions (`draft` means the draft).
    Diff(WorkflowDiffArgs),
    /// Publish the draft as the next version. The next trigger uses it.
    Publish(WorkflowPublishArgs),
    /// Publish an old version again as a new one.
    Rollback(WorkflowRollbackArgs),
    /// Turn a workflow's triggers on.
    Enable(WorkflowNameArg),
    /// Turn a workflow's triggers off.
    Disable(WorkflowNameArg),
    /// Delete a workflow. Its runs stay.
    Remove(WorkflowNameArg),
    /// Start a run now and follow it until it ends or needs a person.
    Run(WorkflowRunArgs),
    /// List runs, newest first.
    Runs(WorkflowRunsArgs),
    /// Show one run: every step, what it did and what it cost.
    RunShow(WorkflowRunIdArg),
    /// Cancel a run.
    Cancel(WorkflowRunIdArg),
    /// List what is waiting for a person: approvals, questions and actions.
    Waits,
    /// Answer a wait: approve, refuse, or write the answer to a question.
    Answer(WorkflowAnswerArgs),
    /// Show a workflow's webhook address and signing secret, or replace them.
    Webhook(WorkflowWebhookArgs),
    /// List the starters.
    Templates,
    /// Add a starter as a draft that is off.
    Add(WorkflowTemplateArg),
}

/// A workflow's name.
#[derive(Debug, Args)]
pub struct WorkflowNameArg {
    /// The workflow's short name.
    pub workflow: String,
}

/// Options for `workflow show`.
#[derive(Debug, Args)]
pub struct WorkflowShowArgs {
    /// The workflow's short name.
    pub workflow: String,
    /// A published version instead of the draft.
    #[arg(long)]
    pub version: Option<u32>,
}

/// Options for `workflow import`.
#[derive(Debug, Args)]
pub struct WorkflowImportArgs {
    /// The file.
    pub file: std::path::PathBuf,
}

/// Options for `workflow export`.
#[derive(Debug, Args)]
pub struct WorkflowExportArgs {
    /// The workflow's short name.
    pub workflow: String,
    /// A published version instead of the draft.
    #[arg(long)]
    pub version: Option<u32>,
    /// Write to this file instead of printing.
    #[arg(long, short = 'o')]
    pub out: Option<std::path::PathBuf>,
}

/// Options for `workflow diff`.
#[derive(Debug, Args)]
pub struct WorkflowDiffArgs {
    /// The workflow's short name.
    pub workflow: String,
    /// The older version, or `draft`.
    pub from: String,
    /// The newer version, or `draft`.
    pub to: String,
}

/// Options for `workflow publish`.
#[derive(Debug, Args)]
pub struct WorkflowPublishArgs {
    /// The workflow's short name.
    pub workflow: String,
    /// Why, in a few words.
    #[arg(long)]
    pub note: Option<String>,
    /// Confirm that the steps that send without asking (or remind in chat) may do so.
    #[arg(long)]
    pub confirm_unattended: bool,
}

/// Options for `workflow rollback`.
#[derive(Debug, Args)]
pub struct WorkflowRollbackArgs {
    /// The workflow's short name.
    pub workflow: String,
    /// The version to publish again.
    pub version: u32,
    /// Confirm the steps that send without asking.
    #[arg(long)]
    pub confirm_unattended: bool,
}

/// Options for `workflow run`.
#[derive(Debug, Args)]
pub struct WorkflowRunArgs {
    /// The workflow's short name.
    pub workflow: String,
    /// What the trigger carries, as JSON.
    #[arg(long, value_name = "JSON")]
    pub input: Option<String>,
    /// Run the draft as a test: every action asks first.
    #[arg(long)]
    pub test: bool,
    /// Start from this trigger step (the manual one by default).
    #[arg(long)]
    pub trigger: Option<String>,
    #[command(flatten)]
    pub model: ModelArgs,
}

/// Options for `workflow runs`.
#[derive(Debug, Args)]
pub struct WorkflowRunsArgs {
    /// Only this workflow's runs.
    pub workflow: Option<String>,
    /// How many to show.
    #[arg(long, default_value_t = 20)]
    pub limit: usize,
}

/// A workflow run's id.
#[derive(Debug, Args)]
pub struct WorkflowRunIdArg {
    /// The run's id.
    pub run: String,
}

/// Options for `workflow answer`.
#[derive(Debug, Args)]
pub struct WorkflowAnswerArgs {
    /// The wait's id (`workflow waits` lists them).
    pub wait: String,
    /// Approve.
    #[arg(long, conflicts_with_all = ["refuse", "text"])]
    pub approve: bool,
    /// Refuse.
    #[arg(long, conflicts_with_all = ["approve", "text"])]
    pub refuse: bool,
    /// The answer to a question.
    #[arg(long, conflicts_with_all = ["approve", "refuse"])]
    pub text: Option<String>,
    /// A note to keep with the answer.
    #[arg(long)]
    pub note: Option<String>,
    /// The fingerprint you were shown, to be sure nothing changed. When absent, the current one is used.
    #[arg(long)]
    pub fingerprint: Option<String>,
    #[command(flatten)]
    pub model: ModelArgs,
}

/// Options for `workflow webhook`.
#[derive(Debug, Args)]
pub struct WorkflowWebhookArgs {
    /// The workflow's short name.
    pub workflow: String,
    /// Replace the secret in the address: the old address stops working.
    #[arg(long)]
    pub reset: bool,
    /// The address ZORIX listens on, when it is not the default.
    #[arg(long, value_name = "ADDRESS")]
    pub bind: Option<String>,
}

/// A starter's name.
#[derive(Debug, Args)]
pub struct WorkflowTemplateArg {
    /// The starter, from `workflow templates`.
    pub template: String,
}

// ---------------------------------------------------------------------------------------------------------------------
// Projects and tasks

/// What `zorix project` does.
#[derive(Debug, Subcommand)]
pub enum ProjectCommand {
    /// List projects with their progress, due date and manager.
    List(ProjectListArgs),
    /// Make a project. Its key starts the keys of its tasks (OPS-1, OPS-2…).
    Create(ProjectCreateArgs),
    /// Show a project: its figures and its tasks by lane.
    Show(ProjectKeyArg),
    /// Change a project's name, description, manager, dates or cost limit.
    Edit(ProjectEditArgs),
    /// Hide a project from the list. Nothing is deleted.
    Archive(ProjectKeyArg),
    /// Bring an archived project back.
    Restore(ProjectKeyArg),
    /// Delete a project with its tasks and history.
    Remove(ProjectKeyArg),
    /// Ask the project's manager to plan it, and follow its run. Every change it makes asks you first.
    Ask(ProjectAskArgs),
}

/// A project's key.
#[derive(Debug, Args)]
pub struct ProjectKeyArg {
    /// The project's key, such as OPS.
    pub project: String,
}

/// Options for `project list`.
#[derive(Debug, Args)]
pub struct ProjectListArgs {
    /// Show archived projects instead of active ones.
    #[arg(long)]
    pub archived: bool,
}

/// Options for `project create`.
#[derive(Debug, Args)]
pub struct ProjectCreateArgs {
    /// 2 to 5 capital letters or digits, starting with a letter. It cannot be changed or reused.
    pub key: String,
    /// What people call the project.
    pub name: String,
    /// What it is for.
    #[arg(long)]
    pub description: Option<String>,
    /// The agent that plans it.
    #[arg(long, value_name = "AGENT")]
    pub manager: Option<String>,
    /// The first day (YYYY-MM-DD).
    #[arg(long, value_name = "DATE")]
    pub start: Option<String>,
    /// The last day (YYYY-MM-DD).
    #[arg(long, value_name = "DATE")]
    pub due: Option<String>,
    /// The most one task run may cost, in dollars.
    #[arg(long, value_name = "USD")]
    pub cost_limit: Option<f64>,
}

/// Options for `project edit`. Say `none` to clear a value.
#[derive(Debug, Args)]
pub struct ProjectEditArgs {
    /// The project's key.
    pub project: String,
    /// A new name.
    #[arg(long)]
    pub name: Option<String>,
    /// A new description.
    #[arg(long)]
    pub description: Option<String>,
    /// The agent that plans it, or `none`.
    #[arg(long, value_name = "AGENT")]
    pub manager: Option<String>,
    /// The first day, or `none`.
    #[arg(long, value_name = "DATE")]
    pub start: Option<String>,
    /// The last day, or `none`.
    #[arg(long, value_name = "DATE")]
    pub due: Option<String>,
    /// The most one task run may cost, in dollars, or `none`.
    #[arg(long, value_name = "USD")]
    pub cost_limit: Option<String>,
}

/// Options for `project ask`.
#[derive(Debug, Args)]
pub struct ProjectAskArgs {
    /// The project's key.
    pub project: String,
    /// What to ask the manager. By default it plans the project.
    pub prompt: Option<String>,
    #[command(flatten)]
    pub model: ModelArgs,
}

/// What `zorix task` does.
#[derive(Debug, Subcommand)]
pub enum TaskCommand {
    /// List tasks of a project (or of all projects).
    List(TaskListArgs),
    /// Add a task to a project.
    Add(TaskAddArgs),
    /// Show a task: its details, what it waits for, its deliverables, runs and history.
    Show(TaskShowArgs),
    /// Change a task. Say `none` to clear a value.
    Edit(TaskEditArgs),
    /// Move a task to a lane. In progress starts the run of an agent's or a team's task.
    Move(TaskMoveArgs),
    /// Run a task's agent again, after a failure or with a note.
    Restart(TaskRestartArgs),
    /// Comment on a task.
    Comment(TaskCommentArgs),
    /// Put a link on a task, such as a pull request.
    Link(TaskLinkArgs),
    /// Put something a task produced on it: text, a file's contents or a link.
    Deliver(TaskDeliverArgs),
    /// Delete a task.
    Remove(TaskKeyArg),
    /// Show what happened in a project, or to one task.
    History(TaskHistoryArgs),
}

/// A task's key.
#[derive(Debug, Args)]
pub struct TaskKeyArg {
    /// The task's key, such as OPS-38.
    pub task: String,
}

/// Options for `task list`.
#[derive(Debug, Args)]
pub struct TaskListArgs {
    /// Only this project's tasks (its key).
    pub project: Option<String>,
    /// Only tasks in this lane: backlog, in-progress, review or done.
    #[arg(long)]
    pub state: Option<String>,
    /// Only tasks you have.
    #[arg(long)]
    pub mine: bool,
    /// Only tasks that need you.
    #[arg(long)]
    pub needs_you: bool,
}

/// Options for `task add`.
#[derive(Debug, Args)]
pub struct TaskAddArgs {
    /// The project's key.
    pub project: String,
    /// The task's title.
    pub title: String,
    /// What to do.
    #[arg(long)]
    pub description: Option<String>,
    /// Who has it: `you`, an agent, `agent:NAME` or `team:NAME`.
    #[arg(long, value_name = "WHO")]
    pub assign: Option<String>,
    /// urgent, high, normal or low.
    #[arg(long, default_value = "normal")]
    pub priority: String,
    /// The first day (YYYY-MM-DD).
    #[arg(long, value_name = "DATE")]
    pub start: Option<String>,
    /// The last day (YYYY-MM-DD).
    #[arg(long, value_name = "DATE")]
    pub due: Option<String>,
    /// A task that must be done first (repeat for more).
    #[arg(long, value_name = "TASK")]
    pub after: Vec<String>,
    /// The lane it starts in (backlog by default).
    #[arg(long)]
    pub state: Option<String>,
    #[command(flatten)]
    pub model: ModelArgs,
}

/// Options for `task show`.
#[derive(Debug, Args)]
pub struct TaskShowArgs {
    /// The task's key.
    pub task: String,
    /// Print the text of deliverables, not only their titles.
    #[arg(long)]
    pub full: bool,
}

/// Options for `task edit`. Say `none` to clear a value.
#[derive(Debug, Args)]
pub struct TaskEditArgs {
    /// The task's key.
    pub task: String,
    /// A new title.
    #[arg(long)]
    pub title: Option<String>,
    /// A new description.
    #[arg(long)]
    pub description: Option<String>,
    /// urgent, high, normal or low.
    #[arg(long)]
    pub priority: Option<String>,
    /// Who has it: `you`, an agent, `agent:NAME`, `team:NAME` or `none`.
    #[arg(long, value_name = "WHO")]
    pub assign: Option<String>,
    /// The first day, or `none`.
    #[arg(long, value_name = "DATE")]
    pub start: Option<String>,
    /// The last day, or `none`.
    #[arg(long, value_name = "DATE")]
    pub due: Option<String>,
    /// How far it is, from 0 to 100.
    #[arg(long)]
    pub progress: Option<u8>,
    /// Replace what it waits for with these tasks (repeat for more; `none` clears).
    #[arg(long, value_name = "TASK")]
    pub after: Vec<String>,
    #[command(flatten)]
    pub model: ModelArgs,
}

/// Options for `task move`.
#[derive(Debug, Args)]
pub struct TaskMoveArgs {
    /// The task's key.
    pub task: String,
    /// The lane: backlog, in-progress, review or done.
    pub state: String,
    /// When sending a task back: what to change. It reaches the run that starts again.
    #[arg(long)]
    pub note: Option<String>,
    /// Stop the run that is going when the move takes the task out of In progress.
    #[arg(long)]
    pub stop_run: bool,
    /// Do not wait for the run: return as soon as it has started.
    #[arg(long)]
    pub no_wait: bool,
    #[command(flatten)]
    pub model: ModelArgs,
}

/// Options for `task restart`.
#[derive(Debug, Args)]
pub struct TaskRestartArgs {
    /// The task's key.
    pub task: String,
    /// What to change this time.
    #[arg(long)]
    pub note: Option<String>,
    /// Do not wait for the run.
    #[arg(long)]
    pub no_wait: bool,
    #[command(flatten)]
    pub model: ModelArgs,
}

/// Options for `task comment`.
#[derive(Debug, Args)]
pub struct TaskCommentArgs {
    /// The task's key.
    pub task: String,
    /// What to say.
    pub text: String,
}

/// Options for `task link`.
#[derive(Debug, Args)]
pub struct TaskLinkArgs {
    /// The task's key.
    pub task: String,
    /// An http or https address.
    pub url: String,
    /// A name for the link.
    #[arg(long)]
    pub title: Option<String>,
}

/// Options for `task deliver`. Give exactly one of --text, --file and --url.
#[derive(Debug, Args)]
pub struct TaskDeliverArgs {
    /// The task's key.
    pub task: String,
    /// What the deliverable is called.
    #[arg(long)]
    pub title: Option<String>,
    /// The text.
    #[arg(long)]
    pub text: Option<String>,
    /// A file whose text to attach.
    #[arg(long, value_name = "FILE")]
    pub file: Option<PathBuf>,
    /// An address.
    #[arg(long)]
    pub url: Option<String>,
}

/// Options for `task history`.
#[derive(Debug, Args)]
pub struct TaskHistoryArgs {
    /// The project's key, or a task's key to see only that task.
    pub target: String,
    /// How many lines.
    #[arg(long, default_value_t = 30)]
    pub limit: usize,
}

// ---------------------------------------------------------------------------------------------------------------------
// The workforce

/// What `zorix settings` does.
#[derive(Debug, Subcommand)]
pub enum SettingsCommand {
    /// Show every setting and its value.
    Show,
    /// Change one setting. The keys are `context.summaries` (on or off), `retention.days` (a number, or off to keep runs for ever),
    /// `retention.mode` (compact or delete), `telemetry.endpoint` (an address, or off) and `knowledge.ocr` (on or off).
    Set(SettingsSetArgs),
}

/// What `zorix retention` does.
#[derive(Debug, Subcommand)]
pub enum RetentionCommand {
    /// Say what a pass would do with the current settings, without changing anything.
    Show,
    /// Do a pass now. With `--dry-run` it only says what it would do.
    Run(RetentionRunArgs),
}

/// Options for `retention run`.
#[derive(Debug, Args)]
pub struct RetentionRunArgs {
    /// Count what would be compacted or deleted and change nothing.
    #[arg(long)]
    pub dry_run: bool,
}

/// Options for `settings set`.
#[derive(Debug, Args)]
pub struct SettingsSetArgs {
    /// The setting, such as `retention.days`.
    pub key: String,
    /// Its new value.
    pub value: String,
}

/// What `zorix org` does.
#[derive(Debug, Subcommand)]
pub enum OrgCommand {
    /// Show the organisation: the numbers, the chart and the settings.
    Show,
    /// Change the weekly budget for everyone or the level new workers start at.
    Settings(OrgSettingsArgs),
    /// Departments: groups of workers.
    #[command(subcommand)]
    Dept(DeptCommand),
    /// What the workers did, newest first.
    Activity(ActivityArgs),
    /// What is worth your eye: plans and reports, proposals, calls that ran without asking, budget warnings.
    Notices(NoticesArgs),
}

/// Options for `org settings`. Say `none` for no limit.
#[derive(Debug, Args)]
pub struct OrgSettingsArgs {
    /// The most all workers may spend in a week, in dollars.
    #[arg(long, value_name = "USD")]
    pub budget: Option<String>,
    /// The level new workers start at: suggest, ask, notify, budgeted or full.
    #[arg(long, value_name = "LEVEL")]
    pub level: Option<String>,
}

/// What `zorix org dept` does.
#[derive(Debug, Subcommand)]
pub enum DeptCommand {
    /// Add a department.
    Add(DeptAddArgs),
    /// Rename a department or change its description or lead. Say `none` to clear the lead.
    Edit(DeptEditArgs),
    /// Remove a department. It must have no workers.
    Remove(DeptNameArg),
}

/// A department's short name.
#[derive(Debug, Args)]
pub struct DeptNameArg {
    /// Its short name: lower case, digits and hyphens.
    pub department: String,
}

/// Options for `org dept add`.
#[derive(Debug, Args)]
pub struct DeptAddArgs {
    /// Its short name (it never changes).
    pub department: String,
    /// What people call it.
    pub name: String,
    /// What it is for.
    #[arg(long)]
    pub description: Option<String>,
    /// The worker that leads it.
    #[arg(long, value_name = "WORKER")]
    pub lead: Option<String>,
}

/// Options for `org dept edit`.
#[derive(Debug, Args)]
pub struct DeptEditArgs {
    /// Its short name.
    pub department: String,
    /// What people call it.
    #[arg(long)]
    pub name: Option<String>,
    /// What it is for.
    #[arg(long)]
    pub description: Option<String>,
    /// The worker that leads it, or `none`.
    #[arg(long, value_name = "WORKER")]
    pub lead: Option<String>,
}

/// Options for `org activity`.
#[derive(Debug, Args)]
pub struct ActivityArgs {
    /// How many lines.
    #[arg(long, default_value_t = 30)]
    pub limit: usize,
}

/// Options for `org notices`.
#[derive(Debug, Args)]
pub struct NoticesArgs {
    /// Mark everything up to now as read.
    #[arg(long)]
    pub read: bool,
}

/// What `zorix worker` does.
#[derive(Debug, Subcommand)]
pub enum WorkerCommand {
    /// List the workers with their level, tasks, on-time rate, eval and spend this week.
    List,
    /// Make an agent a worker. It starts at the level the organisation sets for new workers unless you say.
    Hire(WorkerHireArgs),
    /// Show a worker: its figures, its autonomy, its open tasks and its week.
    Show(WorkerNameArg),
    /// Change a worker. Say `none` to clear a value. A worker cannot be freer or spend more than the one it reports to.
    Edit(WorkerEditArgs),
    /// Change how free a worker is: suggest, ask, notify, budgeted or full.
    Level(WorkerLevelArgs),
    /// Let a worker go. It must have no reports.
    Fire(WorkerNameArg),
    /// Ask a manager to plan the week. It runs like any other run of that worker, and follows its level and budget.
    Plan(WorkerAskArgs),
    /// Ask a manager for the weekly report.
    Report(WorkerAskArgs),
    /// Turn a manager's Monday plan and Friday report on or off. They run only while the app or `zorix serve` is open.
    Schedule(WorkerScheduleArgs),
}

/// A worker's agent.
#[derive(Debug, Args)]
pub struct WorkerNameArg {
    /// The agent's name.
    pub worker: String,
}

/// Options for `worker hire`.
#[derive(Debug, Args)]
pub struct WorkerHireArgs {
    /// The agent to hire.
    pub agent: String,
    /// What it does there, such as "Engineering lead".
    #[arg(long)]
    pub role: String,
    /// Its department's short name.
    #[arg(long, value_name = "DEPARTMENT")]
    pub dept: String,
    /// Who it answers to: a worker, or `you`.
    #[arg(long, value_name = "WORKER", default_value = "you")]
    pub reports_to: String,
    /// A virtual manager: it plans the week and writes the report.
    #[arg(long)]
    pub manager: bool,
    /// How free it is: suggest, ask, notify, budgeted or full.
    #[arg(long, value_name = "LEVEL")]
    pub level: Option<String>,
    /// The most it may spend in a week, in dollars.
    #[arg(long, value_name = "USD")]
    pub budget: Option<f64>,
    /// The most active time a task may take, in minutes.
    #[arg(long, value_name = "MINUTES")]
    pub task_time: Option<u64>,
    /// An access pattern whose calls always ask, such as `repo:acme/web:write`. Repeat for several.
    #[arg(long, value_name = "ACCESS")]
    pub still_ask: Vec<String>,
}

/// Options for `worker edit`. Only what you give changes.
#[derive(Debug, Args)]
pub struct WorkerEditArgs {
    /// The agent's name.
    pub worker: String,
    /// What it does there.
    #[arg(long)]
    pub role: Option<String>,
    /// Its department's short name.
    #[arg(long, value_name = "DEPARTMENT")]
    pub dept: Option<String>,
    /// Who it answers to: a worker, or `you`.
    #[arg(long, value_name = "WORKER")]
    pub reports_to: Option<String>,
    /// Make it a manager (`true`) or not (`false`).
    #[arg(long, value_name = "BOOL")]
    pub manager: Option<bool>,
    /// How free it is.
    #[arg(long, value_name = "LEVEL")]
    pub level: Option<String>,
    /// The most it may spend in a week, in dollars, or `none`.
    #[arg(long, value_name = "USD")]
    pub budget: Option<String>,
    /// The most active time a task may take, in minutes.
    #[arg(long, value_name = "MINUTES")]
    pub task_time: Option<u64>,
    /// Replace the access patterns that always ask. Repeat for several; `none` clears them.
    #[arg(long, value_name = "ACCESS")]
    pub still_ask: Vec<String>,
}

/// Options for `worker level`.
#[derive(Debug, Args)]
pub struct WorkerLevelArgs {
    /// The agent's name.
    pub worker: String,
    /// suggest, ask, notify, budgeted or full.
    pub level: String,
}

/// Options for `worker plan` and `worker report`.
#[derive(Debug, Args)]
pub struct WorkerAskArgs {
    /// The manager's agent.
    pub worker: String,
    /// Something to add to the request.
    pub prompt: Option<String>,
    #[command(flatten)]
    pub model: ModelArgs,
}

/// Options for `worker schedule`.
#[derive(Debug, Args)]
pub struct WorkerScheduleArgs {
    /// The manager's agent.
    pub worker: String,
    /// `on` or `off`.
    pub state: String,
}
