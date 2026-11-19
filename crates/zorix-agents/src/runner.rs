//! The AgentRunner: turns a version of an agent and a request into an execution (ADR-0024).
//!
//! A run made from an agent can only narrow what the agent may do: its grants, limits, memory and models. Nothing here can
//! widen them, so a client that asks for more gets exactly what the agent had.

use std::collections::BTreeMap;
use std::path::PathBuf;

use serde_json::Value;
use zorix_core::{
    Action, Agent, AgentBinding, AgentStore, AgentVersion, ConversationId, ExecutionId, Grants, Limits, MemoryOptions,
    NewExecution, Permission, RecallMode, StepPhase, TaskRequest, TenantId, model_allowed,
};
use zorix_providers::Router;

/// Which models a model name stands for. The router answers this for real; tests answer it with a list.
pub trait ModelResolver: Send + Sync {
    /// The `(provider, model)` candidates a name resolves to, or why it does not.
    fn candidates(&self, model: &str) -> Result<Vec<(Option<String>, String)>, String>;
}

impl ModelResolver for Router {
    fn candidates(&self, model: &str) -> Result<Vec<(Option<String>, String)>, String> {
        let resolved = self.resolve(model, StepPhase::Start)?;
        Ok(resolved.candidates.iter().map(|c| (Some(c.provider.clone()), c.model.clone())).collect())
    }
}

/// What a person (or a client) asks of an agent.
#[derive(Clone, Debug, Default)]
pub struct AgentAsk {
    /// What to do.
    pub prompt: String,
    /// Data that goes with it.
    pub input: Option<Value>,
    /// A model to use instead of the agent's default. Must be allowed.
    pub model: Option<String>,
    /// The conversation it belongs to.
    pub conversation: Option<ConversationId>,
    /// The folder file tools work in.
    pub workspace: Option<PathBuf>,
    /// Narrower grants than the agent's. Anything outside the agent's grants is dropped.
    pub narrow: Option<Grants>,
    /// Lower limits than the agent's. Each is the smaller of the two.
    pub limits: Option<Limits>,
    /// Less memory than the agent's. Each switch is on only if both are.
    pub memory: Option<MemoryOptions>,
    /// Asking twice with the same key returns the same execution.
    pub idempotency_key: Option<String>,
    /// Labels for the client.
    pub metadata: BTreeMap<String, String>,
    /// The execution that asked, when an agent delegates (Phase 6).
    pub parent: Option<ExecutionId>,
}

/// Why a run could not be made.
#[derive(Debug, thiserror::Error)]
pub enum RunnerError {
    /// The request does not fit the agent.
    #[error("{0}")]
    Refused(String),
    /// The store failed.
    #[error("{0}")]
    Store(String),
}

impl From<zorix_core::Error> for RunnerError {
    fn from(error: zorix_core::Error) -> Self {
        RunnerError::Store(error.to_string())
    }
}

/// The smaller of two optional limits (`None` means no limit).
fn smaller<T: Ord + Copy>(a: Option<T>, b: Option<T>) -> Option<T> {
    match (a, b) {
        (Some(a), Some(b)) => Some(a.min(b)),
        (a, None) => a,
        (None, b) => b,
    }
}

/// The limits of a run: field by field, the smaller of the agent's and the request's.
pub fn narrow_limits(agent: &Limits, ask: &Limits) -> Limits {
    Limits {
        max_iterations: agent.max_iterations.min(ask.max_iterations),
        max_tool_calls: agent.max_tool_calls.min(ask.max_tool_calls),
        max_total_tokens: smaller(agent.max_total_tokens, ask.max_total_tokens),
        max_cost_micros: smaller(agent.max_cost_micros, ask.max_cost_micros),
        max_active_ms: agent.max_active_ms.min(ask.max_active_ms),
        model_timeout_ms: agent.model_timeout_ms.min(ask.model_timeout_ms),
        max_model_attempts: agent.max_model_attempts.min(ask.max_model_attempts),
        max_validation_retries: agent.max_validation_retries.min(ask.max_validation_retries),
    }
}

fn both(a: RecallMode, b: RecallMode) -> RecallMode {
    if a == RecallMode::Auto && b == RecallMode::Auto { RecallMode::Auto } else { RecallMode::Off }
}

/// The memory a run uses: a switch is on only if the agent and the request both have it on.
pub fn narrow_memory(agent: &MemoryOptions, ask: &MemoryOptions) -> MemoryOptions {
    MemoryOptions {
        conversation: agent.conversation && ask.conversation,
        long_term: agent.long_term && ask.long_term,
        knowledge: both(agent.knowledge, ask.knowledge),
        operations: both(agent.operations, ask.operations),
    }
}

/// The instructions of a run: the agent's, then a list of its skills (name and description only), so the model knows what it can
/// load and when.
pub fn instructions_with_skills(instructions: &str, skills: &[(String, String)]) -> String {
    if skills.is_empty() {
        return instructions.to_string();
    }
    let mut text = instructions.trim_end().to_string();
    text.push_str("\n\n<available_skills>\n");
    for (name, description) in skills {
        text.push_str(&format!("- {name}: {}\n", description.replace('\n', " ")));
    }
    text.push_str("</available_skills>\nWhen a skill's description fits the task, load it with the skill.load tool before you start.");
    text
}

