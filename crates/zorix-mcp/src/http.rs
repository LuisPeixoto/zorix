//! A server reached over Streamable HTTP (ADR-0018): every message is a POST, the answer is a JSON body or a stream of
//! server-sent events scoped to that request.
//!
//! Both eras are spoken. A modern request carries its protocol version and method in headers (mirroring the body); a
//! legacy one carries the negotiated version and, when the server gave one, its session id. A token only goes to the
//! server it was issued for: redirects are never followed.

use std::collections::VecDeque;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, AtomicI64, Ordering};
use std::time::Duration;

use async_trait::async_trait;
use futures_util::StreamExt;
use parking_lot::Mutex;
use reqwest::header::{ACCEPT, AUTHORIZATION, CONTENT_TYPE, HeaderMap, HeaderName, HeaderValue, WWW_AUTHENTICATE};
use reqwest::{Client, Response, Url};
use serde_json::{Value, json};
use tokio::sync::{broadcast, mpsc};
use tokio_util::sync::CancellationToken;
use zorix_core::redact_text;
use zorix_providers::sse::{MAX_EVENT, MAX_LINE, TooLong};
use zorix_providers::{SseParser, is_loopback};

use crate::auth::AuthProvider;
use crate::error::{METHOD_NOT_FOUND, McpError};
use crate::headers::encode_value;
use crate::transport::{LOG_BYTES, MAX_MESSAGE_BYTES, Responder, ServerEvent, Transport};
use crate::wire::{self, Incoming, RpcError};

const PROTOCOL_HEADER: &str = "mcp-protocol-version";
const SESSION_HEADER: &str = "mcp-session-id";
const META_VERSION: &str = "io.modelcontextprotocol/protocolVersion";
/// The most of an error body that is read.
const ERROR_BODY: usize = 64 * 1024;

/// Only HTTPS, or plain HTTP to this computer, and no credentials inside the address.
pub fn validate_url(text: &str) -> Result<Url, McpError> {
    let url = Url::parse(text.trim()).map_err(|e| McpError::Spawn(format!("“{text}” is not a valid address: {e}")))?;
    let host = url.host_str().ok_or_else(|| McpError::Spawn(format!("“{text}” has no host")))?;
    if !url.username().is_empty() || url.password().is_some() {
        return Err(McpError::Spawn(
            "the address must not contain a user name or password; use the sign-in instead".into(),
        ));
    }
    if url.fragment().is_some() {
        return Err(McpError::Spawn("the address must not contain a #fragment".into()));
    }
    match url.scheme() {
        "https" => Ok(url),
        "http" if is_loopback(host) => Ok(url),
        "http" => Err(McpError::Spawn(format!(
            "“{text}” uses plain HTTP. Use HTTPS (plain HTTP is only allowed to this computer)."
        ))),
        other => Err(McpError::Spawn(format!("“{text}” uses “{other}”; only https is allowed"))),
    }
}

#[derive(Default)]
struct Session {
    /// The legacy revision agreed in `initialize`.
    version: Option<String>,
    /// The session id a legacy server gave.
    id: Option<String>,
}

struct Ring {
    lines: VecDeque<String>,
    bytes: usize,
}

struct Inner {
    url: Url,
    auth: Arc<dyn AuthProvider>,
    client: Client,
    local: Client,
    next: AtomicI64,
    session: Mutex<Session>,
    events: broadcast::Sender<ServerEvent>,
    log: Mutex<Ring>,
    /// Requests of the server, handed to the task that answers them.
    asked: Mutex<Option<mpsc::UnboundedSender<(Value, String, Value)>>>,
    closed: AtomicBool,
}

/// The most bytes of one streamed answer (all its events together) that is read.
const MAX_STREAM_BYTES: usize = 16 * 1024 * 1024;

/// A remote MCP server.
pub struct HttpTransport {
    inner: Arc<Inner>,
}

