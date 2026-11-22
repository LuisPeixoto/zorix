//! `zorix project` and `zorix task`: what a person does with projects from the terminal (ADR-0036, ADR-0037).
//!
//! Managing needs no engine, so it works while the desktop app is open: the app (or `zorix serve`) keeps tasks moving. Starting a
//! run (a task moving to In progress, `task restart`, `project ask`) needs the engine, and with it the data folder.

use std::path::Path;
use std::sync::Arc;
use std::time::Duration;

use async_trait::async_trait;
use serde_json::Value;
use zorix_core::{
    AgentSlug, Date, DeliverableKind, ExecutionId, NeedsYou, ProjectKey, SystemClock, TaskKey, TaskPriority, TaskState,
    TenantId, Timestamp, Who,
};
use zorix_projects::{Move, NewProject, NewTask, ProjectFields, ProjectService, TaskFields, TaskView, sentence};
use zorix_store::{SqliteProjectStore, database_path};
use zorix_workflows::launcher::{Outcome, RunStart, StepLauncher, ToolCall};

use crate::app::{CliError, open_engine};
use crate::cli::{
    ProjectAskArgs, ProjectCommand, ProjectCreateArgs, ProjectEditArgs, ProjectListArgs, TaskAddArgs, TaskCommand,
    TaskDeliverArgs, TaskEditArgs, TaskHistoryArgs, TaskListArgs, TaskMoveArgs, TaskRestartArgs,
};
use crate::follow::{Exit, Printer};
use crate::render::shorten;

fn print_json(value: &impl serde::Serialize) -> Result<(), CliError> {
    println!("{}", serde_json::to_string(value)?);
    Ok(())
}

fn when(at: Timestamp) -> String {
    at.to_rfc3339().chars().take(16).collect::<String>().replace('T', " ")
}

fn usd(micros: u64) -> String {
    format!("${:.2}", micros as f64 / 1_000_000.0)
}

/// What stands in for the engine when a command only manages: it can read nothing and start nothing.
pub(crate) struct NoEngine;

const NEEDS_ENGINE: &str =
    "This needs the engine, and the data folder with it. Close the desktop app or `zorix serve`, or do it there.";

#[async_trait]
impl StepLauncher for NoEngine {
    async fn start(&self, _start: RunStart) -> Result<ExecutionId, String> {
        Err(NEEDS_ENGINE.into())
    }

    async fn outcome(&self, _id: ExecutionId) -> Result<Outcome, String> {
        Err(NEEDS_ENGINE.into())
    }

    async fn cancel(&self, _id: ExecutionId, _reason: &str) -> Result<(), String> {
        Err(NEEDS_ENGINE.into())
    }

    async fn call_tool(&self, _call: &ToolCall) -> Result<Value, String> {
        Err(NEEDS_ENGINE.into())
    }
}

fn quiet_service(home: &Path, tenant: &TenantId) -> Result<ProjectService, CliError> {
    let store = Arc::new(SqliteProjectStore::open(&database_path(home))?);
    Ok(ProjectService::new(store, Arc::new(NoEngine), tenant.clone(), Arc::new(SystemClock)).without_runs())
}

// ---------------------------------------------------------------------------------------------------------------------
// Reading what a person typed

fn project_key(text: &str) -> Result<ProjectKey, CliError> {
    ProjectKey::new(text.to_ascii_uppercase()).map_err(CliError)
}

fn task_key(text: &str) -> Result<TaskKey, CliError> {
    text.to_ascii_uppercase().parse().map_err(CliError)
}

fn state(text: &str) -> Result<TaskState, CliError> {
    text.parse().map_err(CliError)
}

fn priority(text: &str) -> Result<TaskPriority, CliError> {
    text.parse().map_err(CliError)
}

/// `you`, `agent:NAME`, `team:NAME`, a bare agent name, or `none`.
fn who(text: &str) -> Result<Option<Who>, CliError> {
    let slug = |s: &str| AgentSlug::new(s).map_err(CliError);
    Ok(match text {
        "none" | "" => None,
        "you" | "me" => Some(Who::You),
        other => match other.split_once(':') {
            Some(("agent", name)) => Some(Who::Agent { slug: slug(name)? }),
            Some(("team", name)) => Some(Who::Team { slug: slug(name)? }),
            Some(_) => return Err(CliError("Say who: `you`, `agent:NAME`, `team:NAME` or `none`.".into())),
            None => Some(Who::Agent { slug: slug(other)? }),
        },
    })
}

