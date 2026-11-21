//! `zorix workflow`: what a person does with workflows from the terminal (ADR-0033 to ADR-0035).
//!
//! Managing workflows needs no engine, so it works while the desktop app is open. Running one, answering a wait and cancelling a
//! run need the engine, and with it the data folder: close the app, or let the app or `zorix serve` do it.

use std::path::Path;
use std::sync::Arc;
use std::time::Duration;

use serde_json::{Value, json};
use zorix_core::{
    Confirm, StepState, SystemClock, TenantId, Timestamp, WaitId, WaitKind, WaitStatus, Workflow, WorkflowRun,
    WorkflowRunId, WorkflowSpec, WorkflowStore,
};
use zorix_store::{SqliteWorkflowStore, database_path};
use zorix_workflows::service::WorkflowFile;
use zorix_workflows::{
    DEFAULT_WEBHOOK_ADDRESS, Decision, Health, StartRequest, Verdict, WorkflowService, Workflows, parse_address,
};

use crate::agents::open_hub;
use crate::app::{CliError, load_config, open_engine, secrets};
use crate::cli::{ModelArgs, WorkflowCommand};
use crate::follow::{Exit, Printer};
use crate::render::shorten;

fn print_json(value: &impl serde::Serialize) -> Result<(), CliError> {
    println!("{}", serde_json::to_string(value)?);
    Ok(())
}

fn when(at: Timestamp) -> String {
    at.to_rfc3339().chars().take(19).collect::<String>().replace('T', " ")
}

fn duration(ms: u64) -> String {
    let minutes = ms / 60_000;
    match minutes {
        0 => format!("{} s", ms / 1000),
        1..=59 => format!("{minutes} min"),
        _ if minutes < 60 * 48 => format!("{} h {} min", minutes / 60, minutes % 60),
        _ => format!("{} d", minutes / (60 * 24)),
    }
}

fn usd(micros: u64) -> String {
    format!("${:.2}", micros as f64 / 1_000_000.0)
}

fn service(home: &Path, tenant: &TenantId) -> Result<WorkflowService, CliError> {
    let hub = Arc::new(open_hub(home, tenant)?);
    let store = Arc::new(SqliteWorkflowStore::open(&database_path(home))?);
    Ok(WorkflowService::new(store, tenant.clone(), hub, Arc::new(SystemClock)))
}

fn bar(state: zorix_core::RunState) -> char {
    use zorix_core::RunState::*;
    match state {
        Completed => '▮',
        Failed => '✗',
        Waiting => '◔',
        Running => '▶',
        Cancelled => '–',
    }
}

fn health_text(health: Health) -> String {
    match health {
        Health::Paused => "paused".into(),
        Health::Waiting { count } => format!("{count} waiting"),
        Health::Failed { count } => format!("{count} failed"),
        Health::Healthy => "healthy".into(),
    }
}

