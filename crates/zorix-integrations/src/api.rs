//! A small client for the JSON APIs of the services ZORIX connects to (Git hosts, Jira, Slack, Discord).
//!
//! The address comes from a connection a person set up (or a built-in default), so it is trusted in a way an address chosen by a
//! model is not: system proxy settings apply and private addresses are allowed (a company's own GitHub Enterprise). It still
//! uses HTTPS (plain HTTP only to this computer), never follows a redirect, limits what it reads and time spent, and never puts a
//! secret in an address or in anything it returns.

use std::time::Duration;

use reqwest::{Client, Method, Url};
use serde_json::Value;
use zorix_core::{ToolError, redact_text};
use zorix_providers::is_loopback;

/// How long a call may take.
pub const TIMEOUT: Duration = Duration::from_secs(30);
/// The most of a response that is read.
pub const MAX_BODY: usize = 2 * 1024 * 1024;

fn failed(message: impl Into<String>) -> ToolError {
    ToolError::Failed(message.into())
}

/// A client for one service.
pub struct Api {
    client: Client,
    base: Url,
    headers: Vec<(String, String)>,
    service: String,
}

/// What came back.
pub struct Reply {
    /// The status code.
    pub status: u16,
    /// The headers worth keeping, lowercase names.
    pub headers: Vec<(String, String)>,
    /// The body, read up to [`MAX_BODY`].
    pub body: Vec<u8>,
}

impl Reply {
    /// A header by lowercase name.
    pub fn header(&self, name: &str) -> Option<&str> {
        self.headers.iter().find(|(n, _)| n == name).map(|(_, v)| v.as_str())
    }

    /// The body as JSON.
    pub fn json(&self) -> Result<Value, ToolError> {
        serde_json::from_slice(&self.body).map_err(|_| failed("The service answered something that is not JSON."))
    }
}

/// Check an address a person gave for a service: HTTPS, or plain HTTP to this computer, no credentials inside.
pub fn validate_base(text: &str) -> Result<Url, ToolError> {
    let url = Url::parse(text.trim().trim_end_matches('/'))
        .map_err(|_| failed(format!("“{text}” is not a valid address.")))?;
    let host = url.host_str().ok_or_else(|| failed("The address has no host."))?;
    if !url.username().is_empty() || url.password().is_some() {
        return Err(failed("The address must not contain a user name or password."));
    }
    match url.scheme() {
        "https" => Ok(url),
        "http" if is_loopback(host) => Ok(url),
        _ => Err(failed(format!("{host} must be reached over HTTPS (plain HTTP is only for this computer)."))),
    }
}

impl Api {
    /// A client for `base`, sending `headers` (including any credential) with every request.
    pub fn new(service: &str, base: &str, headers: Vec<(String, String)>) -> Result<Self, ToolError> {
        let base = validate_base(base)?;
        let _ = rustls::crypto::ring::default_provider().install_default();
        let client = Client::builder()
            .connect_timeout(Duration::from_secs(10))
            .user_agent(concat!("zorix/", env!("CARGO_PKG_VERSION")))
            .redirect(reqwest::redirect::Policy::none())
            .build()
            .map_err(|e| failed(format!("The HTTP client could not start: {e}")))?;
        let service = service.to_string();
        // Loopback servers are reached directly, whatever the system proxy says.
        let client = if base.host_str().is_some_and(is_loopback) {
            Client::builder()
                .connect_timeout(Duration::from_secs(10))
                .user_agent(concat!("zorix/", env!("CARGO_PKG_VERSION")))
                .redirect(reqwest::redirect::Policy::none())
                .no_proxy()
                .build()
                .map_err(|e| failed(format!("The HTTP client could not start: {e}")))?
        } else {
            client
        };
        Ok(Self { client, base, headers, service })
    }

