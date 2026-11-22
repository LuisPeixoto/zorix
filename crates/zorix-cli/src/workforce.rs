//! `zorix org` and `zorix worker`: what a person does with the workforce from the terminal (ADR-0038, ADR-0039).
//!
//! Managing needs no engine, so it works while the desktop app is open: the app (or `zorix serve`) keeps plans, reports, schedules and
//! budget notices moving. Starting a plan or a report needs the engine, and with it the data folder.

use std::path::Path;
use std::sync::Arc;

use zorix_core::{AgentSlug, AutonomyLevel, Department, OrgSettings, SystemClock, TenantId, Timestamp};
use zorix_store::{SqliteAgentStore, SqliteOrgStore, SqliteProjectStore, SqliteRepository, database_path};
use zorix_workforce::{
    ActivityLine, NewWorker, Overview, SettingsFields, WorkerDetail, WorkerFields, WorkerView, WorkforceService,
};

use crate::app::{CliError, open_engine};
use crate::cli::{
    ActivityArgs, DeptCommand, NoticesArgs, OrgCommand, OrgSettingsArgs, WorkerAskArgs, WorkerCommand, WorkerEditArgs,
    WorkerHireArgs, WorkerLevelArgs, WorkerScheduleArgs,
};
use crate::follow::{Exit, Printer};
use crate::projects::NoEngine;
use crate::render::shorten;

fn print_json(value: &impl serde::Serialize) -> Result<(), CliError> {
    println!("{}", serde_json::to_string(value)?);
    Ok(())
}

fn usd(micros: u64) -> String {
    format!("${:.2}", micros as f64 / 1_000_000.0)
}

fn when(at: Timestamp) -> String {
    at.to_rfc3339().chars().take(16).collect::<String>().replace('T', " ")
}

fn quiet_service(home: &Path, tenant: &TenantId) -> Result<WorkforceService, CliError> {
    let path = database_path(home);
    Ok(WorkforceService::new(
        Arc::new(SqliteOrgStore::open(&path)?),
        Arc::new(SqliteProjectStore::open(&path)?),
        Arc::new(SqliteAgentStore::open(&path)?),
        Arc::new(SqliteRepository::open(&path)?),
        Arc::new(NoEngine),
        tenant.clone(),
        Arc::new(SystemClock),
    ))
}

// ---------------------------------------------------------------------------------------------------------------------
// Reading what a person typed

fn slug(text: &str) -> Result<AgentSlug, CliError> {
    AgentSlug::new(text).map_err(CliError)
}

fn level(text: &str) -> Result<AutonomyLevel, CliError> {
    AutonomyLevel::parse(text).map_err(CliError)
}

/// A worker's chief: a worker's name, or `you` (or `none`) for the root.
fn boss(text: &str) -> Result<Option<AgentSlug>, CliError> {
    match text {
        "you" | "none" | "" => Ok(None),
        other => slug(other).map(Some),
    }
}

fn dollars(value: f64) -> Result<u64, CliError> {
    if !(value.is_finite() && value > 0.0 && value <= 1_000_000.0) {
        return Err(CliError("A budget is a number of dollars above zero.".into()));
    }
    Ok((value * 1_000_000.0).round() as u64)
}

fn budget_text(text: &str) -> Result<Option<u64>, CliError> {
    if text == "none" {
        return Ok(None);
    }
    let value: f64 = text.parse().map_err(|_| CliError(format!("“{text}” is not an amount of dollars.")))?;
    dollars(value).map(Some)
}

fn minutes(value: u64) -> Result<u64, CliError> {
    value.checked_mul(60_000).ok_or_else(|| CliError("That is too long.".into()))
}

// ---------------------------------------------------------------------------------------------------------------------
// Showing

fn percent(done: u32, all: u32) -> String {
    if all == 0 { "—".into() } else { format!("{}%", (f64::from(done) / f64::from(all) * 100.0).round() as u32) }
}