pub async fn workflow(
    home: &Path,
    tenant: TenantId,
    printer: Printer,
    command: WorkflowCommand,
) -> Result<Exit, CliError> {
    match command {
        WorkflowCommand::Run(args) => return run(home, tenant, printer, args).await,
        WorkflowCommand::Answer(args) => return answer(home, tenant, printer, args).await,
        WorkflowCommand::Cancel(args) => return cancel(home, tenant, printer, &args.run).await,
        _ => {}
    }
    let service = service(home, &tenant)?;
    match command {
        WorkflowCommand::List => list(&service, printer).await,
        WorkflowCommand::Show(args) => show(&service, printer, &args.workflow, args.version).await,
        WorkflowCommand::Create(arg) => create(&service, printer, &arg.workflow).await,
        WorkflowCommand::Edit(arg) => edit(&service, printer, &arg.workflow).await,
        WorkflowCommand::Import(args) => import(&service, printer, &args.file).await,
        WorkflowCommand::Export(args) => export(&service, &args.workflow, args.version, args.out.as_deref()).await,
        WorkflowCommand::Versions(arg) => versions(&service, printer, &arg.workflow).await,
        WorkflowCommand::Diff(args) => diff(&service, printer, &args.workflow, &args.from, &args.to).await,
        WorkflowCommand::Publish(args) => {
            publish(&service, printer, &args.workflow, args.note, args.confirm_unattended).await
        }
        WorkflowCommand::Rollback(args) => {
            rollback(&service, printer, &args.workflow, args.version, args.confirm_unattended).await
        }
        WorkflowCommand::Enable(arg) => switch(&service, printer, &arg.workflow, true).await,
        WorkflowCommand::Disable(arg) => switch(&service, printer, &arg.workflow, false).await,
        WorkflowCommand::Remove(arg) => remove(&service, printer, &arg.workflow).await,
        WorkflowCommand::Runs(args) => runs(&service, printer, args.workflow.as_deref(), args.limit).await,
        WorkflowCommand::RunShow(arg) => run_show(&service, printer, &arg.run).await,
        WorkflowCommand::Waits => waits(&service, printer).await,
        WorkflowCommand::Webhook(args) => webhook(home, &service, printer, args).await,
        WorkflowCommand::Templates => templates(printer),
        WorkflowCommand::Add(arg) => add(&service, printer, &arg.template).await,
        WorkflowCommand::Run(_) | WorkflowCommand::Answer(_) | WorkflowCommand::Cancel(_) => {
            unreachable!("handled above")
        }
    }
}

async fn list(service: &WorkflowService, printer: Printer) -> Result<Exit, CliError> {
    let cards = service.cards().await?;
    if printer.json {
        print_json(&cards)?;
        return Ok(Exit::Done);
    }
    if cards.is_empty() {
        eprintln!(
            "No workflows yet. `zorix workflow templates` lists starters; `zorix workflow create NAME` makes an empty one."
        );
        return Ok(Exit::Done);
    }
    let figures = service.figures().await?;
    println!(
        "{} on, {} paused · {} runs this week{} · {} waiting for people{}",
        figures.active,
        figures.paused,
        figures.runs_week,
        figures.finished_percent.map(|p| format!(" ({p}% finished)")).unwrap_or_default(),
        figures.waiting,
        figures.saved_minutes.map(|m| format!(" · about {} h saved", m / 60)).unwrap_or_default(),
    );
    println!();
    for card in &cards {
        let triggers: Vec<&str> = card.triggers.iter().map(|t| t.label.as_str()).collect();
        let bars: String = card.bars.iter().map(|b| bar(b.state)).collect();
        let version = card.workflow.live_version.map(|v| format!("v{v}")).unwrap_or_else(|| "draft".into());
        let changes = if card.unpublished_changes && card.workflow.live_version.is_some() { " *" } else { "" };
        println!(
            "{:<22} {:<28} {:<14} {:<5}{changes:<2} {}",
            card.workflow.slug.to_string(),
            shorten(&triggers.join(", "), 28),
            bars,
            version,
            health_text(card.health)
        );
    }
    if cards.iter().any(|c| c.unpublished_changes && c.workflow.live_version.is_some()) {
        println!("\n* has changes that are not published yet");
    }
    Ok(Exit::Done)
}

