//! Agents: named, reusable definitions that create executions (ADR-0024).
//!
//! An agent is a slug plus a sequence of immutable versions. A version holds everything that shapes a run: instructions,
//! skills (pinned by content hash), tools and access, the models it may use, limits and memory. A run records the version it
//! came from as an [`AgentBinding`], frozen at creation like the tool pins.

use std::collections::BTreeMap;
use std::fmt;
use std::str::FromStr;

use async_trait::async_trait;
use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::approval::canonical_hash;
use crate::error::{Error, Result};
use crate::execution::Limits;
use crate::ids::{AgentId, ConversationId, EvalCaseId, EvalRunId, ExecutionId, TenantId};
use crate::memory::MemoryOptions;
use crate::model::GenerationParams;
use crate::permission::{Grants, validate_tool_pattern};
use crate::time::Timestamp;

/// The metadata key that marks an execution as part of an evaluation. Such runs are not recorded as past work.
pub const EVAL_LABEL: &str = "zorix.eval";
/// Most agents a tenant keeps.
pub const MAX_AGENTS: usize = 200;
/// Most versions kept for one agent.
pub const MAX_VERSIONS: usize = 500;
/// Most skills one agent version names.
pub const MAX_SKILLS_PER_AGENT: usize = 32;
/// Most characters of instructions.
pub const MAX_INSTRUCTIONS: usize = 50_000;
/// Most characters of a description.
pub const MAX_DESCRIPTION: usize = 280;

/// An agent's short name: lowercase letters, digits and single hyphens, 1 to 48 characters.
#[derive(Clone, Debug, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(try_from = "String", into = "String")]
pub struct AgentSlug(String);

impl AgentSlug {
    /// Check and wrap a slug.
    pub fn new(value: impl Into<String>) -> std::result::Result<Self, String> {
        let value = value.into();
        let ok = (1..=48).contains(&value.len())
            && !value.starts_with('-')
            && !value.ends_with('-')
            && !value.contains("--")
            && value.chars().all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '-');
        if ok {
            Ok(Self(value))
        } else {
            Err(format!(
                "“{value}” is not a valid agent name: use 1 to 48 lowercase letters, digits and single hyphens."
            ))
        }
    }

    /// The slug as text.
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl fmt::Display for AgentSlug {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}

impl FromStr for AgentSlug {
    type Err = String;

    fn from_str(text: &str) -> std::result::Result<Self, String> {
        Self::new(text)
    }
}

impl TryFrom<String> for AgentSlug {
    type Error = String;

    fn try_from(value: String) -> std::result::Result<Self, String> {
        Self::new(value)
    }
}

impl From<AgentSlug> for String {
    fn from(slug: AgentSlug) -> String {
        slug.0
    }
}

/// How an agent looks: an emoji and one of the design system's colour names.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Avatar {
    /// One emoji (or a short symbol).
    pub emoji: String,
    /// A colour name: `accent`, `ok`, `warn`, `err`, `info` or `muted`.
    pub color: String,
}

impl Default for Avatar {
    fn default() -> Self {
        Self { emoji: "🤖".into(), color: "accent".into() }
    }
}

/// A skill an agent uses, pinned to the exact content it had when the version was written.
#[derive(Clone, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct SkillPin {
    /// The skill's name.
    pub name: String,
    /// The SHA-256 of its content (64 hex characters).
    pub hash: String,
}

/// The models an agent may use.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct AgentModels {
    /// What a run uses when it names no model: `provider/model`, a route, an alias, or `demo`.
    pub default: String,
    /// Which models are allowed: `provider/model`, `provider/*`, `demo` or `*`. Empty means any (ADR-0027).
    #[serde(default)]
    pub allowed: Vec<String>,
}

/// Whether `model` (served by `provider`, or none for the built-in simulated models) matches one of the patterns.
/// An empty list matches everything.
pub fn model_allowed(patterns: &[String], provider: Option<&str>, model: &str) -> bool {
    if patterns.is_empty() {
        return true;
    }
    patterns.iter().any(|pattern| match pattern.as_str() {
        "*" => true,
        p => match p.split_once('/') {
            Some((pp, "*")) => provider == Some(pp),
            Some((pp, pm)) => provider == Some(pp) && pm == model,
            // The built-in simulated models (`demo`, `script`) have no real provider; the router names it `sim`.
            None => matches!(provider, None | Some("sim")) && p == model,
        },
    })
}

