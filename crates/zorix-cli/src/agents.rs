//! `zorix agent`, `zorix skill` and `zorix eval`: what a person does with agents from the terminal (ADR-0024 to ADR-0028).

use std::io::Read;
use std::path::Path;
use std::sync::Arc;

use serde_json::json;
use tokio_util::sync::CancellationToken;
use zorix_agents::diff::Change as Line;
use zorix_agents::evals::{Change, EvalOptions, Evaluator, compare};
use zorix_agents::{AgentCard, format};
use zorix_core::{
    AgentSpec, AgentStore, EvalCase, EvalRun, EvalRunId, ExecutionEngine, ModelGateway, TenantId, Timestamp,
};
use zorix_hub::Hub;
use zorix_hub::agents::Imported;

use crate::app::{CliError, builtin_registry, load_config, open_engine, secrets};
use crate::cli::{
    AgentAddArgs, AgentArchiveArgs, AgentCommand, AgentCreateArgs, AgentDiffArgs, AgentExportArgs, AgentImportArgs,
    AgentListArgs, AgentRemoveArgs, AgentRollbackArgs, AgentShowArgs, AgentSlugArg, EvalAddArgs, EvalCommand,
    EvalCompareArgs, EvalRemoveArgs, EvalRunArg, EvalRunArgs, ModelArgs, SkillCommand, SkillImportArgs, SkillNameArg,
};
use crate::follow::{Exit, Printer};
use crate::render::shorten;

/// The hub without an engine: no data-folder lock, so these commands work while the desktop app is open.
pub fn open_hub(home: &Path, tenant: &TenantId) -> Result<Hub, CliError> {
    let registry = builtin_registry(home)?;
    Ok(Hub::open(home, tenant.clone(), secrets(&load_config(home)?), registry)?)
}

fn print_json(value: &impl serde::Serialize) -> Result<(), CliError> {
    println!("{}", serde_json::to_string(value)?);
    Ok(())
}

fn short(hash: &str) -> &str {
    &hash[..hash.len().min(8)]
}

fn when(at: Timestamp) -> String {
    at.to_rfc3339().chars().take(19).collect::<String>().replace('T', " ")
}

fn tenant_of(hub: &Hub) -> TenantId {
    hub.tenant().clone()
}

// ---------------------------------------------------------------------------------------------------------------------
// zorix agent

pub async fn agent(home: &Path, tenant: &TenantId, printer: Printer, command: AgentCommand) -> Result<Exit, CliError> {
    let hub = open_hub(home, tenant)?;
    match command {
        AgentCommand::List(args) => list(&hub, printer, args).await,
        AgentCommand::Show(args) => show(&hub, printer, args).await,
        AgentCommand::Create(args) => create(&hub, printer, args).await,
        AgentCommand::Edit(arg) => edit(&hub, printer, arg).await,
        AgentCommand::Import(args) => import(&hub, printer, args).await,
        AgentCommand::Export(args) => export(&hub, args).await,
        AgentCommand::Versions(arg) => versions(&hub, printer, arg).await,
        AgentCommand::Diff(args) => diff(&hub, printer, args).await,
        AgentCommand::Rollback(args) => rollback(&hub, printer, args).await,
        AgentCommand::Archive(args) => archive(&hub, printer, args).await,
        AgentCommand::Remove(args) => remove(&hub, printer, args).await,
        AgentCommand::Add(args) => add(&hub, printer, args).await,
        AgentCommand::Templates => templates(&hub, printer).await,
    }
}

async fn list(hub: &Hub, printer: Printer, args: AgentListArgs) -> Result<Exit, CliError> {
    let cards: Vec<AgentCard> = hub.agent_cards(args.all).await?;
    if printer.json {
        print_json(&cards)?;
    } else if cards.is_empty() {
        eprintln!(
            "No agents yet. `zorix agent templates` lists ready-made ones; `zorix agent add --template NAME` adds one."
        );
    } else {
        for c in &cards {
            let archived = if c.archived { "  (archived)" } else { "" };
            println!(
                "{:<20} v{:<3} {:<14} {}{archived}",
                c.slug,
                c.version,
                shorten(&c.default_model, 14),
                shorten(&c.description, 60)
            );
        }
    }
    Ok(Exit::Done)
}

