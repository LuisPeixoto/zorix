//! `http.get`, `http.request` and `webhook.send` (ADR-0021).
//!
//! A call can only reach hosts a person granted (`network` on `http:<host>`), never a private or special address unless that
//! exact host is granted, never over plain HTTP except to this computer, and never with a credential the model supplied: secrets
//! come from a connection, are chosen by host, and do not follow a redirect to another host.

use std::sync::Arc;
use std::time::Duration;

use async_trait::async_trait;
use hmac::{Hmac, Mac};
use reqwest::{Method, Url};
use serde_json::{Value, json};
use sha2::Sha256;
use zorix_core::{
    Action, Clock, Connection, ConnectionKind, IntegrationStore, Permission, SystemClock, Tool, ToolAnnotations,
    ToolContext, ToolError, ToolSource, ToolSpec, redact_text,
};
use zorix_providers::{SecretStore, get_async, is_loopback};

use crate::net::{NetError, resolve};
use crate::secret_name;

/// How long a whole call may take.
pub const TIMEOUT: Duration = Duration::from_secs(30);
/// The most of a response that is read.
pub const MAX_RESPONSE_BYTES: usize = 2 * 1024 * 1024;
/// The most redirects followed.
pub const MAX_REDIRECTS: usize = 5;
/// The most text of a response given to the model unless it asks for less or more (up to [`MAX_CHARS`]).
pub const DEFAULT_CHARS: usize = 20_000;
/// The most text of a response that can be asked for.
pub const MAX_CHARS: usize = 100_000;
/// The largest request body.
pub const MAX_REQUEST_BYTES: usize = 1024 * 1024;
const MAX_HEADERS: usize = 20;

fn failed(message: impl Into<String>) -> ToolError {
    ToolError::Failed(message.into())
}

/// What the tools need from the rest of ZORIX.
pub struct Env {
    store: Arc<dyn IntegrationStore>,
    secrets: Arc<dyn SecretStore>,
}

fn args_error(error: &str) -> ToolError {
    failed(format!("The arguments are not valid: {error}"))
}

/// Headers a model may not set: the ones that carry credentials or change how the connection works.
const FORBIDDEN_HEADER_PARTS: &[&str] =
    &["auth", "token", "secret", "cookie", "password", "key", "credential", "session"];
const FORBIDDEN_HEADERS: &[&str] = &[
    "host",
    "content-length",
    "connection",
    "transfer-encoding",
    "upgrade",
    "te",
    "trailer",
    "expect",
    "proxy-connection",
    "keep-alive",
];

fn clean_headers(value: Option<&Value>) -> Result<Vec<(String, String)>, ToolError> {
    let Some(object) = value.and_then(Value::as_object) else { return Ok(Vec::new()) };
    if object.len() > MAX_HEADERS {
        return Err(args_error(&format!("at most {MAX_HEADERS} headers")));
    }
    let mut headers = Vec::new();
    for (name, value) in object {
        let lower = name.to_ascii_lowercase();
        if name.is_empty() || !name.chars().all(|c| c.is_ascii_alphanumeric() || c == '-') {
            return Err(args_error(&format!("“{name}” is not a header name")));
        }
        if FORBIDDEN_HEADERS.contains(&lower.as_str()) || FORBIDDEN_HEADER_PARTS.iter().any(|p| lower.contains(p)) {
            return Err(failed(format!(
                "The header {name} cannot be set by a tool call: credentials come from a connection a person set up, not from the request."
            )));
        }
        let text = value.as_str().ok_or_else(|| args_error(&format!("the header {name} must be text")))?;
        if text.len() > 1024 || text.chars().any(char::is_control) {
            return Err(args_error(&format!("the value of {name} is too long or has control characters")));
        }
        headers.push((name.clone(), text.to_string()));
    }
    Ok(headers)
}