/// Check one entry of an allow-list.
pub fn validate_model_pattern(pattern: &str) -> std::result::Result<(), String> {
    let ok = pattern == "*"
        || pattern == "demo"
        || pattern.split_once('/').is_some_and(|(provider, model)| {
            !provider.is_empty()
                && !model.is_empty()
                && provider.len() <= 64
                && model.len() <= 128
                && !provider.contains('*')
                && (model == "*" || !model.contains('*'))
                && !pattern.chars().any(|c| c.is_whitespace() || c.is_control())
        });
    if ok {
        Ok(())
    } else {
        Err(format!("“{pattern}” is not a model pattern: use provider/model, provider/*, demo or *."))
    }
}

/// What a version of an agent is. The hash of its canonical JSON identifies it.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct AgentSpec {
    /// Its short name.
    pub slug: AgentSlug,
    /// What people call it.
    pub name: String,
    /// One sentence about what it is for.
    #[serde(default)]
    pub description: String,
    /// How it looks.
    #[serde(default)]
    pub avatar: Avatar,
    /// What it is told, in Markdown.
    #[serde(default)]
    pub instructions: String,
    /// The skills it can load.
    #[serde(default)]
    pub skills: Vec<SkillPin>,
    /// Tool patterns it may use (`math.*`, `clock.now`).
    #[serde(default)]
    pub tools: Vec<String>,
    /// What it may reach, in the syntax of [`crate::access`]: `http:HOST`, `repo:OWNER/REPO[:write]`, `db:NAME`, `mcp:SERVER`…
    #[serde(default)]
    pub access: Vec<String>,
    /// The models it may use.
    pub models: AgentModels,
    /// Its limits.
    #[serde(default)]
    pub limits: Limits,
    /// What it uses from memory.
    #[serde(default = "default_memory")]
    pub memory: MemoryOptions,
    /// Whether it has memory of its own (the agent scope).
    #[serde(default = "yes")]
    pub agent_memory: bool,
    /// Output size and effort.
    #[serde(default)]
    pub params: GenerationParams,
    /// A JSON Schema the final answer must satisfy.
    #[serde(default)]
    pub expected_output: Option<Value>,
    /// Free labels.
    #[serde(default)]
    pub metadata: BTreeMap<String, String>,
}

fn yes() -> bool {
    true
}

fn default_memory() -> MemoryOptions {
    MemoryOptions::default()
}

impl AgentSpec {
    /// A spec with a name, a default model and nothing else.
    pub fn new(slug: AgentSlug, name: impl Into<String>, default_model: impl Into<String>) -> Self {
        Self {
            slug,
            name: name.into(),
            description: String::new(),
            avatar: Avatar::default(),
            instructions: String::new(),
            skills: Vec::new(),
            tools: Vec::new(),
            access: Vec::new(),
            models: AgentModels { default: default_model.into(), allowed: Vec::new() },
            limits: Limits::default(),
            memory: MemoryOptions::default(),
            agent_memory: true,
            params: GenerationParams::default(),
            expected_output: None,
            metadata: BTreeMap::new(),
        }
    }

    /// The SHA-256 of the spec in canonical JSON (sorted keys, no spaces). Two specs with the same hash are the same version.
    pub fn hash(&self) -> String {
        canonical_hash(&serde_json::to_value(self).unwrap_or(Value::Null))
    }

    /// The grants of the agent's tools and access, before a run narrows them.
    pub fn grants(&self) -> Result<Grants> {
        let mut grants = crate::access::compile_access(&self.access).map_err(Error::InvalidInput)?;
        for pattern in &self.tools {
            grants = grants.with(crate::permission::tool_grant(pattern).map_err(Error::InvalidInput)?);
        }
        Ok(grants)
    }

