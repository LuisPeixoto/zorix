//! `zorix memory`, `zorix knowledge`, `zorix embeddings` and `zorix calibration`.

use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::Instant;

use serde_json::json;
use tokio_util::sync::CancellationToken;
use zorix_core::{
    Clock, Corpus, EmbedPurpose, EmbeddingRequest, KnowledgeRoot, MemoryFilter, MemoryId, MemoryItem, MemoryKind,
    MemoryOrigin, MemoryPatch, MemorySettings, MemoryStore, ModelGateway, NewMemory, Remembered, Scope, SystemClock,
    TenantId, Timestamp, Visibility, redact_text,
};
use zorix_memory::MemoryStack;
use zorix_memory::index::{Phase, Progress};
use zorix_memory::retrieve::hybrid_search;
use zorix_memory::tools::end_of_day;
use zorix_providers::{Router, embedding_choices};
use zorix_store::{DataDirLock, SqliteCalibration, SqliteMemoryStore, SqliteRepository, database_path};

use crate::app::{CliError, open_router};
use crate::cli::{
    CorpusArg, EmbeddingsCommand, EmbeddingsSetArgs, KnowledgeAddArgs, KnowledgeCommand, KnowledgeFilesArgs,
    KnowledgeRootArg, KnowledgeSearchArgs, KnowledgeSyncArgs, MemoryAddArgs, MemoryCommand, MemoryEditArgs,
    MemoryExportArgs, MemoryForgetArgs, MemoryIdArg, MemoryListArgs, MemorySearchArgs, ModelArgs,
};
use crate::follow::{Exit, Printer, ask, decider};

/// What a memory command works with. Commands that change things hold the data-folder lock.
struct Ctx {
    store: Arc<SqliteMemoryStore>,
    stack: Arc<MemoryStack>,
    router: Arc<Router>,
    _lock: Option<DataDirLock>,
}

fn open(home: &Path, write: bool) -> Result<Ctx, CliError> {
    let lock = if write { Some(DataDirLock::acquire(home)?) } else { None };
    let database = database_path(home);
    let repository = Arc::new(SqliteRepository::open(&database)?);
    let store = Arc::new(SqliteMemoryStore::open(&database)?);
    let router = open_router(home, &ModelArgs::default_demo())?;
    let gateway: Arc<dyn ModelGateway> = router.clone();
    let stack = Arc::new(MemoryStack::new(store.clone(), repository, Some(gateway)));
    Ok(Ctx { store, stack, router, _lock: lock })
}

fn now() -> Timestamp {
    SystemClock.now()
}

fn parse_id(text: &str) -> Result<MemoryId, CliError> {
    text.trim()
        .trim_start_matches("m:")
        .parse()
        .map_err(|_| CliError(format!("“{text}” is not a memory id (m:… as `zorix memory list` shows).")))
}

fn until(text: &str) -> Result<Timestamp, CliError> {
    let at = end_of_day(text).ok_or_else(|| CliError(format!("“{text}” is not a date: write it as YYYY-MM-DD.")))?;
    if at.millis() <= now().millis() {
        return Err(CliError(format!("{text} is already in the past.")));
    }
    Ok(at)
}

fn describe(item: &MemoryItem, now: Timestamp) -> String {
    let status = match (item.status, item.superseded_by, item.valid_until) {
        (zorix_core::MemoryStatus::Superseded, Some(by), _) => format!("  (replaced by m:{by})"),
        (zorix_core::MemoryStatus::Superseded, None, _) => "  (replaced)".to_string(),
        (_, _, Some(until)) if until.millis() <= now.millis() => "  (expired)".to_string(),
        (_, _, Some(until)) => format!("  (until {})", &until.to_rfc3339()[..10]),
        _ => String::new(),
    };
    let subject = item.subject.as_ref().map(|s| format!(" [{s}]")).unwrap_or_default();
    format!(
        "m:{}  {:<10}  {}  confirmed {}{subject}  {}{status}",
        item.id,
        item.kind.as_str(),
        item.scope.label(),
        &item.confirmed_at.to_rfc3339()[..10],
        item.content
    )
}

