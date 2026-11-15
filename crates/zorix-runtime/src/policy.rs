//! The default policy engine (ADR-0006).
//!
//! Order of decisions for a tool call: forbidden actions, explicit deny rules, environment restrictions, grants, then
//! approval requirements. Anything not allowed explicitly is denied.

use serde::{Deserialize, Serialize};
use zorix_core::{Execution, Failure, LimitKind, Permission, PolicyDecision, PolicyEngine, PolicyRequest};

/// What a rule does to the calls it matches.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Effect {
    /// Never run these calls.
    Deny,
    /// Always ask a person first.
    RequireApproval,
}

/// A rule about tools. `tool` is a tool id, or a prefix ending in `*`.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Rule {
    /// The tool id or pattern.
    pub tool: String,
    /// What to do.
    pub effect: Effect,
    /// Why, shown to the person or the model.
    #[serde(default)]
    pub reason: Option<String>,
}

impl Rule {
    fn matches(&self, tool_id: &str) -> bool {
        match self.tool.strip_suffix('*') {
            Some(prefix) => tool_id.starts_with(prefix),
            None => self.tool == tool_id,
        }
    }
}

/// The settings of the default policy engine.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct Policy {
    /// Rules about specific tools.
    pub rules: Vec<Rule>,
    /// Permissions that are refused whatever the grants say (same pattern syntax as grants).
    pub forbidden: Vec<Permission>,
    /// Ask a person before running a tool that is marked destructive and not read-only.
    pub approve_destructive: bool,
    /// Allow tools that reach outside this computer. Turn off for an offline environment.
    pub allow_open_world: bool,
}

impl Default for Policy {
    fn default() -> Self {
        Self { rules: Vec::new(), forbidden: Vec::new(), approve_destructive: true, allow_open_world: true }
    }
}

/// The default [`PolicyEngine`].
#[derive(Clone, Debug, Default)]
pub struct DefaultPolicy {
    policy: Policy,
}

impl DefaultPolicy {
    /// An engine with these settings.
    pub fn new(policy: Policy) -> Self {
        Self { policy }
    }

    /// The settings.
    pub fn policy(&self) -> &Policy {
        &self.policy
    }
}

fn deny(reason: String) -> PolicyDecision {
    PolicyDecision::Deny { reason }
}

impl PolicyEngine for DefaultPolicy {
    fn evaluate_tool_call(&self, request: &PolicyRequest<'_>) -> PolicyDecision {
        let tool = request.tool;
        let required = tool.all_required_permissions();

        if let Some(hit) = required.iter().find(|p| self.policy.forbidden.iter().any(|f| f.covers(p))) {
            return deny(format!("The action “{hit}” is forbidden by policy."));
        }
        if let Some(rule) = self.policy.rules.iter().find(|r| r.effect == Effect::Deny && r.matches(&tool.id)) {
            return deny(rule.reason.clone().unwrap_or_else(|| format!("The tool {} is blocked by policy.", tool.id)));
        }
        if !self.policy.allow_open_world && tool.annotations.open_world {
            return deny(format!("{} reaches outside this computer, which this environment does not allow.", tool.id));
        }
        let missing = request.execution.grants.missing(&required);
        if !missing.is_empty() {
            let list: Vec<String> = missing.iter().map(|p| p.to_string()).collect();
            return deny(format!("This execution has no permission for: {}.", list.join(", ")));
        }
        if let Some(rule) =
            self.policy.rules.iter().find(|r| r.effect == Effect::RequireApproval && r.matches(&tool.id))
        {
            return PolicyDecision::RequireApproval {
                reason: rule.reason.clone().unwrap_or_else(|| format!("Policy requires approval to use {}.", tool.id)),
            };
        }
        if tool.requires_approval {
            return PolicyDecision::RequireApproval { reason: format!("{} asks for approval on every call.", tool.id) };
        }
        if self.policy.approve_destructive && tool.annotations.destructive && !tool.annotations.read_only {
            return PolicyDecision::RequireApproval { reason: format!("{} can change or remove data.", tool.id) };
        }
        PolicyDecision::Allow
    }