/// The host of `url` in lowercase, and whether this call may go there.
fn check_target(ctx: &ToolContext, url: &Url) -> Result<(String, bool), ToolError> {
    let host = url.host_str().ok_or_else(|| failed("The address has no host."))?.to_ascii_lowercase();
    if !url.username().is_empty() || url.password().is_some() {
        return Err(failed("The address must not contain a user name or password."));
    }
    match url.scheme() {
        "https" => {}
        "http" if is_loopback(&host) => {}
        "http" => {
            return Err(failed(format!(
                "{host} must be reached over HTTPS: plain HTTP is only allowed to this computer."
            )));
        }
        other => return Err(failed(format!("Only https addresses can be used, not {other}."))),
    }
    // A grant names the host (`http:api.example.com`, any port), or the host and one port when the address has one written
    // (`http:api.example.com:8443`): a step of a workflow names the port it was written with.
    let by_host = Permission::new(Action::Network, format!("http:{host}"));
    let by_port = url.port().map(|port| Permission::new(Action::Network, format!("http:{host}:{port}")));
    let allowed = |p: &Permission| ctx.grants.allows(p);
    if !(allowed(&by_host) || by_port.as_ref().is_some_and(allowed)) {
        return Err(failed(format!(
            "This execution has no permission to reach {host}. A person can grant it with `network http:{host}`."
        )));
    }
    // A private or special address needs the host named exactly: a wildcard does not count.
    let exact = ctx.grants.iter().any(|g| {
        g.action == Action::Network
            && (g.resource == by_host.resource || by_port.as_ref().is_some_and(|p| g.resource == p.resource))
    });
    Ok((host, exact))
}

impl Env {
    async fn connection_for(
        &self,
        tenant: &zorix_core::TenantId,
        host: &str,
        kind: ConnectionKind,
    ) -> Result<Option<Connection>, ToolError> {
        let all = self.store.connections(tenant).await.map_err(|e| failed(e.to_string()))?;
        Ok(all
            .into_iter()
            .find(|c| c.kind == kind && c.settings["host"].as_str().is_some_and(|h| h.eq_ignore_ascii_case(host))))
    }

    async fn secret(&self, connection: &Connection) -> Result<Option<String>, ToolError> {
        if !connection.has_secret {
            return Ok(None);
        }
        let found =
            get_async(self.secrets.clone(), secret_name(connection.id)).await.map_err(|e| failed(e.to_string()))?;
        Ok(found.map(|s| s.expose().to_string()))
    }

    /// The header a connection adds to requests to its host, when there is one. Only over HTTPS (or to this computer).
    async fn credential(
        &self,
        ctx: &ToolContext,
        host: &str,
        url: &Url,
    ) -> Result<Option<(String, String)>, ToolError> {
        if url.scheme() != "https" && !is_loopback(host) {
            return Ok(None);
        }
        let Some(connection) = self.connection_for(&ctx.tenant, host, ConnectionKind::Http).await? else {
            return Ok(None);
        };
        let Some(secret) = self.secret(&connection).await? else { return Ok(None) };
        Ok(Some(match connection.settings["auth"].as_str().unwrap_or("bearer") {
            "bearer" => ("Authorization".to_string(), format!("Bearer {secret}")),
            "header" => (connection.settings["header"].as_str().unwrap_or("X-Api-Key").to_string(), secret),
            "basic" => {
                let user = connection.settings["username"].as_str().unwrap_or("");
                (
                    "Authorization".to_string(),
                    format!(
                        "Basic {}",
                        base64::Engine::encode(
                            &base64::engine::general_purpose::STANDARD,
                            format!("{user}:{secret}").as_bytes()
                        )
                    ),
                )
            }
            _ => return Ok(None),
        }))
    }
}

fn is_text(content_type: &str) -> bool {
    let kind = content_type.split(';').next().unwrap_or("").trim().to_ascii_lowercase();
    kind.is_empty()
        || kind.starts_with("text/")
        || kind.ends_with("json")
        || kind.ends_with("xml")
        || kind.contains("javascript")
        || kind.contains("yaml")
        || kind == "application/x-www-form-urlencoded"
}