async fn show(service: &WorkflowService, printer: Printer, key: &str, version: Option<u32>) -> Result<Exit, CliError> {
    let workflow = service.find(key).await?;
    let (spec, label) = match version {
        Some(v) => {
            let found = service.store().version(service.tenant(), workflow.id, v).await?.ok_or_else(|| {
                CliError(format!(
                    "{} has no version {v}. `zorix workflow versions {}` lists them.",
                    workflow.slug, workflow.slug
                ))
            })?;
            (found.spec, format!("version {v}"))
        }
        None => (workflow.draft.clone(), "draft".to_string()),
    };
    let missing = service.missing(&spec).await?;
    if printer.json {
        print_json(&json!({"workflow": workflow, "shown": label, "spec": spec, "missing": missing}))?;
        return Ok(Exit::Done);
    }
    println!(
        "{} · {} · {}{}",
        workflow.slug,
        label,
        if workflow.enabled { "on" } else { "off" },
        workflow.live_version.map(|v| format!(" · v{v} live")).unwrap_or_else(|| " · not published".into())
    );
    println!();
    print!("{}", WorkflowService::outline(&spec));
    if !missing.is_empty() {
        println!();
        for m in &missing {
            println!("Missing      step {}: {} “{}” is not set up here.", m.step, m.kind, m.name);
        }
    }
    Ok(Exit::Done)
}

async fn create(service: &WorkflowService, printer: Printer, key: &str) -> Result<Exit, CliError> {
    let slug = zorix_core::WorkflowSlug::new(key).map_err(CliError)?;
    let spec = WorkflowSpec::blank(key.replace('-', " "));
    let made = service.create(slug, spec).await?;
    if printer.json {
        print_json(&made)?;
    } else {
        println!("Made {} (a draft with a manual start). `zorix workflow edit {}` opens it.", made.slug, made.slug);
    }
    Ok(Exit::Done)
}

fn editor_command() -> Result<String, CliError> {
    ["VISUAL", "EDITOR"]
        .iter()
        .filter_map(|k| std::env::var(k).ok())
        .find(|v| !v.trim().is_empty())
        .ok_or_else(|| CliError("Set $VISUAL or $EDITOR to the editor you want, for example `EDITOR=nano`.".into()))
}

async fn edit(service: &WorkflowService, printer: Printer, key: &str) -> Result<Exit, CliError> {
    let editor = editor_command()?;
    let workflow = service.find(key).await?;
    let original = service.export(workflow.id, None).await?;
    let dir = tempfile::tempdir().map_err(|e| CliError(e.to_string()))?;
    let path = dir.path().join(format!("{}.workflow.json", workflow.slug));
    std::fs::write(&path, &original)?;
    let status = if cfg!(windows) {
        std::process::Command::new("cmd").arg("/C").arg(format!("{editor} \"{}\"", path.display())).status()
    } else {
        std::process::Command::new("sh").arg("-c").arg(format!("{editor} \"$1\"")).arg("sh").arg(&path).status()
    }
    .map_err(|e| CliError(format!("The editor could not be started: {e}")))?;
    if !status.success() {
        return Err(CliError("The editor did not finish normally, so nothing was saved.".into()));
    }
    let edited = std::fs::read_to_string(&path)?;
    let file = WorkflowFile::parse(&edited)?;
    if file.slug != workflow.slug {
        return Err(CliError(
            "The name of a workflow does not change. Import the file to make a copy under another name.".into(),
        ));
    }
    report_import(printer, &service.import(&edited).await?)
}

fn report_import(printer: Printer, imported: &zorix_workflows::Imported) -> Result<Exit, CliError> {
    if printer.json {
        print_json(imported)?;
    } else {
        let what = match (imported.created, imported.changed) {
            (true, _) => "Made",
            (false, true) => "Changed the draft of",
            (false, false) => "No change to",
        };
        println!(
            "{what} {}. It is a draft: `zorix workflow publish {}` makes it live.",
            imported.workflow.slug, imported.workflow.slug
        );
    }
    Ok(Exit::Done)
}

async fn import(service: &WorkflowService, printer: Printer, file: &Path) -> Result<Exit, CliError> {
    let text = std::fs::read_to_string(file).map_err(|e| CliError(format!("Cannot read {}: {e}", file.display())))?;
    report_import(printer, &service.import(&text).await?)
}