    /// Check everything a person typed.
    pub fn validate(&self) -> Result<()> {
        let bad = |what: String| Err(Error::InvalidInput(what));
        if self.name.trim().is_empty() || self.name.chars().count() > 80 {
            return bad("The name has 1 to 80 characters.".into());
        }
        if self.description.chars().count() > MAX_DESCRIPTION {
            return bad(format!("The description has {MAX_DESCRIPTION} characters at most."));
        }
        if self.instructions.chars().count() > MAX_INSTRUCTIONS {
            return bad(format!("The instructions have {MAX_INSTRUCTIONS} characters at most."));
        }
        if !["accent", "ok", "warn", "err", "info", "muted"].contains(&self.avatar.color.as_str())
            || self.avatar.emoji.trim().is_empty()
            || self.avatar.emoji.chars().count() > 8
        {
            return bad("The avatar is one emoji and one of the colours accent, ok, warn, err, info or muted.".into());
        }
        if self.skills.len() > MAX_SKILLS_PER_AGENT {
            return bad(format!("An agent names {MAX_SKILLS_PER_AGENT} skills at most."));
        }
        let mut seen = std::collections::HashSet::new();
        for pin in &self.skills {
            if !valid_skill_name(&pin.name) {
                return bad(format!("“{}” is not a skill name.", pin.name));
            }
            if pin.hash.len() != 64 || !pin.hash.chars().all(|c| c.is_ascii_hexdigit()) {
                return bad(format!("The hash of the skill {} is not a SHA-256.", pin.name));
            }
            if !seen.insert(&pin.name) {
                return bad(format!("The skill {} is named twice.", pin.name));
            }
        }
        if self.tools.len() > 64 || self.access.len() > 64 {
            return bad("An agent has 64 tool patterns and 64 access entries at most.".into());
        }
        for pattern in &self.tools {
            validate_tool_pattern(pattern).map_err(Error::InvalidInput)?;
        }
        self.grants()?;
        if self.models.default.trim().is_empty() {
            return bad("Choose the model the agent uses by default.".into());
        }
        if self.models.allowed.len() > 32 {
            return bad("An agent allows 32 model patterns at most.".into());
        }
        for pattern in &self.models.allowed {
            validate_model_pattern(pattern).map_err(Error::InvalidInput)?;
        }
        if self.metadata.len() > 32 || self.metadata.iter().any(|(k, v)| k.len() > 64 || v.len() > 512) {
            return bad("Metadata has 32 entries at most, with short keys and values.".into());
        }
        if self.expected_output.as_ref().is_some_and(|schema| schema.to_string().len() > 64 * 1024) {
            return bad("The expected output schema is larger than 64 KB.".into());
        }
        Ok(())
    }
}

/// A skill name in the Agent Skills format: 1 to 64 lowercase letters, digits and single hyphens, ASCII.
pub fn valid_skill_name(name: &str) -> bool {
    (1..=64).contains(&name.len())
        && !name.starts_with('-')
        && !name.ends_with('-')
        && !name.contains("--")
        && name.chars().all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '-')
}

/// Most teams a tenant may have.
pub const MAX_TEAMS: usize = 100;

/// A lead and the colleagues it may ask (ADR-0031). A plain, editable record: it grants, it does not script.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Team {
    /// The owner.
    pub tenant: TenantId,
    /// Its short name (never changes).
    pub slug: AgentSlug,
    /// What people call it.
    pub name: String,
    /// One sentence about what it is for.
    #[serde(default)]
    pub description: String,
    /// How it looks.
    #[serde(default)]
    pub avatar: Avatar,
    /// The agent that receives the work and may ask the others.
    pub lead: AgentSlug,
    /// The agents the lead may ask.
    #[serde(default)]
    pub members: Vec<AgentSlug>,
    /// When it was last changed.
    pub updated_at: Timestamp,
}

impl Team {
    /// Check what a person typed.
    pub fn validate(&self) -> Result<()> {
        let bad = |what: String| Err(Error::InvalidInput(what));
        if self.name.trim().is_empty() || self.name.chars().count() > 80 {
            return bad("The name has 1 to 80 characters.".into());
        }
        if self.description.chars().count() > MAX_DESCRIPTION {
            return bad(format!("The description has {MAX_DESCRIPTION} characters at most."));
        }
        if self.members.len() > 12 {
            return bad("A team has 12 members at most.".into());
        }
        if self.members.contains(&self.lead) {
            return bad("The lead is not also a member: the lead asks the members.".into());
        }
        for (i, member) in self.members.iter().enumerate() {
            if self.members[..i].contains(member) {
                return bad(format!("{member} is in the team twice."));
            }
        }
        if !["accent", "ok", "warn", "err", "info", "muted"].contains(&self.avatar.color.as_str()) {
            return bad("The colour is accent, ok, warn, err, info or muted.".into());
        }
        Ok(())
    }

    /// The access entries the lead gets for this team: ask each member, and delegate.
    pub fn access(&self) -> Vec<String> {
        self.members.iter().map(|m| format!("agent:{m}")).collect()
    }
}

/// An agent as listed.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Agent {
    /// Its id.
    pub id: AgentId,
    /// The owner.
    pub tenant: TenantId,
    /// Its short name (never changes).
    pub slug: AgentSlug,
    /// The newest version.
    pub current_version: u32,
    /// Hidden from lists and pickers, kept with its history and memory.
    pub archived: bool,
    /// When it was made.
    pub created_at: Timestamp,
    /// When its newest version was written.
    pub updated_at: Timestamp,
}

