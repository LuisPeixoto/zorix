//! Workflows: the host that starts them by itself, and its settings. The screens' own commands are added with them.

use std::sync::Arc;

use serde::Serialize;
use tauri::State;

use crate::settings::Settings;
use crate::state::{AppState, Services};

/// Whether workflows start by themselves, and how webhooks are set.
#[derive(Serialize)]
pub struct HostInfo {
    /// Whether the host is running (schedules and events work while it is).
    pub running: bool,
    /// Whether the person turned webhooks on.
    pub webhooks: bool,
    /// The address the listener is bound to, when it is.
    pub listening: Option<String>,
    /// The address saved in Settings.
    pub address: String,
}

async fn info(services: &Services, home: &std::path::Path) -> HostInfo {
    let settings = Settings::load(home);
    let host = services.host.lock().await;
    HostInfo {
        running: host.is_some(),
        webhooks: settings.webhooks,
        listening: host.as_ref().and_then(|h| h.webhook_addr()).map(|a| a.to_string()),
        address: settings.webhook_address,
    }
}

#[tauri::command]
pub async fn workflow_host(state: State<'_, AppState>) -> Result<HostInfo, String> {
    let services = state.services()?;
    Ok(info(services, &state.home).await)
}

/// Turn webhooks on or off, or move them to another address. The host restarts; nothing it was running is lost, because runs
/// live in the store.
#[tauri::command]
pub async fn workflow_settings_set(
    state: State<'_, AppState>,
    webhooks: bool,
    address: String,
) -> Result<HostInfo, String> {
    let services = state.services_arc()?;
    let wanted = Settings { webhooks, webhook_address: address.trim().to_string() };
    let listen = wanted.listen()?;
    let mut host = services.host.lock().await;
    if let Some(old) = host.take() {
        old.shutdown().await;
    }
    let tickers = || -> Vec<Arc<dyn zorix_workflows::host::Ticker>> {
        vec![zorix_projects::runtime::ticker(&services.projects), zorix_workforce::runtime::ticker(&services.workforce)]
    };
    let started = services.workflows.start_host_with(services.secrets.clone(), listen, tickers()).await;
    match started {
        Ok(new) => {
            *host = Some(Arc::new(new));
            wanted.save(&state.home)?;
        }
        Err(error) => {
            // The old setting stays: start again with it so schedules keep working.
            let previous = Settings::load(&state.home).listen().ok().flatten();
            if let Ok(again) = services.workflows.start_host_with(services.secrets.clone(), previous, tickers()).await {
                *host = Some(Arc::new(again));
            }
            return Err(error.to_string());
        }
    }
    drop(host);
    Ok(info(&services, &state.home).await)
}

// ---------------------------------------------------------------------------------------------------------------------
// The workflows screens

use serde::Deserialize;
use zorix_core::{
    Confirm, Timestamp, WaitId, Workflow, WorkflowId, WorkflowRun, WorkflowRunId, WorkflowSlug, WorkflowSpec,
    WorkflowStore, WorkflowWait,
};
use zorix_workflows::{
    Card, Decision, DiffLine, Figures, Imported, Missing, StartRequest, StepHistory, Template, Verdict,
};

fn text(error: impl std::fmt::Display) -> String {
    error.to_string()
}

fn id_of(text: &str) -> Result<WorkflowId, String> {
    text.parse().map_err(|_| format!("“{text}” is not a workflow id."))
}

fn run_id_of(value: &str) -> Result<WorkflowRunId, String> {
    value.parse().map_err(|_| format!("“{value}” is not a run id."))
}

fn confirmer(confirm: bool) -> Option<Confirm> {
    confirm.then(|| Confirm { by: "desktop".into() })
}

/// The list, its figures and the host.
#[derive(Serialize)]
pub struct Overview {
    /// One row per workflow.
    pub cards: Vec<Card>,
    /// The four figures.
    pub figures: Figures,
    /// Whether schedules and webhooks are being served.
    pub host: HostInfo,
}

#[tauri::command]
pub async fn workflows_overview(state: State<'_, AppState>) -> Result<Overview, String> {
    let services = state.services()?;
    let service = &services.workflows.service;
    Ok(Overview {
        cards: service.cards().await.map_err(text)?,
        figures: service.figures().await.map_err(text)?,
        host: info(services, &state.home).await,
    })
}