/// Make the execution for an agent version.
pub async fn prepare(
    agent: &Agent,
    version: &AgentVersion,
    ask: AgentAsk,
    store: &dyn AgentStore,
    resolver: &dyn ModelResolver,
) -> Result<NewExecution, RunnerError> {
    let spec = &version.spec;
    let tenant: TenantId = agent.tenant.clone();
    let binding = AgentBinding::of(agent, version);

    // 1. The model: asked for, or the default, and at least one candidate must be allowed.
    let model = ask.model.clone().filter(|m| !m.trim().is_empty()).unwrap_or_else(|| spec.models.default.clone());
    let candidates = resolver.candidates(&model).map_err(RunnerError::Refused)?;
    if !candidates.iter().any(|(provider, name)| model_allowed(&binding.allowed_models, provider.as_deref(), name)) {
        return Err(RunnerError::Refused(format!(
            "{} may not use {model}. It allows: {}.",
            agent.slug,
            if spec.models.allowed.is_empty() { "any model".to_string() } else { spec.models.allowed.join(", ") }
        )));
    }

    // 2. The skills the model is told about.
    let mut described = Vec::new();
    for pin in &spec.skills {
        let record = store.skill_version(&tenant, &pin.name, &pin.hash).await?.ok_or_else(|| {
            RunnerError::Refused(format!(
                "The skill {} that {} uses is not in the library (its content {}…). Import it again.",
                pin.name,
                agent.slug,
                &pin.hash[..12]
            ))
        })?;
        described.push((pin.name.clone(), record.description));
    }

    // 3. Grants: the agent's tools and access, plus the skill tools; a request may only narrow them.
    let mut grants = spec.grants()?;
    if !spec.skills.is_empty() {
        for tool in ["skill.load", "skill.read"] {
            grants = grants.with(Permission::use_tool(tool));
        }
        let may_run_code = grants.iter().any(|p| p.action == Action::Execute && p.resource.starts_with("code:"));
        if may_run_code {
            grants = grants.with(Permission::use_tool("skill.run"));
        }
    }
    if let Some(narrow) = &ask.narrow {
        grants = grants.intersect(narrow);
    }

    // 4. Limits and memory: the smaller, the stricter.
    let limits = match &ask.limits {
        Some(wanted) => narrow_limits(&spec.limits, wanted),
        None => spec.limits,
    };
    let memory = match &ask.memory {
        Some(wanted) => narrow_memory(&spec.memory, wanted),
        None => spec.memory,
    };

    // 5. Labels: the agent's, the request's, and which version made the run.
    let mut metadata = spec.metadata.clone();
    metadata.extend(ask.metadata);
    metadata.insert("agent".into(), agent.slug.to_string());
    metadata.insert("agent_version".into(), version.version.to_string());

    Ok(NewExecution {
        tenant: Some(tenant),
        conversation: ask.conversation,
        parent: ask.parent,
        request: TaskRequest {
            instructions: Some(instructions_with_skills(&spec.instructions, &described))
                .filter(|t| !t.trim().is_empty()),
            prompt: ask.prompt,
            input: ask.input,
            model,
            expected_output: spec.expected_output.clone(),
            metadata,
            params: spec.params,
        },
        limits,
        grants,
        workspace: ask.workspace,
        idempotency_key: ask.idempotency_key,
        memory,
        agent: Some(binding),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn limits_take_the_smaller_of_each_and_an_unset_limit_does_not_widen_a_set_one() {
        let agent =
            Limits { max_iterations: 20, max_total_tokens: Some(100_000), max_cost_micros: None, ..Limits::default() };
        let ask = Limits {
            max_iterations: 50,
            max_total_tokens: None,
            max_cost_micros: Some(2_000_000),
            max_tool_calls: 5,
            ..Limits::default()
        };
        let run = narrow_limits(&agent, &ask);
        assert_eq!(run.max_iterations, 20, "the agent's smaller cap stands");
        assert_eq!(run.max_total_tokens, Some(100_000), "asking for no limit does not remove the agent's");
        assert_eq!(run.max_cost_micros, Some(2_000_000), "a limit the agent lacked can be added");
        assert_eq!(run.max_tool_calls, 5);
    }

    #[test]
    fn memory_is_on_only_when_both_say_so() {
        let agent = MemoryOptions {
            conversation: true,
            long_term: false,
            knowledge: RecallMode::Auto,
            operations: RecallMode::Off,
        };
        let all = MemoryOptions::default();
        let run = narrow_memory(&agent, &all);
        assert_eq!(
            run,
            MemoryOptions {
                conversation: true,
                long_term: false,
                knowledge: RecallMode::Auto,
                operations: RecallMode::Off
            }
        );
        assert_eq!(narrow_memory(&all, &MemoryOptions::none()), MemoryOptions::none());
    }

    #[test]
    fn the_skills_are_listed_by_name_and_description_after_the_instructions() {
        assert_eq!(instructions_with_skills("Be brief.", &[]), "Be brief.");
        let text =
            instructions_with_skills("Be brief.\n", &[("pdf-tools".into(), "Reads PDFs.\nUse for PDFs.".into())]);
        assert!(
            text.starts_with(
                "Be brief.\n\n<available_skills>\n- pdf-tools: Reads PDFs. Use for PDFs.\n</available_skills>"
            ),
            "{text}"
        );
        assert!(text.contains("skill.load"));
    }
}