fn print_items(printer: Printer, items: &[MemoryItem]) -> Result<(), CliError> {
    if printer.json {
        println!("{}", serde_json::to_string(items)?);
    } else if items.is_empty() {
        eprintln!("Nothing is remembered.");
    } else {
        let at = now();
        for item in items {
            println!("{}", describe(item, at));
        }
    }
    Ok(())
}

// ---------------------------------------------------------------------------------------------------------------------
// memory

/// `zorix memory …`.
pub async fn memory(
    home: &Path,
    tenant: &TenantId,
    printer: Printer,
    command: MemoryCommand,
) -> Result<Exit, CliError> {
    match command {
        MemoryCommand::List(args) => list(home, tenant, printer, args).await,
        MemoryCommand::Add(args) => add(home, tenant, printer, args).await,
        MemoryCommand::Edit(args) => edit(home, tenant, printer, args).await,
        MemoryCommand::Confirm(MemoryIdArg { id }) => {
            let ctx = open(home, true)?;
            let item = ctx
                .store
                .update_memory(tenant, parse_id(&id)?, MemoryPatch { confirm: true, ..MemoryPatch::default() }, now())
                .await?;
            print_items(printer, &[item])?;
            Ok(Exit::Done)
        }
        MemoryCommand::Forget(args) => forget(home, tenant, printer, args).await,
        MemoryCommand::Search(args) => search(home, tenant, printer, args).await,
        MemoryCommand::Export(args) => export(home, tenant, args).await,
    }
}

async fn list(home: &Path, tenant: &TenantId, printer: Printer, args: MemoryListArgs) -> Result<Exit, CliError> {
    let ctx = open(home, false)?;
    let mut items = ctx
        .store
        .all_memory(tenant, &MemoryFilter { include_inactive: args.all, ..MemoryFilter::default() }, now())
        .await?;
    if let Some(scope) = &args.scope {
        items.retain(|i| i.scope == scope.0);
    }
    print_items(printer, &items)?;
    Ok(Exit::Done)
}

async fn add(home: &Path, tenant: &TenantId, printer: Printer, args: MemoryAddArgs) -> Result<Exit, CliError> {
    let text = args.text.trim().to_string();
    if redact_text(&text) != text {
        return Err(CliError(
            "That looks like a credential or a key, so it was not remembered. Memory is not for secrets.".into(),
        ));
    }
    let new = NewMemory {
        scope: args.scope.0,
        kind: if args.kind == crate::cli::KindArg::Fact { MemoryKind::Fact } else { MemoryKind::Preference },
        subject: args.subject,
        content: text,
        valid_until: args.until.as_deref().map(until).transpose()?,
        origin: MemoryOrigin::User { name: decider() },
        replaces: vec![],
    };
    let ctx = open(home, true)?;
    let outcome = ctx.store.remember(tenant, new, now()).await?;
    // Best effort: make its vector now so it can be found by meaning.
    let _ = ctx.stack.embed_missing(tenant, &CancellationToken::new()).await;
    if printer.json {
        println!("{}", serde_json::to_string(&outcome)?);
    } else {
        match &outcome {
            Remembered::Added(item) => eprintln!("Remembered m:{}.", item.id),
            Remembered::Duplicate(item) => eprintln!("Already remembered as m:{}.", item.id),
            Remembered::Replaced { new, old } => eprintln!(
                "Remembered m:{}, replacing {}.",
                new.id,
                old.iter().map(|i| format!("m:{i}")).collect::<Vec<_>>().join(", ")
            ),
        }
    }
    Ok(Exit::Done)
}