/// One published version, without its definition.
#[derive(Serialize)]
pub struct VersionRow {
    /// 1, 2, 3 …
    pub version: u32,
    /// The hash of its definition.
    pub hash: String,
    /// Why it was published.
    pub note: Option<String>,
    /// When.
    pub created_at: Timestamp,
    /// How many unattended steps a person confirmed for it.
    pub confirmed: usize,
}

/// A workflow, its versions and what it needs.
#[derive(Serialize)]
pub struct Detail {
    /// The workflow with its draft.
    pub workflow: Workflow,
    /// Newest first.
    pub versions: Vec<VersionRow>,
    /// What the draft names that is not set up here.
    pub missing: Vec<Missing>,
    /// The steps a person must confirm before the draft is published.
    pub to_confirm: Vec<String>,
    /// Whether the draft differs from the live version.
    pub unpublished_changes: bool,
}

#[tauri::command]
pub async fn workflow_detail(state: State<'_, AppState>, id: String) -> Result<Detail, String> {
    let services = state.services()?;
    let service = &services.workflows.service;
    let id = id_of(&id)?;
    let workflow =
        service.store().workflow(service.tenant(), id).await.map_err(text)?.ok_or("That workflow no longer exists.")?;
    let versions = service.store().versions(service.tenant(), id).await.map_err(text)?;
    let unpublished_changes = versions.first().is_none_or(|v| v.hash != workflow.draft.hash());
    Ok(Detail {
        missing: service.missing(&workflow.draft).await.map_err(text)?,
        to_confirm: service.to_confirm(id).await.map_err(text)?,
        versions: versions
            .into_iter()
            .map(|v| VersionRow {
                version: v.version,
                hash: v.hash,
                note: v.note,
                created_at: v.created_at,
                confirmed: v.confirmations.len(),
            })
            .collect(),
        unpublished_changes,
        workflow,
    })
}

#[tauri::command]
pub async fn workflow_create(state: State<'_, AppState>, slug: String, name: String) -> Result<Workflow, String> {
    let service = &state.services()?.workflows.service;
    let slug = WorkflowSlug::new(slug.trim()).map_err(text)?;
    let name = if name.trim().is_empty() { slug.as_str().replace('-', " ") } else { name.trim().to_string() };
    service.create(slug, WorkflowSpec::blank(name)).await.map_err(text)
}

#[tauri::command]
pub async fn workflow_save(state: State<'_, AppState>, id: String, spec: WorkflowSpec) -> Result<Workflow, String> {
    let services = state.services()?;
    let saved = services.workflows.service.save_draft(id_of(&id)?, spec).await.map_err(text)?;
    Ok(saved)
}

/// What publishing did.
#[derive(Serialize)]
pub struct Published {
    /// The version the next trigger uses.
    pub version: u32,
    /// False when the draft was the same as that version and nothing was written.
    pub written: bool,
}

async fn refresh(state: &AppState) {
    if let Ok(services) = state.services()
        && let Some(host) = services.host.lock().await.as_ref()
    {
        host.refresh();
    }
}

#[tauri::command]
pub async fn workflow_publish(
    state: State<'_, AppState>,
    id: String,
    note: Option<String>,
    confirm: bool,
) -> Result<Published, String> {
    let services = state.services()?;
    let (version, written) = services
        .workflows
        .service
        .publish(id_of(&id)?, note.filter(|n| !n.trim().is_empty()), confirmer(confirm))
        .await
        .map_err(text)?;
    refresh(&state).await;
    Ok(Published { version: version.version, written })
}

#[tauri::command]
pub async fn workflow_rollback(
    state: State<'_, AppState>,
    id: String,
    to: u32,
    confirm: bool,
) -> Result<Published, String> {
    let services = state.services()?;
    let version = services.workflows.service.rollback(id_of(&id)?, to, confirmer(confirm)).await.map_err(text)?;
    refresh(&state).await;
    Ok(Published { version: version.version, written: true })
}

#[tauri::command]
pub async fn workflow_set_enabled(state: State<'_, AppState>, id: String, enabled: bool) -> Result<(), String> {
    state.services()?.workflows.service.set_enabled(id_of(&id)?, enabled).await.map_err(text)?;
    refresh(&state).await;
    Ok(())
}

