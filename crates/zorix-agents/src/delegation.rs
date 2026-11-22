//! The planner behind `agents.delegate` (ADR-0029 to ADR-0031): who may be asked, what the child may do and spend, and what it is
//! told. The runtime only runs what is planned here.

use std::sync::Arc;

use async_trait::async_trait;
use serde_json::json;
use zorix_core::{
    Action, AgentStore, DelegateRequest, DelegateTask, DelegationPlanner, EVAL_LABEL, Execution, Grants, Limits,
    MAX_DEPTH, Permission, PlanContext, PlannedChild, workspace_grant,
};

use crate::runner::{AgentAsk, ModelResolver, RunnerError, prepare};

/// Plans delegation to the agents in a store.
pub struct AgentPlanner {
    store: Arc<dyn AgentStore>,
    resolver: Arc<dyn ModelResolver>,
}

impl AgentPlanner {
    /// A planner that finds agents in `store` and resolves models with `resolver`.
    pub fn new(store: Arc<dyn AgentStore>, resolver: Arc<dyn ModelResolver>) -> Self {
        Self { store, resolver }
    }
}

/// The agents a run may ask, from its grants (`Execute agent:SLUG` and `agent:*`).
fn askable(grants: &Grants) -> Vec<String> {
    let mut names: Vec<String> = grants
        .iter()
        .filter(|p| p.action == Action::Execute)
        .filter_map(|p| p.resource.strip_prefix("agent:").map(str::to_string))
        .collect();
    names.sort();
    names.dedup();
    names
}

/// What is left of a limit once `used` is gone, split between `n` children (at least 1 each). `None` when there is no limit.
fn share(limit: Option<u64>, used: u64, n: u64) -> Result<Option<u64>, String> {
    match limit {
        None => Ok(None),
        Some(limit) => {
            let left = limit.saturating_sub(used);
            if left == 0 {
                Err("The budget for this work is used up, so nothing more can be delegated.".into())
            } else {
                Ok(Some((left / n.max(1)).max(1)))
            }
        }
    }
}

/// The limits a child gets: its share of what the parent has left (ADR-0030). The agent's own limits still apply on top.
pub fn child_limits(parent: &Execution, children: usize) -> Result<Limits, String> {
    let n = children.max(1) as u64;
    let own = &parent.limits;
    let tool_calls_left = u64::from(own.max_tool_calls.saturating_sub(parent.tool_calls_requested));
    if tool_calls_left == 0 {
        return Err("This run has used all its tool calls, so nothing more can be delegated.".into());
    }
    let active_left = own.max_active_ms.saturating_sub(parent.active_ms);
    if active_left == 0 {
        return Err("This run has used all its time, so nothing more can be delegated.".into());
    }
    Ok(Limits {
        max_iterations: own.max_iterations,
        max_tool_calls: ((tool_calls_left / n).max(1)).min(u64::from(u32::MAX)) as u32,
        max_total_tokens: share(own.max_total_tokens, parent.usage.total_tokens(), n)?,
        max_cost_micros: share(own.max_cost_micros, parent.usage.cost_micros, n)?,
        max_active_ms: active_left,
        model_timeout_ms: own.model_timeout_ms,
        max_model_attempts: own.max_model_attempts,
        max_validation_retries: own.max_validation_retries,
    })
}

/// Narrow the workspace reads of `grants` to the patterns the task named: each read stays only as far as a pattern covers it.
pub fn narrow_reads(grants: Grants, files: &[String]) -> Result<Grants, String> {
    if files.is_empty() {
        return Ok(grants);
    }
    let wanted: Vec<Permission> = files.iter().map(|f| workspace_grant(Action::Read, f)).collect::<Result<_, _>>()?;
    let mut kept: Vec<Permission> = Vec::new();
    for permission in grants.iter() {
        if permission.action != Action::Read || !permission.resource.starts_with("workspace:") {
            kept.push(permission.clone());
            continue;
        }
        for narrower in &wanted {
            if narrower.covers(permission) {
                kept.push(permission.clone());
            } else if permission.covers(narrower) {
                kept.push(narrower.clone());
            }
        }
    }
    Ok(Grants::new(kept))
}