fn day(text: &str) -> Result<Option<Date>, CliError> {
    if text == "none" || text.is_empty() {
        return Ok(None);
    }
    Date::new(text).map(Some).map_err(CliError)
}

fn dollars(text: &str) -> Result<Option<u64>, CliError> {
    if text == "none" {
        return Ok(None);
    }
    let value: f64 = text.parse().map_err(|_| CliError(format!("“{text}” is not an amount of dollars.")))?;
    micros(value).map(Some)
}

fn micros(dollars: f64) -> Result<u64, CliError> {
    if !(dollars.is_finite() && dollars > 0.0 && dollars <= 100_000.0) {
        return Err(CliError("A cost limit is a number of dollars above zero.".into()));
    }
    Ok((dollars * 1_000_000.0).round() as u64)
}

fn who_text(who: &Option<Who>) -> String {
    who.as_ref().map(Who::label).unwrap_or_else(|| "—".into())
}

fn lane_name(state: TaskState) -> &'static str {
    match state {
        TaskState::Backlog => "Backlog",
        TaskState::InProgress => "In progress",
        TaskState::Review => "Review",
        TaskState::Done => "Done",
    }
}

fn priority_name(priority: TaskPriority) -> &'static str {
    match priority {
        TaskPriority::Urgent => "urgent",
        TaskPriority::High => "high",
        TaskPriority::Normal => "normal",
        TaskPriority::Low => "low",
    }
}

fn needs_text(needs: NeedsYou) -> &'static str {
    match needs {
        NeedsYou::Review => "needs you: review",
        NeedsYou::RunFailed => "needs you: the run failed",
        NeedsYou::RunWaiting => "needs you: the run waits for an approval",
    }
}

fn task_line(view: &TaskView) -> String {
    let t = &view.task;
    let mut extra = String::new();
    if let Some(needs) = view.needs_you {
        extra.push_str(&format!("  [{}]", needs_text(needs)));
    }
    if !view.blocked_by.is_empty() {
        let keys: Vec<String> = view.blocked_by.iter().map(ToString::to_string).collect();
        extra.push_str(&format!("  [waits for {}]", keys.join(", ")));
    }
    if view.working {
        extra.push_str("  [running]");
    }
    format!(
        "{:<8} {:<7} {:<10} {:>4} {:<10} {}{extra}",
        t.key.to_string(),
        priority_name(t.priority),
        shorten(&who_text(&t.assignee), 10),
        format!("{}%", t.progress),
        t.due.as_ref().map(ToString::to_string).unwrap_or_default(),
        shorten(&t.title, 50),
    )
}

// ---------------------------------------------------------------------------------------------------------------------
// project

pub async fn project(
    home: &Path,
    tenant: TenantId,
    printer: Printer,
    command: ProjectCommand,
) -> Result<Exit, CliError> {
    if let ProjectCommand::Ask(args) = command {
        return ask(home, tenant, printer, args).await;
    }
    let service = quiet_service(home, &tenant)?;
    match command {
        ProjectCommand::List(args) => list(&service, printer, args).await,
        ProjectCommand::Create(args) => create(&service, printer, args).await,
        ProjectCommand::Show(arg) => show(&service, printer, &arg.project).await,
        ProjectCommand::Edit(args) => edit(&service, printer, args).await,
        ProjectCommand::Archive(arg) => archive(&service, printer, &arg.project, true).await,
        ProjectCommand::Restore(arg) => archive(&service, printer, &arg.project, false).await,
        ProjectCommand::Remove(arg) => remove(&service, printer, &arg.project).await,
        ProjectCommand::Ask(_) => unreachable!("handled above"),
    }
}

