//! What each command does.

use std::path::{Path, PathBuf};
use std::time::Duration;

use zorix_core::{
    Action, ApprovalDecision, ApprovalStatus, ExecutionDetail, ExecutionEngine, ExecutionId, ExecutionRepository,
    ExecutionState, Grants, Limits, ListFilter, NewExecution, Permission, TaskRequest, TenantId,
};
use zorix_runtime::summarize;

use crate::app::{CliError, builtin_registry, home_dir, open_engine, open_reader};
use crate::cli::{
    CancelArgs, Cli, Command, DecideArgs, EventsArgs, IdArg, ListArgs, ModelArgs, ModelChoice, RecoverArgs, ResumeArgs,
    RunArgs,
};
use crate::follow::{Exit, Printer, decider, default_mode, follow, hint_for_pending};
use crate::render::{event_line, execution_row, shorten};

/// Run the command the person asked for.
pub async fn dispatch(cli: Cli) -> Result<Exit, CliError> {
    let home = home_dir(cli.home.clone());
    let printer = Printer { json: cli.json };
    let tenant = cli.tenant.clone();
    match cli.command {
        Command::Run(args) => run(&home, tenant, printer, args).await,
        Command::List(args) => list(&home, &tenant, printer, args).await,
        Command::Show(arg) => show(&home, &tenant, printer, arg).await,
        Command::Events(args) => events(&home, &tenant, printer, args).await,
        Command::Approve(args) => decide(&home, tenant, printer, args, true).await,
        Command::Deny(args) => decide(&home, tenant, printer, args, false).await,
        Command::Cancel(args) => cancel(&home, tenant, printer, args).await,
        Command::Resume(args) => resume(&home, tenant, printer, args).await,
        Command::Recover(args) => recover(&home, tenant, printer, args).await,
        Command::Tools => tools(&home, printer),
        Command::DataDir => {
            println!("{}", home.display());
            Ok(Exit::Done)
        }
    }
}

fn grants_from(args: &RunArgs) -> Grants {
    let mut grants = Grants::none();
    if args.allow_all_tools {
        grants = grants.with(Permission::use_tool("*"));
    }
    for tool in &args.allow {
        grants = grants.with(Permission::use_tool(tool));
    }
    for pattern in &args.allow_read {
        grants = grants.with(Permission::new(Action::Read, format!("workspace:{pattern}")));
    }
    for pattern in &args.allow_write {
        grants = grants.with(Permission::new(Action::Write, format!("workspace:{pattern}")));
    }
    grants
}

fn workspace_from(path: &Option<PathBuf>) -> Result<Option<PathBuf>, CliError> {
    let Some(path) = path else { return Ok(None) };
    let real = std::fs::canonicalize(path)
        .map_err(|e| CliError(format!("The workspace {} cannot be used: {e}", path.display())))?;
    if !real.is_dir() {
        return Err(CliError(format!("The workspace {} is not a folder.", path.display())));
    }
    Ok(Some(real))
}

async fn last_seq(engine: &zorix_runtime::Engine, tenant: &TenantId, id: ExecutionId) -> Result<u64, CliError> {
    Ok(engine.events(tenant, id, 0).await?.last().map_or(0, |e| e.seq))
}

async fn run(home: &Path, tenant: TenantId, printer: Printer, args: RunArgs) -> Result<Exit, CliError> {
    let workspace = workspace_from(&args.workspace)?;
    let mut limits = Limits::default();
    if let Some(n) = args.max_iterations {
        limits.max_iterations = n;
    }
    if let Some(n) = args.max_tool_calls {
        limits.max_tool_calls = n;
    }
    if let Some(seconds) = args.max_seconds {
        limits.max_active_ms = seconds * 1_000;
    }
    let new = NewExecution {
        tenant: Some(tenant.clone()),
        conversation: args.conversation.clone(),
        parent: None,
        request: TaskRequest {
            instructions: args.instructions.clone(),
            prompt: args.prompt.clone(),
            model: args.model.model.label().to_string(),
            ..TaskRequest::default()
        },
        limits,
        grants: grants_from(&args),
        workspace,
        idempotency_key: args.idempotency_key.clone(),
    };
    let app = open_engine(home, &args.model.model)?;
    let engine = &app.engine;
    let created = engine.create(new).await?;
    // Subscribe before starting, so no event is missed.
    let stream = engine.subscribe(&tenant, created.id, 0).await?;
    if created.state == ExecutionState::Created {
        engine.start(&tenant, created.id).await?;
    }
    let mode = args.approve.unwrap_or_else(default_mode);
    let exit = follow(engine, &tenant, created.id, stream, mode, printer).await;
    engine.shutdown().await;
    exit
}