fn describe(spec: &AgentSpec, version: u32, hash: &str) {
    println!("Agent        {} {}  (v{version}, {})", spec.avatar.emoji, spec.slug, short(hash));
    println!("Name         {}", spec.name);
    println!("About        {}", spec.description);
    println!(
        "Model        {}{}",
        spec.models.default,
        if spec.models.allowed.is_empty() {
            String::new()
        } else {
            format!("  (allowed: {})", spec.models.allowed.join(", "))
        }
    );
    let list = |label: &str, items: &[String]| {
        if !items.is_empty() {
            println!("{label:<12} {}", items.join(", "));
        }
    };
    list("Tools", &spec.tools);
    list("Access", &spec.access);
    let skills: Vec<String> = spec.skills.iter().map(|s| format!("{}@{}", s.name, short(&s.hash))).collect();
    list("Skills", &skills);
    println!("Own memory   {}", if spec.agent_memory { "yes" } else { "no" });
    println!("\n{}", spec.instructions);
}

async fn show(hub: &Hub, printer: Printer, args: AgentShowArgs) -> Result<Exit, CliError> {
    let (agent, version) = hub.agent_version(&args.agent, args.version).await?;
    if printer.json {
        print_json(&json!({"agent": agent, "version": version}))?;
    } else {
        describe(&version.spec, version.version, &version.hash);
    }
    Ok(Exit::Done)
}

fn report(printer: Printer, what: &str, imported: &Imported) -> Result<(), CliError> {
    if printer.json {
        return print_json(&json!({
            "agent": imported.agent.slug.as_str(), "version": imported.version.version,
            "created": imported.created, "changed": imported.changed
        }));
    }
    let slug = imported.agent.slug.as_str();
    if imported.created {
        eprintln!("{what} {slug} (v{}).", imported.version.version);
    } else if imported.changed {
        eprintln!("{slug} is now at v{}.", imported.version.version);
    } else {
        eprintln!("{slug} is unchanged (v{}).", imported.version.version);
    }
    Ok(())
}

async fn create(hub: &Hub, printer: Printer, args: AgentCreateArgs) -> Result<Exit, CliError> {
    if hub.agent(&args.agent).await.is_ok() {
        return Err(CliError(format!(
            "There is already an agent called {}. `zorix agent edit {}` changes it.",
            args.agent, args.agent
        )));
    }
    let imported = if let Some(file) = &args.from_file {
        let text =
            std::fs::read_to_string(file).map_err(|e| CliError(format!("{} cannot be read: {e}", file.display())))?;
        hub.import_agent(&text, Some(&args.agent), Some("created from a file".into())).await?
    } else {
        let instructions = args
            .instructions
            .clone()
            .ok_or_else(|| CliError("Give --instructions \"…\" or --from-file FILE.md.".into()))?;
        let slug = zorix_core::AgentSlug::new(&args.agent).map_err(CliError)?;
        let mut spec =
            AgentSpec::new(slug, args.name.clone().unwrap_or_else(|| args.agent.clone()), args.model.clone());
        spec.description = args.description.clone().unwrap_or_default();
        spec.instructions = instructions;
        spec.tools = args.tools.clone();
        spec.access = args.access.clone();
        for name in &args.skills {
            let record = hub.skill(name).await?;
            spec.skills.push(zorix_core::SkillPin { name: record.name, hash: record.hash });
        }
        hub.save_spec(spec, Some("created".into())).await?
    };
    report(printer, "Made", &imported)?;
    Ok(Exit::Done)
}

fn editor_command() -> Result<String, CliError> {
    ["VISUAL", "EDITOR"]
        .iter()
        .filter_map(|k| std::env::var(k).ok())
        .find(|v| !v.trim().is_empty())
        .ok_or_else(|| CliError("Set $VISUAL or $EDITOR to the editor you want, for example `EDITOR=nano`.".into()))
}

