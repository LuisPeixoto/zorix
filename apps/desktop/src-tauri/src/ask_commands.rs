//! A server's sampling and elicitation requests while a run's tool runs (ADR-0051): list them, answer them, and tell the
//! window when the list changes.

use tauri::{AppHandle, Emitter, State};
use zorix_core::{Answer, AskId, PendingAsk, Question, ServerAsk};
use zorix_mcp::{BrowserOpener, SystemBrowser};
use zorix_runtime::Engine;

use crate::state::AppState;
use crate::ui_error::UiError;

/// Holding this watch says that somebody can answer: while the app is open, a request waits for the person.
pub fn forward_asks(app: AppHandle, engine: Engine) {
    let mut changed = engine.asks().watch();
    tauri::async_runtime::spawn(async move {
        // The engine lives as long as the app; a lagging receiver only means the list is read once more.
        while !matches!(changed.recv().await, Err(tokio::sync::broadcast::error::RecvError::Closed)) {
            let _ = app.emit("server-asks", ());
        }
    });
}

/// What waits for an answer.
#[tauri::command]
pub fn server_asks(state: State<'_, AppState>) -> Result<Vec<PendingAsk>, UiError> {
    let services = state.services()?;
    Ok(services.engine.asks().pending(&services.tenant))
}

/// Answer one request. A page to open is taken from the waiting request itself, never from the window, and opened only with a yes.
#[tauri::command]
pub fn server_ask_answer(state: State<'_, AppState>, id: String, hash: String, answer: Answer) -> Result<(), UiError> {
    let services = state.services()?;
    let id: AskId = id.parse().map_err(crate::ui_error::ui)?;
    let asks = services.engine.asks();
    let link =
        asks.pending(&services.tenant).into_iter().find(|a| a.id == id && a.hash == hash).and_then(|a| match a.ask {
            ServerAsk::Person(Question::Link { url, .. }) => Some(url),
            _ => None,
        });
    let yes = matches!(answer, Answer::Yes { .. });
    asks.answer(&services.tenant, id, &hash, answer).map_err(crate::ui_error::ui)?;
    if yes && let Some(url) = link {
        SystemBrowser.open(&url).map_err(crate::ui_error::ui)?;
    }
    Ok(())
}