struct Request {
    method: Method,
    url: Url,
    headers: Vec<(String, String)>,
    body: Option<Vec<u8>>,
}

/// Send a request, following redirects of GET and HEAD by hand, each hop checked again.
async fn fetch(
    env: &Env,
    ctx: &ToolContext,
    request: Request,
    follow: bool,
    max_chars: usize,
) -> Result<Value, ToolError> {
    let started = std::time::Instant::now();
    let mut url = request.url.clone();
    let mut via: Vec<String> = Vec::new();
    let mut not_followed: Option<Value> = None;
    for hop in 0..=MAX_REDIRECTS {
        let (host, private_ok) = check_target(ctx, &url)?;
        let port = url.port_or_known_default().unwrap_or(443);
        let addresses = resolve(&host, port, private_ok).await.map_err(|e: NetError| failed(e.to_string()))?;
        let _ = rustls::crypto::ring::default_provider().install_default();
        // Always direct: a proxy would resolve names itself and could be sent anywhere, which is what the checks are for.
        let mut builder = reqwest::Client::builder()
            .connect_timeout(Duration::from_secs(10))
            .user_agent(concat!("zorix/", env!("CARGO_PKG_VERSION")))
            .redirect(reqwest::redirect::Policy::none())
            .no_proxy();
        if host.trim_start_matches('[').trim_end_matches(']').parse::<std::net::IpAddr>().is_err() {
            builder = builder.resolve_to_addrs(&host, &addresses);
        }
        let client = builder.build().map_err(|e| failed(format!("The HTTP client could not start: {e}")))?;
        let mut call = client.request(request.method.clone(), url.clone());
        for (name, value) in &request.headers {
            call = call.header(name, value);
        }
        if let Some((name, value)) = env.credential(ctx, &host, &url).await? {
            call = call.header(name, value);
        }
        if let Some(body) = &request.body {
            call = call.body(body.clone());
        }
        let remaining = TIMEOUT.saturating_sub(started.elapsed());
        let work = async {
            let mut response = call.timeout(remaining.max(Duration::from_secs(1))).send().await?;
            let status = response.status().as_u16();
            let headers: Vec<(String, String)> = response
                .headers()
                .iter()
                .filter(|(name, _)| {
                    matches!(
                        name.as_str(),
                        "content-type"
                            | "content-length"
                            | "location"
                            | "retry-after"
                            | "etag"
                            | "last-modified"
                            | "link"
                    )
                })
                .filter_map(|(name, value)| value.to_str().ok().map(|v| (name.to_string(), v.to_string())))
                .collect();
            let mut body = Vec::new();
            let mut truncated = false;
            if request.method != Method::HEAD {
                while let Some(chunk) = response.chunk().await? {
                    if body.len() + chunk.len() > MAX_RESPONSE_BYTES {
                        body.extend_from_slice(&chunk[..MAX_RESPONSE_BYTES - body.len()]);
                        truncated = true;
                        break;
                    }
                    body.extend_from_slice(&chunk);
                }
            }
            Ok::<_, reqwest::Error>((status, headers, body, truncated))
        };
        let (status, headers, body, mut truncated) = tokio::select! {
            result = work => result.map_err(|e| {
                let unreachable = e.is_timeout() || e.is_connect();
                let text = redact_text(&e.without_url().to_string());
                if unreachable { ToolError::Retryable(format!("{host} could not be reached: {text}")) } else { failed(format!("The request failed: {text}")) }
            })?,
            _ = ctx.cancel.cancelled() => return Err(failed("The call was cancelled.")),
        };
        let header = |name: &str| headers.iter().find(|(n, _)| n == name).map(|(_, v)| v.as_str());
        if follow
            && matches!(status, 301 | 302 | 303 | 307 | 308)
            && let Some(location) = header("location")
        {
            let next =
                url.join(location).map_err(|_| failed("The server redirected to an address that is not valid."))?;
            // Each hop must be somewhere this call may go; if not, the redirect is reported instead of followed.
            match check_target(ctx, &next) {
                Ok(_) if hop == MAX_REDIRECTS => {
                    return Err(failed(format!("The server redirected more than {MAX_REDIRECTS} times.")));
                }
                Ok(_) => {
                    via.push(url.to_string());
                    url = next;
                    continue;
                }
                Err(why) => not_followed = Some(json!({"to": next.as_str(), "why": why.to_string()})),
            }
        }
        let content_type = header("content-type").unwrap_or("").to_string();
        let (text, note) = if body.is_empty() {
            (Some(String::new()), None)
        } else if is_text(&content_type) {
            let all = String::from_utf8_lossy(&body);
            let kept: String = all.chars().take(max_chars).collect();
            truncated |= kept.len() < all.len();
            (Some(kept), None)
        } else {
            (
                None,
                Some(format!(
                    "The response is {} ({} bytes) and is not shown.",
                    if content_type.is_empty() { "of an unknown type" } else { &content_type },
                    body.len()
                )),
            )
        };
        let mut shown: serde_json::Map<String, Value> = headers.iter().map(|(n, v)| (n.clone(), json!(v))).collect();
        shown.remove("content-length");
        let mut result = json!({
            "status": status,
            "ok": (200..300).contains(&status),
            "url": url.as_str(),
            "headers": shown,
            "body": text,
            "bytes": body.len(),
            "truncated": truncated,
        });
        if let Some(note) = note {
            result["note"] = json!(note);
        }
        if !via.is_empty() {
            result["redirected_via"] = json!(via);
        }
        if let Some(stopped) = not_followed {
            result["redirect_not_followed"] = stopped;
        } else if matches!(status, 301 | 302 | 303 | 307 | 308) && !follow {
            result["note"] =
                json!("The server redirected, and a call that is not a GET or HEAD does not follow redirects.");
        }
        return Ok(result);
    }
    Err(failed(format!("The server redirected more than {MAX_REDIRECTS} times.")))
}