async fn edit(hub: &Hub, printer: Printer, arg: AgentSlugArg) -> Result<Exit, CliError> {
    let editor = editor_command()?;
    let original = hub.export_agent(&arg.agent, None).await?;
    let dir = tempfile::tempdir().map_err(|e| CliError(e.to_string()))?;
    let path = dir.path().join(format!("{}.md", arg.agent));
    std::fs::write(&path, &original).map_err(|e| CliError(e.to_string()))?;
    let status = if cfg!(windows) {
        std::process::Command::new("cmd").arg("/C").arg(format!("{editor} \"{}\"", path.display())).status()
    } else {
        std::process::Command::new("sh").arg("-c").arg(format!("{editor} \"$1\"")).arg("sh").arg(&path).status()
    }
    .map_err(|e| CliError(format!("The editor could not be started: {e}")))?;
    if !status.success() {
        return Err(CliError("The editor did not finish normally, so nothing was saved.".into()));
    }
    let edited = std::fs::read_to_string(&path).map_err(|e| CliError(e.to_string()))?;
    let imported = hub.import_agent(&edited, Some(&arg.agent), Some("edited".into())).await?;
    report(printer, "Made", &imported)?;
    Ok(Exit::Done)
}

async fn import(hub: &Hub, printer: Printer, args: AgentImportArgs) -> Result<Exit, CliError> {
    let text = std::fs::read_to_string(&args.file)
        .map_err(|e| CliError(format!("{} cannot be read: {e}", args.file.display())))?;
    if let Some(dir) = &args.skills {
        let parsed = format::parse(&text).map_err(CliError)?;
        for skill in &parsed.skills {
            let folder = dir.join(&skill.name);
            if folder.is_dir() {
                let record = hub.import_skill(&folder).await?;
                eprintln!("Skill {} imported ({}).", record.name, short(&record.hash));
            }
        }
    }
    let imported = hub.import_agent(&text, args.rename.as_deref(), args.note.clone()).await?;
    report(printer, "Imported", &imported)?;
    Ok(Exit::Done)
}

async fn export(hub: &Hub, args: AgentExportArgs) -> Result<Exit, CliError> {
    let text = hub.export_agent(&args.agent, args.version).await?;
    match &args.output {
        Some(path) => {
            std::fs::write(path, &text).map_err(|e| CliError(format!("{} cannot be written: {e}", path.display())))?;
            eprintln!("Wrote {}.", path.display());
        }
        None => print!("{text}"),
    }
    if let Some(dir) = &args.with_skills {
        let written = hub.export_skills(&args.agent, args.version, dir).await?;
        eprintln!(
            "Skills written to {}: {}.",
            dir.display(),
            if written.is_empty() { "none".into() } else { written.join(", ") }
        );
    }
    Ok(Exit::Done)
}

async fn versions(hub: &Hub, printer: Printer, arg: AgentSlugArg) -> Result<Exit, CliError> {
    let (agent, _) = hub.agent(&arg.agent).await?;
    let versions = hub.agents.versions(&tenant_of(hub), agent.id).await?;
    if printer.json {
        print_json(&versions)?;
    } else {
        for v in &versions {
            let current = if v.version == agent.current_version { "  ← current" } else { "" };
            println!(
                "v{:<3} {}  {}  {}{current}",
                v.version,
                short(&v.hash),
                when(v.created_at),
                v.note.as_deref().unwrap_or("")
            );
        }
    }
    Ok(Exit::Done)
}

async fn diff(hub: &Hub, printer: Printer, args: AgentDiffArgs) -> Result<Exit, CliError> {
    let lines = hub.diff_agent(&args.agent, args.from, args.to).await?;
    if printer.json {
        let rows: Vec<_> = lines
            .iter()
            .map(|l| {
                let change = match l.kind {
                    Line::Same => "same",
                    Line::Added => "added",
                    Line::Removed => "removed",
                };
                json!({"change": change, "text": l.text})
            })
            .collect();
        print_json(&rows)?;
        return Ok(Exit::Done);
    }
    if lines.iter().all(|l| l.kind == Line::Same) {
        eprintln!("v{} and v{} are the same.", args.from, args.to);
    }
    for l in lines.iter().filter(|l| l.kind != Line::Same) {
        println!("{} {}", if l.kind == Line::Added { '+' } else { '-' }, l.text);
    }
    Ok(Exit::Done)
}