#[async_trait]
impl DelegationPlanner for AgentPlanner {
    async fn plan(&self, context: PlanContext<'_>, request: &DelegateRequest) -> Result<Vec<PlannedChild>, String> {
        let parent = context.parent;
        let tenant = &parent.tenant;
        let limits = child_limits(parent, request.tasks.len())?;
        let allowed_to_ask = askable(&parent.grants);
        // The agents already in this chain of work: asking one of them again would loop.
        let mut chain: Vec<String> = parent.agent.iter().map(|a| a.slug.to_string()).collect();
        chain.extend(context.ancestors.iter().filter_map(|e| e.agent.as_ref()).map(|a| a.slug.to_string()));

        let mut planned = Vec::with_capacity(request.tasks.len());
        for task in &request.tasks {
            let slug = task.agent.trim();
            let may = parent.grants.allows(&Permission::new(Action::Execute, format!("agent:{slug}")));
            if !may {
                return Err(if allowed_to_ask.is_empty() {
                    format!("You may not ask {slug}: no agent is open to you in this run.")
                } else {
                    format!("You may not ask {slug}. You may ask: {}.", allowed_to_ask.join(", "))
                });
            }
            if chain.iter().any(|c| c == slug) {
                return Err(format!(
                    "{slug} is already part of this chain of work ({}), so asking it again would loop.",
                    chain.join(" → ")
                ));
            }
            let parsed = zorix_core::AgentSlug::new(slug).map_err(|_| format!("“{slug}” is not an agent's name."))?;
            let agent = self
                .store
                .agent_by_slug(tenant, &parsed)
                .await
                .map_err(|e| e.to_string())?
                .ok_or_else(|| format!("There is no agent called {slug}."))?;
            if agent.archived {
                return Err(format!("{slug} is archived and cannot be asked."));
            }
            let version = self
                .store
                .version(tenant, agent.id, agent.current_version)
                .await
                .map_err(|e| e.to_string())?
                .ok_or_else(|| format!("The newest version of {slug} is missing."))?;

            // The parent's own list of models holds for everything below it, so every model the child might fall back to is checked.
            let model = version.spec.models.default.clone();
            if let Some(binding) = &parent.agent {
                let candidates = self.resolver.candidates(&model).map_err(|e| e.to_string())?;
                if let Some((provider, name)) = candidates.iter().find(|(p, n)| !binding.allows_model(p.as_deref(), n))
                {
                    return Err(format!(
                        "{slug} would use {}{name}, which {} is not allowed to use, so the work cannot be handed over.",
                        provider.as_ref().map(|p| format!("{p}/")).unwrap_or_default(),
                        binding.slug
                    ));
                }
            }

            let mut metadata = std::collections::BTreeMap::new();
            if let Some(by) = &parent.agent {
                metadata.insert("delegated_by".to_string(), by.slug.to_string());
            }
            // An evaluation's label is inherited, so a sub-run is never recorded as past work either.
            if let Some(label) = parent.request.metadata.get(EVAL_LABEL) {
                metadata.insert(EVAL_LABEL.to_string(), label.clone());
            }
            let ask = AgentAsk {
                prompt: task.task.clone(),
                input: handoff(task),
                // The parent's grants, memory and limits only narrow what the child agent has.
                narrow: Some(parent.grants.clone()),
                limits: Some(limits),
                memory: Some(parent.memory),
                workspace: parent.workspace.clone(),
                metadata,
                parent: Some(parent.id),
                root: Some(parent.root.unwrap_or(parent.id)),
                depth: parent.depth + 1,
                // A delegated child runs at the level of the run that asked it, never freer (ADR-0039).
                autonomy: parent.agent.as_ref().and_then(|a| a.autonomy.clone()),
                ..AgentAsk::default()
            };
            let mut new = prepare(&agent, &version, ask, &*self.store, &*self.resolver).await.map_err(|e| match e {
                RunnerError::Refused(why) | RunnerError::Store(why) => why,
            })?;
            new.grants = narrow_reads(new.grants, &task.files)?;
            if task.expected_output.is_some() {
                new.request.expected_output = task.expected_output.clone();
            }
            if new.depth > MAX_DEPTH {
                return Err("Delegation is nested too deep.".into());
            }
            planned.push(PlannedChild { task: task.clone(), new });
        }
        Ok(planned)
    }
}

/// The data that goes with a task: the input the caller sent, and its notes.
fn handoff(task: &DelegateTask) -> Option<serde_json::Value> {
    match (&task.input, task.context.is_empty()) {
        (None, true) => None,
        (input, true) => input.clone(),
        (input, false) => Some(json!({ "input": input, "notes": task.context })),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reads_are_narrowed_to_the_patterns_a_task_names() {
        let wide = Grants::none()
            .with(workspace_grant(Action::Read, "*").unwrap())
            .with(Permission::use_tool("files.read"))
            .with(workspace_grant(Action::Write, "out/*").unwrap());
        let narrowed = narrow_reads(wide.clone(), &["docs/*".into()]).unwrap();
        assert!(narrowed.allows(&workspace_grant(Action::Read, "docs/a.md").unwrap()));
        assert!(!narrowed.allows(&workspace_grant(Action::Read, "src/a.rs").unwrap()), "outside the named pattern");
        assert!(narrowed.allows(&Permission::use_tool("files.read")), "other grants are untouched");
        assert!(narrowed.allows(&workspace_grant(Action::Write, "out/x").unwrap()));
        // No patterns named: nothing changes. A pattern the parent never had adds nothing.
        assert_eq!(narrow_reads(wide.clone(), &[]).unwrap().iter().count(), wide.iter().count());
        let tight = Grants::none().with(workspace_grant(Action::Read, "docs/*").unwrap());
        let still = narrow_reads(tight, &["src/*".into()]).unwrap();
        assert!(!still.allows(&workspace_grant(Action::Read, "src/a.rs").unwrap()));
        assert!(!still.allows(&workspace_grant(Action::Read, "docs/a.md").unwrap()));
    }

    #[test]
    fn a_limit_is_shared_between_the_children_and_the_budget_cannot_be_spent_twice() {
        assert_eq!(share(Some(1_000), 200, 4), Ok(Some(200)));
        assert_eq!(share(Some(3), 0, 8), Ok(Some(1)), "at least 1 each");
        assert_eq!(share(None, 500, 4), Ok(None));
        assert!(share(Some(100), 100, 2).unwrap_err().contains("used up"));
    }

    #[test]
    fn the_handoff_carries_input_and_notes_only_when_there_are_some() {
        let task = |input: Option<serde_json::Value>, notes: &[&str]| DelegateTask {
            agent: "a".into(),
            task: "t".into(),
            input,
            expected_output: None,
            context: notes.iter().map(|n| n.to_string()).collect(),
            files: vec![],
        };
        assert_eq!(handoff(&task(None, &[])), None);
        assert_eq!(handoff(&task(Some(json!({"a": 1})), &[])), Some(json!({"a": 1})));
        assert_eq!(handoff(&task(Some(json!(1)), &["note"])), Some(json!({"input": 1, "notes": ["note"]})));
        assert_eq!(handoff(&task(None, &["only a note"])), Some(json!({"input": null, "notes": ["only a note"]})));
    }
}