fn parse_url(args: &Value) -> Result<Url, ToolError> {
    let text = args.get("url").and_then(Value::as_str).ok_or_else(|| args_error("url is missing"))?;
    Url::parse(text).map_err(|_| failed(format!("“{text}” is not a valid address.")))
}

fn chars(args: &Value) -> usize {
    args.get("max_chars").and_then(Value::as_u64).map(|n| (n as usize).clamp(1, MAX_CHARS)).unwrap_or(DEFAULT_CHARS)
}

fn short(url: &Url) -> String {
    format!("{}{}", url.host_str().unwrap_or(""), url.path())
}

fn headers_schema() -> Value {
    json!({"type": "object", "additionalProperties": {"type": "string"}, "description": "Extra request headers (Accept, Content-Type…). Credentials cannot be set here: they come from a connection a person set up for the host."})
}

fn base_spec(id: &str, name: &str, description: &str, schema: Value) -> ToolSpec {
    let mut spec = ToolSpec::new(id, name, description, schema);
    spec.source = ToolSource::Integration { connection: None };
    spec.timeout_ms = TIMEOUT.as_millis() as u64 + 5_000;
    spec.max_output_bytes = 256 * 1024;
    spec
}

/// `http.get`: read a web address.
pub struct HttpGet {
    env: Arc<Env>,
    spec: ToolSpec,
}

/// `http.request`: send any request. Always asks first.
pub struct HttpRequest {
    env: Arc<Env>,
    spec: ToolSpec,
}

/// `webhook.send`: post a signed event to a URL a person set up. Always asks first.
pub struct WebhookSend {
    env: Arc<Env>,
    spec: ToolSpec,
}