async fn rollback(hub: &Hub, printer: Printer, args: AgentRollbackArgs) -> Result<Exit, CliError> {
    let version = hub.rollback_agent(&args.agent, args.version).await?;
    if printer.json {
        print_json(&json!({"agent": args.agent, "version": version.version}))?;
    } else {
        eprintln!("{} is now at v{}, a copy of v{}.", args.agent, version.version, args.version);
    }
    Ok(Exit::Done)
}

async fn archive(hub: &Hub, printer: Printer, args: AgentArchiveArgs) -> Result<Exit, CliError> {
    let agent = hub.archive_agent(&args.agent, !args.undo).await?;
    if printer.json {
        print_json(&json!({"agent": args.agent, "archived": agent.archived}))?;
    } else {
        eprintln!("{} is {}.", args.agent, if agent.archived { "archived" } else { "shown again" });
    }
    Ok(Exit::Done)
}

async fn remove(hub: &Hub, printer: Printer, args: AgentRemoveArgs) -> Result<Exit, CliError> {
    hub.agent(&args.agent).await?;
    if !args.yes {
        return Err(CliError(format!(
            "This deletes {} with all its versions and evaluation cases, and cannot be undone. Add --yes to do it (or `zorix agent archive {}` to only hide it).",
            args.agent, args.agent
        )));
    }
    hub.remove_agent(&args.agent).await?;
    if printer.json {
        print_json(&json!({"agent": args.agent, "removed": true}))?;
    } else {
        eprintln!("{} was deleted.", args.agent);
    }
    Ok(Exit::Done)
}

async fn add(hub: &Hub, printer: Printer, args: AgentAddArgs) -> Result<Exit, CliError> {
    let done = hub.add_template(&args.template, args.rename.as_deref()).await?;
    if printer.json {
        print_json(&json!({
            "agent": done.imported.agent.slug.as_str(), "version": done.imported.version.version,
            "created": done.imported.created, "skills": done.skills.iter().map(|s| &s.name).collect::<Vec<_>>(),
            "cases": done.cases
        }))?;
    } else {
        report(printer, "Added", &done.imported)?;
        let names: Vec<&str> = done.skills.iter().map(|s| s.name.as_str()).collect();
        eprintln!(
            "Skills: {}. Evaluation cases: {}. Try it: zorix run --agent {} \"…\"",
            names.join(", "),
            done.cases,
            done.imported.agent.slug
        );
    }
    Ok(Exit::Done)
}

async fn templates(hub: &Hub, printer: Printer) -> Result<Exit, CliError> {
    let all = hub.templates().await?;
    if printer.json {
        let rows: Vec<_> = all
            .iter()
            .map(|t| json!({"template": t.id, "name": t.name, "description": t.description, "skills": t.skills, "cases": t.cases, "installed": t.installed}))
            .collect();
        print_json(&rows)?;
    } else {
        for t in &all {
            println!(
                "{:<14} {} {:<14} {}{}",
                t.id,
                t.emoji,
                t.name,
                shorten(&t.description, 70),
                if t.installed { "  (added)" } else { "" }
            );
        }
    }
    Ok(Exit::Done)
}

// ---------------------------------------------------------------------------------------------------------------------
// zorix skill