/// One immutable version of an agent.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct AgentVersion {
    /// The agent.
    pub agent: AgentId,
    /// 1, 2, 3…
    pub version: u32,
    /// [`AgentSpec::hash`] of `spec`.
    pub hash: String,
    /// What it was.
    pub spec: AgentSpec,
    /// Why it was made ("rollback to v2", a person's note).
    #[serde(default)]
    pub note: Option<String>,
    /// When.
    pub created_at: Timestamp,
}

/// What an execution records about the agent that made it. Frozen at creation (ADR-0024).
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct AgentBinding {
    /// The agent.
    pub agent: AgentId,
    /// Its slug.
    pub slug: AgentSlug,
    /// The version the run came from.
    pub version: u32,
    /// That version's hash.
    pub hash: String,
    /// The skills the run may load.
    #[serde(default)]
    pub skills: Vec<SkillPin>,
    /// The model patterns the run may use (never empty: `*` means any).
    pub allowed_models: Vec<String>,
    /// Whether the run sees and writes the agent's own memory (ADR-0026).
    #[serde(default = "yes")]
    pub agent_memory: bool,
    /// The team the run was made for, when one was asked (ADR-0031).
    #[serde(default)]
    pub team: Option<String>,
}

impl AgentBinding {
    /// The binding of a version.
    pub fn of(agent: &Agent, version: &AgentVersion) -> Self {
        let mut allowed = version.spec.models.allowed.clone();
        if allowed.is_empty() {
            allowed.push("*".into());
        }
        Self {
            agent: agent.id,
            slug: agent.slug.clone(),
            version: version.version,
            hash: version.hash.clone(),
            skills: version.spec.skills.clone(),
            allowed_models: allowed,
            agent_memory: version.spec.agent_memory,
            team: None,
        }
    }

    /// Whether a model may be used by a run with this binding.
    pub fn allows_model(&self, provider: Option<&str>, model: &str) -> bool {
        model_allowed(&self.allowed_models, provider, model)
    }
}

// ---------------------------------------------------------------------------------------------------------------------
// Skills

/// One file of a skill.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct SkillFile {
    /// Its path relative to the skill folder, with `/`.
    pub path: String,
    /// Its size in bytes.
    pub size: u64,
    /// The SHA-256 of its content.
    pub sha256: String,
}

/// A skill as known to the library: what its `SKILL.md` says and what it contains.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct SkillRecord {
    /// Its name.
    pub name: String,
    /// The SHA-256 of its content.
    pub hash: String,
    /// What it does and when to use it.
    pub description: String,
    /// Its files, sorted by path.
    pub files: Vec<SkillFile>,
    /// The licence text, when it says one.
    #[serde(default)]
    pub license: Option<String>,
    /// What it needs.
    #[serde(default)]
    pub compatibility: Option<String>,
    /// Its metadata.
    #[serde(default)]
    pub metadata: BTreeMap<String, String>,
    /// The tools it asks to be pre-approved. Shown to a person; never granted.
    #[serde(default)]
    pub allowed_tools: Vec<String>,
    /// Where it came from (a folder, a zip, a template).
    #[serde(default)]
    pub source: Option<String>,
    /// When it was imported.
    pub imported_at: Timestamp,
}

// ---------------------------------------------------------------------------------------------------------------------
// Evaluations

/// A check on what an execution did.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum EvalCheck {
    /// The answer contains this text.
    Contains {
        /// The text.
        text: String,
        /// Ignore case.
        #[serde(default)]
        ignore_case: bool,
    },
    /// The answer does not contain this text.
    NotContains {
        /// The text.
        text: String,
        /// Ignore case.
        #[serde(default)]
        ignore_case: bool,
    },
    /// The answer matches a regular expression.
    Regex {
        /// The expression.
        pattern: String,
    },
    /// The answer (its structured form, or the text read as JSON) satisfies a JSON Schema.
    JsonSchema {
        /// The schema.
        schema: Value,
    },
    /// A tool matching the pattern was called at least `min` times (default 1).
    ToolUsed {
        /// A tool id or a prefix ending in `*`.
        pattern: String,
        /// At least this many calls.
        #[serde(default = "one")]
        min: u32,
    },
    /// No tool matching the pattern was called.
    ToolNotUsed {
        /// A tool id or a prefix ending in `*`.
        pattern: String,
    },
    /// The run cost at most this many US dollars.
    MaxCostUsd {
        /// The most.
        usd: f64,
    },
    /// The run took at most this many model steps.
    MaxSteps {
        /// The most.
        steps: u32,
    },
    /// The run made at most this many tool calls.
    MaxToolCalls {
        /// The most.
        calls: u32,
    },
    /// The run ended in this state (`completed` by default).
    Status {
        /// The state's name.
        state: String,
    },
    /// The run asked this agent for at least `min` tasks (default 1) with `agents.delegate`.
    Delegated {
        /// The agent's slug.
        agent: String,
        /// At least this many tasks.
        #[serde(default = "one")]
        min: u32,
    },
}

