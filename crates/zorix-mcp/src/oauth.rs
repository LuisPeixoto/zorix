//! Signing in to a remote MCP server (ADR-0019): OAuth 2.1 with PKCE, the `resource` parameter, issuer validation and a
//! loopback redirect, and the tokens that come out of it, kept in the keychain.
//!
//! Discovery follows the specification: the server's `WWW-Authenticate` challenge points to its protected-resource metadata
//! (RFC 9728), which names the authorization server, whose own metadata (RFC 8414 or OpenID Connect) gives the endpoints.
//! A token is only ever sent to the server it was issued for.

use std::sync::Arc;
use std::time::Duration;

use async_trait::async_trait;
use reqwest::{Client, Url};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use tokio::net::TcpListener;
use tokio_util::sync::CancellationToken;
use zorix_core::{Clock, redact_text};
use zorix_oauth::{
    Access, Renewable, Session, TokenResponse, endpoint, form_body, get_json, pkce_pair, random_token, read_json,
    token_request, wait_for_callback,
};
pub use zorix_oauth::{BrowserOpener, LOGIN_TIMEOUT, Reach, SystemBrowser, Vault};

use crate::auth::AuthProvider;
use crate::error::McpError;
use crate::protocol::modern_meta;

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
    /// The scopes the next sign-in asks for, after a server said it needs more than were granted (step-up): what was granted and what
    /// was missing. Cleared by the sign-in that gets them.
    #[serde(default)]
    wanted_scope: Option<String>,
}

/// Two lists of scopes written with spaces, as one list with each scope once, in the order they were first named.
fn union_of_scopes(a: &str, b: &str) -> String {
    let mut out: Vec<&str> = Vec::new();
    for scope in a.split_whitespace().chain(b.split_whitespace()) {
        if !out.contains(&scope) {
            out.push(scope);
        }
    }
    out.join(" ")
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
        if let Ok(Some(old)) = self.vault.read::<Registration>(self.vault.registration()).await
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
        // What a person chose wins; then what a server said it needed after the last sign-in (step-up); then what the server's challenge
        // or its metadata name.
        let wanted = self
            .vault
            .read::<Registration>(self.vault.registration())
            .await
            .ok()
            .flatten()
            .and_then(|r| r.wanted_scope);
        let scope = if !settings.scopes.is_empty() {
            Some(settings.scopes.join(" "))
        } else {
            wanted.or_else(|| challenge.scope.clone()).or_else(|| found.default_scope.clone())
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

        let callback = wait_for_callback(listener, port, &state, cancel).await?;
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
                self.vault.registration(),
                &Registration {
                    client_id,
                    client_secret,
                    issuer: found.issuer.clone(),
                    token_endpoint: found.token_endpoint.to_string(),
                    revocation_endpoint: found.revocation_endpoint.as_ref().map(Url::to_string),
                    resource: found.resource.clone(),
                    scope: tokens.scope.clone().or(scope),
                    refresh_token: tokens.refresh_token,
                    wanted_scope: None,
                },
            )
            .await?;
        self.vault
            .write(
                self.vault.access(),
                &Access { token: tokens.access_token, expires_at_ms: tokens.expires_in.map(|s| now + s * 1000) },
            )
            .await?;
        Ok(LoginSummary { issuer: found.issuer, scope: tokens.scope, expires_in: tokens.expires_in, refreshable })
    }

    /// Forget the sign-in, and tell the authorization server when it allows that.
    pub async fn logout(&self) {
        if let Ok(Some(reg)) = self.vault.read::<Registration>(self.vault.registration()).await
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

impl Renewable for Registration {
    fn token_endpoint(&self) -> &str {
        &self.token_endpoint
    }

    fn renewal(&self) -> Option<Vec<(&'static str, String)>> {
        let refresh_token = self.refresh_token.clone()?;
        let mut form = vec![
            ("grant_type", "refresh_token".to_string()),
            ("refresh_token", refresh_token),
            ("client_id", self.client_id.clone()),
            ("resource", self.resource.clone()),
        ];
        if let Some(secret) = &self.client_secret {
            form.push(("client_secret", secret.clone()));
        }
        Some(form)
    }

    fn renewed(&mut self, tokens: &TokenResponse) {
        // Refresh tokens may rotate; keep the new one, or the old one when none came.
        if let Some(new) = &tokens.refresh_token {
            self.refresh_token = Some(new.clone());
        }
        if tokens.scope.is_some() {
            self.scope.clone_from(&tokens.scope);
        }
    }

    fn resource(&self) -> &str {
        &self.resource
    }
}

/// The sign-in of one server as the HTTP transport uses it: hands out the access token, refreshing it when it runs out.
pub struct OAuthSession {
    session: Session<Registration>,
}

impl OAuthSession {
    /// A session over the tokens in `vault`. `hint` names the server in messages ("zorix mcp login github").
    pub fn new(vault: Vault, clock: Arc<dyn Clock>, hint: &str) -> Result<Self, McpError> {
        Ok(Self { session: Session::new(vault, clock, hint) })
    }
}

#[async_trait]
impl AuthProvider for OAuthSession {
    async fn authorization(&self) -> Result<Option<String>, McpError> {
        Ok(Some(format!("Bearer {}", self.session.current().await?.token)))
    }

    async fn challenged(&self, status: u16, www_authenticate: Option<&str>) -> Result<(), McpError> {
        let challenge = www_authenticate.map(parse_challenge).unwrap_or_default();
        if status == 403 && challenge.error.as_deref() == Some("insufficient_scope") {
            // Step-up: remember what is missing, so that signing in again asks for what was granted and what was missing, and not for the
            // same thing that was refused.
            let vault = self.session.vault();
            if let Some(missing) = &challenge.scope
                && let Ok(Some(mut registration)) = vault.read::<Registration>(vault.registration()).await
            {
                let granted = registration.scope.clone().unwrap_or_default();
                registration.wanted_scope = Some(union_of_scopes(&granted, missing));
                let _ = vault.write(vault.registration(), &registration).await;
            }
            let needed = challenge.scope.unwrap_or_else(|| "more permission".into());
            return Err(self.session.needs_login(&format!("this needs more permission ({needed})")).into());
        }
        Ok(self.session.refresh().await?)
    }
}
