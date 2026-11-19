//! The `code.*` tools: grants, approval, and what the model gets back (ADR-0022).

use std::sync::Arc;

use serde_json::{Value, json};
use tokio_util::sync::CancellationToken;
use zorix_core::{Action, ExecutionId, Grants, Permission, TenantId, Tool, ToolContext, ToolError, ToolSource};
use zorix_sandbox::{Sandbox, sandbox_tools};

fn tools() -> (tempfile::TempDir, Vec<Arc<dyn Tool>>) {
    let dir = tempfile::tempdir().unwrap();
    let sandbox = Arc::new(Sandbox::new(dir.path()).unwrap());
    (dir, sandbox_tools(sandbox))
}

fn context(grants: Grants, workspace: Option<std::path::PathBuf>) -> ToolContext {
    ToolContext {
        execution_id: ExecutionId::new(),
        tenant: TenantId::local(),
        call_id: zorix_core::ToolCallId("c".into()),
        grants,
        cancel: CancellationToken::new(),
        workspace_root: workspace,
        conversation: None,
    }
}

fn run_grant(language: &str) -> Grants {
    Grants::none().with(Permission::new(Action::Execute, format!("code:{language}")))
}

fn message(error: ToolError) -> String {
    match error {
        ToolError::Failed(m) | ToolError::Retryable(m) => m,
    }
}

fn tool<'a>(tools: &'a [Arc<dyn Tool>], id: &str) -> &'a Arc<dyn Tool> {
    tools.iter().find(|t| t.spec().id == id).unwrap()
}

#[tokio::test]
async fn running_code_needs_a_grant_for_the_language_and_returns_what_it_printed() {
    let (_dir, tools) = tools();
    let run = tool(&tools, "code.run");
    assert_eq!(run.spec().source, ToolSource::Sandbox);
    assert!(!run.spec().requires_approval, "it can touch nothing outside the sandbox");
    let args = json!({"language": "javascript", "code": "console.log(2 ** 10); console.error('warn');"});
    assert!(
        message(run.call(&context(Grants::none(), None), args.clone()).await.unwrap_err())
            .contains("--allow-code javascript")
    );
    assert!(
        run.call(&context(run_grant("python"), None), args.clone()).await.is_err(),
        "another language's grant does not help"
    );
    let got = run.call(&context(run_grant("javascript"), None), args.clone()).await.unwrap();
    assert_eq!(
        (got["stdout"].as_str(), got["stderr"].as_str(), got["exit_code"].as_i64()),
        (Some("1024\n"), Some("warn\n"), Some(0))
    );
    assert!(got.get("stopped").is_none());
    // A pattern grant covers both languages.
    let any = Grants::none().with(Permission::new(Action::Execute, "code:*"));
    assert!(run.call(&context(any, None), args).await.is_ok());
}

#[tokio::test]
async fn a_stopped_program_says_why_in_words_and_bad_arguments_are_refused() {
    let (_dir, tools) = tools();
    let run = tool(&tools, "code.run");
    let ctx = context(run_grant("javascript"), None);
    let output = run
        .call(&ctx, json!({"language": "javascript", "code": "while (true) { console.log('x'.repeat(2000)); }"}))
        .await
        .unwrap();
    assert!(output["stopped"].as_str().unwrap().contains("more than 1024 KB of output"), "{output}");
    assert!(output["stdout"].as_str().unwrap().contains("[cut: "), "a huge output is cut for the model too");
    for bad in [
        json!({"language": "ruby", "code": "x"}),
        json!({"language": "javascript", "code": ""}),
        json!({"language": "javascript", "code": "x".repeat(100_001)}),
        json!({"language": "javascript", "code": "1", "stdin": "x".repeat(100_001)}),
    ] {
        assert!(run.call(&ctx, bad).await.is_err());
    }
}

#[tokio::test]
async fn python_without_its_module_says_how_to_get_it() {
    let (_dir, tools) = tools();
    let error = message(
        tool(&tools, "code.run")
            .call(&context(run_grant("python"), None), json!({"language": "python", "code": "print(1)"}))
            .await
            .unwrap_err(),
    );
    assert!(error.contains("zorix sandbox install python"), "{error}");
}

#[tokio::test]
async fn showing_the_workspace_to_a_program_asks_every_time_and_needs_a_full_read_grant() {
    let (_dir, tools) = tools();
    let with = tool(&tools, "code.run_in_workspace");
    assert!(with.spec().requires_approval);
    let workspace = tempfile::tempdir().unwrap();
    std::fs::write(workspace.path().join("data.txt"), "7 8 9").unwrap();
    let args = json!({"language": "javascript", "code": "console.log(std.loadFile('/workspace/data.txt').split(' ').map(Number).reduce((a, b) => a + b))"});
    let some_paths = run_grant("javascript").with(Permission::new(Action::Read, "workspace:data/*"));
    assert!(
        message(with.call(&context(some_paths, Some(workspace.path().into())), args.clone()).await.unwrap_err())
            .contains("permission to read all of it")
    );
    let full = run_grant("javascript").with(Permission::new(Action::Read, "workspace:*"));
    assert!(
        message(with.call(&context(full.clone(), None), args.clone()).await.unwrap_err())
            .contains("No workspace folder")
    );
    let got = with.call(&context(full, Some(workspace.path().into())), args).await.unwrap();
    assert_eq!(got["stdout"], "24\n");
    let card = with
        .describe_call(&json!({"language": "python", "code": "import os\nprint(os.listdir('/workspace'))"}))
        .unwrap();
    assert_eq!(card, "Run a python program that can read the workspace: import os");
    let _: Value = got;
}