async fn list(service: &ProjectService, printer: Printer, args: ProjectListArgs) -> Result<Exit, CliError> {
    let cards = service.projects(args.archived).await?;
    if printer.json {
        print_json(&cards)?;
        return Ok(Exit::Done);
    }
    if cards.is_empty() {
        eprintln!(
            "No {}projects. `zorix project create KEY NAME` makes one.",
            if args.archived { "archived " } else { "" }
        );
        return Ok(Exit::Done);
    }
    for card in &cards {
        let p = &card.project;
        let mut line = format!(
            "{:<6} {:<26} {:>3} of {:<3} tasks  {:<11} {:<10}",
            p.key.to_string(),
            shorten(&p.name, 26),
            card.counts.done,
            card.counts.total(),
            p.due.as_ref().map(|d| format!("due {d}")).unwrap_or_default(),
            p.manager.as_ref().map(ToString::to_string).unwrap_or_default(),
        );
        if card.needs_you > 0 {
            line.push_str(&format!("  {} need you", card.needs_you));
        }
        println!("{}", line.trim_end());
    }
    Ok(Exit::Done)
}

async fn create(service: &ProjectService, printer: Printer, args: ProjectCreateArgs) -> Result<Exit, CliError> {
    let manager = args.manager.as_deref().map(AgentSlug::new).transpose().map_err(CliError)?;
    let project = service
        .create_project(
            NewProject {
                key: project_key(&args.key)?,
                name: args.name,
                description: args.description.unwrap_or_default(),
                manager,
                start: args.start.as_deref().map(day).transpose()?.flatten(),
                due: args.due.as_deref().map(day).transpose()?.flatten(),
                cost_limit_micros: args.cost_limit.map(micros).transpose()?,
            },
            None,
        )
        .await?;
    if printer.json {
        print_json(&project)?;
    } else {
        println!(
            "Made {} ({}). `zorix task add {} \"Title\"` adds the first task.",
            project.name, project.key, project.key
        );
    }
    Ok(Exit::Done)
}

async fn show(service: &ProjectService, printer: Printer, key: &str) -> Result<Exit, CliError> {
    let board = service.board(&project_key(key)?).await?;
    if printer.json {
        print_json(&board)?;
        return Ok(Exit::Done);
    }
    let p = &board.project;
    let done = board.tasks.iter().filter(|t| t.task.state == TaskState::Done).count();
    println!(
        "{} ({}) · {} of {} tasks{}{}{}",
        p.name,
        p.key,
        done,
        board.tasks.len(),
        p.due.as_ref().map(|d| format!(" · due {d}")).unwrap_or_default(),
        p.manager.as_ref().map(|m| format!(" · manager {m}")).unwrap_or_default(),
        p.cost_limit_micros.map(|c| format!(" · {} a task run at most", usd(c))).unwrap_or_default(),
    );
    if p.archived {
        println!("Archived. `zorix project restore {}` brings it back.", p.key);
    }
    if !p.description.trim().is_empty() {
        println!("{}", p.description.trim());
    }
    if board.manager_run.is_some() {
        println!("The manager is planning now.");
    }
    for lane in TaskState::ALL {
        let tasks: Vec<&TaskView> = board.tasks.iter().filter(|t| t.task.state == lane).collect();
        println!("\n{} ({})", lane_name(lane), tasks.len());
        for view in tasks {
            println!("  {}", task_line(view));
        }
    }
    Ok(Exit::Done)
}

async fn edit(service: &ProjectService, printer: Printer, args: ProjectEditArgs) -> Result<Exit, CliError> {
    let key = project_key(&args.project)?;
    let current = service.project(&key).await?;
    let mut fields = ProjectFields::of(&current);
    if let Some(name) = args.name {
        fields.name = name;
    }
    if let Some(description) = args.description {
        fields.description = description;
    }
    if let Some(manager) = args.manager {
        fields.manager = if manager == "none" { None } else { Some(AgentSlug::new(manager).map_err(CliError)?) };
    }
    if let Some(start) = args.start {
        fields.start = day(&start)?;
    }
    if let Some(due) = args.due {
        fields.due = day(&due)?;
    }
    if let Some(limit) = args.cost_limit {
        fields.cost_limit_micros = dollars(&limit)?;
    }
    let saved = service.update_project(&key, fields, None).await?;
    if printer.json {
        print_json(&saved)?;
    } else {
        println!("Saved {}.", saved.key);
    }
    Ok(Exit::Done)
}

async fn archive(service: &ProjectService, printer: Printer, key: &str, archived: bool) -> Result<Exit, CliError> {
    let key = project_key(key)?;
    let current = service.project(&key).await?;
    let mut fields = ProjectFields::of(&current);
    fields.archived = archived;
    let saved = service.update_project(&key, fields, None).await?;
    if printer.json {
        print_json(&saved)?;
    } else {
        println!("{} {}.", saved.key, if archived { "archived" } else { "is active again" });
    }
    Ok(Exit::Done)
}