#[tauri::command]
pub async fn workflow_remove(state: State<'_, AppState>, id: String) -> Result<bool, String> {
    let removed = state.services()?.workflows.service.remove(id_of(&id)?).await.map_err(text)?;
    refresh(&state).await;
    Ok(removed)
}

#[tauri::command]
pub async fn workflow_export(state: State<'_, AppState>, id: String, version: Option<u32>) -> Result<String, String> {
    state.services()?.workflows.service.export(id_of(&id)?, version).await.map_err(text)
}

#[tauri::command]
pub async fn workflow_import(state: State<'_, AppState>, contents: String) -> Result<Imported, String> {
    state.services()?.workflows.service.import(&contents).await.map_err(text)
}

#[tauri::command]
pub async fn workflow_templates() -> Result<Vec<Template>, String> {
    Ok(zorix_workflows::WorkflowService::templates())
}

/// A starter added, with what it needs that is not here yet.
#[derive(Serialize)]
pub struct Added {
    /// The new workflow.
    pub workflow: Workflow,
    /// What it names that is not set up.
    pub missing: Vec<Missing>,
}

#[tauri::command]
pub async fn workflow_add_template(state: State<'_, AppState>, template: String) -> Result<Added, String> {
    let service = &state.services()?.workflows.service;
    let workflow = service.add_template(&template).await.map_err(text)?;
    let missing = service.missing(&workflow.draft).await.map_err(text)?;
    Ok(Added { workflow, missing })
}

async fn spec_of(services: &Services, id: WorkflowId, which: &str) -> Result<WorkflowSpec, String> {
    let service = &services.workflows.service;
    if which == "draft" {
        return Ok(service
            .store()
            .workflow(service.tenant(), id)
            .await
            .map_err(text)?
            .ok_or("That workflow no longer exists.")?
            .draft);
    }
    let number: u32 = which.parse().map_err(|_| format!("“{which}” is not a version."))?;
    Ok(service
        .store()
        .version(service.tenant(), id, number)
        .await
        .map_err(text)?
        .ok_or_else(|| format!("There is no version {number}."))?
        .spec)
}

/// Compare two definitions: a version number or `draft` on each side.
#[tauri::command]
pub async fn workflow_diff(
    state: State<'_, AppState>,
    id: String,
    from: String,
    to: String,
) -> Result<Vec<DiffLine>, String> {
    let services = state.services()?;
    let id = id_of(&id)?;
    Ok(zorix_workflows::WorkflowService::diff(&spec_of(services, id, &from).await?, &spec_of(services, id, &to).await?))
}

/// A definition as readable lines, for the two sides of a comparison.
#[tauri::command]
pub async fn workflow_outline(state: State<'_, AppState>, id: String, which: String) -> Result<String, String> {
    let services = state.services()?;
    Ok(zorix_workflows::WorkflowService::outline(&spec_of(services, id_of(&id)?, &which).await?))
}

/// What starts a run by hand.
#[derive(Deserialize)]
pub struct Start {
    /// The workflow.
    pub id: String,
    /// The trigger step, when not the manual one.
    pub trigger: Option<String>,
    /// What the trigger carries.
    pub payload: Option<serde_json::Value>,
    /// Run the draft as a test.
    pub test: bool,
}

#[tauri::command]
pub async fn workflow_start(state: State<'_, AppState>, request: Start) -> Result<WorkflowRun, String> {
    let services = state.services_arc()?;
    let mut start = StartRequest::manual(id_of(&request.id)?, request.payload.unwrap_or_else(|| serde_json::json!({})));
    start.test = request.test;
    start.trigger = request.trigger;
    services.workflows.engine.start(start).await.map_err(text)
}

#[tauri::command]
pub async fn workflow_runs(
    state: State<'_, AppState>,
    id: Option<String>,
    limit: Option<usize>,
) -> Result<Vec<WorkflowRun>, String> {
    let service = &state.services()?.workflows.service;
    let workflow = id.as_deref().map(id_of).transpose()?;
    service.store().runs(service.tenant(), workflow, limit.unwrap_or(50).min(200)).await.map_err(text)
}

/// One run with its waits.
#[derive(Serialize)]
pub struct RunDetail {
    /// The run.
    pub run: WorkflowRun,
    /// What it stopped for, newest first.
    pub waits: Vec<WorkflowWait>,
}