async fn export(
    service: &WorkflowService,
    key: &str,
    version: Option<u32>,
    out: Option<&Path>,
) -> Result<Exit, CliError> {
    let workflow = service.find(key).await?;
    let text = service.export(workflow.id, version).await?;
    match out {
        Some(path) => {
            std::fs::write(path, text).map_err(|e| CliError(format!("Cannot write {}: {e}", path.display())))?;
            eprintln!("Wrote {}.", path.display());
        }
        None => print!("{text}"),
    }
    Ok(Exit::Done)
}

async fn versions(service: &WorkflowService, printer: Printer, key: &str) -> Result<Exit, CliError> {
    let workflow = service.find(key).await?;
    let versions = service.store().versions(service.tenant(), workflow.id).await?;
    if printer.json {
        print_json(&versions)?;
        return Ok(Exit::Done);
    }
    if versions.is_empty() {
        eprintln!("{} has not been published yet.", workflow.slug);
    }
    for v in &versions {
        let live = if Some(v.version) == workflow.live_version { "live" } else { "" };
        let confirmed = if v.confirmations.is_empty() {
            String::new()
        } else {
            format!("  ({} unattended confirmed)", v.confirmations.len())
        };
        println!(
            "v{:<4} {}  {}  {:<5} {}{confirmed}",
            v.version,
            &v.hash[..8],
            when(v.created_at),
            live,
            v.note.as_deref().unwrap_or("")
        );
    }
    Ok(Exit::Done)
}

async fn spec_of(service: &WorkflowService, workflow: &Workflow, which: &str) -> Result<WorkflowSpec, CliError> {
    if which == "draft" {
        return Ok(workflow.draft.clone());
    }
    let number: u32 = which
        .trim_start_matches('v')
        .parse()
        .map_err(|_| CliError(format!("“{which}” is not a version: use a number or `draft`.")))?;
    Ok(service
        .store()
        .version(service.tenant(), workflow.id, number)
        .await?
        .ok_or_else(|| CliError(format!("{} has no version {number}.", workflow.slug)))?
        .spec)
}

async fn diff(service: &WorkflowService, printer: Printer, key: &str, from: &str, to: &str) -> Result<Exit, CliError> {
    let workflow = service.find(key).await?;
    let lines =
        WorkflowService::diff(&spec_of(service, &workflow, from).await?, &spec_of(service, &workflow, to).await?);
    if printer.json {
        print_json(&lines)?;
        return Ok(Exit::Done);
    }
    let changed = lines.iter().filter(|l| l.kind != "same").count();
    if changed == 0 {
        println!("No difference between {from} and {to}.");
        return Ok(Exit::Done);
    }
    for line in &lines {
        match line.kind {
            "added" => println!("+ {}", line.text),
            "removed" => println!("- {}", line.text),
            _ => println!("  {}", line.text),
        }
    }
    Ok(Exit::Done)
}

fn confirmation(needed: &[String], confirm: bool) -> Result<Option<Confirm>, CliError> {
    if needed.is_empty() {
        return Ok(None);
    }
    if !confirm {
        return Err(CliError(format!(
            "These steps send something without asking first (or remind in chat): {}.\nIf that is what you want, run the command again with --confirm-unattended. The version records that you did.",
            needed.join(", ")
        )));
    }
    Ok(Some(Confirm {
        by: std::env::var("USER").or_else(|_| std::env::var("USERNAME")).unwrap_or_else(|_| "cli".into()),
    }))
}

async fn publish(
    service: &WorkflowService,
    printer: Printer,
    key: &str,
    note: Option<String>,
    confirm: bool,
) -> Result<Exit, CliError> {
    let workflow = service.find(key).await?;
    let confirm = confirmation(&service.to_confirm(workflow.id).await?, confirm)?;
    let (version, written) = service.publish(workflow.id, note, confirm).await?;
    if printer.json {
        print_json(&json!({"version": version.version, "written": written, "hash": version.hash}))?;
    } else if written {
        println!(
            "Published {} as v{}. Turn it on with `zorix workflow enable {}`.",
            workflow.slug, version.version, workflow.slug
        );
        let missing = service.missing(&version.spec).await?;
        for m in &missing {
            println!("Note         step {}: {} “{}” is not set up here yet.", m.step, m.kind, m.name);
        }
    } else {
        println!("The draft is the same as v{}: nothing to publish.", version.version);
    }
    Ok(Exit::Done)
}