async fn list(home: &Path, tenant: &TenantId, printer: Printer, args: ListArgs) -> Result<Exit, CliError> {
    let repository = open_reader(home)?;
    let filter =
        ListFilter { states: args.state.into_iter().collect(), limit: args.limit as usize, ..ListFilter::default() };
    let executions = repository.list(tenant, &filter).await?;
    if printer.json {
        println!("{}", serde_json::to_string(&executions)?);
    } else if executions.is_empty() {
        eprintln!("No executions yet.");
    } else {
        for execution in &executions {
            println!("{}", execution_row(execution));
        }
    }
    Ok(Exit::Done)
}

async fn detail_of(
    repository: &impl ExecutionRepository,
    tenant: &TenantId,
    id: ExecutionId,
) -> Result<ExecutionDetail, CliError> {
    let execution = repository.get(tenant, id).await?.ok_or(zorix_core::Error::NotFound(id))?;
    Ok(ExecutionDetail {
        tool_calls: repository.tool_calls(id).await?,
        approvals: repository.approvals(id).await?,
        execution,
    })
}

async fn show(home: &Path, tenant: &TenantId, printer: Printer, arg: IdArg) -> Result<Exit, CliError> {
    let repository = open_reader(home)?;
    let detail = detail_of(&repository, tenant, arg.id).await?;
    let events = repository.events(arg.id, 0, 100_000).await?;
    let summary = summarize(&detail, &events);
    if printer.json {
        println!(
            "{}",
            serde_json::json!({ "summary": summary, "execution": detail.execution, "approvals": detail.approvals })
        );
        return Ok(Exit::Done);
    }
    let e = &detail.execution;
    println!("Execution  {}", e.id);
    println!("State      {}", e.state);
    println!("Task       {}", shorten(&e.request.prompt, 100));
    println!("Model      {}", summary.model);
    if !summary.models.is_empty() {
        println!(
            "Served by  {}{}",
            summary.models.join(", "),
            if summary.fallbacks > 0 { format!(" ({} switch(es))", summary.fallbacks) } else { String::new() }
        );
    }
    println!("Steps      {} model call(s), {} retried", summary.model_calls, summary.model_retries);
    println!("Usage      {} tokens, {} tool call(s)", summary.usage.total_tokens(), summary.usage.tool_calls);
    if let Some(total) = summary.total_ms {
        println!("Took       {} ms ({} ms of work)", total, summary.active_ms);
    }
    for call in &summary.tool_calls {
        let took = call.duration_ms.map(|ms| format!(", {ms} ms")).unwrap_or_default();
        println!("Tool       {} → {:?} ({} attempt(s){took})", call.tool, call.status, call.attempts);
    }
    for approval in detail.approvals.iter().filter(|a| a.status == ApprovalStatus::Pending) {
        println!(
            "Waiting    approval {} for {} {}",
            approval.id,
            approval.tool,
            shorten(&approval.arguments.to_string(), 80)
        );
    }
    if let Some(failure) = &summary.failure {
        println!("Failure    {}: {}", serde_json::to_string(&failure.kind)?.trim_matches('"'), failure.message);
    }
    if let Some(result) = &e.result {
        println!("\n{}", result.text);
    }
    Ok(Exit::Done)
}

async fn events(home: &Path, tenant: &TenantId, printer: Printer, args: EventsArgs) -> Result<Exit, CliError> {
    let repository = open_reader(home)?;
    detail_of(&repository, tenant, args.id).await?;
    let mut after = args.after;
    loop {
        for event in repository.events(args.id, after, 1_000).await? {
            after = event.seq;
            if printer.json {
                println!("{}", serde_json::to_string(&event)?);
            } else {
                println!("{}", event_line(&event));
            }
        }
        if !args.follow {
            return Ok(Exit::Done);
        }
        let state = detail_of(&repository, tenant, args.id).await?.execution.state;
        let more = repository.events(args.id, after, 1).await?;
        if more.is_empty()
            && (state.is_terminal() || matches!(state, ExecutionState::WaitingForApproval | ExecutionState::Paused))
        {
            return Ok(Exit::Done);
        }
        tokio::time::sleep(Duration::from_millis(300)).await;
    }
}