/// The HTTP tools.
pub fn http_tools(store: Arc<dyn IntegrationStore>, secrets: Arc<dyn SecretStore>) -> Vec<Arc<dyn Tool>> {
    let env = Arc::new(Env { store, secrets });
    let mut get = base_spec(
        "http.get",
        "Read a web address",
        "Fetch a web address with GET and return its status, a few headers and its text. Only hosts a person granted can be reached, over HTTPS. Use it to read documentation or an API. It cannot sign in unless a connection was set up for the host.",
        json!({"type": "object", "properties": {
            "url": {"type": "string", "minLength": 1, "maxLength": 2048, "description": "The https address."},
            "headers": headers_schema(),
            "max_chars": {"type": "integer", "minimum": 1, "maximum": MAX_CHARS, "description": "The most characters of the response to return."}
        }, "required": ["url"], "additionalProperties": false}),
    );
    get.annotations = ToolAnnotations { read_only: true, destructive: false, idempotent: true, open_world: true };
    let mut request = base_spec(
        "http.request",
        "Send a web request",
        "Send a request with any method (POST, PUT, PATCH, DELETE…) to a host a person granted, over HTTPS. A person approves every call. Give a body as text, or JSON as `json`.",
        json!({"type": "object", "properties": {
            "method": {"type": "string", "enum": ["GET", "HEAD", "POST", "PUT", "PATCH", "DELETE"]},
            "url": {"type": "string", "minLength": 1, "maxLength": 2048},
            "headers": headers_schema(),
            "body": {"type": "string", "maxLength": MAX_REQUEST_BYTES, "description": "The request body as text."},
            "json": {"description": "The request body as JSON. Sets Content-Type to application/json."},
            "max_chars": {"type": "integer", "minimum": 1, "maximum": MAX_CHARS}
        }, "required": ["method", "url"], "additionalProperties": false}),
    );
    request.requires_approval = true;
    request.annotations = ToolAnnotations { read_only: false, destructive: true, idempotent: false, open_world: true };
    let mut webhook = base_spec(
        "webhook.send",
        "Send a webhook",
        "Post an event to a webhook a person set up (a connection of kind webhook), signed so the receiver can check it came from ZORIX. The address is fixed by the connection. A person approves every call.",
        json!({"type": "object", "properties": {
            "connection": {"type": "string", "minLength": 1, "maxLength": 32, "description": "The name of the webhook connection."},
            "event": {"type": "string", "pattern": "^[a-z0-9._-]{1,64}$", "description": "The event name, such as `deploy.finished`."},
            "payload": {"type": "object", "description": "The event data."}
        }, "required": ["connection", "event", "payload"], "additionalProperties": false}),
    );
    webhook.requires_approval = true;
    webhook.annotations = ToolAnnotations { read_only: false, destructive: false, idempotent: false, open_world: true };
    vec![
        Arc::new(HttpGet { env: env.clone(), spec: get }),
        Arc::new(HttpRequest { env: env.clone(), spec: request }),
        Arc::new(WebhookSend { env, spec: webhook }),
    ]
}

#[async_trait]
impl Tool for HttpGet {
    fn spec(&self) -> &ToolSpec {
        &self.spec
    }

    async fn call(&self, ctx: &ToolContext, args: Value) -> Result<Value, ToolError> {
        let request = Request {
            method: Method::GET,
            url: parse_url(&args)?,
            headers: clean_headers(args.get("headers"))?,
            body: None,
        };
        fetch(&self.env, ctx, request, true, chars(&args)).await
    }

    fn describe_call(&self, args: &Value) -> Option<String> {
        parse_url(args).ok().map(|u| format!("GET {}", short(&u)))
    }
}

#[async_trait]
impl Tool for HttpRequest {
    fn spec(&self) -> &ToolSpec {
        &self.spec
    }