fn worker_line(v: &WorkerView) -> String {
    let w = &v.worker;
    let f = &v.figures;
    format!(
        "{:<12} {:<22} {:<11} {:<9} {:>3} tasks  {:>5} on time  {:>5} eval  {:>7} week{}",
        w.agent.to_string(),
        shorten(&w.role, 22),
        w.department,
        w.autonomy.label(),
        f.tasks,
        percent(f.on_time_done, f.on_time_all),
        v.own.eval.map(|e| format!("{e:.2}")).unwrap_or_else(|| "—".into()),
        usd(f.week_micros),
        match v.status {
            Some("warn") => "  waiting for you",
            Some("err") => "  stopped",
            Some("ok") => "  working",
            _ => "",
        }
    )
}

fn print_chart(workers: &[WorkerView]) {
    fn walk(workers: &[WorkerView], parent: Option<&AgentSlug>, depth: usize) {
        for v in workers.iter().filter(|v| v.worker.reports_to.as_ref() == parent) {
            println!("{}{}", "  ".repeat(depth + 1), worker_line(v));
            walk(workers, Some(&v.worker.agent), depth + 1);
        }
    }
    println!("You");
    walk(workers, None, 0);
}

fn print_overview(o: &Overview) {
    println!(
        "Workforce · week of {} · {} worker{} in {} department{}",
        o.week,
        o.workers.len(),
        if o.workers.len() == 1 { "" } else { "s" },
        o.settings.departments.len(),
        if o.settings.departments.len() == 1 { "" } else { "s" }
    );
    println!(
        "Open tasks {} ({} due this week) · waiting for you: {} approval{}, {} review{} · spend this week {}{}",
        o.open_tasks,
        o.due_this_week,
        o.waiting_approvals,
        if o.waiting_approvals == 1 { "" } else { "s" },
        o.waiting_reviews,
        if o.waiting_reviews == 1 { "" } else { "s" },
        usd(o.week_micros),
        o.budget_micros.map(|b| format!(" of {}", usd(b))).unwrap_or_else(|| " (no weekly budget)".into())
    );
    println!("New workers start at {}.", o.settings.default_level);
    println!();
    print_chart(&o.workers);
}

fn print_activity(lines: &[ActivityLine]) {
    for line in lines {
        println!("{}  {:<4} {:<10} {}", when(line.at), line.level, shorten(&line.source, 10), line.text);
    }
}

fn print_detail(d: &WorkerDetail) {
    let v = &d.view;
    let w = &v.worker;
    println!(
        "{} · {} · {} · reports to {}",
        w.agent,
        w.role,
        w.department,
        w.reports_to.as_ref().map(ToString::to_string).unwrap_or_else(|| "you".into())
    );
    println!("Autonomy     {}{}", w.autonomy.label(), if w.manager { " · virtual manager" } else { "" });
    println!(
        "Limits       {} a week · {} min a task{}",
        w.weekly_budget_micros.map(usd).unwrap_or_else(|| "no weekly budget".into()),
        w.task_time_ms / 60_000,
        if w.still_ask.is_empty() { String::new() } else { format!(" · always asks: {}", w.still_ask.join(", ")) }
    );
    println!(
        "Week         {} spent{} · {} open task{} · {} on time · eval {}",
        usd(v.own.week_micros),
        v.budget_left_micros.map(|l| format!(", {} left", usd(l))).unwrap_or_default(),
        v.own.tasks,
        if v.own.tasks == 1 { "" } else { "s" },
        percent(v.own.on_time_done, v.own.on_time_all),
        v.own.eval.map(|e| format!("{e:.2}")).unwrap_or_else(|| "—".into())
    );
    if w.manager {
        println!(
            "Schedule     {}",
            if w.schedule.enabled {
                format!("plan {} · report {}", w.schedule.plan.describe(), w.schedule.report.describe())
            } else {
                "off".into()
            }
        );
    }
    if !v.reports.is_empty() {
        let names: Vec<String> = v.reports.iter().map(ToString::to_string).collect();
        println!("Reports      {}", names.join(", "));
    }
    if !d.tasks.is_empty() {
        println!();
        println!("Open tasks");
        for t in &d.tasks {
            println!(
                "  {:<8} {:<40} {}{}",
                t.key,
                shorten(&t.title, 40),
                t.due.as_ref().map(|x| x.to_string()).unwrap_or_default(),
                if t.late { "  late" } else { "" }
            );
        }
    }
    if !d.activity.is_empty() {
        println!();
        println!("This week");
        print_activity(&d.activity.iter().take(8).cloned().collect::<Vec<_>>());
    }
    if let Some(report) = d.reports.iter().find(|r| r.status == zorix_core::ReportStatus::Done) {
        println!();
        println!(
            "Latest {} ({})",
            if report.kind == zorix_core::ReportKind::Plan { "plan" } else { "report" },
            report.week
        );
        println!("{}", report.body.trim());
    }
}

