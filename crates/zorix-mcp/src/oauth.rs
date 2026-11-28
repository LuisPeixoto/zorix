//! Signing in to a remote MCP server (ADR-0019): OAuth 2.1 with PKCE, the `resource` parameter, issuer validation and a
//! loopback redirect, and the tokens that come out of it, kept in the keychain.
//!
//! Discovery follows the specification: the server's `WWW-Authenticate` challenge points to its protected-resource metadata
//! (RFC 9728), which names the authorization server, whose own metadata (RFC 8414 or OpenID Connect) gives the endpoints.
//! A token is only ever sent to the server it was issued for.

use std::collections::HashMap;
use std::sync::Arc;
use std::time::Duration;

use async_trait::async_trait;
use base64::Engine;
use base64::engine::general_purpose::URL_SAFE_NO_PAD;
use reqwest::{Client, Url};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use sha2::{Digest, Sha256};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::TcpListener;
use tokio::sync::Mutex;
use tokio_util::sync::CancellationToken;
use zorix_core::{Clock, Secret, redact_text};
use zorix_net::{NetError, forbidden, resolve};
use zorix_providers::{SecretStore, get_async, is_loopback};

use crate::auth::AuthProvider;
use crate::error::McpError;
use crate::protocol::modern_meta;

/// How long a person has to finish signing in.
pub const LOGIN_TIMEOUT: Duration = Duration::from_secs(300);
/// A token this close to expiring is refreshed first.
const REFRESH_MARGIN_MS: i64 = 60_000;
/// The most of a discovery or token response that is read.
const MAX_BODY: usize = 1024 * 1024;

/// What a person chose about the sign-in.
#[derive(Clone, Debug, Default)]
pub struct OAuthSettings {
    /// A client id registered in advance, for servers that do not register clients themselves.
    pub client_id: Option<String>,
    /// Scopes to ask for. Empty means what the server asks for.
    pub scopes: Vec<String>,
}

/// What is kept about a sign-in apart from the short-lived access token.
#[derive(Clone, Serialize, Deserialize)]
struct Registration {
    client_id: String,
    client_secret: Option<String>,
    issuer: String,
    token_endpoint: String,
    revocation_endpoint: Option<String>,
    resource: String,
    scope: Option<String>,
    refresh_token: Option<String>,
}

#[derive(Clone, Serialize, Deserialize)]
struct Access {
    token: String,
    expires_at_ms: Option<i64>,
}

/// Where the tokens of one server are kept: two keychain entries, so each stays under the keychain's size limit.
#[derive(Clone)]
pub struct Vault {
    secrets: Arc<dyn SecretStore>,
    registration: String,
    access: String,
}

impl Vault {
    /// The vault for the server whose entries are `<base>.oauth` and `<base>.access`.
    pub fn new(secrets: Arc<dyn SecretStore>, base: &str) -> Self {
        Self { secrets, registration: format!("{base}.oauth"), access: format!("{base}.access") }
    }

    async fn read<T: for<'de> Deserialize<'de>>(&self, name: &str) -> Result<Option<T>, McpError> {
        let Some(secret) =
            get_async(self.secrets.clone(), name.to_string()).await.map_err(|e| McpError::NeedsLogin(e.to_string()))?
        else {
            return Ok(None);
        };
        let bytes = URL_SAFE_NO_PAD
            .decode(secret.expose())
            .map_err(|_| McpError::NeedsLogin("the stored sign-in is damaged; sign in again".into()))?;
        serde_json::from_slice(&bytes)
            .map(Some)
            .map_err(|_| McpError::NeedsLogin("the stored sign-in is damaged; sign in again".into()))
    }

    async fn write<T: Serialize>(&self, name: &str, value: &T) -> Result<(), McpError> {
        let text = URL_SAFE_NO_PAD.encode(serde_json::to_vec(value).map_err(|e| McpError::Protocol(e.to_string()))?);
        let secrets = self.secrets.clone();
        let name = name.to_string();
        tokio::task::spawn_blocking(move || secrets.set(&name, &Secret::new(text)))
            .await
            .map_err(|e| McpError::Protocol(e.to_string()))?
            .map_err(|e| McpError::Protocol(format!("the sign-in could not be kept: {e}")))
    }

    /// Whether anything is kept.
    pub async fn signed_in(&self) -> bool {
        matches!(self.read::<Registration>(&self.registration).await, Ok(Some(_)))
    }

    /// Forget the sign-in.
    pub async fn forget(&self) {
        let secrets = self.secrets.clone();
        let (a, b) = (self.registration.clone(), self.access.clone());
        let _ = tokio::task::spawn_blocking(move || {
            let _ = secrets.remove(&a);
            let _ = secrets.remove(&b);
        })
        .await;
    }
}

/// Opens the sign-in page for a person. The command line prints the address and tries the browser; the desktop app uses
/// its own opener.
pub trait BrowserOpener: Send + Sync {
    /// Show `url` to the person.
    fn open(&self, url: &str) -> Result<(), String>;
}

/// The system's default browser.
pub struct SystemBrowser;