fn build_client(direct: bool) -> Result<Client, McpError> {
    let _ = rustls::crypto::ring::default_provider().install_default();
    let builder = Client::builder()
        .connect_timeout(Duration::from_secs(15))
        .user_agent(concat!("zorix/", env!("CARGO_PKG_VERSION")))
        .redirect(reqwest::redirect::Policy::none());
    let builder = if direct { builder.no_proxy() } else { builder };
    builder.build().map_err(|e| McpError::Spawn(format!("could not start the HTTP client: {e}")))
}

fn header(name: &str, value: &str) -> Result<(HeaderName, HeaderValue), McpError> {
    let name = HeaderName::from_bytes(name.as_bytes())
        .map_err(|_| McpError::Protocol(format!("“{name}” is not a valid header name")))?;
    let value = HeaderValue::from_str(value)
        .map_err(|_| McpError::Protocol("a header value had characters a header cannot hold".into()))?;
    Ok((name, value))
}

impl HttpTransport {
    /// A transport for the MCP endpoint at `url`.
    pub fn new(url: &str, auth: Arc<dyn AuthProvider>) -> Result<Self, McpError> {
        let url = validate_url(url)?;
        let (events, _) = broadcast::channel(256);
        Ok(Self {
            inner: Arc::new(Inner {
                url,
                auth,
                client: build_client(false)?,
                local: build_client(true)?,
                next: AtomicI64::new(1),
                session: Mutex::new(Session::default()),
                events,
                log: Mutex::new(Ring { lines: VecDeque::new(), bytes: 0 }),
                asked: Mutex::new(None),
                closed: AtomicBool::new(false),
            }),
        })
    }
}

impl Inner {
    fn note(&self, line: String) {
        let line = redact_text(&line);
        let mut ring = self.log.lock();
        ring.bytes += line.len();
        ring.lines.push_back(line);
        while ring.bytes > LOG_BYTES {
            match ring.lines.pop_front() {
                Some(old) => ring.bytes -= old.len(),
                None => break,
            }
        }
    }

    fn client(&self) -> &Client {
        if self.url.host_str().is_some_and(is_loopback) { &self.local } else { &self.client }
    }

    /// The headers every request carries, besides its own.
    async fn base_headers(&self, method: &str, params: &Value) -> Result<HeaderMap, McpError> {
        let mut headers = HeaderMap::new();
        headers.insert(ACCEPT, HeaderValue::from_static("application/json, text/event-stream"));
        headers.insert(CONTENT_TYPE, HeaderValue::from_static("application/json"));
        if let Some(value) = self.auth.authorization().await? {
            let mut value = HeaderValue::from_str(&value)
                .map_err(|_| McpError::Protocol("the token has characters a header cannot hold".into()))?;
            value.set_sensitive(true);
            headers.insert(AUTHORIZATION, value);
        }
        let modern = params.get("_meta").and_then(|m| m.get(META_VERSION)).and_then(Value::as_str);
        if let Some(version) = modern {
            let (n, v) = header(PROTOCOL_HEADER, version)?;
            headers.insert(n, v);
            let (n, v) = header("mcp-method", method)?;
            headers.insert(n, v);
            let name = match method {
                "tools/call" | "prompts/get" => params.get("name").and_then(Value::as_str),
                "resources/read" => params.get("uri").and_then(Value::as_str),
                _ => None,
            };
            if let Some(name) = name {
                let (n, v) = header("mcp-name", &encode_value(name))?;
                headers.insert(n, v);
            }
        } else {
            let session = self.session.lock();
            if let Some(version) = &session.version {
                let (n, v) = header(PROTOCOL_HEADER, version)?;
                headers.insert(n, v);
            }
            if let Some(id) = &session.id {
                let (n, v) = header(SESSION_HEADER, id)?;
                headers.insert(n, v);
            }
        }
        Ok(headers)
    }