    fn check_limits(&self, execution: &Execution) -> Option<Failure> {
        let limits = &execution.limits;
        if execution.iteration > limits.max_iterations {
            return Some(Failure::limit(
                LimitKind::Iterations,
                format!("Stopped after {} model steps, the limit for this execution.", limits.max_iterations),
            ));
        }
        if execution.tool_calls_requested > limits.max_tool_calls {
            return Some(Failure::limit(
                LimitKind::ToolCalls,
                format!("Stopped: the model asked for more than {} tool calls.", limits.max_tool_calls),
            ));
        }
        if let Some(max) = limits.max_total_tokens
            && execution.usage.total_tokens() > max
        {
            return Some(Failure::limit(LimitKind::Tokens, format!("Stopped: more than {max} tokens were used.")));
        }
        if let Some(max) = limits.max_cost_micros
            && execution.usage.cost_micros > max
        {
            return Some(Failure::limit(LimitKind::Cost, "Stopped: the cost limit was reached."));
        }
        if execution.active_ms > limits.max_active_ms {
            return Some(Failure::limit(
                LimitKind::ActiveTime,
                format!("Stopped: the work took longer than {} ms.", limits.max_active_ms),
            ));
        }
        None
    }
}

#[cfg(test)]
mod tests {
    use serde_json::{Value, json};
    use zorix_core::{
        Action, ExecutionId, FailureKind, Grants, NewExecution, TaskRequest, Timestamp, ToolAnnotations, ToolSpec,
    };

    use super::*;

    fn execution(grants: Grants) -> Execution {
        let new = NewExecution {
            request: TaskRequest { prompt: "p".into(), model: "m".into(), ..TaskRequest::default() },
            grants,
            ..NewExecution::default()
        };
        Execution::new(ExecutionId::new(), new, Timestamp(0))
    }

    fn reader() -> ToolSpec {
        let mut spec = ToolSpec::new("files.read", "Read", "reads", json!({"type": "object"}));
        spec.annotations = ToolAnnotations::safe_read();
        spec.required_permissions = vec![Permission::new(Action::Read, "workspace:docs/a.txt")];
        spec
    }

    fn writer() -> ToolSpec {
        let mut spec = ToolSpec::new("files.write", "Write", "writes", json!({"type": "object"}));
        spec.annotations =
            ToolAnnotations { read_only: false, destructive: true, idempotent: false, open_world: false };
        spec.required_permissions = vec![Permission::new(Action::Write, "workspace:out.txt")];
        spec
    }

    fn decide(policy: &DefaultPolicy, execution: &Execution, tool: &ToolSpec) -> PolicyDecision {
        policy.evaluate_tool_call(&PolicyRequest { execution, tool, arguments: &Value::Null })
    }

    fn full_grants() -> Grants {
        Grants::none()
            .with(Permission::new(Action::UseTool, "files.*"))
            .with(Permission::new(Action::Read, "workspace:*"))
            .with(Permission::new(Action::Write, "workspace:*"))
    }

    #[test]
    fn with_no_grants_everything_is_denied_and_the_reason_names_the_missing_permissions() {
        let decision = decide(&DefaultPolicy::default(), &execution(Grants::none()), &reader());
        let PolicyDecision::Deny { reason } = decision else { panic!("expected a denial") };
        assert!(reason.contains("use_tool files.read"), "{reason}");
        assert!(reason.contains("read workspace:docs/a.txt"), "{reason}");
    }

    #[test]
    fn a_covered_read_only_tool_is_allowed() {
        assert_eq!(decide(&DefaultPolicy::default(), &execution(full_grants()), &reader()), PolicyDecision::Allow);
    }

    #[test]
    fn a_tool_without_the_use_grant_is_denied_even_if_the_resource_is_granted() {
        let grants = Grants::none().with(Permission::new(Action::Read, "workspace:*"));
        assert!(matches!(
            decide(&DefaultPolicy::default(), &execution(grants), &reader()),
            PolicyDecision::Deny { .. }
        ));
    }

    #[test]
    fn destructive_tools_need_approval_by_default_and_not_when_that_is_turned_off() {
        let execution = execution(full_grants());
        assert!(matches!(
            decide(&DefaultPolicy::default(), &execution, &writer()),
            PolicyDecision::RequireApproval { .. }
        ));
        let relaxed = DefaultPolicy::new(Policy { approve_destructive: false, ..Policy::default() });
        assert_eq!(decide(&relaxed, &execution, &writer()), PolicyDecision::Allow);
    }