impl BrowserOpener for SystemBrowser {
    fn open(&self, url: &str) -> Result<(), String> {
        #[cfg(target_os = "macos")]
        let mut command = std::process::Command::new("open");
        #[cfg(target_os = "windows")]
        let mut command = std::process::Command::new("rundll32");
        #[cfg(not(any(target_os = "macos", target_os = "windows")))]
        let mut command = std::process::Command::new("xdg-open");
        #[cfg(target_os = "windows")]
        command.arg("url.dll,FileProtocolHandler");
        command
            .arg(url)
            .stdin(std::process::Stdio::null())
            .stdout(std::process::Stdio::null())
            .stderr(std::process::Stdio::null());
        command.spawn().map(|_| ()).map_err(|e| e.to_string())
    }
}

fn random_bytes<const N: usize>() -> [u8; N] {
    let mut bytes = [0u8; N];
    getrandom::fill(&mut bytes).expect("the system has a source of randomness");
    bytes
}

/// A random URL-safe string made of `N` random bytes.
pub fn random_token<const N: usize>() -> String {
    URL_SAFE_NO_PAD.encode(random_bytes::<N>())
}

/// A PKCE verifier and its S256 challenge.
pub fn pkce_pair() -> (String, String) {
    let verifier = random_token::<32>();
    let challenge = URL_SAFE_NO_PAD.encode(Sha256::digest(verifier.as_bytes()));
    (verifier, challenge)
}

/// The address of a server as the `resource` parameter names it: no fragment or query, no trailing slash.
pub fn canonical_resource(url: &Url) -> String {
    let mut text = format!("{}://{}", url.scheme(), url.host_str().unwrap_or(""));
    if let Some(port) = url.port() {
        text.push_str(&format!(":{port}"));
    }
    let path = url.path().trim_end_matches('/');
    text.push_str(path);
    text
}

/// What a `WWW-Authenticate: Bearer …` header says.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Challenge {
    /// Where the server's protected-resource metadata is.
    pub resource_metadata: Option<String>,
    /// The scopes the failed operation needs.
    pub scope: Option<String>,
    /// The error, such as `insufficient_scope`.
    pub error: Option<String>,
}

/// Read the parameters of a `Bearer` challenge.
pub fn parse_challenge(header: &str) -> Challenge {
    let mut challenge = Challenge::default();
    let rest = header.trim();
    let rest = rest.strip_prefix("Bearer").or_else(|| rest.strip_prefix("bearer")).unwrap_or(rest).trim();
    let bytes = rest.as_bytes();
    let mut i = 0;
    while i < bytes.len() {
        while i < bytes.len() && (bytes[i] == b' ' || bytes[i] == b',') {
            i += 1;
        }
        let start = i;
        while i < bytes.len() && bytes[i] != b'=' && bytes[i] != b',' {
            i += 1;
        }
        let key = rest[start..i].trim().to_ascii_lowercase();
        if i >= bytes.len() || bytes[i] != b'=' {
            continue;
        }
        i += 1;
        let value = if i < bytes.len() && bytes[i] == b'"' {
            i += 1;
            let mut value = String::new();
            while i < bytes.len() && bytes[i] != b'"' {
                if bytes[i] == b'\\' && i + 1 < bytes.len() {
                    i += 1;
                }
                value.push(bytes[i] as char);
                i += 1;
            }
            i += 1;
            value
        } else {
            let start = i;
            while i < bytes.len() && bytes[i] != b',' {
                i += 1;
            }
            rest[start..i].trim().to_string()
        };
        match key.as_str() {
            "resource_metadata" => challenge.resource_metadata = Some(value),
            "scope" => challenge.scope = Some(value),
            "error" => challenge.error = Some(value),
            _ => {}
        }
    }
    challenge
}

/// How far a sign-in may reach. Discovery and the token calls follow addresses that the *server* names (the metadata address in its
/// challenge, the authorization server, its endpoints), so a server on the internet could otherwise send ZORIX to this computer's
/// own services, the cloud metadata address or the local network. A server that is itself on a private network (a loopback address, a
/// company's own host) may name private addresses; any other may not, and a name that resolves to one is refused as well.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Reach {
    private_ok: bool,
}

impl Reach {
    /// The reach of a sign-in to `server`: private addresses are allowed only when the server's own address is private.
    pub async fn of(server: &Url) -> Self {
        let Some(host) = server.host_str() else { return Self::internet_only() };
        let port = server.port_or_known_default().unwrap_or(443);
        let private = match resolve(host, port, true).await {
            Ok(addresses) => addresses.iter().any(|a| forbidden(a.ip()).is_some()),
            Err(_) => false,
        };
        Self { private_ok: private }
    }

    /// Public addresses only, whatever the server's own address is.
    pub fn internet_only() -> Self {
        Self { private_ok: false }
    }

    /// Whether private addresses may be reached.
    pub fn allows_private(&self) -> bool {
        self.private_ok
    }