    async fn post(&self, body: String, headers: HeaderMap) -> Result<Response, McpError> {
        self.client().post(self.url.clone()).headers(headers).body(body).send().await.map_err(|e| {
            let (timed_out, connect) = (e.is_timeout(), e.is_connect());
            let detail = redact_text(&e.without_url().to_string());
            if timed_out {
                McpError::Timeout("the server did not answer in time".into())
            } else if connect {
                McpError::Transport(format!("could not connect to {}", self.url.host_str().unwrap_or("the server")))
            } else {
                McpError::Transport(detail)
            }
        })
    }

    /// Tell a legacy server what we think of something it asked, without waiting for anything.
    async fn answer(&self, line: String) {
        if let Ok(headers) = self.base_headers("", &Value::Null).await {
            let _ = self.post(line, headers).await;
        }
    }

    async fn handle(&self, message: Incoming, want: i64) -> Option<Result<Value, RpcError>> {
        match message {
            Incoming::Response { id, result } => (wire::id_number(&id) == Some(want)).then_some(result),
            Incoming::Notification { method, params } => {
                let _ = self.events.send(ServerEvent::Notification { method, params });
                None
            }
            Incoming::Request { id, method, params } => {
                let asked = self.asked.lock().clone();
                let line = match asked {
                    _ if method == "ping" => wire::result_line(&id, &json!({})),
                    // Answered elsewhere: a person may take minutes, and the stream must keep being read.
                    Some(asked) => {
                        let _ = asked.send((id, method, params));
                        return None;
                    }
                    None => {
                        self.note(format!("[ZORIX refused the server's {method}: it does not answer that]"));
                        let _ = self.events.send(ServerEvent::Refused { method: method.clone() });
                        wire::error_line(&id, METHOD_NOT_FOUND, "ZORIX does not answer requests from servers.")
                    }
                };
                self.answer(line).await;
                None
            }
        }
    }

    async fn read_capped(&self, response: Response, cap: usize) -> Result<Vec<u8>, McpError> {
        let mut body = Vec::new();
        let mut stream = response.bytes_stream();
        while let Some(chunk) = stream.next().await {
            let chunk = chunk.map_err(|e| McpError::Transport(redact_text(&e.without_url().to_string())))?;
            if body.len() + chunk.len() > cap {
                return Err(McpError::TooLarge { limit: cap });
            }
            body.extend_from_slice(&chunk);
        }
        Ok(body)
    }

    async fn fail(&self, response: Response) -> McpError {
        let status = response.status().as_u16();
        let body = self.read_capped(response, ERROR_BODY).await.unwrap_or_default();
        if let Ok(Incoming::Response { result: Err(error), .. }) = wire::parse(&String::from_utf8_lossy(&body)) {
            return error.into();
        }
        self.note(format!("HTTP {status}"));
        McpError::Transport(format!("the server answered HTTP {status}"))
    }

