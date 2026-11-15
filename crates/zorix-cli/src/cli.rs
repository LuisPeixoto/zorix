//! The command line, as `clap` types. Every value is checked here, by value parsers, so a bad argument is reported
//! before anything touches the data folder.

use std::path::PathBuf;
use std::str::FromStr;

use clap::{Args, Parser, Subcommand, ValueEnum};
use zorix_core::{ExecutionId, ExecutionState, TenantId};

/// Run AI activities under control: permissions, approvals, limits, and a record of everything.
#[derive(Debug, Parser)]
#[command(name = "zorix", version, about, long_about = None)]
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
    /// Print where ZORIX keeps its data.
    DataDir,
}

/// Which model answers.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ModelChoice {
    /// The built-in rule-based demo model.
    Demo,
    /// A scripted model read from a JSON file. Its position in the script lives in this process only.
    Script(PathBuf),
}

impl FromStr for ModelChoice {
    type Err = String;

    fn from_str(text: &str) -> Result<Self, String> {
        match text {
            "demo" => Ok(ModelChoice::Demo),
            other => match other.strip_prefix("script:") {
                Some(path) if !path.is_empty() => Ok(ModelChoice::Script(PathBuf::from(path))),
                _ => Err("use “demo” or “script:<file.json>”".into()),
            },
        }
    }
}

impl ModelChoice {
    /// The name recorded in the execution.
    pub fn label(&self) -> &'static str {
        match self {
            ModelChoice::Demo => "demo",
            ModelChoice::Script(_) => "script",
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
    /// The model: `demo`, or `script:<file.json>` for a scripted one.
    #[arg(long, default_value = "demo", value_name = "MODEL")]
    pub model: ModelChoice,
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
        for bad in ["", "script:", "gpt", "Demo"] {
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