    /// A client for one request to `url`: the name is resolved here, every answer is checked, and the connection goes to the address
    /// that was checked, so a second DNS answer cannot rebind it. Redirects are never followed.
    async fn client(&self, url: &Url) -> Result<Client, McpError> {
        let host = url.host_str().ok_or_else(|| McpError::Protocol("a sign-in address has no host".into()))?;
        let port = url.port_or_known_default().unwrap_or(443);
        let addresses = resolve(host, port, self.private_ok).await.map_err(|e| match e {
            NetError::Private { host, why, .. } => McpError::Protocol(format!(
                "the server asked ZORIX to contact {host}, which is {why}, and a server that is not on your own network may not send it there, so the sign-in was stopped"
            )),
            NetError::NotFound(host) => McpError::Transport(format!("{host} could not be found")),
        })?;
        let _ = rustls::crypto::ring::default_provider().install_default();
        let mut builder = Client::builder()
            .connect_timeout(Duration::from_secs(15))
            .user_agent(concat!("zorix/", env!("CARGO_PKG_VERSION")))
            .redirect(reqwest::redirect::Policy::none());
        if through_a_proxy() {
            // A proxy resolves names itself, so the connection cannot be pinned; the name was still checked above.
        } else {
            builder = builder.no_proxy();
            if host.trim_start_matches('[').trim_end_matches(']').parse::<std::net::IpAddr>().is_err() {
                builder = builder.resolve_to_addrs(host, &addresses);
            }
        }
        builder.build().map_err(|e| McpError::Spawn(format!("could not start the HTTP client: {e}")))
    }
}

/// Whether the environment names a proxy for web requests.
fn through_a_proxy() -> bool {
    ["HTTPS_PROXY", "https_proxy", "ALL_PROXY", "all_proxy"]
        .iter()
        .any(|v| std::env::var(v).is_ok_and(|p| !p.is_empty()))
}

fn endpoint(text: &str, what: &str) -> Result<Url, McpError> {
    let url = Url::parse(text)
        .map_err(|_| McpError::Protocol(format!("the authorization server's {what} is not a valid address")))?;
    match (url.scheme(), url.host_str()) {
        ("https", Some(_)) => Ok(url),
        ("http", Some(host)) if is_loopback(host) => Ok(url),
        _ => Err(McpError::Protocol(format!("the authorization server's {what} does not use HTTPS"))),
    }
}

async fn read_json(response: reqwest::Response) -> Result<Value, McpError> {
    let mut body = Vec::new();
    let mut response = response;
    while let Some(chunk) =
        response.chunk().await.map_err(|e| McpError::Transport(redact_text(&e.without_url().to_string())))?
    {
        if body.len() + chunk.len() > MAX_BODY {
            return Err(McpError::TooLarge { limit: MAX_BODY });
        }
        body.extend_from_slice(&chunk);
    }
    serde_json::from_slice(&body).map_err(|_| McpError::Protocol("a sign-in response was not JSON".into()))
}

async fn get_json(reach: &Reach, url: &Url) -> Result<Option<Value>, McpError> {
    let response = reach
        .client(url)
        .await?
        .get(url.clone())
        .header("Accept", "application/json")
        .timeout(Duration::from_secs(15))
        .send()
        .await
        .map_err(|e| McpError::Transport(redact_text(&e.without_url().to_string())))?;
    if !response.status().is_success() {
        return Ok(None);
    }
    read_json(response).await.map(Some)
}

/// What discovery found out about a server's sign-in.
#[derive(Clone, Debug)]
pub struct Discovered {
    /// The server, as the `resource` parameter names it.
    pub resource: String,
    /// The authorization server's issuer.
    pub issuer: String,
    /// Where a person is sent.
    pub authorization_endpoint: Url,
    /// Where codes and refresh tokens are exchanged.
    pub token_endpoint: Url,
    /// Where clients can register themselves, if the server allows it.
    pub registration_endpoint: Option<Url>,
    /// Where tokens are revoked, if the server allows it.
    pub revocation_endpoint: Option<Url>,
    /// The server includes `iss` in its authorization responses and says so.
    pub iss_supported: bool,
    /// The scopes to ask for when nothing else says.
    pub default_scope: Option<String>,
}

fn prm_urls(server: &Url, challenge: &Challenge) -> Vec<Url> {
    let mut urls = Vec::new();
    if let Some(text) = &challenge.resource_metadata
        && let Ok(url) = endpoint(text, "protected-resource metadata address")
    {
        urls.push(url);
    }
    let path = server.path().trim_end_matches('/');
    let base = format!(
        "{}://{}{}",
        server.scheme(),
        server.host_str().unwrap_or(""),
        server.port().map(|p| format!(":{p}")).unwrap_or_default()
    );
    if !path.is_empty() {
        urls.extend(Url::parse(&format!("{base}/.well-known/oauth-protected-resource{path}")));
    }
    urls.extend(Url::parse(&format!("{base}/.well-known/oauth-protected-resource")));
    urls
}

fn as_metadata_urls(issuer: &Url) -> Vec<Url> {
    let base = format!(
        "{}://{}{}",
        issuer.scheme(),
        issuer.host_str().unwrap_or(""),
        issuer.port().map(|p| format!(":{p}")).unwrap_or_default()
    );
    let path = issuer.path().trim_end_matches('/');
    let mut urls = Vec::new();
    if path.is_empty() {
        urls.extend(Url::parse(&format!("{base}/.well-known/oauth-authorization-server")));
        urls.extend(Url::parse(&format!("{base}/.well-known/openid-configuration")));
    } else {
        urls.extend(Url::parse(&format!("{base}/.well-known/oauth-authorization-server{path}")));
        urls.extend(Url::parse(&format!("{base}/.well-known/openid-configuration{path}")));
        urls.extend(Url::parse(&format!("{base}{path}/.well-known/openid-configuration")));
    }
    urls
}

