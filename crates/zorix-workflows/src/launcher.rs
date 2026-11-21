//! What a workflow asks of the rest of ZORIX: start an agent or a team, watch an execution, call a tool.
//!
//! The engine only knows this trait, so it is tested without a model or a network. [`HubLauncher`] is the real one: it makes
//! ordinary executions through the hub and calls the native tools, with grants taken from the step itself.

use std::collections::BTreeMap;
use std::sync::Arc;

use async_trait::async_trait;
use serde_json::{Value, json};
use tokio_util::sync::CancellationToken;
use zorix_agents::{AgentAsk, ModelResolver};
use zorix_core::{
    Action, AgentSlug, ExecutionEngine, ExecutionId, ExecutionState, Grants, Limits, Permission, TenantId, Tool,
    ToolCallId, ToolContext, ToolRegistry, network_grant,
};
use zorix_hub::Hub;

/// What to start.
#[derive(Clone, Debug, PartialEq)]
pub enum Target {
    /// An agent, at a version (the newest when absent).
    Agent {
        /// The agent.
        agent: AgentSlug,
        /// The version.
        version: Option<u32>,
    },
    /// A team, through its lead.
    Team {
        /// The team.
        team: AgentSlug,
    },
}

/// A request to start an execution for a step.
#[derive(Clone, Debug, PartialEq)]
pub struct RunStart {
    /// Who runs.
    pub target: Target,
    /// What to ask.
    pub prompt: String,
    /// Data that goes with it.
    pub input: Option<Value>,
    /// A model for this run.
    pub model: Option<String>,
    /// The most it may cost, in millionths of a dollar.
    pub max_cost_micros: Option<u64>,
    /// Asking twice with the same key gives the same execution, so a retry never doubles the work.
    pub key: String,
    /// Labels that link the execution back to the workflow.
    pub metadata: BTreeMap<String, String>,
    /// Access the run gets besides the agent's own, in the syntax of an agent's access list (`task:OPS-38`).
    pub access: Vec<String>,
}

/// How an execution a step started is going.
#[derive(Clone, Debug, PartialEq)]
pub enum Outcome {
    /// Working.
    Running {
        /// What it has cost so far.
        cost_micros: u64,
    },
    /// Stopped for a person, in its own tree.
    WaitingForPerson {
        /// What it has cost so far.
        cost_micros: u64,
    },
    /// Done.
    Completed {
        /// The answer.
        text: String,
        /// The answer as data, when it was asked for.
        structured: Option<Value>,
        /// What it cost.
        cost_micros: u64,
        /// Model and tool steps it took.
        steps: u32,
    },
    /// It could not finish.
    Failed {
        /// Why, for a person.
        reason: String,
        /// What it cost.
        cost_micros: u64,
    },
    /// Somebody cancelled it.
    Cancelled,
}

/// A tool call an action step makes.
#[derive(Clone, Debug, PartialEq)]
pub struct ToolCall {
    /// The tool, such as `slack.send_message`.
    pub tool: String,
    /// Its arguments.
    pub arguments: Value,
}

/// The rest of ZORIX, as a workflow sees it.
#[async_trait]
pub trait StepLauncher: Send + Sync {
    /// Start an execution. Errors are sentences for a person.
    async fn start(&self, start: RunStart) -> Result<ExecutionId, String>;
    /// How an execution is going.
    async fn outcome(&self, id: ExecutionId) -> Result<Outcome, String>;
    /// Stop an execution (and what it started).
    async fn cancel(&self, id: ExecutionId, reason: &str) -> Result<(), String>;
    /// Call a tool for an action step. The step's own settings are the grant: a person wrote and published them.
    async fn call_tool(&self, call: &ToolCall) -> Result<Value, String>;
}

/// The arguments of the tool a Send step calls.
pub fn send_call(service: zorix_core::Service, connection: Option<&str>, channel: &str, text: &str) -> ToolCall {
    let tool = match service {
        zorix_core::Service::Slack => "slack.send_message",
        zorix_core::Service::Discord => "discord.send_message",
    };
    let mut arguments = json!({"channel": channel, "text": text});
    if let Some(connection) = connection {
        arguments["connection"] = json!(connection);
    }
    ToolCall { tool: tool.into(), arguments }
}

