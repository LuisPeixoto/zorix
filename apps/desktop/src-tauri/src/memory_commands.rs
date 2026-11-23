//! The commands behind the memory screens. Like every command, they return plain text errors and decide nothing the
//! engine or the store does not decide.

use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use tauri::State;
use zorix_core::{
    Clock, ConversationId, ExecutionId, MemoryFilter, MemoryId, MemoryItem, MemoryKind, MemoryOrigin, MemoryPatch,
    MemoryStatus, MemoryStore, NewMemory, Remembered, Scope, SystemClock, Timestamp, memory_key, redact_text,
};
use zorix_memory::tools::end_of_day;

use crate::state::AppState;
use crate::ui_error::{UiError, ui};

const STALE_DAYS: i64 = 180;

fn now() -> Timestamp {
    SystemClock.now()
}

/// `global`, `project:<folder>` or `conversation:<id>`.
fn parse_scope(text: &str) -> Result<Scope, UiError> {
    match text.split_once(':') {
        None if text == "global" => Ok(Scope::Global),
        Some(("project", path)) if !path.is_empty() => Ok(Scope::Project(path.to_string())),
        Some(("conversation", id)) => ConversationId::new(id).map(Scope::Conversation).map_err(ui),
        Some(("agent", id)) => id.parse().map(Scope::Agent).map_err(ui),
        _ => Err("The scope is global, project:<folder>, conversation:<id> or agent:<id>.".into()),
    }
}

fn scope_text(scope: &Scope) -> String {
    match scope {
        Scope::Global => "global".into(),
        Scope::Project(path) => format!("project:{path}"),
        Scope::Conversation(id) => format!("conversation:{id}"),
        Scope::Agent(id) => format!("agent:{id}"),
    }
}

fn parse_id(text: &str) -> Result<MemoryId, UiError> {
    text.trim().trim_start_matches("m:").parse().map_err(|_| UiError::message(format!("“{text}” is not a memory id.")))
}

fn day(text: &str) -> Result<Timestamp, UiError> {
    let at =
        end_of_day(text).ok_or_else(|| UiError::message(format!("“{text}” is not a date: write it as YYYY-MM-DD.")))?;
    if at.millis() <= now().millis() {
        return Err(UiError::message(format!("{text} is already in the past.")));
    }
    Ok(at)
}

/// One memory as the screen shows it.
#[derive(Serialize)]
pub struct MemoryRow {
    id: String,
    kind: String,
    scope: String,
    scope_label: String,
    subject: Option<String>,
    content: String,
    /// `active`, `replaced` or `expired`.
    status: String,
    replaced_by: Option<String>,
    by: String,
    proposed_in: Option<String>,
    created_at: i64,
    confirmed_at: i64,
    valid_until: Option<i64>,
    stale_days: Option<i64>,
    use_count: u32,
}

fn row(item: &MemoryItem) -> MemoryRow {
    let at = now();
    let expired = item.valid_until.is_some_and(|u| u.millis() <= at.millis());
    let age = (at.millis() - item.confirmed_at.millis()) / 86_400_000;
    let (by, proposed_in) = match &item.origin {
        MemoryOrigin::User { .. } => ("You".to_string(), None),
        MemoryOrigin::Model { execution_id, approved_by, .. } => {
            (format!("Proposed by a run, approved by {approved_by}"), Some(execution_id.to_string()))
        }
    };
    MemoryRow {
        id: format!("m:{}", item.id),
        kind: item.kind.as_str().into(),
        scope: scope_text(&item.scope),
        scope_label: item.scope.label(),
        subject: item.subject.clone(),
        content: item.content.clone(),
        status: match (item.status, expired) {
            (MemoryStatus::Superseded, _) => "replaced",
            (_, true) => "expired",
            _ => "active",
        }
        .into(),
        replaced_by: item.superseded_by.map(|id| format!("m:{id}")),
        by,
        proposed_in,
        created_at: item.created_at.millis(),
        confirmed_at: item.confirmed_at.millis(),
        valid_until: item.valid_until.map(|t| t.millis()),
        stale_days: (age >= STALE_DAYS && item.status == MemoryStatus::Active).then_some(age),
        use_count: item.use_count,
    }
}

#[tauri::command]
pub async fn list_memory(state: State<'_, AppState>, include_inactive: bool) -> Result<Vec<MemoryRow>, UiError> {
    let services = state.services()?;
    let items = services
        .memory_store
        .all_memory(&services.tenant, &MemoryFilter { include_inactive, ..MemoryFilter::default() }, now())
        .await
        .map_err(ui)?;
    Ok(items.iter().map(row).collect())
}