/// Follow the discovery chain from a server's challenge to its authorization server's endpoints.
pub async fn discover(reach: &Reach, server: &Url, challenge: &Challenge) -> Result<Discovered, McpError> {
    let ours = canonical_resource(server);
    let mut prm = None;
    for url in prm_urls(server, challenge) {
        if let Some(found) = get_json(reach, &url).await? {
            prm = Some(found);
            break;
        }
    }
    let prm = prm.ok_or_else(|| {
        McpError::Protocol("the server does not publish its sign-in details (protected-resource metadata)".into())
    })?;
    // The metadata must be about this server, or it could be a different resource's. A server may name itself by its origin
    // (or any parent path of its endpoint); the name it gives is the one used for the `resource` parameter, as RFC 9728 says.
    let resource = match prm.get("resource").and_then(Value::as_str) {
        Some(declared) => {
            let declared_url = Url::parse(declared)
                .map_err(|_| McpError::Protocol("the server's metadata names an invalid resource".into()))?;
            let theirs = canonical_resource(&declared_url);
            if theirs != ours && !ours.starts_with(&format!("{theirs}/")) {
                return Err(McpError::Protocol(format!(
                    "the server's sign-in metadata is for {declared}, not for {ours}"
                )));
            }
            declared.to_string()
        }
        None => ours,
    };
    let issuer_text = prm
        .get("authorization_servers")
        .and_then(Value::as_array)
        .and_then(|a| a.first())
        .and_then(Value::as_str)
        .ok_or_else(|| McpError::Protocol("the server names no authorization server".into()))?
        .to_string();
    let issuer_url = endpoint(&issuer_text, "issuer")?;
    let mut meta = None;
    for url in as_metadata_urls(&issuer_url) {
        if let Some(found) = get_json(reach, &url).await? {
            meta = Some(found);
            break;
        }
    }
    let meta = meta.ok_or_else(|| {
        McpError::Protocol(format!("the authorization server {issuer_text} does not publish its metadata"))
    })?;
    if meta.get("issuer").and_then(Value::as_str) != Some(issuer_text.as_str()) {
        return Err(McpError::Protocol(
            "the authorization server's metadata names a different issuer than the one the server pointed to".into(),
        ));
    }
    let methods = meta.get("code_challenge_methods_supported").and_then(Value::as_array);
    if !methods.is_some_and(|m| m.iter().any(|v| v == "S256")) {
        return Err(McpError::Protocol(
            "the authorization server does not say it supports PKCE (S256), so ZORIX will not sign in to it".into(),
        ));
    }
    let get = |name: &str| meta.get(name).and_then(Value::as_str);
    let scopes = prm
        .get("scopes_supported")
        .and_then(Value::as_array)
        .map(|a| a.iter().filter_map(Value::as_str).collect::<Vec<_>>().join(" "));
    Ok(Discovered {
        resource,
        issuer: issuer_text.clone(),
        authorization_endpoint: endpoint(
            get("authorization_endpoint").ok_or_else(|| McpError::Protocol("no authorization endpoint".into()))?,
            "authorization endpoint",
        )?,
        token_endpoint: endpoint(
            get("token_endpoint").ok_or_else(|| McpError::Protocol("no token endpoint".into()))?,
            "token endpoint",
        )?,
        registration_endpoint: get("registration_endpoint")
            .map(|u| endpoint(u, "registration endpoint"))
            .transpose()?,
        revocation_endpoint: get("revocation_endpoint").map(|u| endpoint(u, "revocation endpoint")).transpose()?,
        iss_supported: meta.get("authorization_response_iss_parameter_supported") == Some(&Value::Bool(true)),
        default_scope: scopes.filter(|s| !s.is_empty()),
    })
}

struct Callback {
    params: HashMap<String, String>,
}

const PAGE: &str = "<!doctype html><meta charset=utf-8><title>ZORIX</title><body style=\"font-family:system-ui;margin:3rem\"><h2>You are signed in</h2><p>You can close this window and go back to ZORIX.</p>";
const FAILED_PAGE: &str = "<!doctype html><meta charset=utf-8><title>ZORIX</title><body style=\"font-family:system-ui;margin:3rem\"><h2>Sign-in did not finish</h2><p>Go back to ZORIX to see why.</p>";

async fn respond(stream: &mut tokio::net::TcpStream, status: &str, body: &str) {
    let text = format!(
        "HTTP/1.1 {status}\r\nContent-Type: text/html; charset=utf-8\r\nContent-Length: {}\r\nCache-Control: no-store\r\nReferrer-Policy: no-referrer\r\nConnection: close\r\n\r\n{body}",
        body.len()
    );
    let _ = stream.write_all(text.as_bytes()).await;
    let _ = stream.shutdown().await;
}