async fn edit(home: &Path, tenant: &TenantId, printer: Printer, args: MemoryEditArgs) -> Result<Exit, CliError> {
    let patch = MemoryPatch {
        content: args.text,
        subject: args.subject.map(Some),
        valid_until: if args.no_until { Some(None) } else { args.until.as_deref().map(until).transpose()?.map(Some) },
        scope: args.scope.map(|s| s.0),
        confirm: false,
    };
    if patch.content.is_none() && patch.subject.is_none() && patch.valid_until.is_none() && patch.scope.is_none() {
        return Err(CliError("Say what to change: --text, --subject, --until, --no-until or --scope.".into()));
    }
    if patch.content.as_ref().is_some_and(|t| redact_text(t) != *t) {
        return Err(CliError(
            "That looks like a credential or a key, so it was not saved. Memory is not for secrets.".into(),
        ));
    }
    let ctx = open(home, true)?;
    let item = ctx.store.update_memory(tenant, parse_id(&args.id)?, patch, now()).await?;
    let _ = ctx.stack.embed_missing(tenant, &CancellationToken::new()).await;
    print_items(printer, &[item])?;
    Ok(Exit::Done)
}

async fn forget(home: &Path, tenant: &TenantId, printer: Printer, args: MemoryForgetArgs) -> Result<Exit, CliError> {
    let ctx = open(home, true)?;
    let id = parse_id(&args.id)?;
    let item = ctx.store.memory(tenant, id).await?.ok_or_else(|| CliError(format!("There is no memory m:{id}.")))?;
    let used = ctx.store.snapshots_using(tenant, &zorix_core::memory_key(id)).await?;
    if !args.yes {
        if !std::io::IsTerminal::is_terminal(&std::io::stdin()) {
            return Err(CliError("Deleting needs a yes: add --yes.".into()));
        }
        eprintln!("{}", describe(&item, now()));
        if !used.is_empty() {
            eprintln!("It was used in {} run(s); the copy in each run's record is removed too.", used.len());
        }
        if !ask("  Delete it everywhere ZORIX keeps it?".to_string()).await {
            eprintln!("Kept.");
            return Ok(Exit::Done);
        }
    }
    ctx.store.forget(tenant, id).await?;
    if printer.json {
        println!("{}", json!({"forgotten": format!("m:{id}"), "used_in": used.len()}));
    } else {
        eprintln!(
            "Forgotten m:{id}. (Text that already reached a run's transcript or a provider cannot be taken back.)"
        );
    }
    Ok(Exit::Done)
}

fn visibility(tenant: &TenantId, project: Option<&PathBuf>) -> Result<Visibility, CliError> {
    let project = match project {
        Some(dir) => Some(
            std::fs::canonicalize(dir)
                .map_err(|e| CliError(format!("The project folder {} cannot be used: {e}", dir.display())))?
                .to_string_lossy()
                .into_owned(),
        ),
        None => None,
    };
    Ok(Visibility { tenant: tenant.clone(), project, conversation: None })
}

