//! The webhook listener (ADR-0034): `POST /hooks/<workflow>/<secret>`.
//!
//! Off unless the host is given an address. A wrong secret, an unknown workflow and a workflow that is off all answer the same
//! 404, so the listener says nothing about what exists. The body is capped at 1 MB. A trigger marked `signed` also needs a valid
//! `X-Hub-Signature-256`. The payload reaches steps only through templates: it is data, never an instruction.

use std::net::SocketAddr;
use std::sync::Arc;

use axum::Router;
use axum::body::Bytes;
use axum::extract::{DefaultBodyLimit, Path, State};
use axum::http::{HeaderMap, StatusCode};
use axum::response::{IntoResponse, Response};
use axum::routing::post;
use hmac::{Hmac, Mac};
use parking_lot::Mutex;
use serde_json::{Value, json};
use sha2::Sha256;
use tokio::task::JoinHandle;
use tokio_util::sync::CancellationToken;
use zorix_core::{Error, Result, StepKind, WorkflowSlug};

use crate::expr::Expr;
use crate::host::{Inner, signing_key, token_key};

/// The most a request body may be.
pub const MAX_BODY: usize = 1024 * 1024;

fn same(a: &[u8], b: &[u8]) -> bool {
    // Compared in full whatever the contents, so the time taken says nothing about how much matched.
    let mut diff = (a.len() ^ b.len()) as u8;
    for i in 0..a.len().max(b.len()) {
        diff |= a.get(i).copied().unwrap_or(0) ^ b.get(i).copied().unwrap_or(0);
    }
    diff == 0
}

fn not_found() -> Response {
    (StatusCode::NOT_FOUND, "Not found.").into_response()
}

fn signature_ok(secret: &str, body: &[u8], header: Option<&str>) -> bool {
    let Some(hex) = header.and_then(|h| h.strip_prefix("sha256=")) else { return false };
    let Some(expected) =
        (0..hex.len()).step_by(2).map(|i| u8::from_str_radix(hex.get(i..i + 2)?, 16).ok()).collect::<Option<Vec<u8>>>()
    else {
        return false;
    };
    let Ok(mut mac) = Hmac::<Sha256>::new_from_slice(secret.as_bytes()) else { return false };
    mac.update(body);
    mac.verify_slice(&expected).is_ok()
}

async fn receive(
    State(inner): State<Arc<Inner>>,
    Path((slug, token)): Path<(String, String)>,
    headers: HeaderMap,
    body: Bytes,
) -> Response {
    let Ok(slug) = WorkflowSlug::new(slug) else { return not_found() };
    let store = inner.engine.store();
    let Ok(Some(workflow)) = store.workflow_by_slug(&inner.tenant, &slug).await else { return not_found() };
    let expected = match inner.secret(token_key(workflow.id)).await {
        Ok(Some(secret)) => secret,
        _ => return not_found(),
    };
    if !same(expected.as_bytes(), token.as_bytes()) || !workflow.enabled {
        return not_found();
    }
    let Ok(live) = inner.workflows(inner.clock.now(), true).await else {
        return StatusCode::INTERNAL_SERVER_ERROR.into_response();
    };
    let Some((workflow, spec)) = live.into_iter().find(|(w, _)| w.id == workflow.id) else { return not_found() };
    let hooks: Vec<_> = spec
        .triggers()
        .filter_map(|s| match &s.kind {
            StepKind::Webhook { filter, signed, .. } => Some((s.id.clone(), filter.clone(), *signed)),
            _ => None,
        })
        .collect();
    if hooks.is_empty() {
        return not_found();
    }
    let payload: Value = if body.is_empty() {
        json!({})
    } else {
        serde_json::from_slice(&body).unwrap_or_else(|_| json!({"body": String::from_utf8_lossy(&body)}))
    };
    let mut started = Vec::new();
    for (step, filter, signed) in hooks {
        if signed {
            let secret = inner.secret(signing_key(workflow.id)).await.ok().flatten();
            let header = headers.get("x-hub-signature-256").and_then(|v| v.to_str().ok());
            if !secret.is_some_and(|s| signature_ok(&s, &body, header)) {
                return (StatusCode::UNAUTHORIZED, "The signature is missing or wrong.").into_response();
            }
        }
        if let Some(filter) = filter {
            match Expr::parse(&filter) {
                Ok(expr) if expr.test(&payload) => {}
                _ => continue,
            }
        }
        if let Some(run) = inner.fire(&workflow, &step, payload.clone(), 0, None).await {
            started.push(run.id.to_string());
        }
    }
    if started.is_empty() {
        return (StatusCode::OK, axum::Json(json!({"ignored": true}))).into_response();
    }
    (StatusCode::ACCEPTED, axum::Json(json!({"runs": started}))).into_response()
}

/// Listen on `address` until `cancel` fires. Returns the address actually bound (useful with port 0).
pub(crate) async fn serve(
    inner: Arc<Inner>,
    address: SocketAddr,
    cancel: CancellationToken,
    tasks: &Mutex<Vec<JoinHandle<()>>>,
) -> Result<SocketAddr> {
    let listener = tokio::net::TcpListener::bind(address)
        .await
        .map_err(|e| Error::Internal(format!("Webhooks could not listen on {address}: {e}")))?;
    let bound = listener.local_addr().map_err(|e| Error::Internal(e.to_string()))?;
    let app = Router::new()
        .route("/hooks/{workflow}/{token}", post(receive))
        .layer(DefaultBodyLimit::max(MAX_BODY))
        .with_state(inner);
    let task = tokio::spawn(async move {
        let served = axum::serve(listener, app).with_graceful_shutdown(async move { cancel.cancelled().await });
        if let Err(error) = served.await {
            tracing::warn!("the webhook listener stopped: {error}");
        }
    });
    tasks.lock().push(task);
    Ok(bound)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_signature_is_checked_against_the_body() {
        let secret = "s3cret";
        let body = br#"{"action":"labeled"}"#;
        let mut mac = Hmac::<Sha256>::new_from_slice(secret.as_bytes()).unwrap();
        mac.update(body);
        let hex: String = mac.finalize().into_bytes().iter().map(|b| format!("{b:02x}")).collect();
        let header = format!("sha256={hex}");
        assert!(signature_ok(secret, body, Some(&header)));
        assert!(!signature_ok(secret, b"other", Some(&header)));
        assert!(!signature_ok("wrong", body, Some(&header)));
        assert!(!signature_ok(secret, body, None));
        assert!(!signature_ok(secret, body, Some("sha256=zz")));
        assert!(!signature_ok(secret, body, Some(&hex)));
    }

    #[test]
    fn comparison_looks_at_every_byte() {
        assert!(same(b"abc", b"abc"));
        assert!(!same(b"abc", b"abd"));
        assert!(!same(b"abc", b"abcd"));
        assert!(!same(b"", b"a"));
    }
}