/// Wait for the browser to come back to `http://127.0.0.1:<port>/callback`. Anything else that knocks is turned away.
async fn wait_for_callback(
    listener: TcpListener,
    port: u16,
    state: &str,
    cancel: &CancellationToken,
) -> Result<Callback, McpError> {
    let deadline = tokio::time::sleep(LOGIN_TIMEOUT);
    tokio::pin!(deadline);
    loop {
        let (mut stream, _) = tokio::select! {
            accepted = listener.accept() => accepted.map_err(|e| McpError::Transport(e.to_string()))?,
            _ = cancel.cancelled() => return Err(McpError::Cancelled),
            _ = &mut deadline => return Err(McpError::Timeout("nobody finished signing in within five minutes".into())),
        };
        let mut head = Vec::new();
        let mut buf = [0u8; 2048];
        let read = tokio::time::timeout(Duration::from_secs(5), async {
            loop {
                let n = stream.read(&mut buf).await.unwrap_or(0);
                if n == 0 {
                    break;
                }
                head.extend_from_slice(&buf[..n]);
                if head.windows(4).any(|w| w == b"\r\n\r\n") || head.len() > 16 * 1024 {
                    break;
                }
            }
        })
        .await;
        if read.is_err() {
            continue;
        }
        let text = String::from_utf8_lossy(&head).into_owned();
        let mut lines = text.lines();
        let request_line = lines.next().unwrap_or("");
        let host = lines
            .find_map(|l| l.strip_prefix("Host:").or_else(|| l.strip_prefix("host:")))
            .map(str::trim)
            .unwrap_or("");
        let target = request_line.strip_prefix("GET ").and_then(|r| r.split(' ').next()).unwrap_or("");
        let host_ok = host == format!("127.0.0.1:{port}") || host == format!("localhost:{port}");
        if !host_ok || !(target == "/callback" || target.starts_with("/callback?")) {
            respond(&mut stream, "404 Not Found", "").await;
            continue;
        }
        let params: HashMap<String, String> = Url::parse(&format!("http://127.0.0.1{target}"))
            .map(|u| u.query_pairs().map(|(k, v)| (k.into_owned(), v.into_owned())).collect())
            .unwrap_or_default();
        // Only the answer to *this* sign-in ends the wait: its `state` is a number nobody else has. Another program on this computer
        // that knocks on the port cannot cut a sign-in short, and a person is not told they are signed in by a stranger's request. An
        // error that carries no state is shown the failure page, and the wait goes on until the person cancels or the time is up.
        if params.get("state").map(String::as_str) != Some(state) {
            let page = if params.contains_key("error") { FAILED_PAGE } else { "" };
            respond(&mut stream, if page.is_empty() { "400 Bad Request" } else { "200 OK" }, page).await;
            continue;
        }
        respond(&mut stream, "200 OK", if params.contains_key("code") { PAGE } else { FAILED_PAGE }).await;
        return Ok(Callback { params });
    }
}

/// The answer of a token endpoint.
struct TokenResponse {
    access_token: String,
    refresh_token: Option<String>,
    expires_in: Option<i64>,
    scope: Option<String>,
}

/// A form body, `application/x-www-form-urlencoded`.
fn form_body(pairs: &[(&str, String)]) -> String {
    let mut url = Url::parse("http://form.invalid/").expect("a constant address");
    url.query_pairs_mut().extend_pairs(pairs.iter().map(|(k, v)| (*k, v.as_str())));
    url.query().unwrap_or("").to_string()
}

async fn token_request(reach: &Reach, endpoint: &Url, form: &[(&str, String)]) -> Result<TokenResponse, McpError> {
    let response = reach
        .client(endpoint)
        .await?
        .post(endpoint.clone())
        .header("Accept", "application/json")
        .header("Content-Type", "application/x-www-form-urlencoded")
        .timeout(Duration::from_secs(30))
        .body(form_body(form))
        .send()
        .await
        .map_err(|e| McpError::Transport(redact_text(&e.without_url().to_string())))?;
    let ok = response.status().is_success();
    let body = read_json(response).await?;
    if !ok {
        let error = body.get("error").and_then(Value::as_str).unwrap_or("error");
        let why = body.get("error_description").and_then(Value::as_str).map(redact_text).unwrap_or_default();
        return Err(if error == "invalid_grant" {
            McpError::NeedsLogin("the sign-in expired or was revoked; sign in again".into())
        } else {
            McpError::Protocol(format!(
                "the authorization server refused the request ({error}{}{why})",
                if why.is_empty() { "" } else { ": " }
            ))
        });
    }
    if !body.get("token_type").and_then(Value::as_str).is_some_and(|t| t.eq_ignore_ascii_case("bearer")) {
        return Err(McpError::Protocol("the authorization server returned a token that is not a bearer token".into()));
    }
    Ok(TokenResponse {
        access_token: body
            .get("access_token")
            .and_then(Value::as_str)
            .ok_or_else(|| McpError::Protocol("the token response had no access token".into()))?
            .to_string(),
        refresh_token: body.get("refresh_token").and_then(Value::as_str).map(str::to_string),
        expires_in: body.get("expires_in").and_then(Value::as_i64),
        scope: body.get("scope").and_then(Value::as_str).map(str::to_string),
    })
}

/// What a sign-in achieved.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct LoginSummary {
    /// The authorization server.
    pub issuer: String,
    /// The scopes granted, when the server said.
    pub scope: Option<String>,
    /// When the access token runs out, in seconds, when the server said.
    pub expires_in: Option<i64>,
    /// A refresh token was issued, so the sign-in lasts.
    pub refreshable: bool,
}