async fn search(home: &Path, tenant: &TenantId, printer: Printer, args: MemorySearchArgs) -> Result<Exit, CliError> {
    let ctx = open(home, false)?;
    let vis = visibility(tenant, args.project.as_ref())?;
    let (corpus, label) = match args.corpus {
        CorpusArg::Memory => (Corpus::Memory, "memory"),
        CorpusArg::Knowledge => (Corpus::Knowledge, "documents"),
        CorpusArg::History => (Corpus::Operations, "past work"),
    };
    let found = hybrid_search(&*ctx.store, ctx.stack.embedder().map(|e| e.as_ref()), &vis, corpus, &args.query)
        .await
        .map_err(|e| CliError(e.to_string()))?;
    if let Some(why) = &found.degraded {
        eprintln!("({why}; showing keyword results)");
    }
    let mut rows: Vec<serde_json::Value> = Vec::new();
    for hit in found.hits.iter().take(10) {
        match corpus {
            Corpus::Memory => {
                if let Some(item) = match hit.id.parse::<MemoryId>() {
                    Ok(id) => ctx.store.memory(tenant, id).await?,
                    Err(_) => None,
                } {
                    rows.push(json!({"id": format!("m:{}", item.id), "text": describe(&item, now()), "found_by": hit.found_by()}));
                }
            }
            Corpus::Knowledge => {
                if let Some(chunk) = match hit.id.parse::<i64>() {
                    Ok(id) => ctx.store.chunks(tenant, &[id]).await?.into_iter().next(),
                    Err(_) => None,
                } {
                    rows.push(json!({"source": chunk.citation(), "text": zorix_memory::text::cut(&chunk.text.replace('\n', " "), 200), "found_by": hit.found_by()}));
                }
            }
            Corpus::Operations => {
                if let Some(record) = match hit.id.parse() {
                    Ok(id) => ctx.store.operation(tenant, id).await?,
                    Err(_) => None,
                } {
                    rows.push(json!({"execution": record.execution_id.to_string(), "state": record.state, "text": zorix_memory::text::cut(&record.text.replace('\n', " "), 200), "found_by": hit.found_by()}));
                }
            }
        }
    }
    if printer.json {
        println!("{}", serde_json::to_string(&rows)?);
    } else if rows.is_empty() {
        eprintln!("Nothing found in {label}.");
    } else {
        for row in rows {
            let head = row["id"].as_str().or(row["source"].as_str()).or(row["execution"].as_str()).unwrap_or("");
            println!("{head}  {}", row["text"].as_str().unwrap_or(""));
        }
    }
    Ok(Exit::Done)
}

async fn export(home: &Path, tenant: &TenantId, args: MemoryExportArgs) -> Result<Exit, CliError> {
    let ctx = open(home, false)?;
    let items = ctx
        .store
        .all_memory(tenant, &MemoryFilter { include_inactive: true, ..MemoryFilter::default() }, now())
        .await?;
    let text = serde_json::to_string_pretty(&items)?;
    match args.out {
        Some(path) => {
            std::fs::write(&path, text)?;
            eprintln!("Wrote {} memories to {}.", items.len(), path.display());
        }
        None => println!("{text}"),
    }
    Ok(Exit::Done)
}

// ---------------------------------------------------------------------------------------------------------------------
// knowledge

fn resolve_root(roots: &[KnowledgeRoot], text: &str) -> Result<KnowledgeRoot, CliError> {
    let wanted = text.trim();
    let canonical = std::fs::canonicalize(wanted).ok().map(|p| p.to_string_lossy().into_owned());
    let found: Vec<&KnowledgeRoot> = roots
        .iter()
        .filter(|r| {
            (wanted.len() >= 4 && r.id.to_string().starts_with(wanted))
                || canonical.as_deref() == Some(r.path.as_str())
                || r.path == wanted
        })
        .collect();
    match found.as_slice() {
        [one] => Ok((*one).clone()),
        [] => Err(CliError(format!("There is no folder “{text}”. `zorix knowledge list` shows them."))),
        _ => Err(CliError(format!("“{text}” matches more than one folder; use more of its id."))),
    }
}

fn progress_printer() -> impl Fn(Progress) + Send + Sync {
    let last = std::sync::Mutex::new((Phase::Scanning, 0u64));
    move |p: Progress| {
        let mut last = last.lock().unwrap_or_else(|e| e.into_inner());
        let step = p.done == 0 || p.phase != last.0 || p.done.saturating_sub(last.1) >= 50 || p.phase == Phase::Done;
        if !step {
            return;
        }
        *last = (p.phase, p.done);
        let what = match p.phase {
            Phase::Scanning => "Looking at the folder…".to_string(),
            Phase::Indexing => format!("Reading files… {}/{}", p.done, p.total),
            Phase::Embedding => format!("Making vectors… {}/{}", p.done, p.total),
            Phase::Done => "Done.".to_string(),
        };
        eprintln!("{what}");
    }
}