    /// Read the answer to request `want` from a response, whether it is one JSON body or a stream of events.
    async fn answer_to(&self, response: Response, want: i64) -> Result<Value, McpError> {
        let kind =
            response.headers().get(CONTENT_TYPE).and_then(|v| v.to_str().ok()).unwrap_or("").to_ascii_lowercase();
        if kind.starts_with("text/event-stream") {
            let mut parser = SseParser::new();
            let mut stream = response.bytes_stream();
            let mut received = 0usize;
            while let Some(chunk) = stream.next().await {
                let chunk = chunk.map_err(|e| McpError::Transport(redact_text(&e.without_url().to_string())))?;
                // However small its events, a stream that keeps going is not read for ever: the call's own timeout is not a limit on
                // how much arrives before it.
                received += chunk.len();
                if received > MAX_STREAM_BYTES {
                    return Err(McpError::TooLarge { limit: MAX_STREAM_BYTES });
                }
                let events = parser.feed(&chunk).map_err(|too_long| McpError::TooLarge {
                    limit: if too_long == TooLong::Line { MAX_LINE } else { MAX_EVENT },
                })?;
                for event in events {
                    if event.data.trim().is_empty() {
                        continue;
                    }
                    match wire::parse(&event.data) {
                        Ok(message) => {
                            if let Some(result) = self.handle(message, want).await {
                                return result.map_err(Into::into);
                            }
                        }
                        Err(e) => self.note(format!("ignored an event that was not valid: {e}")),
                    }
                }
            }
            return Err(McpError::Transport("the server ended the stream without answering".into()));
        }
        let body = self.read_capped(response, MAX_MESSAGE_BYTES).await?;
        let text = String::from_utf8_lossy(&body).into_owned();
        let values: Vec<Value> = match serde_json::from_str::<Value>(&text) {
            Ok(Value::Array(items)) => items,
            Ok(other) => vec![other],
            Err(e) => return Err(McpError::Protocol(format!("the answer was not JSON ({e})"))),
        };
        for value in values {
            let message = wire::parse(&value.to_string())?;
            if let Some(result) = self.handle(message, want).await {
                return result.map_err(Into::into);
            }
        }
        Err(McpError::Protocol("the answer did not contain a response to the request".into()))
    }

    /// A legacy server answers 404 to a request that carries a session id it no longer knows. The session is over: this connection is
    /// ended (so nothing else is sent into it and the manager starts a new one, with a new `initialize` and no session id, as the
    /// specification asks) and the call that found out says so.
    fn session_ended(&self, status: u16) -> Option<McpError> {
        if status != 404 || self.session.lock().id.is_none() {
            return None;
        }
        self.session.lock().id = None;
        self.closed.store(true, Ordering::SeqCst);
        let why = "the server ended the session; ZORIX connects again";
        let _ = self.events.send(ServerEvent::Closed(why.into()));
        Some(McpError::Transport(why.into()))
    }

    /// Tell a legacy server that a request was given up on. Best effort: the answer is not waited for, and `initialize` is never cancelled.
    async fn cancelled(&self, id: i64, method: &str, reason: &str) {
        if method == "initialize" || self.session.lock().id.is_none() {
            return;
        }
        let line = wire::notification("notifications/cancelled", &json!({"requestId": id, "reason": reason}));
        if let Ok(headers) = self.base_headers("", &Value::Null).await {
            let _ = tokio::time::timeout(Duration::from_secs(2), self.post(line, headers)).await;
        }
    }

    async fn send(&self, id: i64, method: &str, params: Value, extra: &[(String, String)]) -> Result<Value, McpError> {
        let body = wire::request(id, method, &params);
        let mut retried = false;
        loop {
            let mut headers = self.base_headers(method, &params).await?;
            for (name, value) in extra {
                let (n, v) = header(name, value)?;
                headers.insert(n, v);
            }
            let response = self.post(body.clone(), headers).await?;
            let status = response.status();
            if status.as_u16() == 401 || status.as_u16() == 403 {
                let challenge =
                    response.headers().get(WWW_AUTHENTICATE).and_then(|v| v.to_str().ok()).map(str::to_string);
                self.auth.challenged(status.as_u16(), challenge.as_deref()).await?;
                if retried {
                    return Err(McpError::NeedsLogin(format!(
                        "the server still answers {status} after signing in again"
                    )));
                }
                retried = true;
                continue;
            }
            if let Some(ended) = self.session_ended(status.as_u16()) {
                return Err(ended);
            }
            if !status.is_success() {
                return Err(self.fail(response).await);
            }
            if method == "initialize"
                && let Some(session) = response.headers().get(SESSION_HEADER).and_then(|v| v.to_str().ok())
            {
                self.session.lock().id = Some(session.to_string());
            }
            let result = self.answer_to(response, id).await?;
            if method == "initialize"
                && let Some(version) = result.get("protocolVersion").and_then(Value::as_str)
            {
                self.session.lock().version = Some(version.to_string());
            }
            return Ok(result);
        }
    }
}

