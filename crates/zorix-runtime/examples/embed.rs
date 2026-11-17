//! Use the runtime from your own program: a tool, a model, a permission and a human approval, all in memory.
//!
//! Run it with `cargo run -p zorix-runtime --example embed`.

use std::sync::Arc;

use async_trait::async_trait;
use serde_json::{Value, json};
use zorix_core::{
    ApprovalDecision, ExecutionEngine, ExecutionState, Grants, NewExecution, Permission, TaskRequest, TenantId, Tool,
    ToolAnnotations, ToolContext, ToolError, ToolRegistry, ToolSpec,
};
use zorix_runtime::{Engine, InMemoryToolRegistry, MemoryRepository};
use zorix_sim::{ScriptedModel, Turn};

/// A tool is a spec (what the model sees, and the rules around it) and a function.
struct SendEmail {
    spec: ToolSpec,
}

impl SendEmail {
    fn new() -> Self {
        let schema = json!({
            "type": "object",
            "properties": {"to": {"type": "string"}, "body": {"type": "string"}},
            "required": ["to", "body"],
            "additionalProperties": false
        });
        let mut spec = ToolSpec::new("email.send", "Send email", "Sends an email.", schema);
        // It reaches outside this computer and cannot be undone, so the default policy asks a person first.
        spec.annotations = ToolAnnotations { read_only: false, destructive: true, idempotent: false, open_world: true };
        Self { spec }
    }
}

#[async_trait]
impl Tool for SendEmail {
    fn spec(&self) -> &ToolSpec {
        &self.spec
    }

    async fn call(&self, _ctx: &ToolContext, args: Value) -> Result<Value, ToolError> {
        println!("   (the tool really runs now: email to {})", args["to"]);
        Ok(json!({ "sent": true }))
    }
}

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    // Any `ModelGateway` works here. This one plays a script: ask for the tool, then answer.
    let model = ScriptedModel::new([
        Turn::call("email.send", json!({"to": "team@example.com", "body": "The report is ready."})),
        Turn::Say("I sent the email.".into()),
    ]);
    let tools = Arc::new(InMemoryToolRegistry::new());
    tools.register(Arc::new(SendEmail::new()))?;
    let engine = Engine::builder(Arc::new(MemoryRepository::new()), Arc::new(model)).tools(tools).build();

    // Nothing is allowed unless it is granted.
    let tenant = TenantId::local();
    let started = engine
        .submit(NewExecution {
            request: TaskRequest {
                prompt: "Tell the team the report is ready".into(),
                model: "sim".into(),
                ..TaskRequest::default()
            },
            grants: Grants::none().with(Permission::use_tool("email.send")),
            ..NewExecution::default()
        })
        .await?;

    let waiting = engine.settle(&tenant, started.id).await?;
    assert_eq!(waiting.state, ExecutionState::WaitingForApproval);
    let approval = engine.detail(&tenant, started.id).await?.approvals.remove(0);
    println!("Waiting for a person to decide on {} with {}", approval.tool, approval.arguments);

    // The answer is bound to this exact call through its hash.
    let decision = ApprovalDecision {
        approve: true,
        decided_by: "you".into(),
        note: None,
        call_hash: Some(approval.call_hash.clone()),
        automatic: false,
    };
    engine.decide(&tenant, started.id, approval.id, decision).await?;

    let done = engine.settle(&tenant, started.id).await?;
    println!("Final answer: {}", done.result.expect("a result").text);
    for event in engine.events(&tenant, started.id, 0).await? {
        println!("  #{:<2} {}", event.seq, event.kind);
    }
    engine.shutdown().await;
    Ok(())
}