fn print_report(
    printer: Printer,
    root: &KnowledgeRoot,
    report: &zorix_memory::index::SyncReport,
) -> Result<(), CliError> {
    if printer.json {
        println!("{}", json!({"folder": root.path, "report": report}));
        return Ok(());
    }
    eprintln!(
        "{}: {} indexed, {} unchanged, {} left out, {} failed, {} removed; {} pieces, {} vectors.{}{}",
        root.path,
        report.indexed,
        report.unchanged,
        report.skipped,
        report.failed,
        report.removed,
        report.chunks,
        report.embedded,
        if report.cancelled { " Stopped early; run sync again to finish." } else { "" },
        report.degraded.as_ref().map(|d| format!(" ({d})")).unwrap_or_default()
    );
    if report.too_many_files {
        eprintln!("  The folder has more files than are looked at (20000).");
    }
    Ok(())
}

/// A Ctrl-C stops the work between files; what is done stays.
fn cancel_on_ctrl_c() -> CancellationToken {
    let token = CancellationToken::new();
    let trigger = token.clone();
    tokio::spawn(async move {
        if tokio::signal::ctrl_c().await.is_ok() {
            eprintln!("\nStopping after this file; what is done stays indexed.");
            trigger.cancel();
        }
    });
    token
}

/// `zorix knowledge …`.
pub async fn knowledge(
    home: &Path,
    tenant: &TenantId,
    printer: Printer,
    command: KnowledgeCommand,
) -> Result<Exit, CliError> {
    match command {
        KnowledgeCommand::Add(args) => knowledge_add(home, tenant, printer, args).await,
        KnowledgeCommand::List => {
            let ctx = open(home, false)?;
            let roots = ctx.store.all_roots(tenant).await?;
            if printer.json {
                println!("{}", serde_json::to_string(&roots)?);
                return Ok(Exit::Done);
            }
            if roots.is_empty() {
                eprintln!("No folders yet. Add one with `zorix knowledge add <folder>`.");
            }
            for root in &roots {
                let files = ctx.store.document_states(root.id).await?.len();
                let synced =
                    root.last_sync_at.map(|t| t.to_rfc3339()[..16].replace('T', " ")).unwrap_or_else(|| "never".into());
                println!(
                    "{}  {:<22} {:<8} {} files, synced {synced}{}",
                    &root.id.to_string()[..8],
                    root.scope.label(),
                    if root.enabled { "on" } else { "off" },
                    files,
                    if root.auto_sync { ", auto" } else { "" }
                );
                println!("          {}", root.path);
            }
            Ok(Exit::Done)
        }
        KnowledgeCommand::Sync(KnowledgeSyncArgs { folder }) => {
            let ctx = open(home, true)?;
            let roots = ctx.store.all_roots(tenant).await?;
            let chosen: Vec<KnowledgeRoot> = match folder {
                Some(text) => vec![resolve_root(&roots, &text)?],
                None => roots.into_iter().filter(|r| r.enabled).collect(),
            };
            if chosen.is_empty() {
                eprintln!("No folders to sync.");
            }
            let indexer = ctx.stack.indexer();
            let cancel = cancel_on_ctrl_c();
            for root in chosen {
                let report =
                    indexer.sync(&root, &progress_printer(), &cancel).await.map_err(|e| CliError(e.to_string()))?;
                print_report(printer, &root, &report)?;
                if report.cancelled {
                    return Ok(Exit::Cancelled);
                }
            }
            Ok(Exit::Done)
        }
        KnowledgeCommand::Files(KnowledgeFilesArgs { folder, failed }) => {
            let ctx = open(home, false)?;
            let root = resolve_root(&ctx.store.all_roots(tenant).await?, &folder)?;
            if failed {
                let problems = ctx.store.problem_documents(root.id).await?;
                if printer.json {
                    println!(
                        "{}",
                        serde_json::to_string(
                            &problems
                                .iter()
                                .map(|(f, s, why)| json!({"file": f, "status": s.as_str(), "reason": why}))
                                .collect::<Vec<_>>()
                        )?
                    );
                } else if problems.is_empty() {
                    eprintln!("Every file was indexed.");
                }
                if !printer.json {
                    for (file, status, why) in problems {
                        println!("{:<8} {file}  {why}", status.as_str());
                    }
                }
            } else {
                let mut states: Vec<_> = ctx.store.document_states(root.id).await?.into_iter().collect();
                states.sort_by(|a, b| a.0.cmp(&b.0));
                for (file, state) in states {
                    println!("{:<8} {file}", state.status.as_str());
                }
            }
            Ok(Exit::Done)
        }
        KnowledgeCommand::Remove(KnowledgeRootArg { folder }) => {
            let ctx = open(home, true)?;
            let root = resolve_root(&ctx.store.all_roots(tenant).await?, &folder)?;
            ctx.store.remove_root(tenant, root.id).await?;
            eprintln!("Removed {} from the index. The files were not touched.", root.path);
            Ok(Exit::Done)
        }
        KnowledgeCommand::Search(KnowledgeSearchArgs { query, project }) => {
            search(home, tenant, printer, MemorySearchArgs { query, corpus: CorpusArg::Knowledge, project }).await
        }
    }
}