async fn remove(service: &ProjectService, printer: Printer, key: &str) -> Result<Exit, CliError> {
    let key = project_key(key)?;
    service.remove_project(&key).await?;
    if printer.json {
        print_json(&serde_json::json!({"removed": key.to_string()}))?;
    } else {
        println!("Deleted {key} with its tasks and history.");
    }
    Ok(Exit::Done)
}

async fn ask(home: &Path, tenant: TenantId, printer: Printer, args: ProjectAskArgs) -> Result<Exit, CliError> {
    let app = open_engine(home, &tenant, &args.model)?;
    let result = async {
        let key = project_key(&args.project)?;
        let execution = app.projects.ask_manager(&key, args.prompt.as_deref()).await?;
        if printer.json {
            print_json(&serde_json::json!({"execution": execution.to_string()}))?;
        } else {
            println!("Asked the manager of {key}. Run {execution}.");
            println!("Every change it wants asks you first: `zorix approve {execution}` or `zorix deny {execution}`.");
        }
        // The run is followed like any other: what it asks for waits for you.
        use zorix_core::ExecutionEngine as _;
        let stream = app.engine.subscribe(&tenant, execution, 0).await?;
        crate::follow::follow(&app.engine, &tenant, execution, stream, crate::follow::default_mode(), printer).await
    }
    .await;
    app.shutdown().await;
    result
}

// ---------------------------------------------------------------------------------------------------------------------
// task

pub async fn task(home: &Path, tenant: TenantId, printer: Printer, command: TaskCommand) -> Result<Exit, CliError> {
    match command {
        TaskCommand::Move(args) => return move_task(home, tenant, printer, args).await,
        TaskCommand::Restart(args) => return restart(home, tenant, printer, args).await,
        TaskCommand::Add(args)
            if args.state.as_deref().is_some_and(|s| s.starts_with("in")) && args.assign.is_some() =>
        {
            return add_with_engine(home, tenant, printer, args).await;
        }
        TaskCommand::Edit(args) if args.assign.is_some() => return edit_with_engine(home, tenant, printer, args).await,
        _ => {}
    }
    let service = quiet_service(home, &tenant)?;
    match command {
        TaskCommand::List(args) => list_tasks(&service, printer, args).await,
        TaskCommand::Add(args) => add(&service, printer, args).await,
        TaskCommand::Show(args) => show_task(&service, printer, &args.task, args.full).await,
        TaskCommand::Edit(args) => edit_task(&service, printer, args).await,
        TaskCommand::Comment(args) => {
            service.comment(&task_key(&args.task)?, &args.text, None).await?;
            done(printer, "commented", &args.task)
        }
        TaskCommand::Link(args) => {
            let key = task_key(&args.task)?;
            service.add_link(&key, &args.url, args.title, None).await?;
            done(printer, "linked", &key.to_string())
        }
        TaskCommand::Deliver(args) => deliver(&service, printer, args).await,
        TaskCommand::Remove(arg) => {
            let key = task_key(&arg.task)?;
            service.remove_task(&key).await?;
            done(printer, "removed", &key.to_string())
        }
        TaskCommand::History(args) => history(&service, printer, args).await,
        TaskCommand::Move(_) | TaskCommand::Restart(_) => unreachable!("handled above"),
    }
}

fn done(printer: Printer, what: &str, key: &str) -> Result<Exit, CliError> {
    if printer.json {
        print_json(&serde_json::json!({what: key}))?;
    } else {
        println!("{key} {what}.");
    }
    Ok(Exit::Done)
}