async fn rollback(
    service: &WorkflowService,
    printer: Printer,
    key: &str,
    to: u32,
    confirm: bool,
) -> Result<Exit, CliError> {
    let workflow = service.find(key).await?;
    let old = service
        .store()
        .version(service.tenant(), workflow.id, to)
        .await?
        .ok_or_else(|| CliError(format!("{} has no version {to}.", workflow.slug)))?;
    let needed: Vec<String> = old
        .spec
        .unattended()
        .into_iter()
        .filter(|s| !old.confirmations.iter().any(|c| c.step == s.id && c.hash == s.hash()))
        .map(|s| s.id.clone())
        .collect();
    let version = service.rollback(workflow.id, to, confirmation(&needed, confirm)?).await?;
    if printer.json {
        print_json(&json!({"version": version.version}))?;
    } else {
        println!("Published v{} (a copy of v{to}).", version.version);
    }
    Ok(Exit::Done)
}

async fn switch(service: &WorkflowService, printer: Printer, key: &str, on: bool) -> Result<Exit, CliError> {
    let workflow = service.find(key).await?;
    service.set_enabled(workflow.id, on).await?;
    if printer.json {
        print_json(&json!({"workflow": workflow.slug, "enabled": on}))?;
    } else {
        println!(
            "{} is {}. Schedules and webhooks run while ZORIX is open, or under `zorix serve`.",
            workflow.slug,
            if on { "on" } else { "off" }
        );
    }
    Ok(Exit::Done)
}

async fn remove(service: &WorkflowService, printer: Printer, key: &str) -> Result<Exit, CliError> {
    let workflow = service.find(key).await?;
    service.remove(workflow.id).await?;
    if printer.json {
        print_json(&json!({"removed": workflow.slug}))?;
    } else {
        println!("Removed {}. Its runs are kept.", workflow.slug);
    }
    Ok(Exit::Done)
}

async fn runs(service: &WorkflowService, printer: Printer, key: Option<&str>, limit: usize) -> Result<Exit, CliError> {
    let id = match key {
        Some(key) => Some(service.find(key).await?.id),
        None => None,
    };
    let runs = service.store().runs(service.tenant(), id, limit).await?;
    if printer.json {
        print_json(&runs)?;
        return Ok(Exit::Done);
    }
    if runs.is_empty() {
        eprintln!("No runs yet. `zorix workflow run NAME` starts one.");
    }
    for r in &runs {
        println!(
            "{}  {}  {:<18} {:<10} {:>6}  {}{}",
            r.id,
            when(r.started_at),
            r.slug,
            r.state.as_str(),
            usd(r.cost_micros),
            shorten(&r.fired.label, 24),
            if r.test { "  (test)" } else { "" }
        );
    }
    Ok(Exit::Done)
}

fn state_mark(state: StepState) -> &'static str {
    match state {
        StepState::Pending => "·",
        StepState::Running => "▶",
        StepState::Waiting => "◔",
        StepState::Done => "✓",
        StepState::Failed => "✗",
        StepState::Skipped => "–",
    }
}