/// Signs in to one server and keeps its tokens.
pub struct OAuthClient {
    http: Client,
    vault: Vault,
    clock: Arc<dyn Clock>,
    /// A reach chosen by the host instead of the one the server's own address gives.
    reach: Option<Reach>,
}

fn http_client() -> Result<Client, McpError> {
    let _ = rustls::crypto::ring::default_provider().install_default();
    Client::builder()
        .connect_timeout(Duration::from_secs(15))
        .user_agent(concat!("zorix/", env!("CARGO_PKG_VERSION")))
        .redirect(reqwest::redirect::Policy::none())
        .build()
        .map_err(|e| McpError::Spawn(format!("could not start the HTTP client: {e}")))
}

impl OAuthClient {
    /// A sign-in client that keeps its tokens in `vault`.
    pub fn new(vault: Vault, clock: Arc<dyn Clock>) -> Result<Self, McpError> {
        Ok(Self { http: http_client()?, vault, clock, reach: None })
    }

    /// Never let the server send the sign-in to a private address, even when the server's own address is private. For a host that
    /// signs in only to servers on the internet (and for tests).
    pub fn internet_only(mut self) -> Self {
        self.reach = Some(Reach::internet_only());
        self
    }

    /// Ask the server what it wants, with no token, and read its challenge.
    async fn challenge(&self, server: &Url) -> Result<Challenge, McpError> {
        let body = json!({"jsonrpc": "2.0", "id": 1, "method": "server/discover", "params": {"_meta": modern_meta()}});
        let response = self
            .http
            .post(server.clone())
            .header("Accept", "application/json, text/event-stream")
            .header("Content-Type", "application/json")
            .header("MCP-Protocol-Version", crate::protocol::MODERN_VERSION)
            .header("Mcp-Method", "server/discover")
            .timeout(Duration::from_secs(20))
            .body(body.to_string())
            .send()
            .await
            .map_err(|e| McpError::Transport(redact_text(&e.without_url().to_string())))?;
        if response.status().as_u16() != 401 {
            return Err(McpError::Protocol(format!(
                "the server did not ask for a sign-in (it answered HTTP {})",
                response.status().as_u16()
            )));
        }
        Ok(response
            .headers()
            .get("www-authenticate")
            .and_then(|v| v.to_str().ok())
            .map(parse_challenge)
            .unwrap_or_default())
    }

    async fn register(
        &self,
        reach: &Reach,
        found: &Discovered,
        redirect: &str,
        settings: &OAuthSettings,
    ) -> Result<(String, Option<String>), McpError> {
        if let Some(id) = &settings.client_id {
            return Ok((id.clone(), None));
        }
        if let Ok(Some(old)) = self.vault.read::<Registration>(&self.vault.registration).await
            && old.issuer == found.issuer
            && old.resource == found.resource
        {
            return Ok((old.client_id, old.client_secret));
        }
        let Some(endpoint) = &found.registration_endpoint else {
            return Err(McpError::Protocol(
                "this server does not register clients by itself. Register ZORIX with it and give the client id with --client-id.".into(),
            ));
        };
        let response = reach
            .client(endpoint)
            .await?
            .post(endpoint.clone())
            .header("Accept", "application/json")
            .timeout(Duration::from_secs(30))
            .json(&json!({
                "client_name": "ZORIX",
                "redirect_uris": [redirect],
                "grant_types": ["authorization_code", "refresh_token"],
                "response_types": ["code"],
                "token_endpoint_auth_method": "none",
            }))
            .send()
            .await
            .map_err(|e| McpError::Transport(redact_text(&e.without_url().to_string())))?;
        let ok = response.status().is_success();
        let body = read_json(response).await?;
        if !ok {
            return Err(McpError::Protocol(format!(
                "the server refused to register ZORIX ({})",
                body.get("error").and_then(Value::as_str).unwrap_or("error")
            )));
        }
        let id = body
            .get("client_id")
            .and_then(Value::as_str)
            .ok_or_else(|| McpError::Protocol("the registration had no client id".into()))?;
        Ok((id.to_string(), body.get("client_secret").and_then(Value::as_str).map(str::to_string)))
    }

