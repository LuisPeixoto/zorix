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
    Action, AgentSlug, ExecutionEngine, ExecutionId, ExecutionState, Grants, Limits, Permission, PolicyEngine,
    TenantId, Tool, ToolCallId, ToolContext, ToolRegistry, network_grant,
};
use zorix_hub::Hub;
use zorix_runtime::{DefaultPolicy, ToolExecutor};

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
    /// The most active time it may take, in milliseconds (a worker's time for a task).
    pub max_active_ms: Option<u64>,
    /// How much it may do without asking, when the workforce starts it (ADR-0039).
    pub autonomy: Option<zorix_core::Autonomy>,
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
    // The two tools name their arguments differently: Slack's are `channel` and `text`, Discord's `channel_id` and `content`.
    let mut arguments = match service {
        zorix_core::Service::Slack => json!({"channel": channel, "text": text}),
        zorix_core::Service::Discord => json!({"channel_id": channel, "content": text}),
    };
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

/// Whether the part of an authored address that decides where a request goes (the scheme and the host, with a port) is written in
/// the step itself. Text filled in from data may only come after it: a webhook payload that could name the host would send a
/// published, unattended step to any machine, this computer's own services and the local network included.
pub fn authority_is_literal(authored: &str) -> Result<(), String> {
    let authored = authored.trim();
    let Some((scheme, rest)) = authored.split_once("://") else {
        return Err(
            "An HTTP step's address has to start with https:// (or http:// for this computer), written in the step."
                .into(),
        );
    };
    let authority = rest.split(['/', '?', '#']).next().unwrap_or_default();
    if scheme.contains("{{") || authority.contains("{{") || authority.is_empty() {
        return Err(
            "The host of an HTTP step has to be written in the step, not filled in from data: `{{…}}` may only come after the host."
                .into(),
        );
    }
    Ok(())
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
            let channel = call.arguments["channel_id"].as_str().unwrap_or_default();
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
    /// The rules an action is held to, the same ones an execution's tool calls are held to.
    policy: Arc<dyn PolicyEngine>,
}

impl HubLauncher {
    /// A launcher for one tenant.
    pub fn new(
        hub: Arc<Hub>,
        engine: Arc<dyn ExecutionEngine>,
        resolver: Arc<dyn ModelResolver>,
        tenant: TenantId,
    ) -> Self {
        Self { hub, engine, resolver, tenant, policy: Arc::new(DefaultPolicy::default()) }
    }

    /// Hold actions to this policy instead of the default one. A host that runs executions under its own policy gives the same one
    /// here, so a rule that stops a tool in a run stops it in a workflow too.
    pub fn with_policy(mut self, policy: Arc<dyn PolicyEngine>) -> Self {
        self.policy = policy;
        self
    }
}

#[async_trait]
impl StepLauncher for HubLauncher {
    async fn start(&self, start: RunStart) -> Result<ExecutionId, String> {
        let ask = AgentAsk {
            prompt: start.prompt,
            input: start.input,
            model: start.model,
            limits: Some(Limits {
                max_active_ms: start.max_active_ms.unwrap_or(u64::MAX),
                ..cost_only(start.max_cost_micros)
            }),
            idempotency_key: Some(start.key),
            metadata: start.metadata,
            extra_access: start.access,
            autonomy: start.autonomy,
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
        call_registered_tool(&*self.hub.registry, &*self.policy, &self.tenant, call).await
    }
}

/// Run the tool an action step names, held to what a model's call is held to: the arguments are checked against the tool's schema
/// (a value filled in from a payload is as untrusted as one a model wrote), and the policy's standing refusals apply. Asking a
/// person is the step's own `ask`, decided before this is called.
pub async fn call_registered_tool(
    registry: &dyn ToolRegistry,
    policy: &dyn PolicyEngine,
    tenant: &TenantId,
    call: &ToolCall,
) -> Result<Value, String> {
    let tool: Arc<dyn Tool> =
        registry.get(&call.tool).ok_or_else(|| format!("The tool {} is not available.", call.tool))?;
    let spec = tool.spec();
    ToolExecutor::validate_arguments(spec, &call.arguments).map_err(|e| e.to_string())?;
    let grants = grants_for(call)?;
    if let Some(reason) = policy.standing_denial(spec, &grants) {
        return Err(reason);
    }
    let ctx = ToolContext {
        execution_id: ExecutionId::new(),
        tenant: tenant.clone(),
        call_id: ToolCallId("workflow-action".into()),
        grants,
        cancel: CancellationToken::new(),
        workspace_root: None,
        conversation: None,
        agent: None,
    };
    ToolExecutor::run(&*tool, &ctx, call.arguments.clone()).await.map_err(|e| e.to_string())
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
        let discord = send_call(Service::Discord, Some("guild"), "123456789", "hi");
        assert_eq!(discord.tool, "discord.send_message");
        // The Discord tool reads `channel_id` and `content`; arguments it does not know would fail the schema every time.
        assert_eq!(discord.arguments, json!({"channel_id": "123456789", "content": "hi", "connection": "guild"}));
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
        let discord = grants_for(&send_call(Service::Discord, None, "123456789", "x")).unwrap();
        assert!(discord.allows(&Permission::new(Action::Write, "discord:123456789")));
        assert!(!discord.allows(&Permission::new(Action::Write, "discord:987654321")));
        // An explicit port is part of what the step names.
        let ported = grants_for(&http_call("GET", "http://127.0.0.1:8080/x", None)).unwrap();
        assert!(ported.allows(&Permission::new(Action::Network, "http:127.0.0.1:8080")));
        assert!(!ported.allows(&Permission::new(Action::Network, "http:127.0.0.1:9090")));
        assert!(grants_for(&http_call("GET", "not a url", None)).is_err());
        assert!(grants_for(&ToolCall { tool: "db.execute".into(), arguments: json!({}) }).is_err());
    }