#[async_trait]
impl Transport for HttpTransport {
    async fn request_with(
        &self,
        method: &str,
        params: Value,
        headers: &[(String, String)],
        timeout: Duration,
        cancel: &CancellationToken,
    ) -> Result<Value, McpError> {
        if self.inner.closed.load(Ordering::SeqCst) {
            return Err(McpError::Closed);
        }
        let id = self.inner.next.fetch_add(1, Ordering::SeqCst);
        let work = self.inner.send(id, method, params, headers);
        // Dropping the request closes its stream, which cancels it for a server without a session; a server with one is told as well.
        let gave_up = |reason: &'static str| {
            let inner = self.inner.clone();
            let method = method.to_string();
            tokio::spawn(async move { inner.cancelled(id, &method, reason).await });
        };
        tokio::select! {
            result = work => result,
            _ = cancel.cancelled() => { gave_up("cancelled by ZORIX"); Err(McpError::Cancelled) }
            _ = tokio::time::sleep(timeout) => {
                gave_up("timed out");
                Err(McpError::Timeout(format!("{method} took longer than {} seconds", timeout.as_secs().max(1))))
            }
        }
    }

    async fn notify(&self, method: &str, params: Value) -> Result<(), McpError> {
        if self.inner.closed.load(Ordering::SeqCst) {
            return Err(McpError::Closed);
        }
        let headers = self.inner.base_headers(method, &params).await?;
        let response = self.inner.post(wire::notification(method, &params), headers).await?;
        if let Some(ended) = self.inner.session_ended(response.status().as_u16()) {
            return Err(ended);
        }
        if response.status().is_success() { Ok(()) } else { Err(self.inner.fail(response).await) }
    }

    fn set_responder(&self, responder: Arc<dyn Responder>) {
        let (asked, mut requests) = mpsc::unbounded_channel::<(Value, String, Value)>();
        *self.inner.asked.lock() = Some(asked);
        // Weak: the task ends with the connection, and does not keep it alive.
        let inner = Arc::downgrade(&self.inner);
        tokio::spawn(async move {
            while let Some((id, method, params)) = requests.recv().await {
                let (inner, responder) = (inner.clone(), responder.clone());
                tokio::spawn(async move {
                    let outcome = responder.respond(&method, params).await;
                    let Some(inner) = inner.upgrade() else { return };
                    let line = match outcome {
                        Ok(result) => wire::result_line(&id, &result),
                        Err((code, message)) => {
                            inner.note(format!("[ZORIX refused the server's {method}: {message}]"));
                            let _ = inner.events.send(ServerEvent::Refused { method });
                            wire::error_line(&id, code, &message)
                        }
                    };
                    inner.answer(line).await;
                });
            }
        });
    }

    fn subscribe(&self) -> broadcast::Receiver<ServerEvent> {
        self.inner.events.subscribe()
    }

    fn alive(&self) -> bool {
        !self.inner.closed.load(Ordering::SeqCst)
    }

    fn log(&self) -> Vec<String> {
        self.inner.log.lock().lines.iter().cloned().collect()
    }

    async fn close(&self) {
        if self.inner.closed.swap(true, Ordering::SeqCst) {
            return;
        }
        // A legacy server may hold a session; ending it is polite and optional.
        let session = self.inner.session.lock().id.clone();
        if let Some(id) = session
            && let Ok(mut headers) = self.inner.base_headers("", &Value::Null).await
        {
            headers.remove(CONTENT_TYPE);
            if let Ok((n, v)) = header(SESSION_HEADER, &id) {
                headers.insert(n, v);
            }
            let _ = tokio::time::timeout(
                Duration::from_secs(2),
                self.inner.client().delete(self.inner.url.clone()).headers(headers).send(),
            )
            .await;
        }
        let _ = self.inner.events.send(ServerEvent::Closed("closed".into()));
    }
}