#[derive(Deserialize)]
pub struct NewFact {
    content: String,
    kind: String,
    scope: String,
    #[serde(default)]
    subject: Option<String>,
    #[serde(default)]
    valid_until: Option<String>,
}

#[tauri::command]
pub async fn add_memory(state: State<'_, AppState>, fact: NewFact) -> Result<String, UiError> {
    let services = state.services()?;
    let content = fact.content.trim().to_string();
    if redact_text(&content) != content {
        return Err(
            "That looks like a credential or a key, so it was not remembered. Memory is not for secrets.".into()
        );
    }
    let new = NewMemory {
        scope: parse_scope(&fact.scope)?,
        kind: MemoryKind::parse(&fact.kind).ok_or("The kind is fact or preference.")?,
        subject: fact.subject.map(|s| s.trim().to_string()).filter(|s| !s.is_empty()),
        content,
        valid_until: fact.valid_until.as_deref().filter(|d| !d.trim().is_empty()).map(day).transpose()?,
        origin: MemoryOrigin::User { name: "desktop".into() },
        replaces: vec![],
    };
    let outcome = services.memory_store.remember(&services.tenant, new, now()).await.map_err(ui)?;
    let _ = services.memory.embed_missing(&services.tenant, &tokio_util::sync::CancellationToken::new()).await;
    Ok(match outcome {
        Remembered::Added(_) => "Remembered.".into(),
        Remembered::Duplicate(_) => "That was already remembered.".into(),
        Remembered::Replaced { .. } => "Remembered, replacing an older memory about the same subject.".into(),
    })
}

#[derive(Deserialize)]
pub struct MemoryChange {
    id: String,
    #[serde(default)]
    content: Option<String>,
    #[serde(default)]
    subject: Option<String>,
    #[serde(default)]
    clear_subject: bool,
    #[serde(default)]
    valid_until: Option<String>,
    #[serde(default)]
    clear_valid_until: bool,
    #[serde(default)]
    scope: Option<String>,
    #[serde(default)]
    confirm: bool,
}

#[tauri::command]
pub async fn update_memory(state: State<'_, AppState>, change: MemoryChange) -> Result<MemoryRow, UiError> {
    let services = state.services()?;
    if change.content.as_ref().is_some_and(|c| redact_text(c) != *c) {
        return Err("That looks like a credential or a key, so it was not saved. Memory is not for secrets.".into());
    }
    let patch = MemoryPatch {
        content: change.content,
        subject: if change.clear_subject { Some(None) } else { change.subject.map(Some) },
        valid_until: if change.clear_valid_until {
            Some(None)
        } else {
            change.valid_until.as_deref().filter(|d| !d.trim().is_empty()).map(day).transpose()?.map(Some)
        },
        scope: change.scope.as_deref().map(parse_scope).transpose()?,
        confirm: change.confirm,
    };
    let item =
        services.memory_store.update_memory(&services.tenant, parse_id(&change.id)?, patch, now()).await.map_err(ui)?;
    let _ = services.memory.embed_missing(&services.tenant, &tokio_util::sync::CancellationToken::new()).await;
    Ok(row(&item))
}

/// The runs that used a memory, for the delete confirmation.
#[tauri::command]
pub async fn memory_usage(state: State<'_, AppState>, id: String) -> Result<Vec<String>, UiError> {
    let services = state.services()?;
    let used =
        services.memory_store.snapshots_using(&services.tenant, &memory_key(parse_id(&id)?)).await.map_err(ui)?;
    Ok(used.iter().map(ExecutionId::to_string).collect())
}

#[tauri::command]
pub async fn forget_memory(state: State<'_, AppState>, id: String) -> Result<bool, UiError> {
    let services = state.services()?;
    let removed = services.memory_store.forget(&services.tenant, parse_id(&id)?).await.map_err(ui)?;
    services.memory.recall().invalidate_all();
    Ok(removed)
}

/// What memory put into a run's context: the frozen recall and what each part cost.
#[tauri::command]
pub async fn run_context(state: State<'_, AppState>, id: String) -> Result<Value, UiError> {
    let services = state.services()?;
    let id: ExecutionId = id.parse().map_err(|e: zorix_core::IdError| e.to_string())?;
    let events = zorix_core::ExecutionEngine::events(&services.engine, &services.tenant, id, 0).await.map_err(ui)?;
    let snapshot = services.memory_store.snapshot(id).await.map_err(ui)?;
    let requested = events.iter().find(|e| e.kind.name() == "execution.model.requested");
    Ok(json!({
        "recall": snapshot.map(|s| s.doc),
        "segments": requested.map(|e| e.payload["segments"].clone()),
        "token_factor": requested.map(|e| e.payload["token_factor"].clone()),
    }))
}
