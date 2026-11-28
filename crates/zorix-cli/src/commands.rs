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
    CancelArgs, Cli, Command, DecideArgs, EventsArgs, IdArg, ListArgs, McpCommand, ModelArgs, RecoverArgs, ResumeArgs,
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
        Command::Run(args) => run(&home, tenant, printer, *args).await,
        Command::List(args) => list(&home, &tenant, printer, args).await,
        Command::Show(arg) => show(&home, &tenant, printer, arg).await,
        Command::Events(args) => events(&home, &tenant, printer, args).await,
        Command::Approve(args) => decide(&home, tenant, printer, args, true).await,
        Command::Deny(args) => decide(&home, tenant, printer, args, false).await,
        Command::Cancel(args) => cancel(&home, tenant, printer, args).await,
        Command::Resume(args) => resume(&home, tenant, printer, args).await,
        Command::Recover(args) => recover(&home, tenant, printer, args).await,
        Command::Agent(command) => crate::agents::agent(&home, &tenant, printer, command).await,
        Command::Team(command) => crate::agents::team(&home, &tenant, printer, command).await,
        Command::Tree(arg) => tree(&home, &tenant, printer, arg).await,
        Command::Workflow(command) => crate::workflows::workflow(&home, tenant, printer, command).await,
        Command::Project(command) => crate::projects::project(&home, tenant, printer, command).await,
        Command::Task(command) => crate::projects::task(&home, tenant, printer, command).await,
        Command::Org(command) => crate::workforce::org(&home, tenant, printer, command).await,
        Command::Worker(command) => crate::workforce::worker(&home, tenant, printer, command).await,
        Command::Skill(command) => crate::agents::skill(&home, &tenant, printer, command).await,
        Command::Eval(command) => crate::agents::eval(&home, &tenant, printer, command).await,
        Command::Tools => tools(&home, printer),
        Command::Models(args) => crate::providers::models(&home, printer, args).await,
        Command::Provider(command) => crate::providers::provider(&home, printer, command).await,
        Command::Route(command) => crate::providers::route(&home, printer, command),
        Command::Memory(command) => crate::memory::memory(&home, &tenant, printer, command).await,
        Command::Knowledge(command) => crate::memory::knowledge(&home, &tenant, printer, command).await,
        Command::Embeddings(command) => crate::memory::embeddings(&home, &tenant, printer, command).await,
        Command::Settings(command) => crate::settings::settings(&home, &tenant, printer, command).await,
        Command::Retention(command) => crate::settings::retention(&home, &tenant, printer, command).await,
        Command::Mcp(McpCommand::Serve(args)) => crate::mcp_serve::serve(&home, tenant, args).await,
        Command::Mcp(command) => crate::integrations::mcp(&home, tenant, printer, command).await,
        Command::Connect(command) => crate::integrations::connect(&home, tenant, printer, command).await,
        Command::Sandbox(command) => crate::integrations::sandbox(&home, tenant, printer, command).await,
        Command::Serve(args) => crate::serve::serve(&home, tenant, printer, args).await,
        Command::Calibration => crate::memory::calibration(&home, printer),
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
    let flags = args
        .allow_http
        .iter()
        .chain(&args.allow_repo)
        .chain(&args.allow_jira)
        .chain(&args.allow_chat)
        .chain(&args.allow_db)
        .chain(&args.allow_code);
    for flag in flags {
        for permission in &flag.0 {
            grants = grants.with(permission.clone());
        }
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
    if args.estimate {
        return crate::estimate::estimate(home, &tenant, printer, &args).await;
    }
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
    if let Some(micros) = args.max_cost {
        limits.max_cost_micros = Some(micros);
    }
    let app = open_engine(home, &tenant, &args.model)?;
    let attachments = attachments_from(home, &app, &args.attach).await?;
    let grants = grants_from(&args);
    app.start_servers(&grants).await;
    let memory = zorix_core::MemoryOptions {
        conversation: !args.no_conversation,
        long_term: !args.no_memory,
        knowledge: if args.no_knowledge { zorix_core::RecallMode::Off } else { zorix_core::RecallMode::Auto },
        operations: if args.no_memory { zorix_core::RecallMode::Off } else { zorix_core::RecallMode::Auto },
    };
    let new = if args.agent.is_some() || args.team.is_some() {
        if args.instructions.is_some() {
            return Err(CliError(
                "An agent has its own instructions; --instructions cannot be used with --agent or --team.".into(),
            ));
        }
        // Flags only narrow: without any --allow… flag the agent keeps everything it has.
        let narrows = args.allow_all_tools
            || !(args.allow.is_empty()
                && args.allow_read.is_empty()
                && args.allow_write.is_empty()
                && args.allow_http.is_empty()
                && args.allow_repo.is_empty()
                && args.allow_jira.is_empty()
                && args.allow_chat.is_empty()
                && args.allow_db.is_empty()
                && args.allow_code.is_empty());
        let mut narrower = Limits::default();
        let mut limited = false;
        for (value, slot) in
            [(args.max_iterations, &mut narrower.max_iterations), (args.max_tool_calls, &mut narrower.max_tool_calls)]
        {
            if let Some(n) = value {
                *slot = n;
                limited = true;
            }
        }
        if let Some(seconds) = args.max_seconds {
            narrower.max_active_ms = seconds * 1_000;
            limited = true;
        }
        if let Some(micros) = args.max_cost {
            narrower.max_cost_micros = Some(micros);
            limited = true;
        }
        let ask = zorix_agents::AgentAsk {
            prompt: args.prompt.clone(),
            input: args.input.clone(),
            attachments,
            model: args.model.model.as_ref().map(|m| m.label()),
            conversation: args.conversation.clone(),
            workspace,
            narrow: narrows.then(|| grants.clone()),
            limits: limited.then_some(narrower),
            memory: Some(memory),
            idempotency_key: args.idempotency_key.clone(),
            ..zorix_agents::AgentAsk::default()
        };
        let mut new = match (&args.agent, &args.team) {
            (Some(slug), _) => app.hub.prepare_agent_run(slug, args.agent_version, ask, &*app.router).await?,
            (None, Some(team)) => app.hub.prepare_team_run(team, ask, &*app.router).await?,
            (None, None) => unreachable!("one of them is set"),
        };
        new.tenant = Some(tenant.clone());
        if let Some(binding) = &new.agent {
            app.start_servers(&new.grants).await;
            match &binding.team {
                Some(team) => eprintln!("zorix: team {team}, led by {} v{}", binding.slug, binding.version),
                None => eprintln!("zorix: {} v{}", binding.slug, binding.version),
            }
        }
        new
    } else {
        NewExecution {
            tenant: Some(tenant.clone()),
            conversation: args.conversation.clone(),
            parent: None,
            root: None,
            depth: 0,
            request: TaskRequest {
                instructions: args.instructions.clone(),
                prompt: args.prompt.clone(),
                model: app.model.clone(),
                attachments,
                ..TaskRequest::default()
            },
            limits,
            grants,
            workspace,
            idempotency_key: args.idempotency_key.clone(),
            memory,
            agent: None,
        }
    };
    let engine = &app.engine;
    let created = engine.create(new).await?;
    // Subscribe before starting, so no event is missed.
    let stream = engine.subscribe(&tenant, created.id, 0).await?;
    if created.state == ExecutionState::Created {
        engine.start(&tenant, created.id).await?;
    }
    let mode = args.approve.unwrap_or_else(default_mode);
    let exit = follow(engine, &tenant, created.id, stream, mode, printer).await;
    app.shutdown().await;
    exit
}