    /// Sign in. `opener` shows the person the sign-in page; `server` is the MCP endpoint.
    pub async fn login(
        &self,
        server: &Url,
        settings: &OAuthSettings,
        opener: &dyn BrowserOpener,
        cancel: &CancellationToken,
    ) -> Result<LoginSummary, McpError> {
        let challenge = self.challenge(server).await?;
        let reach = match self.reach {
            Some(reach) => reach,
            None => Reach::of(server).await,
        };
        let found = discover(&reach, server, &challenge).await?;
        let listener = TcpListener::bind("127.0.0.1:0")
            .await
            .map_err(|e| McpError::Transport(format!("could not listen for the sign-in: {e}")))?;
        let port = listener.local_addr().map_err(|e| McpError::Transport(e.to_string()))?.port();
        let redirect = format!("http://127.0.0.1:{port}/callback");
        let (client_id, client_secret) = self.register(&reach, &found, &redirect, settings).await?;

        let (verifier, code_challenge) = pkce_pair();
        let state = random_token::<24>();
        let scope = if !settings.scopes.is_empty() {
            Some(settings.scopes.join(" "))
        } else {
            challenge.scope.clone().or_else(|| found.default_scope.clone())
        };
        let mut url = found.authorization_endpoint.clone();
        {
            let mut query = url.query_pairs_mut();
            query
                .append_pair("response_type", "code")
                .append_pair("client_id", &client_id)
                .append_pair("redirect_uri", &redirect)
                .append_pair("state", &state)
                .append_pair("code_challenge", &code_challenge)
                .append_pair("code_challenge_method", "S256")
                .append_pair("resource", &found.resource);
            if let Some(scope) = &scope {
                query.append_pair("scope", scope);
            }
        }
        opener.open(url.as_str()).map_err(|e| {
            McpError::Transport(format!("the browser could not be opened ({e}); open this address yourself: {url}"))
        })?;

        let callback = wait_for_callback(listener, port, &state, cancel).await?.params;
        // The issuer was recorded from validated metadata before the browser opened (RFC 9207).
        match (callback.get("iss"), found.iss_supported) {
            (Some(iss), _) if iss != &found.issuer => return Err(McpError::Protocol("the sign-in came back from a different authorization server than the one ZORIX started with, so it was ignored".into())),
            (None, true) => return Err(McpError::Protocol("the authorization server did not say who it is in its answer, so the sign-in was ignored".into())),
            _ => {}
        }
        if callback.get("state") != Some(&state) {
            return Err(McpError::Protocol(
                "the sign-in answer did not match the request that started it, so it was ignored".into(),
            ));
        }
        if let Some(error) = callback.get("error") {
            return Err(McpError::Protocol(format!("the sign-in was not completed ({})", redact_text(error))));
        }
        let code = callback.get("code").ok_or_else(|| McpError::Protocol("the sign-in answer had no code".into()))?;
        let mut form = vec![
            ("grant_type", "authorization_code".to_string()),
            ("code", code.clone()),
            ("redirect_uri", redirect),
            ("client_id", client_id.clone()),
            ("code_verifier", verifier),
            ("resource", found.resource.clone()),
        ];
        if let Some(secret) = &client_secret {
            form.push(("client_secret", secret.clone()));
        }
        let tokens = token_request(&reach, &found.token_endpoint, &form).await?;
        let now = self.clock.now().millis();
        let refreshable = tokens.refresh_token.is_some();
        self.vault
            .write(
                &self.vault.registration,
                &Registration {
                    client_id,
                    client_secret,
                    issuer: found.issuer.clone(),
                    token_endpoint: found.token_endpoint.to_string(),
                    revocation_endpoint: found.revocation_endpoint.as_ref().map(Url::to_string),
                    resource: found.resource.clone(),
                    scope: tokens.scope.clone().or(scope),
                    refresh_token: tokens.refresh_token,
                },
            )
            .await?;
        self.vault
            .write(
                &self.vault.access,
                &Access { token: tokens.access_token, expires_at_ms: tokens.expires_in.map(|s| now + s * 1000) },
            )
            .await?;
        Ok(LoginSummary { issuer: found.issuer, scope: tokens.scope, expires_in: tokens.expires_in, refreshable })
    }

    /// Forget the sign-in, and tell the authorization server when it allows that.
    pub async fn logout(&self) {
        if let Ok(Some(reg)) = self.vault.read::<Registration>(&self.vault.registration).await
            && let (Some(endpoint), Some(token)) =
                (reg.revocation_endpoint.as_deref().and_then(|e| Url::parse(e).ok()), reg.refresh_token.clone())
        {
            // The address was stored from a sign-in that already passed the checks; it is held to them again.
            let reach = match (self.reach, Url::parse(&reg.resource)) {
                (Some(reach), _) => reach,
                (None, Ok(resource)) => Reach::of(&resource).await,
                (None, Err(_)) => Reach::internet_only(),
            };
            if let Ok(client) = reach.client(&endpoint).await {
                let _ = client
                    .post(endpoint)
                    .header("Content-Type", "application/x-www-form-urlencoded")
                    .timeout(Duration::from_secs(10))
                    .body(form_body(&[("token", token), ("client_id", reg.client_id.clone())]))
                    .send()
                    .await;
            }
        }
        self.vault.forget().await;
    }
}

/// The sign-in of one server as the HTTP transport uses it: hands out the access token, refreshing it when it runs out.
pub struct OAuthSession {
    vault: Vault,
    clock: Arc<dyn Clock>,
    cache: Mutex<Option<Access>>,
    hint: String,
}

impl OAuthSession {
    /// A session over the tokens in `vault`. `hint` names the server in messages ("zorix mcp login github").
    pub fn new(vault: Vault, clock: Arc<dyn Clock>, hint: &str) -> Result<Self, McpError> {
        Ok(Self { vault, clock, cache: Mutex::new(None), hint: hint.to_string() })
    }

    fn needs_login(&self, why: &str) -> McpError {
        McpError::NeedsLogin(format!("{why} (sign in again: {})", self.hint))
    }