    async fn call(&self, ctx: &ToolContext, args: Value) -> Result<Value, ToolError> {
        let method = match args.get("method").and_then(Value::as_str) {
            Some(m @ ("GET" | "HEAD" | "POST" | "PUT" | "PATCH" | "DELETE")) => {
                Method::from_bytes(m.as_bytes()).map_err(|_| args_error("method is not valid"))?
            }
            _ => return Err(args_error("method must be GET, HEAD, POST, PUT, PATCH or DELETE")),
        };
        let mut headers = clean_headers(args.get("headers"))?;
        let body = match (args.get("body"), args.get("json")) {
            (Some(_), Some(_)) => return Err(args_error("give a body or json, not both")),
            (Some(text), None) => {
                Some(text.as_str().ok_or_else(|| args_error("body must be text"))?.as_bytes().to_vec())
            }
            (None, Some(value)) => {
                if !headers.iter().any(|(n, _)| n.eq_ignore_ascii_case("content-type")) {
                    headers.push(("Content-Type".into(), "application/json".into()));
                }
                Some(value.to_string().into_bytes())
            }
            (None, None) => None,
        };
        if body.as_ref().is_some_and(|b| b.len() > MAX_REQUEST_BYTES) {
            return Err(args_error("the body is larger than 1 MB"));
        }
        let follow = method == Method::GET || method == Method::HEAD;
        fetch(&self.env, ctx, Request { method, url: parse_url(&args)?, headers, body }, follow, chars(&args)).await
    }

    fn describe_call(&self, args: &Value) -> Option<String> {
        let url = parse_url(args).ok()?;
        Some(format!("{} {}", args.get("method").and_then(Value::as_str).unwrap_or("?"), short(&url)))
    }
}

type HmacSha256 = Hmac<Sha256>;

/// `sha256=<hex>` of the HMAC of `timestamp.body`, as a receiver would recompute it.
pub fn signature(secret: &str, timestamp: i64, body: &str) -> String {
    let mut mac = HmacSha256::new_from_slice(secret.as_bytes()).expect("HMAC accepts keys of any size");
    mac.update(format!("{timestamp}.{body}").as_bytes());
    let bytes = mac.finalize().into_bytes();
    format!("sha256={}", bytes.iter().map(|b| format!("{b:02x}")).collect::<String>())
}

#[async_trait]
impl Tool for WebhookSend {
    fn spec(&self) -> &ToolSpec {
        &self.spec
    }

    async fn call(&self, ctx: &ToolContext, args: Value) -> Result<Value, ToolError> {
        let name = args.get("connection").and_then(Value::as_str).ok_or_else(|| args_error("connection is missing"))?;
        let connection = self
            .env
            .store
            .connection_named(&ctx.tenant, name)
            .await
            .map_err(|e| failed(e.to_string()))?
            .filter(|c| c.kind == ConnectionKind::Webhook)
            .ok_or_else(|| failed(format!("There is no webhook connection called {name}.")))?;
        let url = Url::parse(connection.settings["url"].as_str().unwrap_or(""))
            .map_err(|_| failed("The webhook connection has no valid address."))?;
        let secret = self
            .env
            .secret(&connection)
            .await?
            .ok_or_else(|| failed(format!("The webhook {name} has no signing key stored.")))?;
        let event = args.get("event").and_then(Value::as_str).unwrap_or("");
        let body = json!({"event": event, "payload": args.get("payload").cloned().unwrap_or(Value::Null)}).to_string();
        if body.len() > MAX_REQUEST_BYTES {
            return Err(args_error("the payload is larger than 1 MB"));
        }
        let timestamp = SystemClock.now().millis() / 1000;
        let headers = vec![
            ("Content-Type".to_string(), "application/json".to_string()),
            ("X-Zorix-Event".to_string(), event.to_string()),
            ("X-Zorix-Timestamp".to_string(), timestamp.to_string()),
            ("X-Zorix-Signature".to_string(), signature(&secret, timestamp, &body)),
        ];
        let mut result = fetch(
            &self.env,
            ctx,
            Request { method: Method::POST, url, headers, body: Some(body.into_bytes()) },
            false,
            2_000,
        )
        .await?;
        // The receiver's answer is shown; the signing key never leaves this function.
        result["connection"] = json!(name);
        Ok(result)
    }

    fn describe_call(&self, args: &Value) -> Option<String> {
        Some(format!(
            "Send the event {} to the webhook {}",
            args.get("event").and_then(Value::as_str).unwrap_or("?"),
            args.get("connection").and_then(Value::as_str).unwrap_or("?")
        ))
    }
}