/// Read the files given with `--attach`, off the async threads. Scanned PDFs are read when reading scans is on. The first file that is refused is the
/// answer, named, and nothing starts.
async fn attachments_from(
    home: &Path,
    app: &crate::app::App,
    paths: &[std::path::PathBuf],
) -> Result<Vec<zorix_core::Attachment>, CliError> {
    if paths.is_empty() {
        return Ok(Vec::new());
    }
    zorix_hub::ocr::attach(&app.stack, home, app.settings.get().knowledge.ocr).await?;
    let (stack, paths) = (app.stack.clone(), paths.to_vec());
    let attached = tokio::task::spawn_blocking(move || stack.read_attachments(&paths))
        .await
        .map_err(|e| CliError(e.to_string()))?
        .map_err(|e| CliError(e.to_string()))?;
    for file in &attached {
        eprintln!(
            "zorix: attached {} ({} characters{})",
            file.name,
            file.text.chars().count(),
            if file.truncated { ", cut" } else { "" }
        );
    }
    Ok(attached)
}

async fn list(home: &Path, tenant: &TenantId, printer: Printer, args: ListArgs) -> Result<Exit, CliError> {
    let repository = open_reader(home)?;
    let agent = match &args.agent {
        Some(slug) => {
            let store = zorix_store::SqliteAgentStore::open(&zorix_store::database_path(home))?;
            let slug = zorix_core::AgentSlug::new(slug).map_err(CliError)?;
            use zorix_core::AgentStore as _;
            let found = store
                .agent_by_slug(tenant, &slug)
                .await?
                .ok_or_else(|| CliError(format!("There is no agent called {slug}.")))?;
            Some(found.id)
        }
        None => None,
    };
    let filter = ListFilter {
        states: args.state.into_iter().collect(),
        limit: args.limit as usize,
        agent,
        // The sub-runs that agents started belong to the run that asked for them; `--all` lists them too.
        roots_only: !args.all,
        ..ListFilter::default()
    };
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
    let context = if arg.context { Some(crate::memory::context_of(home, arg.id, &events).await?) } else { None };
    if printer.json {
        println!(
            "{}",
            serde_json::json!({ "summary": summary, "execution": detail.execution, "approvals": detail.approvals, "context": context })
        );
        return Ok(Exit::Done);
    }
    let e = &detail.execution;
    println!("Execution  {}", e.id);
    println!("State      {}", e.state);
    println!("Task       {}", shorten(&e.request.prompt, 100));
    println!("Model      {}", summary.model);
    if let Some(agent) = &e.agent {
        println!(
            "Agent      {} v{}{}",
            agent.slug,
            agent.version,
            agent.team.as_ref().map(|t| format!(" (team {t})")).unwrap_or_default()
        );
    }
    if let Some(parent) = e.parent {
        println!("Part of    {parent} (see `zorix tree {}`)", e.root.unwrap_or(parent));
    } else {
        let kids = repository.children(tenant, e.id).await?;
        if !kids.is_empty() {
            println!("Sub-runs   {} (see `zorix tree {}`)", kids.len(), e.id);
        }
    }
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
    if let Some(context) = &context {
        crate::memory::print_context(context);
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
    let app = open_engine(home, &tenant, &args.model)?;
    let engine = &app.engine;
    let detail = engine.detail(&tenant, args.id).await?;
    app.start_servers(&detail.execution.grants).await;
    // An approval may be asked anywhere in the tree below the execution you name.
    let pending = engine.pending_in_tree(&tenant, args.id).await?;
    if pending.is_empty() {
        app.shutdown().await;
        return Err(CliError(format!(
            "Execution {} is {}, not waiting for an approval (and nothing it started is).",
            args.id, detail.execution.state
        )));
    }
    let chosen = match (args.approval, pending.as_slice()) {
        (Some(id), _) => pending.iter().find(|p| p.approval.id == id),
        (None, [only]) => Some(only),
        (None, _) => None,
    };
    let Some(item) = chosen else {
        app.shutdown().await;
        let ids: Vec<String> = pending
            .iter()
            .map(|p| {
                format!("{} ({} asks for {})", p.approval.id, p.agent.as_deref().unwrap_or("this run"), p.approval.tool)
            })
            .collect();
        return Err(CliError(format!("Say which approval to answer. Pending: {}", ids.join(", "))));
    };
    let (target, approval) = (item.execution, item.approval.clone());
    eprintln!(
        "{} {} with {}{}",
        if approve { "Approving" } else { "Refusing" },
        approval.tool,
        shorten(&approval.arguments.to_string(), 200),
        item.agent.as_ref().map(|a| format!(" (asked by {a})")).unwrap_or_default()
    );
    let after = last_seq(engine, &tenant, args.id).await?;
    let stream = engine.subscribe(&tenant, args.id, after).await?;
    let decision = ApprovalDecision {
        approve,
        decided_by: decider(),
        note: args.note.clone(),
        call_hash: Some(approval.call_hash.clone()),
        automatic: false,
    };
    engine.decide(&tenant, target, approval.id, decision).await?;
    let exit = follow(engine, &tenant, args.id, stream, args.then, printer).await;
    app.shutdown().await;
    exit
}

async fn cancel(home: &Path, tenant: TenantId, printer: Printer, args: CancelArgs) -> Result<Exit, CliError> {
    let app = open_engine(home, &tenant, &ModelArgs::default_demo())?;
    let result = app.engine.cancel(&tenant, args.id, args.reason.clone()).await;
    app.shutdown().await;
    let execution = result?;
    if printer.json {
        println!("{}", serde_json::to_string(&execution)?);
    } else {
        eprintln!("Cancelled execution {}.", execution.id);
    }
    Ok(Exit::Cancelled)
}

async fn resume(home: &Path, tenant: TenantId, printer: Printer, args: ResumeArgs) -> Result<Exit, CliError> {
    let app = open_engine(home, &tenant, &args.model)?;
    let engine = &app.engine;
    if let Ok(execution) = engine.get(&tenant, args.id).await {
        app.start_servers(&execution.grants).await;
    }
    let after = last_seq(engine, &tenant, args.id).await?;
    let stream = engine.subscribe(&tenant, args.id, after).await?;
    engine.resume(&tenant, args.id).await?;
    let exit = follow(engine, &tenant, args.id, stream, args.then, printer).await;
    app.shutdown().await;
    exit
}

async fn recover(home: &Path, tenant: TenantId, printer: Printer, args: RecoverArgs) -> Result<Exit, CliError> {
    let app = open_engine(home, &tenant, &args.model)?;
    let engine = &app.engine;
    let queued =
        engine.list(&tenant, &ListFilter { states: vec![ExecutionState::Queued], ..ListFilter::default() }).await?;
    // Work that was in the middle of a step may need MCP servers; have them ready before it continues.
    let active = engine
        .list(
            &tenant,
            &ListFilter {
                states: vec![
                    ExecutionState::Queued,
                    ExecutionState::Running,
                    ExecutionState::WaitingForModel,
                    ExecutionState::WaitingForTool,
                ],
                ..ListFilter::default()
            },
        )
        .await?;
    for execution in &active {
        app.start_servers(&execution.grants).await;
    }
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
    app.shutdown().await;
    Ok(worst)
}

fn tools(home: &Path, printer: Printer) -> Result<Exit, CliError> {
    use zorix_core::ToolRegistry;
    let registry = builtin_registry(home)?;
    // The native integrations and the sandbox too: they are offered only to runs that are granted them.
    let hub = zorix_hub::Hub::open(
        home,
        TenantId::local(),
        crate::app::secrets(&crate::app::load_config(home)?),
        registry.clone(),
    )?;
    hub.register_native_tools()?;
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
        println!("{:<22} {:<14} {:<12} {}", spec.id, how, spec.source.label(), shorten(&spec.description, 70));
    }
    Ok(Exit::Done)
}

async fn tree(home: &Path, tenant: &TenantId, printer: Printer, arg: IdArg) -> Result<Exit, CliError> {
    let repository = open_reader(home)?;
    let root = repository.get(tenant, arg.id).await?.ok_or(zorix_core::Error::NotFound(arg.id))?;
    // Name the top of the tree even when a sub-run was given.
    let top = match root.root {
        Some(top) => repository.get(tenant, top).await?.unwrap_or(root),
        None => root,
    };
    let mut all = vec![top.clone()];
    all.extend(repository.descendants(tenant, top.id).await?);
    if printer.json {
        println!("{}", serde_json::to_string(&all)?);
    } else {
        println!("{}", crate::render::tree_text(&all));
        let cost: u64 = top.usage.cost_micros;
        eprintln!(
            "\n{} execution(s); the root's usage includes theirs: {} tokens, ${:.4}.",
            all.len(),
            top.usage.total_tokens(),
            cost as f64 / 1_000_000.0
        );
    }
    Ok(Exit::Done)
}