// ---------------------------------------------------------------------------------------------------------------------
// org

pub async fn org(home: &Path, tenant: TenantId, printer: Printer, command: OrgCommand) -> Result<Exit, CliError> {
    let service = quiet_service(home, &tenant)?;
    match command {
        OrgCommand::Show => {
            let overview = service.overview().await?;
            if printer.json {
                print_json(&overview)?;
            } else {
                print_overview(&overview);
            }
        }
        OrgCommand::Settings(args) => settings(&service, printer, args).await?,
        OrgCommand::Dept(command) => dept(&service, printer, command).await?,
        OrgCommand::Activity(ActivityArgs { limit }) => {
            let lines = service.activity(limit).await?;
            if printer.json {
                print_json(&lines)?;
            } else if lines.is_empty() {
                eprintln!("Nothing yet. Workers show up here when they run.");
            } else {
                print_activity(&lines);
            }
        }
        OrgCommand::Notices(NoticesArgs { read }) => {
            let notices = service.notices().await?;
            if printer.json {
                print_json(&notices)?;
            } else if notices.is_empty() {
                eprintln!("Nothing new.");
            } else {
                for n in &notices {
                    println!("{}  {:<10} {} — {}", when(n.at), n.kind, n.title, shorten(&n.text, 100));
                }
            }
            if read {
                service.mark_seen().await?;
            }
        }
    }
    Ok(Exit::Done)
}

fn fields_of(settings: &OrgSettings) -> SettingsFields {
    SettingsFields {
        weekly_budget_micros: settings.weekly_budget_micros,
        default_level: settings.default_level,
        departments: settings.departments.clone(),
    }
}

async fn settings(service: &WorkforceService, printer: Printer, args: OrgSettingsArgs) -> Result<(), CliError> {
    let current = service.settings().await?;
    let mut fields = fields_of(&current);
    if let Some(text) = &args.budget {
        fields.weekly_budget_micros = budget_text(text)?;
    }
    if let Some(text) = &args.level {
        fields.default_level = level(text)?;
    }
    let saved = if args.budget.is_some() || args.level.is_some() {
        service.save_settings(fields, &by()).await?
    } else {
        current
    };
    if printer.json {
        print_json(&saved)?;
    } else {
        println!(
            "Weekly budget {} · new workers start at {} · {} department{}",
            saved.weekly_budget_micros.map(usd).unwrap_or_else(|| "none".into()),
            saved.default_level,
            saved.departments.len(),
            if saved.departments.len() == 1 { "" } else { "s" }
        );
        for d in &saved.departments {
            println!(
                "  {:<14} {}{}",
                d.slug,
                d.name,
                d.lead.as_ref().map(|l| format!(" · led by {l}")).unwrap_or_default()
            );
        }
    }
    Ok(())
}

fn by() -> String {
    "you".into()
}