async fn knowledge_add(
    home: &Path,
    tenant: &TenantId,
    printer: Printer,
    args: KnowledgeAddArgs,
) -> Result<Exit, CliError> {
    let ctx = open(home, true)?;
    let scope = match &args.project {
        Some(dir) => Scope::Project(
            std::fs::canonicalize(dir)
                .map_err(|e| CliError(format!("The project folder {} cannot be used: {e}", dir.display())))?
                .to_string_lossy()
                .into_owned(),
        ),
        None => Scope::Global,
    };
    let indexer = ctx.stack.indexer();
    let root = indexer
        .add_root(tenant, scope, &args.dir, args.include, args.exclude, args.auto_sync)
        .await
        .map_err(|e| CliError(e.to_string()))?;
    eprintln!("Added {} ({}).", root.path, root.scope.label());
    if !args.no_sync {
        let cancel = cancel_on_ctrl_c();
        let report = indexer.sync(&root, &progress_printer(), &cancel).await.map_err(|e| CliError(e.to_string()))?;
        print_report(printer, &root, &report)?;
    }
    Ok(Exit::Done)
}

// ---------------------------------------------------------------------------------------------------------------------
// embeddings

/// `zorix embeddings …`.
pub async fn embeddings(
    home: &Path,
    tenant: &TenantId,
    printer: Printer,
    command: EmbeddingsCommand,
) -> Result<Exit, CliError> {
    match command {
        EmbeddingsCommand::Show => {
            let ctx = open(home, false)?;
            let settings = ctx.store.settings(tenant).await?;
            let coverage = match &settings.embedding_model {
                Some(model) => Some(ctx.store.vector_coverage(tenant, model).await?),
                None => None,
            };
            if printer.json {
                println!(
                    "{}",
                    json!({"settings": settings, "embedded": coverage.map(|c| c.0), "texts": coverage.map(|c| c.1)})
                );
            } else {
                match (&settings.embedding_model, coverage) {
                    (Some(model), Some((done, total))) => {
                        println!(
                            "Model      {model}{}",
                            settings.embedding_dimensions.map(|d| format!(" ({d} dimensions)")).unwrap_or_default()
                        );
                        println!(
                            "Embedded   {done} of {total} texts{}",
                            if done < total { " (run `zorix embeddings reembed`)" } else { "" }
                        );
                        println!(
                            "Cloud      {}",
                            if settings.allow_cloud_embeddings {
                                "allowed: text is sent to the provider"
                            } else {
                                "not allowed"
                            }
                        );
                    }
                    _ => println!(
                        "No embeddings model: search uses keywords only. Choose one with `zorix embeddings set provider/model`."
                    ),
                }
            }
            Ok(Exit::Done)
        }
        EmbeddingsCommand::Models => {
            let ctx = open(home, false)?;
            let (choices, problems) = embedding_choices(&ctx.router, &CancellationToken::new()).await;
            for (provider, why) in &problems {
                eprintln!("{provider}: {why}");
            }
            if printer.json {
                println!("{}", serde_json::to_string(&choices)?);
            } else if choices.is_empty() {
                eprintln!("No embeddings models found. For a local one: `ollama pull all-minilm`.");
            }
            if !printer.json {
                for c in &choices {
                    println!(
                        "{:<40} {:>6} dims  {}",
                        c.reference(),
                        c.dimensions.map(|d| d.to_string()).unwrap_or_else(|| "?".into()),
                        if c.local { "on this computer" } else { "cloud: text is sent to the provider" }
                    );
                }
            }
            Ok(Exit::Done)
        }
        EmbeddingsCommand::Set(args) => set(home, tenant, args).await,
        EmbeddingsCommand::Off => {
            let ctx = open(home, true)?;
            ctx.store.set_settings(tenant, MemorySettings::default()).await?;
            eprintln!(
                "Embeddings are off: search uses keywords only. The vectors already made are kept in case you turn them on again."
            );
            Ok(Exit::Done)
        }
        EmbeddingsCommand::Test => {
            let ctx = open(home, false)?;
            let settings = ctx.store.settings(tenant).await?;
            let Some(model) = settings.embedding_model else {
                return Err(CliError("No embeddings model is set. Use `zorix embeddings set provider/model`.".into()));
            };
            let started = Instant::now();
            let response = embed_one(&ctx.router, &model, settings.embedding_dimensions).await?;
            println!("{model}: {} dimensions in {} ms", response.len(), started.elapsed().as_millis());
            Ok(Exit::Done)
        }
        EmbeddingsCommand::Reembed => {
            let ctx = open(home, true)?;
            let settings = ctx.store.settings(tenant).await?;
            if settings.embedding_model.is_none() {
                return Err(CliError("No embeddings model is set. Use `zorix embeddings set provider/model`.".into()));
            }
            let cancel = cancel_on_ctrl_c();
            let (reports, facts) = ctx
                .stack
                .reembed(tenant, &|_, p| progress_printer()(p), &cancel)
                .await
                .map_err(|e| CliError(e.to_string()))?;
            for (root, report) in &reports {
                print_report(printer, root, report)?;
            }
            eprintln!("{facts} fact(s) and past-work record(s) embedded; vectors of other models were deleted.");
            Ok(Exit::Done)
        }
    }
}