    /// Send a request. `path` is added to the base address and may carry a query. A body is sent as JSON.
    pub async fn send(
        &self,
        method: Method,
        path: &str,
        accept: Option<&str>,
        body: Option<&Value>,
    ) -> Result<Reply, ToolError> {
        let url = Url::parse(&format!("{}{}", self.base.as_str().trim_end_matches('/'), path))
            .map_err(|_| failed("The request address is not valid."))?;
        // Whatever the path was, the request stays on the service's own host.
        if url.host_str() != self.base.host_str() || url.port_or_known_default() != self.base.port_or_known_default() {
            return Err(failed("The request would have left the service's address."));
        }
        let mut call = self.client.request(method.clone(), url).timeout(TIMEOUT);
        for (name, value) in &self.headers {
            call = call.header(name, value);
        }
        if let Some(accept) = accept {
            call = call.header("Accept", accept);
        }
        if let Some(body) = body {
            call = call.json(body);
        }
        let mut response = call.send().await.map_err(|e| {
            let unreachable = e.is_timeout() || e.is_connect();
            let text = redact_text(&e.without_url().to_string());
            if unreachable {
                ToolError::Retryable(format!("{} could not be reached: {text}", self.service))
            } else {
                failed(format!("The request to {} failed: {text}", self.service))
            }
        })?;
        let status = response.status().as_u16();
        let headers: Vec<(String, String)> = response
            .headers()
            .iter()
            .filter(|(name, _)| {
                matches!(
                    name.as_str(),
                    "link"
                        | "x-next-page"
                        | "x-total"
                        | "x-ratelimit-remaining"
                        | "x-ratelimit-reset"
                        | "retry-after"
                        | "content-type"
                        | "x-github-request-id"
                        | "x-ratelimit-limit"
                )
            })
            .filter_map(|(name, value)| value.to_str().ok().map(|v| (name.to_string(), v.to_string())))
            .collect();
        let mut body = Vec::new();
        while let Some(chunk) = response
            .chunk()
            .await
            .map_err(|e| failed(format!("Reading the answer failed: {}", redact_text(&e.without_url().to_string()))))?
        {
            if body.len() + chunk.len() > MAX_BODY {
                return Err(failed("The service answered with more than 2 MB, which is more than ZORIX reads."));
            }
            body.extend_from_slice(&chunk);
        }
        Ok(Reply { status, headers, body })
    }

    /// Turn a non-success status into an error a model (and a person) can act on.
    pub fn check(&self, reply: &Reply, what: &str) -> Result<(), ToolError> {
        if (200..300).contains(&reply.status) {
            return Ok(());
        }
        let said = serde_json::from_slice::<Value>(&reply.body)
            .ok()
            .and_then(|v| {
                v.get("message")
                    .or_else(|| v.get("error"))
                    .or_else(|| v.get("error_description"))
                    .and_then(Value::as_str)
                    .map(str::to_string)
            })
            .map(|m| redact_text(&m).chars().take(300).collect::<String>())
            .unwrap_or_default();
        let service = &self.service;
        let remaining_zero = reply.header("x-ratelimit-remaining") == Some("0");
        Err(match reply.status {
            401 => failed(format!(
                "{service} did not accept the connection's token. A person needs to replace it ({said})."
            )),
            403 | 429 if remaining_zero || reply.status == 429 || reply.header("retry-after").is_some() => {
                let when = reply
                    .header("retry-after")
                    .map(|s| format!("in {s} seconds"))
                    .or_else(|| reply.header("x-ratelimit-reset").map(|t| format!("at {t} (Unix time)")))
                    .unwrap_or_else(|| "later".into());
                ToolError::Retryable(format!("{service} is limiting requests; try again {when}."))
            }
            403 => failed(format!("{service} refused {what}: the token may not be allowed to ({said}).")),
            404 => failed(format!("{service} has no {what}, or the token cannot see it.")),
            422 | 400 => failed(format!("{service} rejected {what}: {said}")),
            500..=599 => ToolError::Retryable(format!("{service} had a problem ({}). Try again later.", reply.status)),
            other => failed(format!("{service} answered {other} for {what}: {said}")),
        })
    }
}