#[tauri::command]
pub async fn workflow_run(state: State<'_, AppState>, run: String) -> Result<RunDetail, String> {
    let services = state.services_arc()?;
    let id = run_id_of(&run)?;
    // Looking at a run moves it along: a timer that ended or an execution that finished shows up now, not at the next beat.
    let run = match services.workflows.engine.advance(id).await {
        Ok(run) => run,
        Err(_) => services
            .workflows
            .store
            .run(&services.tenant, id)
            .await
            .map_err(text)?
            .ok_or("That run no longer exists.")?,
    };
    let waits = services.workflows.service.waits_of(id).await.map_err(text)?;
    Ok(RunDetail { run, waits })
}

#[tauri::command]
pub async fn workflow_cancel(state: State<'_, AppState>, run: String) -> Result<WorkflowRun, String> {
    state.services()?.workflows.engine.cancel(run_id_of(&run)?, "desktop").await.map_err(text)
}

#[tauri::command]
pub async fn workflow_waits(state: State<'_, AppState>) -> Result<Vec<WorkflowWait>, String> {
    let service = &state.services()?.workflows.service;
    service.store().pending_waits(service.tenant()).await.map_err(text)
}

/// A person's answer.
#[derive(Deserialize)]
pub struct Decide {
    /// The wait.
    pub wait: String,
    /// The fingerprint of what they were shown.
    pub fingerprint: String,
    /// `approve`, `refuse` or `answer`.
    pub verdict: String,
    /// The answer, for a question.
    pub text: Option<String>,
    /// A note.
    pub note: Option<String>,
}

#[tauri::command]
pub async fn workflow_decide(state: State<'_, AppState>, request: Decide) -> Result<WorkflowWait, String> {
    let services = state.services_arc()?;
    let wait: WaitId = request.wait.parse().map_err(|_| "That is not a wait id.".to_string())?;
    let verdict = match request.verdict.as_str() {
        "approve" => Verdict::Approve,
        "refuse" => Verdict::Refuse,
        "answer" => Verdict::Answer(request.text.unwrap_or_default()),
        other => return Err(format!("“{other}” is not an answer.")),
    };
    let decision = Decision {
        fingerprint: request.fingerprint,
        verdict,
        by: "desktop".into(),
        note: request.note.filter(|n| !n.trim().is_empty()),
    };
    services.workflows.engine.decide(wait, decision).await.map_err(text)
}

#[tauri::command]
pub async fn workflow_step_history(
    state: State<'_, AppState>,
    id: String,
    step: String,
) -> Result<StepHistory, String> {
    state.services()?.workflows.service.step_history(id_of(&id)?, &step, 5).await.map_err(text)
}

/// A workflow's webhook address.
#[derive(Serialize)]
pub struct WebhookAddress {
    /// The address, with its secret.
    pub url: String,
    /// The secret a sender signs with, when a trigger asks for a signature.
    pub signing_secret: Option<String>,
    /// Whether the listener is running now.
    pub listening: bool,
}

#[tauri::command]
pub async fn workflow_webhook(state: State<'_, AppState>, id: String, reset: bool) -> Result<WebhookAddress, String> {
    let services = state.services()?;
    let id = id_of(&id)?;
    let workflow = services
        .workflows
        .service
        .store()
        .workflow(&services.tenant, id)
        .await
        .map_err(text)?
        .ok_or("That workflow no longer exists.")?;
    let host = services.host.lock().await;
    let host = host.as_ref().ok_or("The workflow host is not running.")?;
    let secret = if reset { host.reset_webhook_token(id).await } else { host.webhook_token(id).await }.map_err(text)?;
    let signed =
        workflow.draft.triggers().any(|t| matches!(t.kind, zorix_core::StepKind::Webhook { signed: true, .. }));
    let signing_secret = if signed { Some(host.signing_secret(id).await.map_err(text)?) } else { None };
    let settings = Settings::load(&state.home);
    let address = host.webhook_addr().map(|a| a.to_string()).unwrap_or(settings.webhook_address);
    Ok(WebhookAddress {
        url: format!("http://{address}/hooks/{}/{secret}", workflow.slug),
        signing_secret,
        listening: host.webhook_addr().is_some(),
    })
}