async fn embed_one(router: &Router, model: &str, dimensions: Option<u32>) -> Result<Vec<f32>, CliError> {
    let request = EmbeddingRequest {
        model: model.into(),
        inputs: vec!["Deploys reach staging first.".into()],
        purpose: EmbedPurpose::Query,
        dimensions,
    };
    let response = router.embed(request, CancellationToken::new()).await.map_err(|e| CliError(e.to_string()))?;
    response.vectors.into_iter().next().ok_or_else(|| CliError("The model returned no vector.".into()))
}

async fn set(home: &Path, tenant: &TenantId, args: EmbeddingsSetArgs) -> Result<Exit, CliError> {
    let ctx = open(home, true)?;
    let (provider, _) = args
        .model
        .split_once('/')
        .ok_or_else(|| CliError("Name the model as provider/model, for example ollama/all-minilm.".into()))?;
    let entry =
        ctx.router.provider(provider).ok_or_else(|| CliError(format!("There is no provider called “{provider}”.")))?;
    if !entry.local && !args.cloud {
        return Err(CliError(format!(
            "{provider} is a cloud provider: the text of your documents and your prompts would be sent to it to be embedded. Add --cloud to allow that."
        )));
    }
    // Try it: this also tells how big the vectors are and whether the key and the server work.
    let vector = embed_one(&ctx.router, &args.model, args.dims).await?;
    ctx.store
        .set_settings(
            tenant,
            MemorySettings {
                embedding_model: Some(args.model.clone()),
                embedding_dimensions: args.dims,
                allow_cloud_embeddings: args.cloud,
            },
        )
        .await?;
    eprintln!(
        "Using {} ({} dimensions). Make vectors for what is indexed with `zorix embeddings reembed`.",
        args.model,
        vector.len()
    );
    Ok(Exit::Done)
}