fn one() -> u32 {
    1
}

/// A model that grades an answer against a rubric.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct EvalJudge {
    /// What a good answer is, in words.
    pub rubric: String,
    /// The model that grades (`provider/model`, a route or an alias).
    pub model: String,
    /// The lowest score (1 to 5) that passes.
    #[serde(default = "pass_score")]
    pub pass_score: u8,
}

fn pass_score() -> u8 {
    4
}

/// One thing to try an agent on.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct EvalCase {
    /// A short name, unique within the agent.
    pub name: String,
    /// What to ask.
    pub prompt: String,
    /// Data that goes with the prompt.
    #[serde(default)]
    pub input: Option<Value>,
    /// Labels for choosing which cases to run.
    #[serde(default)]
    pub tags: Vec<String>,
    /// What must hold.
    #[serde(default)]
    pub checks: Vec<EvalCheck>,
    /// An optional grader.
    #[serde(default)]
    pub judge: Option<EvalJudge>,
    /// Tools (patterns) whose approval is given automatically for this case. Never for tools only a person may approve.
    #[serde(default)]
    pub approve: Vec<String>,
    /// Keep the `write` permissions the agent has (they are dropped by default).
    #[serde(default)]
    pub allow_writes: bool,
}

impl EvalCase {
    /// Check what a person typed.
    pub fn validate(&self) -> Result<()> {
        let bad = |what: &str| Err(Error::InvalidInput(what.to_string()));
        if self.name.trim().is_empty() || self.name.len() > 80 {
            return bad("A case name has 1 to 80 characters.");
        }
        if self.prompt.trim().is_empty() || self.prompt.chars().count() > 20_000 {
            return bad("A case prompt has 1 to 20000 characters.");
        }
        if self.checks.is_empty() && self.judge.is_none() {
            return bad("A case needs at least one check or a judge.");
        }
        if self.checks.len() > 32 || self.tags.len() > 16 || self.approve.len() > 16 {
            return bad("A case has 32 checks, 16 tags and 16 approvals at most.");
        }
        for pattern in self.approve.iter().chain(self.checks.iter().filter_map(|c| match c {
            EvalCheck::ToolUsed { pattern, .. } | EvalCheck::ToolNotUsed { pattern } => Some(pattern),
            _ => None,
        })) {
            validate_tool_pattern(pattern).map_err(Error::InvalidInput)?;
        }
        for check in &self.checks {
            if let EvalCheck::Delegated { agent, .. } = check {
                AgentSlug::new(agent.as_str()).map_err(Error::InvalidInput)?;
            }
        }
        if let Some(judge) = &self.judge
            && (judge.rubric.trim().is_empty() || !(1..=5).contains(&judge.pass_score))
        {
            return bad("A judge needs a rubric and a pass score from 1 to 5.");
        }
        Ok(())
    }
}

/// A case stored for an agent.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct StoredCase {
    /// Its id.
    pub id: EvalCaseId,
    /// The agent it belongs to.
    pub agent: AgentId,
    /// What it is.
    pub case: EvalCase,
    /// When it was last changed.
    pub updated_at: Timestamp,
}

/// What one check found.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct CheckOutcome {
    /// What was checked, in words.
    pub check: String,
    /// It held.
    pub passed: bool,
    /// Why not, or what was found.
    pub detail: String,
}

/// What the judge said.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct JudgeOutcome {
    /// 1 to 5.
    pub score: u8,
    /// Why.
    pub reasons: String,
    /// What the judging call cost, in millionths of a dollar.
    pub cost_micros: u64,
    /// The model that judged.
    pub model: String,
}