fn print_run(run: &WorkflowRun) {
    println!(
        "Run {} of {}{} · {} · started {}{} · {}",
        run.id,
        run.slug,
        run.version.map(|v| format!(" v{v}")).unwrap_or_else(|| " (draft)".into()),
        run.state.as_str(),
        when(run.started_at),
        if run.test { " · test" } else { "" },
        usd(run.cost_micros)
    );
    println!("Started by   {}", run.fired.label);
    if let Some(error) = &run.error {
        println!("Failed       {error}");
    }
    println!();
    for step in &run.spec.steps {
        let record = &run.steps[&step.id];
        let note = record.note.as_deref().or(record.error.as_deref()).unwrap_or("");
        println!("{} {:<14} {:<28} {}", state_mark(record.state), step.id, shorten(&step.title, 28), note);
    }
}

async fn run_show(service: &WorkflowService, printer: Printer, id: &str) -> Result<Exit, CliError> {
    let run_id: WorkflowRunId = id.parse().map_err(|_| CliError(format!("“{id}” is not a run id.")))?;
    let run = service
        .store()
        .run(service.tenant(), run_id)
        .await?
        .ok_or_else(|| CliError(format!("There is no run {id}.")))?;
    if printer.json {
        print_json(&json!({"run": run, "waits": service.waits_of(run_id).await?}))?;
    } else {
        print_run(&run);
        for wait in service.waits_of(run_id).await?.iter().filter(|w| w.status == WaitStatus::Pending) {
            println!("\nWaiting      {} — `zorix workflow answer {} --approve`", wait.title, wait.id);
        }
    }
    Ok(match run.state {
        zorix_core::RunState::Completed => Exit::Done,
        zorix_core::RunState::Failed => Exit::Failed,
        zorix_core::RunState::Cancelled => Exit::Cancelled,
        _ => Exit::Waiting,
    })
}

async fn waits(service: &WorkflowService, printer: Printer) -> Result<Exit, CliError> {
    let waits = service.store().pending_waits(service.tenant()).await?;
    if printer.json {
        print_json(&waits)?;
        return Ok(Exit::Done);
    }
    if waits.is_empty() {
        eprintln!("Nothing is waiting for a person.");
        return Ok(Exit::Done);
    }
    for w in &waits {
        let what = match w.kind {
            WaitKind::Approval => "approval",
            WaitKind::Question => "question",
            WaitKind::Action => "action",
            WaitKind::Timer => "timer",
        };
        println!("{}  {:<9} {}  — {}", w.id, what, shorten(&w.title, 24), shorten(&w.message, 60));
        println!(
            "          run {}  · waiting {}{}",
            w.run,
            duration(zorix_core::Clock::now(&SystemClock).since(w.created_at)),
            w.due_at.map(|d| format!(" · times out {}", when(d))).unwrap_or_default()
        );
    }
    println!("\n`zorix workflow answer ID --approve`, `--refuse`, or `--text \"…\"` for a question.");
    Ok(Exit::Waiting)
}

async fn webhook(
    home: &Path,
    service: &WorkflowService,
    printer: Printer,
    args: crate::cli::WorkflowWebhookArgs,
) -> Result<Exit, CliError> {
    let workflow = service.find(&args.workflow).await?;
    if !workflow.draft.triggers().any(|t| matches!(t.kind, zorix_core::StepKind::Webhook { .. })) {
        return Err(CliError(format!("{} has no webhook trigger.", workflow.slug)));
    }
    let store = secrets(&load_config(home)?);
    let token = if args.reset {
        zorix_workflows::reset_webhook_token(&store, workflow.id).await?
    } else {
        zorix_workflows::webhook_token(&store, workflow.id).await?
    };
    let signed =
        workflow.draft.triggers().any(|t| matches!(t.kind, zorix_core::StepKind::Webhook { signed: true, .. }));
    let signing = if signed { Some(zorix_workflows::signing_secret(&store, workflow.id).await?) } else { None };
    let address = parse_address(args.bind.as_deref().unwrap_or(DEFAULT_WEBHOOK_ADDRESS))?;
    let url = format!("http://{address}/hooks/{}/{token}", workflow.slug);
    if printer.json {
        print_json(&json!({"url": url, "signing_secret": signing}))?;
    } else {
        println!("Address      {url}");
        if let Some(secret) = signing {
            println!("Signing      {secret}  (the sender signs the body with HMAC-SHA256 into X-Hub-Signature-256)");
        }
        println!("\nKeep the address private: anyone who has it can start this workflow. `--reset` replaces it.");
        println!("It answers while ZORIX is open with webhooks turned on, or under `zorix serve --webhooks`.");
    }
    Ok(Exit::Done)
}