/// The arguments of the tool an Http step calls.
pub fn http_call(method: &str, url: &str, body: Option<&str>) -> ToolCall {
    let mut arguments = json!({"method": method, "url": url});
    if let Some(body) = body {
        arguments["body"] = json!(body);
    }
    ToolCall { tool: "http.request".into(), arguments }
}

/// The grants a single call needs, taken from the call itself.
pub fn grants_for(call: &ToolCall) -> Result<Grants, String> {
    let mut permissions = vec![Permission::use_tool(&call.tool)];
    match call.tool.as_str() {
        "http.request" => {
            let url = call.arguments["url"].as_str().unwrap_or_default();
            let parsed = url::Url::parse(url).map_err(|_| format!("“{url}” is not a web address."))?;
            let host = parsed.host_str().ok_or_else(|| format!("“{url}” has no host."))?;
            let host = match parsed.port() {
                Some(port) => format!("{host}:{port}"),
                None => host.to_string(),
            };
            permissions.push(network_grant(&host)?);
        }
        "slack.send_message" => {
            let channel = call.arguments["channel"].as_str().unwrap_or_default();
            permissions.push(Permission::new(Action::Write, format!("slack:{}", channel.trim_start_matches('#'))));
            permissions.push(Permission::new(Action::Write, format!("slack:#{}", channel.trim_start_matches('#'))));
            permissions.push(Permission::new(Action::Read, format!("slack:{}", channel.trim_start_matches('#'))));
            permissions.push(Permission::new(Action::Read, format!("slack:#{}", channel.trim_start_matches('#'))));
        }
        "discord.send_message" => {
            let channel = call.arguments["channel"].as_str().unwrap_or_default();
            permissions.push(Permission::new(Action::Write, format!("discord:{channel}")));
        }
        other => return Err(format!("A workflow does not call {other}.")),
    }
    Ok(Grants::new(permissions))
}

/// Limits that change nothing except the cost: every other field is as high as it may be, so narrowing against the agent's
/// own limits leaves the agent's.
pub fn cost_only(max_cost_micros: Option<u64>) -> Limits {
    Limits {
        max_iterations: 1_000,
        max_tool_calls: 10_000,
        max_total_tokens: None,
        max_cost_micros,
        max_active_ms: u64::MAX,
        model_timeout_ms: u64::MAX,
        max_model_attempts: 10,
        max_validation_retries: 10,
    }
}

/// The real launcher: executions through the hub and the engine, tools through the registry.
pub struct HubLauncher {
    hub: Arc<Hub>,
    engine: Arc<dyn ExecutionEngine>,
    resolver: Arc<dyn ModelResolver>,
    tenant: TenantId,
}

impl HubLauncher {
    /// A launcher for one tenant.
    pub fn new(
        hub: Arc<Hub>,
        engine: Arc<dyn ExecutionEngine>,
        resolver: Arc<dyn ModelResolver>,
        tenant: TenantId,
    ) -> Self {
        Self { hub, engine, resolver, tenant }
    }
}

#[async_trait]
impl StepLauncher for HubLauncher {
    async fn start(&self, start: RunStart) -> Result<ExecutionId, String> {
        let ask = AgentAsk {
            prompt: start.prompt,
            input: start.input,
            model: start.model,
            limits: Some(cost_only(start.max_cost_micros)),
            idempotency_key: Some(start.key),
            metadata: start.metadata,
            extra_access: start.access,
            ..AgentAsk::default()
        };
        let new = match &start.target {
            Target::Agent { agent, version } => {
                self.hub.prepare_agent_run(agent.as_str(), *version, ask, &*self.resolver).await
            }
            Target::Team { team } => self.hub.prepare_team_run(team.as_str(), ask, &*self.resolver).await,
        }
        .map_err(|e| e.to_string())?;
        let created = self.engine.create(new).await.map_err(|e| e.to_string())?;
        let started = if created.state == ExecutionState::Created {
            self.engine.start(&self.tenant, created.id).await.map_err(|e| e.to_string())?
        } else {
            created
        };
        Ok(started.id)
    }