async fn list_tasks(service: &ProjectService, printer: Printer, args: TaskListArgs) -> Result<Exit, CliError> {
    let want = args.state.as_deref().map(state).transpose()?;
    let keys: Vec<ProjectKey> = match &args.project {
        Some(key) => vec![project_key(key)?],
        None => service.projects(false).await?.into_iter().map(|c| c.project.key).collect(),
    };
    let mut views = Vec::new();
    for key in &keys {
        views.extend(service.tasks(key).await?);
    }
    views.retain(|v| {
        want.is_none_or(|s| v.task.state == s)
            && (!args.mine || v.task.assignee == Some(Who::You))
            && (!args.needs_you || v.needs_you.is_some())
    });
    if printer.json {
        print_json(&views)?;
        return Ok(Exit::Done);
    }
    if views.is_empty() {
        eprintln!("No tasks match.");
        return Ok(Exit::Done);
    }
    for lane in TaskState::ALL {
        let in_lane: Vec<&TaskView> = views.iter().filter(|v| v.task.state == lane).collect();
        if in_lane.is_empty() {
            continue;
        }
        println!("{} ({})", lane_name(lane), in_lane.len());
        for view in in_lane {
            println!("  {}", task_line(view));
        }
        println!();
    }
    Ok(Exit::Done)
}

fn new_task(args: &TaskAddArgs) -> Result<NewTask, CliError> {
    Ok(NewTask {
        title: args.title.clone(),
        description: args.description.clone().unwrap_or_default(),
        state: args.state.as_deref().map(state).transpose()?,
        priority: priority(&args.priority)?,
        assignee: args.assign.as_deref().map(who).transpose()?.flatten(),
        start: args.start.as_deref().map(day).transpose()?.flatten(),
        due: args.due.as_deref().map(day).transpose()?.flatten(),
        depends_on: args.after.iter().map(|k| task_key(k)).collect::<Result<_, _>>()?,
        links: Vec::new(),
    })
}

async fn add(service: &ProjectService, printer: Printer, args: TaskAddArgs) -> Result<Exit, CliError> {
    let task = service.add_task(&project_key(&args.project)?, new_task(&args)?, None).await?;
    if printer.json {
        print_json(&task)?;
    } else {
        println!("Added {}: {}.", task.key, task.title);
    }
    Ok(Exit::Done)
}

async fn add_with_engine(home: &Path, tenant: TenantId, printer: Printer, args: TaskAddArgs) -> Result<Exit, CliError> {
    let app = open_engine(home, &tenant, &args.model)?;
    let result = async {
        let key = app.projects.add_task(&project_key(&args.project)?, new_task(&args)?, None).await?.key;
        follow_task(&app.projects, printer, &key, true).await
    }
    .await;
    app.shutdown().await;
    result
}

async fn show_task(service: &ProjectService, printer: Printer, key: &str, full: bool) -> Result<Exit, CliError> {
    let key = task_key(key)?;
    let detail = service.task(&key).await?;
    let links = service.link_states(&key).await.unwrap_or_default();
    if printer.json {
        print_json(&serde_json::json!({"task": detail.task, "events": detail.events, "links": links}))?;
        return Ok(Exit::Done);
    }
    let t = &detail.task.task;
    println!("{} · {}", t.key, t.title);
    println!(
        "{} · {} · {} · {}%{}",
        lane_name(t.state),
        priority_name(t.priority),
        who_text(&t.assignee),
        t.progress,
        match (&t.start, &t.due) {
            (Some(s), Some(d)) => format!(" · {s} to {d}"),
            (None, Some(d)) => format!(" · due {d}"),
            (Some(s), None) => format!(" · from {s}"),
            (None, None) => String::new(),
        }
    );
    if let Some(needs) = detail.task.needs_you {
        println!("{}", needs_text(needs));
    }
    if !detail.task.blocked_by.is_empty() {
        let keys: Vec<String> = detail.task.blocked_by.iter().map(ToString::to_string).collect();
        println!("Waits for {}", keys.join(", "));
    }
    if !t.description.trim().is_empty() {
        println!("\n{}", t.description.trim());
    }
    if !links.is_empty() {
        println!("\nLinks");
        for link in &links {
            let state = link
                .pull
                .as_ref()
                .map(|p| format!("  (#{} {}{})", p.number, p.state, if p.draft { ", draft" } else { "" }))
                .unwrap_or_default();
            println!("  {}{state}", link.url);
        }
    }
    if !t.deliverables.is_empty() {
        println!("\nDeliverables");
        for d in &t.deliverables {
            println!("  {} · {} · {}", d.title, who_text(&d.by), when(d.at));
            if full {
                for line in d.body.lines() {
                    println!("    {line}");
                }
            }
        }
    }
    if !t.runs.is_empty() {
        println!("\nRuns");
        for r in &t.runs {
            println!(
                "  {} · {:?} · {}{}",
                r.execution,
                r.outcome,
                usd(r.cost_micros),
                r.reason.as_ref().map(|x| format!(" · {}", shorten(x, 60))).unwrap_or_default()
            );
        }
    }
    println!("\nHistory");
    for event in detail.events.iter().take(10) {
        println!("  {}  {}", when(event.at), sentence(event));
    }
    Ok(Exit::Done)
}