fn templates(printer: Printer) -> Result<Exit, CliError> {
    let all = WorkflowService::templates();
    if printer.json {
        print_json(&all)?;
        return Ok(Exit::Done);
    }
    for t in &all {
        println!("{:<18} {:<24} {} steps · {}", t.id, shorten(&t.triggers.join(", "), 24), t.steps, t.description);
    }
    println!("\n`zorix workflow add NAME` makes one as a draft that is off.");
    Ok(Exit::Done)
}

async fn add(service: &WorkflowService, printer: Printer, template: &str) -> Result<Exit, CliError> {
    let made = service.add_template(template).await?;
    let missing = service.missing(&made.draft).await?;
    if printer.json {
        print_json(&json!({"workflow": made, "missing": missing}))?;
        return Ok(Exit::Done);
    }
    println!("Added {} as a draft that is off.", made.slug);
    for m in &missing {
        println!("Needs        step {}: {} “{}”", m.step, m.kind, m.name);
    }
    println!("Next         zorix workflow show {}  ·  zorix workflow publish {}", made.slug, made.slug);
    Ok(Exit::Done)
}

// ---------------------------------------------------------------------------------------------------------------------
// With the engine

async fn open(home: &Path, tenant: &TenantId, model: &ModelArgs) -> Result<(crate::app::App, Workflows), CliError> {
    let app = open_engine(home, tenant, model)?;
    let workflows = Workflows::open(home, tenant.clone(), app.hub.clone(), app.engine.clone(), app.router.clone())?;
    Ok((app, workflows))
}

/// Move a run along until it ends or only a person (or a timer longer than a few seconds) can move it.
async fn follow(workflows: &Workflows, printer: Printer, mut run: WorkflowRun) -> Result<Exit, CliError> {
    let mut seen: std::collections::BTreeMap<String, StepState> = std::collections::BTreeMap::new();
    loop {
        for step in &run.spec.steps {
            let record = &run.steps[&step.id];
            if seen.get(&step.id) != Some(&record.state) && record.state != StepState::Pending {
                seen.insert(step.id.clone(), record.state);
                if !printer.json {
                    println!(
                        "{} {:<14} {}",
                        state_mark(record.state),
                        step.id,
                        record.note.as_deref().or(record.error.as_deref()).unwrap_or("")
                    );
                }
            }
        }
        if run.state.is_final() {
            break;
        }
        let waiting_on_people = run.state == zorix_core::RunState::Waiting
            && run.steps.values().all(|s| s.state != StepState::Running)
            && workflows
                .store
                .waits_of(workflows.engine.tenant(), run.id)
                .await?
                .iter()
                .any(|w| w.status == WaitStatus::Pending && w.kind != WaitKind::Timer);
        if waiting_on_people {
            break;
        }
        tokio::time::sleep(Duration::from_secs(1)).await;
        run = workflows.engine.advance(run.id).await?;
    }
    if printer.json {
        print_json(&run)?;
    } else {
        println!();
        println!("{} · {} · {}", run.id, run.state.as_str(), usd(run.cost_micros));
        if let Some(error) = &run.error {
            println!("Failed       {error}");
        }
        for wait in workflows
            .store
            .waits_of(workflows.engine.tenant(), run.id)
            .await?
            .iter()
            .filter(|w| w.status == WaitStatus::Pending)
        {
            println!(
                "Waiting      {} — {}\n             zorix workflow answer {} --approve   (or --refuse, or --text)",
                wait.title,
                shorten(&wait.message, 70),
                wait.id
            );
        }
    }
    Ok(match run.state {
        zorix_core::RunState::Completed => Exit::Done,
        zorix_core::RunState::Failed => Exit::Failed,
        zorix_core::RunState::Cancelled => Exit::Cancelled,
        _ => Exit::Waiting,
    })
}