    #[test]
    fn the_tool_can_ask_for_approval_itself() {
        let mut spec = reader();
        spec.requires_approval = true;
        assert!(matches!(
            decide(&DefaultPolicy::default(), &execution(full_grants()), &spec),
            PolicyDecision::RequireApproval { .. }
        ));
    }

    #[test]
    fn rules_can_deny_or_require_approval_by_pattern() {
        let policy = DefaultPolicy::new(Policy {
            rules: vec![
                Rule { tool: "files.write".into(), effect: Effect::Deny, reason: Some("no writing today".into()) },
                Rule { tool: "files.r*".into(), effect: Effect::RequireApproval, reason: None },
            ],
            ..Policy::default()
        });
        let execution = execution(full_grants());
        assert_eq!(decide(&policy, &execution, &writer()), PolicyDecision::Deny { reason: "no writing today".into() });
        assert!(matches!(decide(&policy, &execution, &reader()), PolicyDecision::RequireApproval { .. }));
    }

    #[test]
    fn forbidden_actions_win_over_grants_and_over_approval() {
        let policy = DefaultPolicy::new(Policy {
            forbidden: vec![Permission::new(Action::Write, "workspace:secrets*")],
            ..Policy::default()
        });
        let mut spec = writer();
        spec.required_permissions = vec![Permission::new(Action::Write, "workspace:secrets/key.txt")];
        let everything =
            Grants::none().with(Permission::new(Action::UseTool, "*")).with(Permission::new(Action::Write, "*"));
        let PolicyDecision::Deny { reason } = decide(&policy, &execution(everything), &spec) else { panic!("denied") };
        assert!(reason.contains("forbidden"), "{reason}");
    }

    #[test]
    fn an_offline_environment_refuses_tools_that_reach_outside() {
        let policy = DefaultPolicy::new(Policy { allow_open_world: false, ..Policy::default() });
        let mut spec = reader();
        spec.annotations.open_world = true;
        let PolicyDecision::Deny { reason } = decide(&policy, &execution(full_grants()), &spec) else {
            panic!("denied")
        };
        assert!(reason.contains("outside this computer"));
    }

    #[test]
    fn limits_trip_one_by_one_and_name_which() {
        let policy = DefaultPolicy::default();
        let mut run = execution(Grants::none());
        assert_eq!(policy.check_limits(&run), None);

        run.iteration = run.limits.max_iterations + 1;
        assert_eq!(policy.check_limits(&run).unwrap().limit, Some(LimitKind::Iterations));
        run.iteration = 1;

        run.tool_calls_requested = run.limits.max_tool_calls + 1;
        assert_eq!(policy.check_limits(&run).unwrap().limit, Some(LimitKind::ToolCalls));
        run.tool_calls_requested = 0;

        run.limits.max_total_tokens = Some(100);
        run.usage.input_tokens = 60;
        run.usage.output_tokens = 41;
        assert_eq!(policy.check_limits(&run).unwrap().limit, Some(LimitKind::Tokens));
        run.usage = Default::default();

        run.limits.max_cost_micros = Some(10);
        run.usage.cost_micros = 11;
        assert_eq!(policy.check_limits(&run).unwrap().limit, Some(LimitKind::Cost));
        run.usage = Default::default();

        run.active_ms = run.limits.max_active_ms + 1;
        let failure = policy.check_limits(&run).unwrap();
        assert_eq!((failure.kind, failure.limit), (FailureKind::LimitExceeded, Some(LimitKind::ActiveTime)));
    }

    #[test]
    fn exactly_at_the_limit_is_still_fine() {
        let mut run = execution(Grants::none());
        run.iteration = run.limits.max_iterations;
        run.tool_calls_requested = run.limits.max_tool_calls;
        assert_eq!(DefaultPolicy::default().check_limits(&run), None);
    }

    #[test]
    fn a_policy_can_be_read_from_partial_json() {
        let policy: Policy = serde_json::from_str(r#"{"allow_open_world": false}"#).unwrap();
        assert!(policy.approve_destructive && !policy.allow_open_world);
    }
}