pub async fn skill(home: &Path, tenant: &TenantId, printer: Printer, command: SkillCommand) -> Result<Exit, CliError> {
    let hub = open_hub(home, tenant)?;
    match command {
        SkillCommand::List => {
            let skills = hub.skills_list().await?;
            if printer.json {
                print_json(&skills)?;
            } else if skills.is_empty() {
                eprintln!("No skills yet. `zorix skill import FOLDER` adds one.");
            } else {
                for s in &skills {
                    println!(
                        "{:<24} {}  {} file(s)  {}",
                        s.name,
                        short(&s.hash),
                        s.files.len(),
                        shorten(&s.description, 56)
                    );
                }
            }
        }
        SkillCommand::Show(SkillNameArg { name }) => {
            let s = hub.skill(&name).await?;
            if printer.json {
                print_json(&s)?;
            } else {
                println!("Skill        {} ({})", s.name, short(&s.hash));
                println!("About        {}", s.description);
                if let Some(v) = &s.license {
                    println!("License      {v}");
                }
                if let Some(v) = &s.compatibility {
                    println!("Works with   {v}");
                }
                if let Some(v) = &s.source {
                    println!("From         {v}");
                }
                if !s.allowed_tools.is_empty() {
                    println!("Asks for     {}  (shown only: a skill never grants anything)", s.allowed_tools.join(" "));
                }
                for f in &s.files {
                    println!("File         {:<40} {} bytes", f.path, f.size);
                }
            }
        }
        SkillCommand::Import(SkillImportArgs { path }) => {
            let s = hub.import_skill(&path).await?;
            if printer.json {
                print_json(&s)?;
            } else {
                eprintln!("Skill {} imported ({}, {} files).", s.name, short(&s.hash), s.files.len());
            }
        }
        SkillCommand::Remove(SkillNameArg { name }) => {
            hub.remove_skill(&name).await?;
            eprintln!("Skill {name} was forgotten. Its files stay until `zorix skill gc`.");
        }
        SkillCommand::Gc => {
            let report = hub.clean_skills().await?;
            if printer.json {
                print_json(&json!({"removed": report.removed, "bytes": report.bytes}))?;
            } else {
                eprintln!("{} skill folder(s) deleted, {} bytes freed.", report.removed, report.bytes);
            }
        }
    }
    Ok(Exit::Done)
}

// ---------------------------------------------------------------------------------------------------------------------
// zorix eval

pub async fn eval(home: &Path, tenant: &TenantId, printer: Printer, command: EvalCommand) -> Result<Exit, CliError> {
    if let EvalCommand::Run(args) = command {
        return eval_run(home, tenant, printer, args).await;
    }
    let hub = open_hub(home, tenant)?;
    match command {
        EvalCommand::Add(args) => eval_add(&hub, printer, args).await,
        EvalCommand::List(arg) => eval_list(&hub, printer, arg).await,
        EvalCommand::Remove(args) => eval_remove(&hub, printer, args).await,
        EvalCommand::Runs(arg) => eval_runs(&hub, printer, arg).await,
        EvalCommand::Show(arg) => eval_show(&hub, printer, arg).await,
        EvalCommand::Compare(args) => eval_compare(&hub, printer, args).await,
        EvalCommand::Run(_) => unreachable!("handled above"),
    }
}

fn read_cases(file: &Path) -> Result<Vec<EvalCase>, CliError> {
    let mut text = String::new();
    if file == Path::new("-") {
        std::io::stdin().read_to_string(&mut text).map_err(|e| CliError(e.to_string()))?;
    } else {
        text =
            std::fs::read_to_string(file).map_err(|e| CliError(format!("{} cannot be read: {e}", file.display())))?;
    }
    let value: serde_json::Value =
        serde_json::from_str(&text).map_err(|e| CliError(format!("That is not JSON: {e}")))?;
    let items = match value {
        serde_json::Value::Array(items) => items,
        single => vec![single],
    };
    items
        .into_iter()
        .map(|v| serde_json::from_value::<EvalCase>(v).map_err(|e| CliError(format!("A case is not valid: {e}"))))
        .collect()
}

async fn eval_add(hub: &Hub, printer: Printer, args: EvalAddArgs) -> Result<Exit, CliError> {
    let (agent, _) = hub.agent(&args.agent).await?;
    let cases = read_cases(&args.file)?;
    for case in &cases {
        case.validate().map_err(|e| CliError(format!("{}: {e}", case.name)))?;
    }
    let now = zorix_core::Clock::now(&zorix_core::SystemClock);
    let mut names = Vec::new();
    for case in cases {
        names.push(case.name.clone());
        hub.agents.put_eval_case(&tenant_of(hub), agent.id, case, now).await?;
    }
    if printer.json {
        print_json(&json!({"agent": args.agent, "cases": names}))?;
    } else {
        eprintln!("{} case(s) saved for {}: {}.", names.len(), args.agent, names.join(", "));
    }
    Ok(Exit::Done)
}