async fn run(
    home: &Path,
    tenant: TenantId,
    printer: Printer,
    args: crate::cli::WorkflowRunArgs,
) -> Result<Exit, CliError> {
    let payload: Value = match &args.input {
        Some(text) => serde_json::from_str(text).map_err(|e| CliError(format!("--input is not JSON: {e}")))?,
        None => json!({}),
    };
    let (app, workflows) = open(home, &tenant, &args.model).await?;
    let service = WorkflowService::new(workflows.store.clone(), tenant.clone(), app.hub.clone(), Arc::new(SystemClock));
    let workflow = service.find(&args.workflow).await?;
    let result = async {
        let mut request = StartRequest::manual(workflow.id, payload);
        request.test = args.test;
        request.trigger = args.trigger.clone();
        let run = workflows.engine.start(request).await?;
        follow(&workflows, printer, run).await
    }
    .await;
    app.shutdown().await;
    result
}

async fn answer(
    home: &Path,
    tenant: TenantId,
    printer: Printer,
    args: crate::cli::WorkflowAnswerArgs,
) -> Result<Exit, CliError> {
    let wait_id: WaitId = args
        .wait
        .parse()
        .map_err(|_| CliError(format!("“{}” is not a wait id. `zorix workflow waits` lists them.", args.wait)))?;
    let verdict = match (args.approve, args.refuse, &args.text) {
        (true, false, None) => Verdict::Approve,
        (false, true, None) => Verdict::Refuse,
        (false, false, Some(text)) => Verdict::Answer(text.clone()),
        _ => return Err(CliError("Say how to answer: --approve, --refuse, or --text \"…\" for a question.".into())),
    };
    let (app, workflows) = open(home, &tenant, &args.model).await?;
    let result = async {
        let wait = workflows
            .store
            .wait(&tenant, wait_id)
            .await?
            .ok_or_else(|| CliError(format!("There is no wait {wait_id}.")))?;
        if !printer.json {
            println!("{} — {}", wait.title, wait.message);
            if let Value::Object(shown) = &wait.shown {
                for (key, value) in shown {
                    println!("  {key}: {}", shorten(&value.to_string(), 100));
                }
            }
        }
        let by = std::env::var("USER").or_else(|_| std::env::var("USERNAME")).unwrap_or_else(|_| "cli".into());
        let fingerprint = args.fingerprint.clone().unwrap_or_else(|| wait.fingerprint.clone());
        let settled =
            workflows.engine.decide(wait_id, Decision { fingerprint, verdict, by, note: args.note.clone() }).await?;
        let run =
            workflows.store.run(&tenant, settled.run).await?.ok_or_else(|| CliError("The run was lost.".into()))?;
        follow(&workflows, printer, run).await
    }
    .await;
    app.shutdown().await;
    result
}

async fn cancel(home: &Path, tenant: TenantId, printer: Printer, id: &str) -> Result<Exit, CliError> {
    let run_id: WorkflowRunId = id.parse().map_err(|_| CliError(format!("“{id}” is not a run id.")))?;
    let (app, workflows) = open(home, &tenant, &ModelArgs::default_demo()).await?;
    let by = std::env::var("USER").or_else(|_| std::env::var("USERNAME")).unwrap_or_else(|_| "cli".into());
    let run = workflows.engine.cancel(run_id, &by).await;
    app.shutdown().await;
    let run = run?;
    if printer.json {
        print_json(&run)?;
    } else {
        println!("Cancelled {}.", run.id);
    }
    Ok(Exit::Cancelled)
}