/// The result of one case in one eval run.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct EvalResult {
    /// The case's name.
    pub case: String,
    /// The execution that ran it, when one started.
    pub execution: Option<ExecutionId>,
    /// All checks held and the judge, if any, passed.
    pub passed: bool,
    /// What each check found.
    pub checks: Vec<CheckOutcome>,
    /// The judge's verdict, when there was a judge.
    pub judge: Option<JudgeOutcome>,
    /// The state the run ended in.
    pub state: String,
    /// Model steps taken.
    pub steps: u32,
    /// Tool calls made.
    pub tool_calls: u32,
    /// What the run cost, in millionths of a dollar (the judge not included).
    pub cost_micros: u64,
    /// Why no execution ran, or why it failed to start.
    #[serde(default)]
    pub error: Option<String>,
}

/// One go at a suite: one version of an agent with one model.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct EvalRun {
    /// Its id.
    pub id: EvalRunId,
    /// The agent.
    pub agent: AgentId,
    /// The version that was tried.
    pub version: u32,
    /// The model it was tried with.
    pub model: String,
    /// `running`, `finished` or `cancelled`.
    pub status: String,
    /// When it started.
    pub started_at: Timestamp,
    /// When it ended.
    pub finished_at: Option<Timestamp>,
    /// One result per case.
    pub results: Vec<EvalResult>,
}

/// A conversation of an agent, for the list beside the chat.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct AgentConversation {
    /// Its id.
    pub conversation: ConversationId,
    /// The first prompt, as a title.
    pub title: String,
    /// How many runs it has.
    pub runs: u32,
    /// When the newest run started.
    pub last_at: Timestamp,
}

// ---------------------------------------------------------------------------------------------------------------------
// The store

/// Where agents, their versions, skills and evaluations are kept. Every call takes the tenant; nothing crosses tenants.
#[async_trait]
pub trait AgentStore: Send + Sync {
    /// The agents of a tenant, by slug. Archived ones only when asked.
    async fn agents(&self, tenant: &TenantId, include_archived: bool) -> Result<Vec<Agent>>;
    /// One agent.
    async fn agent(&self, tenant: &TenantId, id: AgentId) -> Result<Option<Agent>>;
    /// One agent by slug.
    async fn agent_by_slug(&self, tenant: &TenantId, slug: &AgentSlug) -> Result<Option<Agent>>;
    /// Make an agent with its first version. Fails when the slug is taken or the tenant has [`MAX_AGENTS`].
    async fn create_agent(
        &self,
        tenant: &TenantId,
        spec: AgentSpec,
        note: Option<String>,
        now: Timestamp,
    ) -> Result<(Agent, AgentVersion)>;
    /// Write the next version, unless the spec is the same as the newest (then nothing is written and the second value is
    /// false). The slug of a spec cannot change.
    async fn save_version(
        &self,
        tenant: &TenantId,
        id: AgentId,
        spec: AgentSpec,
        note: Option<String>,
        now: Timestamp,
    ) -> Result<(AgentVersion, bool)>;
    /// The versions of an agent, newest first.
    async fn versions(&self, tenant: &TenantId, id: AgentId) -> Result<Vec<AgentVersion>>;
    /// One version.
    async fn version(&self, tenant: &TenantId, id: AgentId, version: u32) -> Result<Option<AgentVersion>>;
    /// Write a new version that copies version `to`.
    async fn rollback(&self, tenant: &TenantId, id: AgentId, to: u32, now: Timestamp) -> Result<AgentVersion>;
    /// Hide or show an agent.
    async fn set_archived(&self, tenant: &TenantId, id: AgentId, archived: bool) -> Result<()>;
    /// Delete an agent with its versions and cases. Executions that used it keep their binding.
    async fn remove_agent(&self, tenant: &TenantId, id: AgentId) -> Result<bool>;
    /// The conversations of an agent, newest first.
    async fn conversations(&self, tenant: &TenantId, id: AgentId, limit: usize) -> Result<Vec<AgentConversation>>;

    /// The skills of a tenant, by name.
    async fn skills(&self, tenant: &TenantId) -> Result<Vec<SkillRecord>>;
    /// The newest version of a skill.
    async fn skill(&self, tenant: &TenantId, name: &str) -> Result<Option<SkillRecord>>;
    /// A version of a skill by its hash.
    async fn skill_version(&self, tenant: &TenantId, name: &str, hash: &str) -> Result<Option<SkillRecord>>;
    /// Record an imported skill as the newest of its name.
    async fn record_skill(&self, tenant: &TenantId, record: SkillRecord) -> Result<()>;
    /// Forget a skill and its versions. The files are removed by a separate, explicit cleanup.
    async fn remove_skill(&self, tenant: &TenantId, name: &str) -> Result<bool>;
    /// Every skill hash some agent version or current skill still names.
    async fn referenced_skill_hashes(&self, tenant: &TenantId) -> Result<Vec<String>>;