async fn eval_list(hub: &Hub, printer: Printer, arg: AgentSlugArg) -> Result<Exit, CliError> {
    let (agent, _) = hub.agent(&arg.agent).await?;
    let cases = hub.agents.eval_cases(&tenant_of(hub), agent.id).await?;
    if printer.json {
        print_json(&cases)?;
    } else if cases.is_empty() {
        eprintln!("{} has no cases yet. `zorix eval add {} FILE.json` adds some.", arg.agent, arg.agent);
    } else {
        for c in &cases {
            let judge = if c.case.judge.is_some() { " +judge" } else { "" };
            println!("{:<32} {} check(s){judge}  {}", c.case.name, c.case.checks.len(), c.case.tags.join(","));
        }
    }
    Ok(Exit::Done)
}

async fn eval_remove(hub: &Hub, printer: Printer, args: EvalRemoveArgs) -> Result<Exit, CliError> {
    let (agent, _) = hub.agent(&args.agent).await?;
    if !hub.agents.remove_eval_case(&tenant_of(hub), agent.id, &args.case).await? {
        return Err(CliError(format!("{} has no case called {}.", args.agent, args.case)));
    }
    if printer.json {
        print_json(&json!({"agent": args.agent, "removed": args.case}))?;
    } else {
        eprintln!("Case {} was deleted.", args.case);
    }
    Ok(Exit::Done)
}

fn money(micros: u64) -> String {
    if micros == 0 { "free".into() } else { format!("${:.4}", micros as f64 / 1_000_000.0) }
}

fn print_run(run: &EvalRun, slug: &str) {
    println!("{slug} v{} · {} · run {} · {}", run.version, run.model, run.id, run.status);
    for r in &run.results {
        let verdict = if r.passed { "PASS" } else { "FAIL" };
        let score = r.judge.as_ref().map(|j| format!(", judge {}/5", j.score)).unwrap_or_default();
        println!(
            "  {verdict}  {:<32} {} step(s), {} tool call(s), {}{score}",
            r.case,
            r.steps,
            r.tool_calls,
            money(r.cost_micros)
        );
        if !r.passed {
            if let Some(error) = &r.error {
                println!("        ✗ {error}");
            }
            for c in r.checks.iter().filter(|c| !c.passed) {
                println!("        ✗ {}: {}", c.check, shorten(&c.detail, 100));
            }
            if let Some(j) = r.judge.as_ref().filter(|j| !j.reasons.is_empty()) {
                println!("        judge: {}", shorten(&j.reasons, 100));
            }
        }
    }
    let passed = run.results.iter().filter(|r| r.passed).count();
    let cost: u64 = run.results.iter().map(|r| r.cost_micros).sum();
    println!("{passed}/{} passed, {}", run.results.len(), money(cost));
}

async fn eval_run(home: &Path, tenant: &TenantId, printer: Printer, args: EvalRunArgs) -> Result<Exit, CliError> {
    let app = open_engine(home, tenant, &ModelArgs { model: None, local_only: args.local_only })?;
    let (agent, version) = app.hub.agent_version(&args.agent, args.version).await?;
    let cases = app.hub.agents.eval_cases(tenant, agent.id).await?;
    let judge: Arc<dyn ModelGateway> = app.router.clone();
    let engine: Arc<dyn ExecutionEngine> = Arc::new(app.engine.clone());
    let evaluator =
        Evaluator { engine, store: app.hub.agents.clone(), resolver: app.router.clone(), judge: Some(judge) };
    let models: Vec<Option<String>> =
        if args.models.is_empty() { vec![None] } else { args.models.iter().cloned().map(Some).collect() };
    let cancel = CancellationToken::new();
    let on_signal = cancel.clone();
    tokio::spawn(async move {
        let _ = tokio::signal::ctrl_c().await;
        on_signal.cancel();
    });
    let mut all_passed = true;
    let mut outcome = Ok(());
    for model in models {
        let options = EvalOptions {
            model,
            offline: args.offline,
            tags: args.tags.clone(),
            concurrency: usize::from(args.concurrency),
            ..EvalOptions::default()
        };
        match evaluator.run(&agent, &version, cases.clone(), options, cancel.clone(), None).await {
            Ok(run) => {
                all_passed &= run.status == "finished" && run.results.iter().all(|r| r.passed);
                if printer.json {
                    print_json(&run)?;
                } else {
                    print_run(&run, agent.slug.as_str());
                }
            }
            Err(error) => {
                outcome = Err(CliError(error.to_string()));
                break;
            }
        }
    }
    app.shutdown().await;
    outcome?;
    Ok(if all_passed { Exit::Done } else { Exit::Failed })
}