    async fn refresh(&self, cache: &mut Option<Access>) -> Result<Access, McpError> {
        let Some(mut registration) = self.vault.read::<Registration>(&self.vault.registration).await? else {
            return Err(self.needs_login("you are not signed in"));
        };
        let Some(refresh_token) = registration.refresh_token.clone() else {
            *cache = None;
            return Err(self.needs_login("the sign-in expired and has no way to renew itself"));
        };
        let endpoint =
            Url::parse(&registration.token_endpoint).map_err(|_| self.needs_login("the stored sign-in is damaged"))?;
        let mut form = vec![
            ("grant_type", "refresh_token".to_string()),
            ("refresh_token", refresh_token),
            ("client_id", registration.client_id.clone()),
            ("resource", registration.resource.clone()),
        ];
        if let Some(secret) = &registration.client_secret {
            form.push(("client_secret", secret.clone()));
        }
        let reach = match Url::parse(&registration.resource) {
            Ok(resource) => Reach::of(&resource).await,
            Err(_) => Reach::internet_only(),
        };
        let tokens = match token_request(&reach, &endpoint, &form).await {
            Ok(tokens) => tokens,
            Err(McpError::NeedsLogin(why)) => {
                self.vault.forget().await;
                *cache = None;
                return Err(self.needs_login(&why));
            }
            Err(other) => return Err(other),
        };
        let access = Access {
            token: tokens.access_token,
            expires_at_ms: tokens.expires_in.map(|s| self.clock.now().millis() + s * 1000),
        };
        // Refresh tokens may rotate; keep the new one, or the old one when none came.
        if let Some(new) = tokens.refresh_token {
            registration.refresh_token = Some(new);
        }
        if tokens.scope.is_some() {
            registration.scope = tokens.scope;
        }
        self.vault.write(&self.vault.registration, &registration).await?;
        self.vault.write(&self.vault.access, &access).await?;
        *cache = Some(access.clone());
        Ok(access)
    }

    async fn current(&self) -> Result<Access, McpError> {
        let mut cache = self.cache.lock().await;
        if cache.is_none() {
            *cache = self.vault.read::<Access>(&self.vault.access).await?;
        }
        let now = self.clock.now().millis();
        match cache.clone() {
            Some(access) if access.expires_at_ms.is_none_or(|at| at - REFRESH_MARGIN_MS > now) => Ok(access),
            Some(_) => self.refresh(&mut cache).await,
            None if self.vault.signed_in().await => self.refresh(&mut cache).await,
            None => Err(self.needs_login("you are not signed in")),
        }
    }
}

#[async_trait]
impl AuthProvider for OAuthSession {
    async fn authorization(&self) -> Result<Option<String>, McpError> {
        Ok(Some(format!("Bearer {}", self.current().await?.token)))
    }

    async fn challenged(&self, status: u16, www_authenticate: Option<&str>) -> Result<(), McpError> {
        let challenge = www_authenticate.map(parse_challenge).unwrap_or_default();
        if status == 403 && challenge.error.as_deref() == Some("insufficient_scope") {
            let needed = challenge.scope.unwrap_or_else(|| "more permission".into());
            return Err(self.needs_login(&format!("this needs more permission ({needed})")));
        }
        let mut cache = self.cache.lock().await;
        self.refresh(&mut cache).await.map(|_| ())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn a_sign_in_that_is_not_on_a_private_network_cannot_be_sent_to_one() {
        let internet = Reach::internet_only();
        for address in [
            "http://127.0.0.1:9/x",
            "https://127.0.0.1/x",
            "https://localhost/x",
            "https://[::1]/x",
            "https://169.254.169.254/latest/meta-data",
            "https://10.0.0.5/x",
            "https://192.168.1.1/x",
            "https://172.16.0.1/x",
            "https://100.64.0.1/x",
            "https://[::ffff:127.0.0.1]/x",
            "https://[fd00::1]/x",
            "https://0.0.0.0/x",
        ] {
            let url = Url::parse(address).unwrap();
            let error = internet.client(&url).await.map(|_| ()).unwrap_err();
            assert!(
                matches!(&error, McpError::Protocol(m) if m.contains("may not send it there")),
                "{address}: {error:?}"
            );
        }
        // The same addresses are fine for a sign-in that is itself on a private network.
        let private = Reach { private_ok: true };
        for address in ["http://127.0.0.1:9/x", "https://10.0.0.5/x", "https://localhost/x"] {
            assert!(private.client(&Url::parse(address).unwrap()).await.is_ok(), "{address}");
        }
        // And an ordinary public address is fine for both (nothing is contacted: only the client is built).
        assert!(internet.client(&Url::parse("https://1.1.1.1/x").unwrap()).await.is_ok());
    }

    #[tokio::test]
    async fn the_reach_of_a_sign_in_follows_the_address_of_its_server() {
        assert!(Reach::of(&Url::parse("http://127.0.0.1:8080/mcp").unwrap()).await.allows_private());
        assert!(Reach::of(&Url::parse("https://localhost/mcp").unwrap()).await.allows_private());
        assert!(Reach::of(&Url::parse("https://10.1.2.3/mcp").unwrap()).await.allows_private());
        assert!(!Reach::of(&Url::parse("https://1.1.1.1/mcp").unwrap()).await.allows_private());
        // A name that cannot be found gives no reach to private addresses.
        assert!(!Reach::of(&Url::parse("https://nothing.invalid/mcp").unwrap()).await.allows_private());
    }
}