    /// The cases of an agent, by name.
    async fn eval_cases(&self, tenant: &TenantId, agent: AgentId) -> Result<Vec<StoredCase>>;
    /// Add a case, or replace the one with the same name.
    async fn put_eval_case(
        &self,
        tenant: &TenantId,
        agent: AgentId,
        case: EvalCase,
        now: Timestamp,
    ) -> Result<StoredCase>;
    /// Delete a case by name.
    async fn remove_eval_case(&self, tenant: &TenantId, agent: AgentId, name: &str) -> Result<bool>;
    /// Keep an eval run (new or updated, with its results so far).
    async fn save_eval_run(&self, tenant: &TenantId, run: &EvalRun) -> Result<()>;
    /// One eval run.
    async fn eval_run(&self, tenant: &TenantId, id: EvalRunId) -> Result<Option<EvalRun>>;
    /// The eval runs of an agent, newest first.
    async fn eval_runs(&self, tenant: &TenantId, agent: AgentId, limit: usize) -> Result<Vec<EvalRun>>;

    /// The teams of a tenant, by slug.
    async fn teams(&self, tenant: &TenantId) -> Result<Vec<Team>>;
    /// One team.
    async fn team(&self, tenant: &TenantId, slug: &AgentSlug) -> Result<Option<Team>>;
    /// Make a team, or replace the one with the same slug. Fails when the tenant has [`MAX_TEAMS`].
    async fn put_team(&self, tenant: &TenantId, team: Team) -> Result<Team>;
    /// Delete a team. The agents in it are not touched.
    async fn remove_team(&self, tenant: &TenantId, slug: &AgentSlug) -> Result<bool>;
}

#[cfg(test)]
mod tests {
    fn team(lead: &str, members: &[&str]) -> Team {
        Team {
            tenant: TenantId::local(),
            slug: AgentSlug::new("desk").unwrap(),
            name: "Desk".into(),
            description: "A team.".into(),
            avatar: Avatar::default(),
            lead: AgentSlug::new(lead).unwrap(),
            members: members.iter().map(|m| AgentSlug::new(*m).unwrap()).collect(),
            updated_at: Timestamp(1),
        }
    }

    #[test]
    fn a_team_is_a_lead_and_members_and_gives_the_lead_one_entry_per_member() {
        let t = team("lead", &["researcher", "writer"]);
        t.validate().unwrap();
        assert_eq!(t.access(), ["agent:researcher", "agent:writer"]);
        let grants = crate::access::compile_access(&t.access()).unwrap();
        assert!(grants.allows(&crate::permission::Permission::new(crate::permission::Action::Execute, "agent:writer")));
        assert!(!grants.allows(&crate::permission::Permission::new(crate::permission::Action::Execute, "agent:lead")));
        assert!(team("lead", &["lead"]).validate().unwrap_err().to_string().contains("not also a member"));
        assert!(team("lead", &["a", "a"]).validate().unwrap_err().to_string().contains("twice"));
        let many: Vec<String> = (0..13).map(|i| format!("m{i}")).collect();
        let refs: Vec<&str> = many.iter().map(String::as_str).collect();
        assert!(team("lead", &refs).validate().is_err());
        let mut unnamed = team("lead", &[]);
        unnamed.name = " ".into();
        assert!(unnamed.validate().is_err());
    }

    use super::*;

    fn spec() -> AgentSpec {
        AgentSpec::new(AgentSlug::new("researcher").unwrap(), "Researcher", "demo")
    }

    #[test]
    fn slugs_are_short_plain_and_never_start_or_end_with_a_hyphen() {
        for good in ["a", "code-reviewer", "x1", &"a".repeat(48)] {
            assert!(AgentSlug::new(good).is_ok(), "{good}");
        }
        for bad in ["", "-a", "a-", "a--b", "A", "a b", "é", "a_b", &"a".repeat(49)] {
            assert!(AgentSlug::new(bad).is_err(), "{bad}");
        }
        let json = serde_json::to_string(&AgentSlug::new("ok").unwrap()).unwrap();
        assert_eq!(json, "\"ok\"");
        assert!(serde_json::from_str::<AgentSlug>("\"No Good\"").is_err(), "deserialising checks too");
    }