fn fields_from(args: &TaskEditArgs, current: &zorix_core::Task) -> Result<TaskFields, CliError> {
    let mut fields = TaskFields::of(current);
    if let Some(v) = &args.title {
        fields.title = v.clone();
    }
    if let Some(v) = &args.description {
        fields.description = v.clone();
    }
    if let Some(v) = &args.priority {
        fields.priority = priority(v)?;
    }
    if let Some(v) = &args.assign {
        fields.assignee = who(v)?;
    }
    if let Some(v) = &args.start {
        fields.start = day(v)?;
    }
    if let Some(v) = &args.due {
        fields.due = day(v)?;
    }
    if let Some(v) = args.progress {
        fields.progress = v;
    }
    if !args.after.is_empty() {
        fields.depends_on = if args.after == ["none"] {
            Vec::new()
        } else {
            args.after.iter().map(|k| task_key(k)).collect::<Result<_, _>>()?
        };
    }
    Ok(fields)
}

async fn edit_task(service: &ProjectService, printer: Printer, args: TaskEditArgs) -> Result<Exit, CliError> {
    let key = task_key(&args.task)?;
    let current = service.task(&key).await?.task.task;
    let saved = service.update_task(&key, fields_from(&args, &current)?, None).await?;
    if printer.json {
        print_json(&saved)?;
    } else {
        println!("Saved {}.", saved.key);
    }
    Ok(Exit::Done)
}

async fn edit_with_engine(
    home: &Path,
    tenant: TenantId,
    printer: Printer,
    args: TaskEditArgs,
) -> Result<Exit, CliError> {
    let app = open_engine(home, &tenant, &args.model)?;
    let result = async {
        let key = task_key(&args.task)?;
        let current = app.projects.task(&key).await?.task.task;
        app.projects.update_task(&key, fields_from(&args, &current)?, None).await?;
        follow_task(&app.projects, printer, &key, true).await
    }
    .await;
    app.shutdown().await;
    result
}

async fn deliver(service: &ProjectService, printer: Printer, args: TaskDeliverArgs) -> Result<Exit, CliError> {
    let key = task_key(&args.task)?;
    let (kind, body, default_title) = match (&args.text, &args.file, &args.url) {
        (Some(text), None, None) => (DeliverableKind::Answer, text.clone(), "Note".to_string()),
        (None, Some(file), None) => {
            let text =
                std::fs::read_to_string(file).map_err(|e| CliError(format!("Cannot read {}: {e}", file.display())))?;
            let name = file.file_name().map(|n| n.to_string_lossy().to_string()).unwrap_or_else(|| "File".into());
            (DeliverableKind::Answer, text, name)
        }
        (None, None, Some(url)) => (DeliverableKind::Link, url.clone(), url.clone()),
        _ => return Err(CliError("Give exactly one of --text, --file and --url.".into())),
    };
    let title = args.title.unwrap_or(default_title);
    service.add_deliverable(&key, kind, &title, &body, Some(Who::You), None).await?;
    done(printer, "has a new deliverable", &key.to_string())
}

async fn history(service: &ProjectService, printer: Printer, args: TaskHistoryArgs) -> Result<Exit, CliError> {
    let (project, task) = if args.target.contains('-') {
        let key = task_key(&args.target)?;
        (key.project.clone(), Some(key))
    } else {
        (project_key(&args.target)?, None)
    };
    let events = service.history(&project, task.as_ref(), args.limit).await?;
    if printer.json {
        print_json(&events)?;
        return Ok(Exit::Done);
    }
    if events.is_empty() {
        eprintln!("Nothing has happened yet.");
    }
    for event in &events {
        println!("{}  {}", when(event.at), sentence(event));
    }
    Ok(Exit::Done)
}

// ---------------------------------------------------------------------------------------------------------------------
// Moves that start runs

