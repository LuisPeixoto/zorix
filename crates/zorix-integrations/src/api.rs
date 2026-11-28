//! A small client for the JSON APIs of the services ZORIX connects to (Git hosts, Jira, Slack, Discord).
//!
//! The address comes from a connection a person set up (or a built-in default), so it is trusted in a way an address chosen by a
//! model is not: system proxy settings apply and private addresses are allowed (a company's own GitHub Enterprise). It still
//! uses HTTPS (plain HTTP only to this computer), never follows a redirect, limits what it reads and time spent, never puts a
//! secret in an address or in anything it returns, and never sends a token to an address where no service lives (the cloud metadata
//! address and the other link-local, multicast and reserved ranges), whether written as an address or as a name that resolves to one.

use std::net::{IpAddr, SocketAddr};
use std::time::Duration;

use reqwest::{Client, Method, Url};
use serde_json::Value;
use zorix_core::{ToolError, redact_text};
use zorix_net::never_a_service;
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
    if let Ok(ip) = host.trim_start_matches('[').trim_end_matches(']').parse::<IpAddr>()
        && let Some(why) = never_a_service(ip)
    {
        return Err(failed(format!("{host} is {why}: no service lives there, so a connection cannot point at it.")));
    }
    match url.scheme() {
        "https" => Ok(url),
        "http" if is_loopback(host) => Ok(url),
        _ => Err(failed(format!("{host} must be reached over HTTPS (plain HTTP is only for this computer)."))),
    }
}

/// Refuse a name that resolves to an address where no service lives. A person's own network (private, loopback, shared) is fine.
fn refuse_special(host: &str, addresses: &[SocketAddr], service: &str) -> Result<(), ToolError> {
    match addresses.iter().find_map(|a| never_a_service(a.ip()).map(|why| (a.ip(), why))) {
        Some((ip, why)) => Err(failed(format!(
            "{service} is set to {host}, which leads to {why} ({ip}); no service lives there, so ZORIX will not send a token to it."
        ))),
        None => Ok(()),
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
        // A name can lead somewhere the written address does not show. When the lookup fails here the request is left to fail (or
        // to succeed through a proxy that resolves company names this computer cannot).
        if let Some(host) = self.base.host_str()
            && let Ok(addresses) =
                zorix_net::resolve(host, self.base.port_or_known_default().unwrap_or(443), true).await
        {
            refuse_special(host, &addresses, &self.service)?;
        }
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn an_address_where_no_service_lives_cannot_be_a_connection() {
        for bad in [
            "https://169.254.169.254",
            "https://169.254.169.254/latest",
            "https://[fe80::1]",
            "https://0.0.0.0",
            "https://224.0.0.1",
            "https://[::ffff:169.254.169.254]",
            "http://169.254.169.254",
        ] {
            let error = validate_base(bad).unwrap_err().to_string();
            assert!(error.contains("no service lives there") || error.contains("HTTPS"), "{bad}: {error}");
            assert!(validate_base(bad).is_err(), "{bad}");
        }
        for fine in [
            "https://api.github.com",
            "https://ghe.corp.example/api/v3",
            "https://10.1.2.3",
            "https://192.168.0.5:8443",
            "http://127.0.0.1:8080",
            "http://localhost:9000",
            "https://[::1]:8443",
        ] {
            assert!(validate_base(fine).is_ok(), "{fine}");
        }
    }

    #[test]
    fn a_name_that_leads_to_a_special_address_is_refused_and_one_on_a_private_network_is_not() {
        let at = |text: &str| -> Vec<SocketAddr> { vec![SocketAddr::new(text.parse().unwrap(), 443)] };
        let error = refuse_special("jira.corp.example", &at("169.254.169.254"), "Jira").unwrap_err().to_string();
        assert!(
            error.contains("jira.corp.example")
                && error.contains("169.254.169.254")
                && error.contains("no service lives there")
        );
        // One bad answer among good ones refuses the name.
        let mixed = vec![at("10.0.0.5")[0], at("169.254.169.254")[0]];
        assert!(refuse_special("x.example", &mixed, "Jira").is_err());
        for fine in ["10.0.0.5", "127.0.0.1", "100.64.1.2", "8.8.8.8", "fd00::1"] {
            assert!(refuse_special("x.example", &at(fine), "Jira").is_ok(), "{fine}");
        }
    }
}