    async fn outcome(&self, id: ExecutionId) -> Result<Outcome, String> {
        let execution = self.engine.get(&self.tenant, id).await.map_err(|e| e.to_string())?;
        let cost = execution.usage.cost_micros;
        Ok(match execution.state {
            ExecutionState::Completed => {
                let result = execution
                    .result
                    .clone()
                    .unwrap_or(zorix_core::ExecutionResult { text: String::new(), structured: None });
                Outcome::Completed {
                    text: result.text,
                    structured: result.structured,
                    cost_micros: cost,
                    steps: execution.usage.model_calls + execution.usage.tool_calls,
                }
            }
            ExecutionState::Failed => Outcome::Failed {
                reason: execution.failure.map(|f| f.message).unwrap_or_else(|| "The run failed.".into()),
                cost_micros: cost,
            },
            ExecutionState::Cancelled => Outcome::Cancelled,
            ExecutionState::WaitingForApproval => Outcome::WaitingForPerson { cost_micros: cost },
            ExecutionState::WaitingForChildren => {
                // A parent waits for its children; it is waiting for a person only if something below it is.
                let waiting =
                    self.engine.pending_in_tree(&self.tenant, id).await.map(|found| !found.is_empty()).unwrap_or(false);
                if waiting {
                    Outcome::WaitingForPerson { cost_micros: cost }
                } else {
                    Outcome::Running { cost_micros: cost }
                }
            }
            _ => Outcome::Running { cost_micros: cost },
        })
    }

    async fn cancel(&self, id: ExecutionId, reason: &str) -> Result<(), String> {
        self.engine.cancel(&self.tenant, id, Some(reason.to_string())).await.map(|_| ()).map_err(|e| e.to_string())
    }

    async fn call_tool(&self, call: &ToolCall) -> Result<Value, String> {
        let tool: Arc<dyn Tool> =
            self.hub.registry.get(&call.tool).ok_or_else(|| format!("The tool {} is not available.", call.tool))?;
        let ctx = ToolContext {
            execution_id: ExecutionId::new(),
            tenant: self.tenant.clone(),
            call_id: ToolCallId("workflow-action".into()),
            grants: grants_for(call)?,
            cancel: CancellationToken::new(),
            workspace_root: None,
            conversation: None,
            agent: None,
        };
        zorix_runtime::ToolExecutor::run(&*tool, &ctx, call.arguments.clone()).await.map_err(|e| e.to_string())
    }
}

#[cfg(test)]
mod tests {
    use zorix_core::Service;

    use super::*;

    #[test]
    fn a_message_is_a_call_to_the_chat_tool() {
        let call = send_call(Service::Slack, Some("acme"), "#incidents", "hello");
        assert_eq!(call.tool, "slack.send_message");
        assert_eq!(call.arguments, json!({"channel": "#incidents", "text": "hello", "connection": "acme"}));
        assert_eq!(send_call(Service::Discord, None, "123", "hi").tool, "discord.send_message");
    }

    #[test]
    fn the_grants_are_exactly_what_the_call_needs() {
        let call = http_call("POST", "https://hooks.example.com/x?y=1", Some("{}"));
        let grants = grants_for(&call).unwrap();
        assert!(grants.allows(&Permission::new(Action::Network, "http:hooks.example.com")));
        assert!(!grants.allows(&Permission::new(Action::Network, "http:evil.example.com")));
        let slack = grants_for(&send_call(Service::Slack, None, "#ops", "x")).unwrap();
        assert!(slack.allows(&Permission::new(Action::Write, "slack:ops")));
        assert!(!slack.allows(&Permission::new(Action::Write, "slack:other")));
        assert!(grants_for(&http_call("GET", "not a url", None)).is_err());
        assert!(grants_for(&ToolCall { tool: "db.execute".into(), arguments: json!({}) }).is_err());
    }

    #[test]
    fn cost_only_limits_leave_everything_else_to_the_agent() {
        let agent = Limits { max_iterations: 40, max_tool_calls: 100, ..Limits::default() };
        let narrowed = zorix_agents::runner::narrow_limits(&agent, &cost_only(Some(500_000)));
        assert_eq!((narrowed.max_iterations, narrowed.max_tool_calls), (40, 100));
        assert_eq!(narrowed.max_cost_micros, Some(500_000));
        assert_eq!(narrowed.max_active_ms, agent.max_active_ms);
    }
}