async fn dept(service: &WorkforceService, printer: Printer, command: DeptCommand) -> Result<(), CliError> {
    let current = service.settings().await?;
    let mut fields = fields_of(&current);
    let said = match command {
        DeptCommand::Add(args) => {
            fields.departments.push(Department {
                slug: args.department.clone(),
                name: args.name,
                description: args.description.unwrap_or_default(),
                lead: args.lead.as_deref().map(boss).transpose()?.flatten(),
            });
            format!("added {}", args.department)
        }
        DeptCommand::Edit(args) => {
            let d = fields
                .departments
                .iter_mut()
                .find(|d| d.slug == args.department)
                .ok_or_else(|| CliError(format!("There is no department {}.", args.department)))?;
            if let Some(name) = args.name {
                d.name = name;
            }
            if let Some(description) = args.description {
                d.description = description;
            }
            if let Some(lead) = args.lead {
                d.lead = boss(&lead)?;
            }
            format!("changed {}", args.department)
        }
        DeptCommand::Remove(arg) => {
            let before = fields.departments.len();
            fields.departments.retain(|d| d.slug != arg.department);
            if fields.departments.len() == before {
                return Err(CliError(format!("There is no department {}.", arg.department)));
            }
            format!("removed {}", arg.department)
        }
    };
    service.save_settings(fields, &by()).await?;
    if printer.json {
        print_json(&serde_json::json!({ "done": said }))?;
    } else {
        println!("Department {said}.");
    }
    Ok(())
}

// ---------------------------------------------------------------------------------------------------------------------
// worker

pub async fn worker(home: &Path, tenant: TenantId, printer: Printer, command: WorkerCommand) -> Result<Exit, CliError> {
    if let WorkerCommand::Plan(args) = command {
        return week(home, tenant, printer, args, true).await;
    }
    if let WorkerCommand::Report(args) = command {
        return week(home, tenant, printer, args, false).await;
    }
    let service = quiet_service(home, &tenant)?;
    match command {
        WorkerCommand::List => {
            let overview = service.overview().await?;
            if printer.json {
                print_json(&overview.workers)?;
            } else if overview.workers.is_empty() {
                eprintln!("No workers. `zorix worker hire AGENT --role ROLE --dept DEPARTMENT` makes one.");
            } else {
                print_chart(&overview.workers);
            }
        }
        WorkerCommand::Hire(args) => hire(&service, printer, args).await?,
        WorkerCommand::Show(arg) => {
            let detail = service.detail(&slug(&arg.worker)?).await?;
            if printer.json {
                print_json(&detail)?;
            } else {
                print_detail(&detail);
            }
        }
        WorkerCommand::Edit(args) => edit(&service, printer, args).await?,
        WorkerCommand::Level(WorkerLevelArgs { worker, level: text }) => {
            let agent = slug(&worker)?;
            let mut fields = WorkerFields::of(&service.worker(&agent).await?);
            fields.autonomy = level(&text)?;
            let saved = service.edit(&agent, fields, &by()).await?;
            said(printer, &saved.agent.to_string(), &format!("is at {}", saved.autonomy))?;
        }
        WorkerCommand::Fire(arg) => {
            service.fire(&slug(&arg.worker)?, &by()).await?;
            said(printer, &arg.worker, "was let go")?;
        }
        WorkerCommand::Schedule(WorkerScheduleArgs { worker, state }) => {
            let agent = slug(&worker)?;
            let on = match state.as_str() {
                "on" => true,
                "off" => false,
                other => return Err(CliError(format!("Say `on` or `off`, not “{other}”."))),
            };
            let current = service.worker(&agent).await?;
            if on && !current.manager {
                return Err(CliError(format!(
                    "{agent} is not a manager. `zorix worker edit {agent} --manager true` makes it one."
                )));
            }
            let mut fields = WorkerFields::of(&current);
            fields.schedule_enabled = on;
            let saved = service.edit(&agent, fields, &by()).await?;
            let sentence = if on {
                format!(
                    "plans {} and reports {} while the app or `zorix serve` is open",
                    saved.schedule.plan.describe(),
                    saved.schedule.report.describe()
                )
            } else {
                "has no schedule".into()
            };
            said(printer, &agent.to_string(), &sentence)?;
        }
        WorkerCommand::Plan(_) | WorkerCommand::Report(_) => unreachable!("handled above"),
    }
    Ok(Exit::Done)
}

