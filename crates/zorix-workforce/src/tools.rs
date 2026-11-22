//! The `org.team` tool and the ticker of the workforce (ADR-0038).
//!
//! `org.team` shows a manager who reports to it, what each has open and spent and how free each one is, so it can plan the week. It
//! reads only, and only for the agent that asks: the answer is about the caller's own reports. Tools hold the service weakly, because
//! the service holds the engine that holds them.

use std::sync::{Arc, Weak};

use async_trait::async_trait;
use serde_json::{Value, json};
use zorix_core::{Action, Permission, Tool, ToolAnnotations, ToolContext, ToolError, ToolSource, ToolSpec};
use zorix_workflows::host::Ticker;

use crate::service::WorkforceService;

/// The tools of the workforce.
pub fn org_tools(service: Weak<WorkforceService>) -> Vec<Arc<dyn Tool>> {
    let mut spec = ToolSpec::new(
        "org.team",
        "org team",
        "Who reports to you, with each one's role, level, open tasks, what they spent this week and what is left of their budget. \
         Use it to plan the week or to write the report.",
        json!({"type": "object", "properties": {}, "additionalProperties": false}),
    );
    spec.source = ToolSource::Builtin;
    spec.timeout_ms = 30_000;
    spec.annotations = ToolAnnotations::safe_read();
    vec![Arc::new(TeamTool { service, spec })]
}

struct TeamTool {
    service: Weak<WorkforceService>,
    spec: ToolSpec,
}

#[async_trait]
impl Tool for TeamTool {
    fn spec(&self) -> &ToolSpec {
        &self.spec
    }

    async fn call(&self, ctx: &ToolContext, _args: Value) -> Result<Value, ToolError> {
        let failed = |m: String| ToolError::Failed(m);
        if !ctx.grants.allows(&Permission::new(Action::Read, "org:team")) {
            return Err(failed("This run has no permission to read the team (org:team).".into()));
        }
        let service =
            self.service.upgrade().ok_or_else(|| failed("The workforce is not available right now.".into()))?;
        let agent =
            ctx.agent.as_ref().map(|a| a.slug.clone()).ok_or_else(|| failed("Only an agent has a team.".into()))?;
        service.team_of(&agent).await.map_err(|e| failed(e.to_string()))
    }
}

/// Keeps the workforce moving: at the end of every beat of the workflow host it reads finished plans and reports, runs the schedules
/// that are due and records budget notices.
pub struct WorkforceTicker {
    service: Arc<WorkforceService>,
}

impl WorkforceTicker {
    /// A ticker for a service.
    pub fn new(service: Arc<WorkforceService>) -> Self {
        Self { service }
    }
}

#[async_trait]
impl Ticker for WorkforceTicker {
    async fn tick(&self) {
        if let Err(error) = self.service.reconcile().await {
            tracing::warn!(%error, "the workforce could not be brought up to date");
        }
    }
}