fn parse_run(text: &str) -> Result<EvalRunId, CliError> {
    text.parse()
        .map_err(|_| CliError(format!("“{text}” is not an evaluation run id. `zorix eval runs AGENT` lists them.")))
}

async fn find_run(hub: &Hub, text: &str) -> Result<EvalRun, CliError> {
    let id = parse_run(text)?;
    hub.agents
        .eval_run(&tenant_of(hub), id)
        .await?
        .ok_or_else(|| CliError(format!("There is no evaluation run {text}.")))
}

async fn eval_runs(hub: &Hub, printer: Printer, arg: AgentSlugArg) -> Result<Exit, CliError> {
    let (agent, _) = hub.agent(&arg.agent).await?;
    let runs = hub.agents.eval_runs(&tenant_of(hub), agent.id, 50).await?;
    if printer.json {
        print_json(&runs)?;
    } else if runs.is_empty() {
        eprintln!("{} has no evaluation runs yet. `zorix eval run {}` makes one.", arg.agent, arg.agent);
    } else {
        for r in &runs {
            let passed = r.results.iter().filter(|x| x.passed).count();
            println!(
                "{}  v{:<3} {:<16} {}  {passed}/{} passed  {}",
                r.id,
                r.version,
                shorten(&r.model, 16),
                when(r.started_at),
                r.results.len(),
                r.status
            );
        }
    }
    Ok(Exit::Done)
}

async fn eval_show(hub: &Hub, printer: Printer, arg: EvalRunArg) -> Result<Exit, CliError> {
    let run = find_run(hub, &arg.run).await?;
    let slug = hub
        .agents
        .agent(&tenant_of(hub), run.agent)
        .await?
        .map(|a| a.slug.to_string())
        .unwrap_or_else(|| "(deleted agent)".into());
    if printer.json {
        print_json(&run)?;
    } else {
        print_run(&run, &slug);
    }
    Ok(Exit::Done)
}

async fn eval_compare(hub: &Hub, printer: Printer, args: EvalCompareArgs) -> Result<Exit, CliError> {
    let (before, after) = (find_run(hub, &args.before).await?, find_run(hub, &args.after).await?);
    if before.agent != after.agent {
        return Err(CliError("These runs are of different agents; compare runs of one agent.".into()));
    }
    let rows = compare(&before, &after);
    if printer.json {
        let out: Vec<_> = rows
            .iter()
            .map(|r| {
                json!({
                    "case": r.case, "change": format!("{:?}", r.change), "cost_delta_micros": r.cost_delta_micros,
                    "steps_delta": r.steps_delta, "score_delta": r.score_delta
                })
            })
            .collect();
        print_json(&out)?;
    } else {
        println!("v{} · {}   →   v{} · {}", before.version, before.model, after.version, after.model);
        for r in &rows {
            let mark = match r.change {
                Change::StillPasses => "same  pass",
                Change::StillFails => "same  FAIL",
                Change::Fixed => "FIXED",
                Change::Broke => "BROKE",
                Change::OnlyInOne => "only in one",
            };
            let score = r.score_delta.map(|d| format!(", judge {d:+}")).unwrap_or_default();
            println!("  {mark:<11} {:<32} steps {:+}, cost {:+} µ${score}", r.case, r.steps_delta, r.cost_delta_micros);
        }
    }
    let broke = rows.iter().any(|r| r.change == Change::Broke);
    Ok(if broke { Exit::Failed } else { Exit::Done })
}