fn said(printer: Printer, worker: &str, what: &str) -> Result<(), CliError> {
    if printer.json {
        print_json(&serde_json::json!({ "worker": worker, "done": what }))?;
    } else {
        println!("{worker} {what}.");
    }
    Ok(())
}

async fn hire(service: &WorkforceService, printer: Printer, args: WorkerHireArgs) -> Result<(), CliError> {
    let new = NewWorker {
        agent: slug(&args.agent)?,
        role: args.role,
        department: args.dept,
        reports_to: boss(&args.reports_to)?,
        manager: args.manager,
        autonomy: args.level.as_deref().map(level).transpose()?,
        weekly_budget_micros: args.budget.map(dollars).transpose()?,
        task_time_ms: args.task_time.map(minutes).transpose()?,
        still_ask: args.still_ask,
    };
    let hired = service.hire(new, &by()).await?;
    if printer.json {
        print_json(&hired)?;
    } else {
        println!("{} was hired as {} in {}, at {}.", hired.agent, hired.role, hired.department, hired.autonomy);
    }
    Ok(())
}

async fn edit(service: &WorkforceService, printer: Printer, args: WorkerEditArgs) -> Result<(), CliError> {
    let agent = slug(&args.worker)?;
    let mut fields = WorkerFields::of(&service.worker(&agent).await?);
    if let Some(role) = args.role {
        fields.role = role;
    }
    if let Some(dept) = args.dept {
        fields.department = dept;
    }
    if let Some(chief) = args.reports_to {
        fields.reports_to = boss(&chief)?;
    }
    if let Some(manager) = args.manager {
        fields.manager = manager;
    }
    if let Some(text) = args.level {
        fields.autonomy = level(&text)?;
    }
    if let Some(text) = args.budget {
        fields.weekly_budget_micros = budget_text(&text)?;
    }
    if let Some(m) = args.task_time {
        fields.task_time_ms = minutes(m)?;
    }
    if !args.still_ask.is_empty() {
        fields.still_ask = if args.still_ask == ["none"] { Vec::new() } else { args.still_ask };
    }
    let saved = service.edit(&agent, fields, &by()).await?;
    if printer.json {
        print_json(&saved)?;
    } else {
        println!("{} was changed.", saved.agent);
    }
    Ok(())
}

async fn week(
    home: &Path,
    tenant: TenantId,
    printer: Printer,
    args: WorkerAskArgs,
    plan: bool,
) -> Result<Exit, CliError> {
    let app = open_engine(home, &tenant, &args.model)?;
    let result = async {
        let agent = slug(&args.worker)?;
        let report = if plan {
            app.workforce.plan_week(&agent, args.prompt.as_deref()).await?
        } else {
            app.workforce.write_report(&agent, args.prompt.as_deref()).await?
        };
        if printer.json {
            print_json(&serde_json::json!({"report": report.id.to_string(), "execution": report.run.to_string()}))?;
        } else {
            println!(
                "Asked {agent} to {}. Run {}.",
                if plan { "plan the week" } else { "write the report" },
                report.run
            );
            println!(
                "What it wants to change asks you first: `zorix approve {}` or `zorix deny {}`.",
                report.run, report.run
            );
        }
        use zorix_core::ExecutionEngine as _;
        let stream = app.engine.subscribe(&tenant, report.run, 0).await?;
        let exit =
            crate::follow::follow(&app.engine, &tenant, report.run, stream, crate::follow::default_mode(), printer)
                .await?;
        // Keep the answer now, without waiting for the app's next beat.
        let _ = app.workforce.reconcile().await;
        Ok(exit)
    }
    .await;
    app.shutdown().await;
    result
}