async fn decide(
    home: &Path,
    tenant: TenantId,
    printer: Printer,
    args: DecideArgs,
    approve: bool,
) -> Result<Exit, CliError> {
    let app = open_engine(home, &args.model.model)?;
    let engine = &app.engine;
    let detail = engine.detail(&tenant, args.id).await?;
    if detail.execution.state != ExecutionState::WaitingForApproval {
        engine.shutdown().await;
        return Err(CliError(format!(
            "Execution {} is {}, not waiting for an approval.",
            args.id, detail.execution.state
        )));
    }
    let pending: Vec<_> = detail.approvals.iter().filter(|a| a.status == ApprovalStatus::Pending).collect();
    let chosen = match (args.approval, pending.as_slice()) {
        (Some(id), _) => pending.iter().find(|a| a.id == id).copied(),
        (None, [only]) => Some(*only),
        (None, _) => None,
    };
    let Some(approval) = chosen else {
        engine.shutdown().await;
        let ids: Vec<String> = pending.iter().map(|a| a.id.to_string()).collect();
        return Err(CliError(format!("Say which approval to answer. Pending: {}", ids.join(", "))));
    };
    eprintln!(
        "{} {} with {}",
        if approve { "Approving" } else { "Refusing" },
        approval.tool,
        shorten(&approval.arguments.to_string(), 200)
    );
    let after = last_seq(engine, &tenant, args.id).await?;
    let stream = engine.subscribe(&tenant, args.id, after).await?;
    let decision = ApprovalDecision {
        approve,
        decided_by: decider(),
        note: args.note.clone(),
        call_hash: Some(approval.call_hash.clone()),
    };
    engine.decide(&tenant, args.id, approval.id, decision).await?;
    let exit = follow(engine, &tenant, args.id, stream, args.then, printer).await;
    engine.shutdown().await;
    exit
}

async fn cancel(home: &Path, tenant: TenantId, printer: Printer, args: CancelArgs) -> Result<Exit, CliError> {
    let app = open_engine(home, &ModelChoice::Demo)?;
    let result = app.engine.cancel(&tenant, args.id, args.reason.clone()).await;
    app.engine.shutdown().await;
    let execution = result?;
    if printer.json {
        println!("{}", serde_json::to_string(&execution)?);
    } else {
        eprintln!("Cancelled execution {}.", execution.id);
    }
    Ok(Exit::Cancelled)
}

async fn resume(home: &Path, tenant: TenantId, printer: Printer, args: ResumeArgs) -> Result<Exit, CliError> {
    let app = open_engine(home, &args.model.model)?;
    let engine = &app.engine;
    let after = last_seq(engine, &tenant, args.id).await?;
    let stream = engine.subscribe(&tenant, args.id, after).await?;
    engine.resume(&tenant, args.id).await?;
    let exit = follow(engine, &tenant, args.id, stream, args.then, printer).await;
    engine.shutdown().await;
    exit
}

async fn recover(home: &Path, tenant: TenantId, printer: Printer, args: RecoverArgs) -> Result<Exit, CliError> {
    let ModelArgs { model } = &args.model;
    let app = open_engine(home, model)?;
    let engine = &app.engine;
    let queued =
        engine.list(&tenant, &ListFilter { states: vec![ExecutionState::Queued], ..ListFilter::default() }).await?;
    let report = engine.recover().await?;
    if printer.json {
        println!("{}", serde_json::to_string(&report)?);
    } else if report.requeued.is_empty() && report.needs_decision.is_empty() && report.untouched == 0 {
        eprintln!("Nothing to recover.");
    } else {
        eprintln!(
            "Recovered: {} continued, {} waiting for a decision, {} left as they were.",
            report.requeued.len(),
            report.needs_decision.len(),
            report.untouched
        );
    }
    for id in &report.needs_decision {
        if let Ok(detail) = engine.detail(&tenant, *id).await
            && let Some(approval) = detail.approvals.iter().find(|a| a.status == ApprovalStatus::Pending)
        {
            hint_for_pending(
                *id,
                &serde_json::json!({
                    "approval_id": approval.id, "tool": approval.tool, "arguments": approval.arguments, "reason": approval.reason,
                }),
            );
        }
    }
    let mut worst = Exit::Done;
    if !args.no_wait {
        let ids: Vec<ExecutionId> = report.requeued.iter().copied().chain(queued.iter().map(|e| e.id)).collect();
        for id in ids {
            let stream = engine.subscribe(&tenant, id, 0).await?;
            let exit = follow(engine, &tenant, id, stream, args.then, printer).await?;
            if exit != Exit::Done {
                worst = exit;
            }
        }
    }
    engine.shutdown().await;
    Ok(worst)
}

fn tools(home: &Path, printer: Printer) -> Result<Exit, CliError> {
    use zorix_core::ToolRegistry;
    let registry = builtin_registry(home)?;
    let specs = registry.list();
    if printer.json {
        println!("{}", serde_json::to_string(&specs)?);
        return Ok(Exit::Done);
    }
    for spec in specs {
        let how = if spec.requires_approval {
            "asks first"
        } else if spec.annotations.read_only {
            "read-only"
        } else {
            "changes things"
        };
        println!("{:<16} {:<14} {}", spec.id, how, spec.description);
    }
    Ok(Exit::Done)
}