// ---------------------------------------------------------------------------------------------------------------------

/// `zorix calibration`.
pub fn calibration(home: &Path, printer: Printer) -> Result<Exit, CliError> {
    let calibration = SqliteCalibration::open(&database_path(home))?;
    let all = calibration.all();
    if printer.json {
        println!(
            "{}",
            serde_json::to_string(
                &all.iter()
                    .map(|(m, c)| json!({"model": m, "factor": c.factor, "samples": c.samples}))
                    .collect::<Vec<_>>()
            )?
        );
    } else if all.is_empty() {
        eprintln!("Nothing learned yet. Estimates are corrected after a few real model calls.");
    }
    if !printer.json {
        for (model, c) in all {
            println!(
                "{model:<44} x{:.2}  {} sample(s){}",
                c.factor,
                c.samples,
                if c.samples < zorix_core::tokens::MIN_SAMPLES { "  (not used yet)" } else { "" }
            );
        }
    }
    Ok(Exit::Done)
}

// ---------------------------------------------------------------------------------------------------------------------
// show --context

/// What memory put into an execution's context, from its frozen snapshot and its first model request.
pub async fn context_of(
    home: &Path,
    id: zorix_core::ExecutionId,
    events: &[zorix_core::ExecutionEvent],
) -> Result<serde_json::Value, CliError> {
    let store = SqliteMemoryStore::open(&database_path(home))?;
    let snapshot = store.snapshot(id).await?;
    let requested = events.iter().find(|e| e.kind.name() == "execution.model.requested");
    Ok(json!({
        "recall": snapshot.map(|s| s.doc),
        "segments": requested.map(|e| e.payload["segments"].clone()),
        "token_factor": requested.map(|e| e.payload["token_factor"].clone()),
        "estimated_tool_tokens": requested.map(|e| e.payload["estimated_tool_tokens"].clone()),
        "history_edited": requested.map(|e| e.payload["history_edited"].clone()),
    }))
}

/// Print it for people.
pub fn print_context(context: &serde_json::Value) {
    println!("\nContext");
    let Some(recall) = context["recall"].as_object() else {
        println!("  Memory was not used (it was switched off, or there was nothing relevant).");
        return;
    };
    if let Some(why) = recall.get("degraded").and_then(|d| d.as_str()) {
        println!("  Degraded   {why}");
    }
    if let Some(model) = recall.get("embedding_model").and_then(|m| m.as_str()) {
        println!("  Embeddings {model}");
    }
    let items = recall.get("items").and_then(|i| i.as_array()).cloned().unwrap_or_default();
    if items.is_empty() {
        println!("  Nothing was retrieved.");
    }
    for item in &items {
        let kind = match item["kind"].as_str() {
            Some("fact") => "fact",
            Some("exchange") => "earlier",
            Some("chunk") => "document",
            Some("past_work") => "past work",
            _ => "item",
        };
        let label = item["label"].as_str().unwrap_or("");
        let text = zorix_memory::text::cut(&item["text"].as_str().unwrap_or("").replace('\n', " "), 90);
        let by = item["found_by"]
            .as_array()
            .map(|a| a.iter().filter_map(|v| v.as_str()).collect::<Vec<_>>().join("+"))
            .filter(|s| !s.is_empty());
        println!("  {kind:<10} {label}  {text}{}", by.map(|b| format!("  [{b}]")).unwrap_or_default());
    }
    if let Some(segments) = context["segments"].as_array() {
        println!("  Parts of the first request (tokens, calibrated):");
        for s in segments {
            println!(
                "    {:<14} {:>6}  {}",
                s["origin"].as_str().unwrap_or(""),
                s["tokens"].as_u64().unwrap_or(0),
                if s["kept"].as_bool().unwrap_or(true) { "kept" } else { "left out to fit" }
            );
        }
    }
}