async fn move_task(home: &Path, tenant: TenantId, printer: Printer, args: TaskMoveArgs) -> Result<Exit, CliError> {
    let to = state(&args.state)?;
    let key = task_key(&args.task)?;
    let starts_a_run = to == TaskState::InProgress;
    if !starts_a_run {
        // Stopping a run needs the engine only when one is going; try without it first.
        let service = quiet_service(home, &tenant)?;
        let current = service.task(&key).await?.task.task;
        if current.current_run().is_none() {
            let task = service
                .move_task(&key, Move { to: Some(to), note: args.note.clone(), ..Move::default() }, None)
                .await?;
            return moved(printer, &task);
        }
    }
    let app = open_engine(home, &tenant, &args.model)?;
    let result = async {
        let task = app
            .projects
            .move_task(
                &key,
                Move { to: Some(to), note: args.note.clone(), cancel_run: args.stop_run, ..Move::default() },
                None,
            )
            .await
            .map_err(|e| {
                if e.to_string().contains(zorix_projects::RUN_GOING) {
                    CliError(format!("{key} has a run going. Add --stop-run to stop it and move the task."))
                } else {
                    CliError::from(e)
                }
            })?;
        if !printer.json {
            println!("{} moved to {}.", task.key, lane_name(task.state));
        }
        follow_task(&app.projects, printer, &key, !args.no_wait).await
    }
    .await;
    app.shutdown().await;
    result
}

fn moved(printer: Printer, task: &zorix_core::Task) -> Result<Exit, CliError> {
    if printer.json {
        print_json(task)?;
    } else {
        println!("{} moved to {}.", task.key, lane_name(task.state));
    }
    Ok(Exit::Done)
}

async fn restart(home: &Path, tenant: TenantId, printer: Printer, args: TaskRestartArgs) -> Result<Exit, CliError> {
    let app = open_engine(home, &tenant, &args.model)?;
    let result = async {
        let key = task_key(&args.task)?;
        app.projects.restart_task(&key, args.note.clone(), None).await?;
        follow_task(&app.projects, printer, &key, !args.no_wait).await
    }
    .await;
    app.shutdown().await;
    result
}

/// Say how a task stands, and (when asked) wait for its run to end or for a person.
async fn follow_task(service: &ProjectService, printer: Printer, key: &TaskKey, wait: bool) -> Result<Exit, CliError> {
    let mut said_running = false;
    loop {
        let detail = service.task(key).await?;
        let view = &detail.task;
        let t = &view.task;
        let finished = t.state != TaskState::InProgress;
        let stopped = matches!(view.needs_you, Some(NeedsYou::RunFailed | NeedsYou::RunWaiting));
        let waiting_start = t.run_requested.is_some() && !view.blocked_by.is_empty();
        if !wait && t.current_run().is_some()
            || finished
            || stopped
            || waiting_start
            || t.current_run().is_none() && t.run_requested.is_none()
        {
            if printer.json {
                print_json(&detail.task)?;
            } else {
                if waiting_start {
                    let keys: Vec<String> = view.blocked_by.iter().map(ToString::to_string).collect();
                    println!("{key} waits for {}. Its run starts when they are done.", keys.join(", "));
                }
                println!();
                println!("{}", task_line(view));
                for d in t.deliverables.iter().rev().take(1) {
                    println!("Deliverable: {}", d.title);
                }
                if let Some(run) = t.runs.last() {
                    println!("Run {} · {:?} · {}", run.execution, run.outcome, usd(run.cost_micros));
                    if let Some(reason) = &run.reason {
                        println!("Failed       {reason}");
                    }
                    if view.needs_you == Some(NeedsYou::RunWaiting) {
                        println!("Waiting      `zorix approve {}` or `zorix deny {}`", run.execution, run.execution);
                    }
                }
                if let Some(error) = t.run_requested.as_ref().and_then(|r| r.error.as_ref()) {
                    println!("Could not start: {error}");
                }
            }
            return Ok(match view.needs_you {
                Some(NeedsYou::RunFailed) => Exit::Failed,
                Some(NeedsYou::RunWaiting) => Exit::Waiting,
                _ => Exit::Done,
            });
        }
        if !said_running && !printer.json {
            println!("{key} is running…");
            said_running = true;
        }
        tokio::time::sleep(Duration::from_secs(1)).await;
    }
}