    #[test]
    fn the_hash_follows_the_content_and_nothing_else() {
        let a = spec();
        let mut b = spec();
        assert_eq!(a.hash(), b.hash());
        assert_eq!(a.hash().len(), 64);
        b.instructions.push('x');
        assert_ne!(a.hash(), b.hash());
        let mut c = spec();
        c.metadata.insert("k".into(), "v".into());
        assert_ne!(a.hash(), c.hash());
    }

    #[test]
    fn a_spec_is_checked_before_it_is_kept() {
        assert!(spec().validate().is_ok());
        let mut s = spec();
        s.name = " ".into();
        assert!(s.validate().is_err());
        let mut s = spec();
        s.access = vec!["ftp:host".into()];
        assert!(s.validate().unwrap_err().to_string().contains("ftp"));
        let mut s = spec();
        s.tools = vec!["bad tool".into()];
        assert!(s.validate().is_err());
        let mut s = spec();
        s.models.allowed = vec!["anthropic/*".into(), "openai/gpt-5".into()];
        assert!(s.validate().is_ok());
        s.models.allowed.push("my-alias".into());
        assert!(s.validate().is_err(), "aliases and routes can change, so they cannot be allowed");
        let mut s = spec();
        s.skills = vec![SkillPin { name: "Bad".into(), hash: "0".repeat(64) }];
        assert!(s.validate().is_err());
        s.skills = vec![SkillPin { name: "ok".into(), hash: "zz".into() }];
        assert!(s.validate().is_err());
        let mut s = spec();
        s.avatar.color = "pink".into();
        assert!(s.validate().is_err());
    }

    #[test]
    fn model_patterns_match_a_provider_a_model_or_anything() {
        let p = |items: &[&str]| items.iter().map(|s| s.to_string()).collect::<Vec<_>>();
        assert!(model_allowed(&[], Some("x"), "y"), "an empty list allows any");
        assert!(model_allowed(&p(&["*"]), Some("x"), "y"));
        assert!(model_allowed(&p(&["anthropic/*"]), Some("anthropic"), "claude-opus-5-5"));
        assert!(!model_allowed(&p(&["anthropic/*"]), Some("openai"), "gpt-5"));
        assert!(model_allowed(&p(&["openai/gpt-5"]), Some("openai"), "gpt-5"));
        assert!(!model_allowed(&p(&["openai/gpt-5"]), Some("openai"), "gpt-5-mini"));
        assert!(model_allowed(&p(&["demo"]), None, "demo"));
        assert!(model_allowed(&p(&["demo"]), Some("sim"), "demo"), "the router calls the simulated provider sim");
        assert!(!model_allowed(&p(&["demo"]), Some("openai"), "demo"));
        assert!(!model_allowed(&p(&["anthropic/*"]), None, "demo"));
    }

    #[test]
    fn a_binding_always_names_what_is_allowed() {
        let agent = Agent {
            id: AgentId::new(),
            tenant: TenantId::local(),
            slug: AgentSlug::new("a").unwrap(),
            current_version: 1,
            archived: false,
            created_at: Timestamp(1),
            updated_at: Timestamp(1),
        };
        let version = AgentVersion {
            agent: agent.id,
            version: 1,
            hash: spec().hash(),
            spec: spec(),
            note: None,
            created_at: Timestamp(1),
        };
        let binding = AgentBinding::of(&agent, &version);
        assert_eq!(binding.allowed_models, vec!["*".to_string()]);
        assert!(binding.allows_model(Some("any"), "thing"));
    }

    #[test]
    fn eval_cases_need_something_to_check() {
        let case = EvalCase {
            name: "greets".into(),
            prompt: "Say hello".into(),
            input: None,
            tags: vec![],
            checks: vec![EvalCheck::Contains { text: "hello".into(), ignore_case: true }],
            judge: None,
            approve: vec![],
            allow_writes: false,
        };
        assert!(case.validate().is_ok());
        let mut empty = case.clone();
        empty.checks.clear();
        assert!(empty.validate().is_err());
        let mut judged = empty.clone();
        judged.judge = Some(EvalJudge { rubric: "Polite".into(), model: "demo".into(), pass_score: 4 });
        assert!(judged.validate().is_ok());
        judged.judge.as_mut().unwrap().pass_score = 9;
        assert!(judged.validate().is_err());
        let json = serde_json::to_string(&case).unwrap();
        assert_eq!(serde_json::from_str::<EvalCase>(&json).unwrap(), case);
    }
}