    #[test]
    fn the_host_of_an_http_step_is_written_in_the_step() {
        for good in [
            "https://hooks.example.com/x",
            "https://hooks.example.com/{{trigger.id}}",
            "https://hooks.example.com:8443/x?y={{trigger.y}}",
            "http://localhost:8080/a",
            "https://hooks.example.com?q={{x}}",
            " https://hooks.example.com/x ",
        ] {
            assert!(authority_is_literal(good).is_ok(), "{good}");
        }
        for bad in [
            "{{trigger.url}}",
            "https://{{trigger.host}}/x",
            "https://hooks.example.com{{trigger.path}}",
            "https://hooks.example.com:{{trigger.port}}/x",
            "{{trigger.scheme}}://hooks.example.com/x",
            "https://{{a}}.example.com/x",
            "https:///x",
            "hooks.example.com/x",
            "",
        ] {
            assert!(authority_is_literal(bad).is_err(), "{bad}");
        }
    }

    mod actions {
        use async_trait::async_trait;
        use zorix_core::{ToolAnnotations, ToolError, ToolSpec};
        use zorix_runtime::{DefaultPolicy, InMemoryToolRegistry, Policy, Rule};

        use super::*;

        /// A tool that echoes what it was given, with the shape of the Discord one.
        struct Echo(ToolSpec);

        #[async_trait]
        impl Tool for Echo {
            fn spec(&self) -> &ToolSpec {
                &self.0
            }
            async fn call(&self, _ctx: &ToolContext, args: Value) -> Result<Value, ToolError> {
                Ok(json!({"saw": args}))
            }
        }

        fn registry() -> InMemoryToolRegistry {
            let registry = InMemoryToolRegistry::new();
            let mut discord = ToolSpec::new(
                "discord.send_message",
                "Send",
                "Send a message",
                json!({"type": "object", "required": ["channel", "text"], "additionalProperties": false, "properties": {
                    "channel": {"type": "string", "pattern": "^[0-9]{5,25}$"}, "text": {"type": "string"}}}),
            );
            discord.annotations = ToolAnnotations { open_world: true, ..ToolAnnotations::default() };
            registry.register(Arc::new(Echo(discord))).unwrap();
            registry
        }

        fn send(channel: &str) -> ToolCall {
            ToolCall { tool: "discord.send_message".into(), arguments: json!({"channel": channel, "text": "hi"}) }
        }

        #[tokio::test]
        async fn an_action_is_checked_against_the_schema_of_its_tool() {
            let (registry, tenant) = (registry(), TenantId::local());
            let policy = DefaultPolicy::default();
            let ok = call_registered_tool(&registry, &policy, &tenant, &send("123456789")).await.unwrap();
            assert_eq!(ok["saw"]["channel"], "123456789");
            // A channel filled in from a payload that is not an id never reaches the tool (it would go into a request path).
            for hostile in ["12/../../x", "abc", "1 2345", "", "123456789/../../guilds"] {
                let error = call_registered_tool(&registry, &policy, &tenant, &send(hostile)).await.unwrap_err();
                assert!(error.contains("discord.send_message"), "{hostile}: {error}");
            }
            let extra = ToolCall {
                tool: "discord.send_message".into(),
                arguments: json!({"channel": "123456789", "text": "x", "admin": true}),
            };
            assert!(call_registered_tool(&registry, &policy, &tenant, &extra).await.is_err());
        }

        #[tokio::test]
        async fn an_action_is_held_to_the_policy_of_the_host() {
            let (registry, tenant) = (registry(), TenantId::local());
            let blocked = DefaultPolicy::new(Policy {
                rules: vec![Rule {
                    tool: "discord.*".into(),
                    effect: zorix_runtime::Effect::Deny,
                    reason: Some("No chat from here.".into()),
                }],
                ..Policy::default()
            });
            let error = call_registered_tool(&registry, &blocked, &tenant, &send("123456789")).await.unwrap_err();
            assert_eq!(error, "No chat from here.");
            let offline = DefaultPolicy::new(Policy { allow_open_world: false, ..Policy::default() });
            let error = call_registered_tool(&registry, &offline, &tenant, &send("123456789")).await.unwrap_err();
            assert!(error.contains("reaches outside this computer"), "{error}");
            let forbidden = DefaultPolicy::new(Policy {
                forbidden: vec![Permission::new(Action::Write, "discord:123456789")],
                ..Policy::default()
            });
            // The permission the action needs is forbidden whatever the step says.
            let spec_with_permission = {
                let registry = InMemoryToolRegistry::new();
                let mut spec = ToolSpec::new("discord.send_message", "Send", "x", json!({"type": "object"}));
                spec.required_permissions = vec![Permission::new(Action::Write, "discord:123456789")];
                registry.register(Arc::new(Echo(spec))).unwrap();
                registry
            };
            let error =
                call_registered_tool(&spec_with_permission, &forbidden, &tenant, &send("123456789")).await.unwrap_err();
            assert!(error.contains("forbidden by policy"), "{error}");
            assert!(
                call_registered_tool(&registry, &DefaultPolicy::default(), &tenant, &send("123456789")).await.is_ok()
            );
        }

        #[tokio::test]
        async fn a_tool_that_is_not_registered_is_not_called() {
            let error = call_registered_tool(
                &InMemoryToolRegistry::new(),
                &DefaultPolicy::default(),
                &TenantId::local(),
                &send("123456789"),
            )
            .await
            .unwrap_err();
            assert!(error.contains("not available"), "{error}");
        }
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
