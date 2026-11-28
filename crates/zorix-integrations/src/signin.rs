//! Signing in to a service instead of pasting a token (ADR-0053): GitHub with a device code, GitLab with PKCE and Jira with
//! Atlassian's three-legged flow, both answered on this computer. Tokens live in the keychain and are renewed when they run out.

use std::collections::HashMap;
use std::sync::Arc;
use std::time::Duration;

use parking_lot::Mutex;
use reqwest::Url;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use tokio::net::TcpListener;
use tokio_util::sync::CancellationToken;
use zorix_core::{ConnectionId, ConnectionKind, Secret, SystemClock, redact_text};
use zorix_oauth::{
    Access, OAuthError, Reach, Renewable, Session, TokenResponse, Vault, endpoint, form_body, pkce_pair, random_token,
    read_json, token_request, token_request_json, wait_for_callback,
};
use zorix_providers::SecretStore;

use crate::secret_name;

/// The port the browser comes back to for GitLab and Jira: their apps accept only the exact address registered.
pub const REDIRECT_PORT: u16 = 53682;
const GITHUB_DEVICE_GRANT: &str = "urn:ietf:params:oauth:grant-type:device_code";
/// The longest a person has to enter a device code, whatever the service says.
const LONGEST_CODE: Duration = Duration::from_secs(15 * 60);

/// The redirect address a person adds to their own app.
pub fn redirect_uri() -> String {
    format!("http://127.0.0.1:{REDIRECT_PORT}/callback")
}

/// Whether a kind of connection can be signed in to. Slack is not: it wants an HTTPS redirect, and the app a person makes
/// already shows its bot token.
pub fn can_sign_in(kind: ConnectionKind) -> bool {
    matches!(kind, ConnectionKind::Github | ConnectionKind::Gitlab | ConnectionKind::Jira)
}

/// ZORIX's own app id, when the program was built with one.
pub fn builtin_client_id(kind: ConnectionKind) -> Option<&'static str> {
    match kind {
        ConnectionKind::Github => option_env!("ZORIX_GITHUB_CLIENT_ID"),
        ConnectionKind::Gitlab => option_env!("ZORIX_GITLAB_CLIENT_ID"),
        _ => None,
    }
    .filter(|id| !id.is_empty())
}

/// The scopes asked for, said to the person before they sign in.
pub fn scopes(kind: ConnectionKind) -> &'static str {
    match kind {
        ConnectionKind::Github => "repo",
        ConnectionKind::Gitlab => "api",
        ConnectionKind::Jira => "read:jira-work write:jira-work offline_access",
        _ => "",
    }
}

/// Where the sign-in of one connection happens.
#[derive(Clone, Debug)]
pub struct Endpoints {
    /// The service's own web address, which decides what the token calls may reach.
    pub web: Url,
    /// GitHub: where a device code is asked for. Others: where the person is sent.
    pub start: Url,
    /// Where codes and refresh tokens are exchanged.
    pub token: Url,
    /// Where tokens are revoked, when the service lets an app without a secret do it.
    pub revoke: Option<Url>,
    /// Jira: where the sites a token can reach are listed.
    pub resources: Option<Url>,
    /// The port the browser comes back to (0 picks any, for tests).
    pub port: u16,
}

fn url(text: &str) -> Result<Url, OAuthError> {
    endpoint(text, "address")
}

impl Endpoints {
    /// The addresses of `kind`, for the base address of the connection (empty for the public service).
    pub fn of(kind: ConnectionKind, base: Option<&str>) -> Result<Self, OAuthError> {
        let base = base.map(|b| b.trim().trim_end_matches('/')).filter(|b| !b.is_empty());
        let (web, start, token, revoke, resources) = match kind {
            ConnectionKind::Github => {
                // GitHub Enterprise's API is under /api/v3 of the web address.
                let web = match base {
                    None | Some("https://api.github.com") => "https://github.com".to_string(),
                    Some(base) => base.trim_end_matches("/api/v3").to_string(),
                };
                let (start, token) = (format!("{web}/login/device/code"), format!("{web}/login/oauth/access_token"));
                (web, start, token, None, None)
            }
            ConnectionKind::Gitlab => {
                let web = base.map_or("https://gitlab.com", |b| b.trim_end_matches("/api/v4")).to_string();
                let start = format!("{web}/oauth/authorize");
                let token = format!("{web}/oauth/token");
                let revoke = format!("{web}/oauth/revoke");
                (web, start, token, Some(revoke), None)
            }
            ConnectionKind::Jira => (
                "https://auth.atlassian.com".to_string(),
                "https://auth.atlassian.com/authorize".to_string(),
                "https://auth.atlassian.com/oauth/token".to_string(),
                None,
                Some("https://api.atlassian.com/oauth/token/accessible-resources".to_string()),
            ),
            other => {
                return Err(OAuthError::Protocol(format!("{} connections are not signed in to.", other.as_str())));
            }
        };
        Ok(Self {
            web: url(&web)?,
            start: url(&start)?,
            token: url(&token)?,
            revoke: revoke.as_deref().map(url).transpose()?,
            resources: resources.as_deref().map(url).transpose()?,
            port: REDIRECT_PORT,
        })
    }
}

/// How a sign-in shows itself to the person.
pub trait SignInUi: Send + Sync {
    /// GitHub: enter `code` at `url`.
    fn device_code(&self, code: &str, url: &str);
    /// Open `url` in the person's browser.
    fn open(&self, url: &str) -> Result<(), String>;
}

/// The app to sign in with.
#[derive(Clone)]
pub struct App {
    /// Its client id.
    pub client_id: String,
    /// Its secret, which Jira needs.
    pub client_secret: Option<Secret>,
}

/// What renews a sign-in, kept in the keychain.
#[derive(Clone, Serialize, Deserialize)]
pub struct Grant {
    /// The app's id.
    client_id: String,
    client_secret: Option<String>,
    token_endpoint: String,
    revocation_endpoint: Option<String>,
    refresh_token: Option<String>,
    /// The scopes granted, when the service said.
    pub scope: Option<String>,
    web: String,
    json: bool,
    /// Where the API answers, when it is not where the connection says (Jira through Atlassian).
    pub api_base: Option<String>,
}

impl Renewable for Grant {
    fn token_endpoint(&self) -> &str {
        &self.token_endpoint
    }

    fn renewal(&self) -> Option<Vec<(&'static str, String)>> {
        let refresh = self.refresh_token.clone()?;
        let mut form = vec![
            ("grant_type", "refresh_token".to_string()),
            ("refresh_token", refresh),
            ("client_id", self.client_id.clone()),
        ];
        if let Some(secret) = &self.client_secret {
            form.push(("client_secret", secret.clone()));
        }
        Some(form)
    }

    fn renewed(&mut self, tokens: &TokenResponse) {
        if let Some(new) = &tokens.refresh_token {
            self.refresh_token = Some(new.clone());
        }
        if tokens.scope.is_some() {
            self.scope.clone_from(&tokens.scope);
        }
    }

    fn resource(&self) -> &str {
        &self.web
    }

    fn json(&self) -> bool {
        self.json
    }
}

/// A finished sign-in, before it is kept.
pub struct SignedIn {
    grant: Grant,
    access: Access,
}

impl SignedIn {
    /// The scopes granted, when the service said.
    pub fn scope(&self) -> Option<&str> {
        self.grant.scope.as_deref()
    }
}

/// Where the tokens of a connection live.
pub fn vault(secrets: Arc<dyn SecretStore>, id: ConnectionId) -> Vault {
    Vault::new(secrets, &secret_name(id))
}

/// Keep a finished sign-in as the credential of a connection.
pub async fn keep(vault: &Vault, signed: &SignedIn) -> Result<(), OAuthError> {
    vault.write(vault.registration(), &signed.grant).await?;
    vault.write(vault.access(), &signed.access).await
}

/// Forget the sign-in, telling the service when it allows that.
pub async fn revoke(vault: &Vault) {
    if let Ok(Some(grant)) = vault.read::<Grant>(vault.registration()).await
        && let (Some(endpoint), Some(token)) =
            (grant.revocation_endpoint.as_deref().and_then(|e| Url::parse(e).ok()), grant.refresh_token.clone())
        && let Ok(web) = Url::parse(&grant.web)
        && let Ok(client) = Reach::of(&web).await.client(&endpoint).await
    {
        let _ = client
            .post(endpoint)
            .header("Content-Type", "application/x-www-form-urlencoded")
            .timeout(Duration::from_secs(10))
            .body(form_body(&[("token", token), ("client_id", grant.client_id.clone())]))
            .send()
            .await;
    }
    vault.forget().await;
}

/// The sessions of the signed-in connections, one each, so renewals happen one at a time.
#[derive(Default)]
pub struct SignIns {
    sessions: Mutex<HashMap<ConnectionId, Arc<Session<Grant>>>>,
}

impl SignIns {
    fn session(&self, secrets: Arc<dyn SecretStore>, id: ConnectionId, name: &str) -> Arc<Session<Grant>> {
        let hint = format!("remove {name} and add it again, signing in");
        let make = || Arc::new(Session::new(vault(secrets, id), Arc::new(SystemClock), &hint));
        self.sessions.lock().entry(id).or_insert_with(make).clone()
    }

    /// The access token to use now, and where the API answers when the sign-in says so.
    pub async fn current(
        &self,
        secrets: Arc<dyn SecretStore>,
        id: ConnectionId,
        name: &str,
    ) -> Result<(String, Option<String>), OAuthError> {
        let session = self.session(secrets, id, name);
        let token = session.current().await?.token;
        let vault = session.vault();
        let base = vault.read::<Grant>(vault.registration()).await?.and_then(|g| g.api_base);
        Ok((token, base))
    }

    /// Forget a removed connection's session.
    pub fn forget(&self, id: ConnectionId) {
        self.sessions.lock().remove(&id);
    }
}

/// Sign in. `site` is the Jira site the connection is for.
pub async fn sign_in(
    kind: ConnectionKind,
    endpoints: &Endpoints,
    app: &App,
    site: Option<&str>,
    ui: &dyn SignInUi,
    cancel: &CancellationToken,
) -> Result<SignedIn, OAuthError> {
    if app.client_id.trim().is_empty() {
        return Err(OAuthError::Protocol("The app's client id is empty.".into()));
    }
    let reach = Reach::of(&endpoints.web).await;
    match kind {
        ConnectionKind::Github => device(endpoints, &reach, app, ui, cancel).await,
        ConnectionKind::Gitlab => browser(kind, endpoints, &reach, app, ui, cancel).await,
        ConnectionKind::Jira => {
            if app.client_secret.is_none() {
                return Err(OAuthError::Protocol("Jira needs your app's client secret as well as its id.".into()));
            }
            let site = site.ok_or_else(|| OAuthError::Protocol("The Jira site address is needed.".into()))?;
            let mut signed = browser(kind, endpoints, &reach, app, ui, cancel).await?;
            signed.grant.api_base = Some(jira_site(endpoints, &reach, &signed.access.token, site).await?);
            Ok(signed)
        }
        other => Err(OAuthError::Protocol(format!("{} connections are not signed in to.", other.as_str()))),
    }
}

fn expiry(expires_in: Option<i64>) -> Option<i64> {
    use zorix_core::Clock;
    expires_in.map(|s| SystemClock.now().millis() + s * 1000)
}

fn signed(endpoints: &Endpoints, app: &App, json: bool, tokens: TokenResponse) -> SignedIn {
    SignedIn {
        access: Access { token: tokens.access_token, expires_at_ms: expiry(tokens.expires_in) },
        grant: Grant {
            client_id: app.client_id.clone(),
            client_secret: app.client_secret.as_ref().map(|s| s.expose().to_string()),
            token_endpoint: endpoints.token.to_string(),
            revocation_endpoint: endpoints.revoke.as_ref().map(Url::to_string),
            refresh_token: tokens.refresh_token,
            scope: tokens.scope,
            web: endpoints.web.to_string(),
            json,
            api_base: None,
        },
    }
}

async fn post_form(reach: &Reach, to: &Url, form: &[(&str, String)]) -> Result<Value, OAuthError> {
    let response = reach
        .client(to)
        .await?
        .post(to.clone())
        .header("Accept", "application/json")
        .header("Content-Type", "application/x-www-form-urlencoded")
        .timeout(Duration::from_secs(30))
        .body(form_body(form))
        .send()
        .await
        .map_err(|e| OAuthError::Transport(redact_text(&e.without_url().to_string())))?;
    read_json(response).await
}

fn refused(body: &Value) -> OAuthError {
    let error = body["error"].as_str().unwrap_or("an answer ZORIX does not understand");
    let why = body["error_description"].as_str().map(redact_text).unwrap_or_default();
    OAuthError::Protocol(format!(
        "the service refused the sign-in ({error}{}{why})",
        if why.is_empty() { "" } else { ": " }
    ))
}

/// GitHub's device flow (RFC 8628).
async fn device(
    endpoints: &Endpoints,
    reach: &Reach,
    app: &App,
    ui: &dyn SignInUi,
    cancel: &CancellationToken,
) -> Result<SignedIn, OAuthError> {
    let form = [("client_id", app.client_id.clone()), ("scope", scopes(ConnectionKind::Github).to_string())];
    let started = post_form(reach, &endpoints.start, &form).await?;
    let (Some(device_code), Some(code), Some(page)) =
        (started["device_code"].as_str(), started["user_code"].as_str(), started["verification_uri"].as_str())
    else {
        return Err(refused(&started));
    };
    // The person types a code on that page: it must be the service they chose, not one an answer made up.
    let page = url(page)?;
    if page.host_str() != endpoints.web.host_str() {
        return Err(OAuthError::Protocol("the service sent the sign-in to another site, so it was stopped".into()));
    }
    let code: String = code.chars().filter(|c| c.is_ascii_alphanumeric() || *c == '-').take(20).collect();
    ui.device_code(&code, page.as_str());
    let mut wait = Duration::from_secs(started["interval"].as_u64().unwrap_or(5).clamp(1, 60));
    let lasts = Duration::from_secs(started["expires_in"].as_u64().unwrap_or(900)).min(LONGEST_CODE);
    let deadline = tokio::time::Instant::now() + lasts;
    loop {
        tokio::select! {
            _ = cancel.cancelled() => return Err(OAuthError::Cancelled),
            _ = tokio::time::sleep(wait) => {}
        }
        if tokio::time::Instant::now() >= deadline {
            return Err(OAuthError::Timeout("the code ran out before it was entered".into()));
        }
        let form = [
            ("client_id", app.client_id.clone()),
            ("device_code", device_code.to_string()),
            ("grant_type", GITHUB_DEVICE_GRANT.to_string()),
        ];
        // GitHub answers 200 whatever happened; the body says which.
        let body = post_form(reach, &endpoints.token, &form).await?;
        if let Some(token) = body["access_token"].as_str() {
            let tokens = TokenResponse {
                access_token: token.to_string(),
                refresh_token: body["refresh_token"].as_str().map(str::to_string),
                expires_in: body["expires_in"].as_i64(),
                scope: body["scope"].as_str().map(str::to_string),
            };
            return Ok(signed(endpoints, app, false, tokens));
        }
        match body["error"].as_str() {
            Some("authorization_pending") => {}
            Some("slow_down") => wait += Duration::from_secs(5),
            _ => return Err(refused(&body)),
        }
    }
}

/// The authorization code flow, answered on this computer.
async fn browser(
    kind: ConnectionKind,
    endpoints: &Endpoints,
    reach: &Reach,
    app: &App,
    ui: &dyn SignInUi,
    cancel: &CancellationToken,
) -> Result<SignedIn, OAuthError> {
    let listener = TcpListener::bind(("127.0.0.1", endpoints.port)).await.map_err(|e| {
        OAuthError::Transport(format!(
            "port {} is in use, and the app accepts only that address ({e}). Close what uses it and try again.",
            endpoints.port
        ))
    })?;
    let port = listener.local_addr().map_err(|e| OAuthError::Transport(e.to_string()))?.port();
    let redirect = format!("http://127.0.0.1:{port}/callback");
    let (verifier, challenge) = pkce_pair();
    let state = random_token::<24>();
    let jira = kind == ConnectionKind::Jira;
    let mut page = endpoints.start.clone();
    {
        let mut query = page.query_pairs_mut();
        query
            .append_pair("response_type", "code")
            .append_pair("client_id", &app.client_id)
            .append_pair("redirect_uri", &redirect)
            .append_pair("state", &state)
            .append_pair("scope", scopes(kind));
        if jira {
            query.append_pair("audience", "api.atlassian.com").append_pair("prompt", "consent");
        } else {
            query.append_pair("code_challenge", &challenge).append_pair("code_challenge_method", "S256");
        }
    }
    ui.open(page.as_str()).map_err(|e| {
        OAuthError::Transport(format!("the browser could not be opened ({e}); open this address yourself: {page}"))
    })?;
    let answer = wait_for_callback(listener, port, &state, cancel).await?;
    if let Some(error) = answer.get("error") {
        return Err(OAuthError::Protocol(format!("the sign-in was not completed ({})", redact_text(error))));
    }
    let code = answer.get("code").ok_or_else(|| OAuthError::Protocol("the sign-in answer had no code".into()))?;
    let mut form = vec![
        ("grant_type", "authorization_code".to_string()),
        ("code", code.clone()),
        ("redirect_uri", redirect),
        ("client_id", app.client_id.clone()),
    ];
    if !jira {
        form.push(("code_verifier", verifier));
    }
    if let Some(secret) = &app.client_secret {
        form.push(("client_secret", secret.expose().to_string()));
    }
    let tokens = if jira {
        token_request_json(reach, &endpoints.token, &form).await?
    } else {
        token_request(reach, &endpoints.token, &form).await?
    };
    Ok(signed(endpoints, app, jira, tokens))
}

/// The address of Atlassian's API for the site the connection is for.
async fn jira_site(endpoints: &Endpoints, reach: &Reach, token: &str, site: &str) -> Result<String, OAuthError> {
    let resources = endpoints.resources.as_ref().ok_or_else(|| OAuthError::Protocol("no list of sites".into()))?;
    let response = reach
        .client(resources)
        .await?
        .get(resources.clone())
        .header("Accept", "application/json")
        .bearer_auth(token)
        .timeout(Duration::from_secs(30))
        .send()
        .await
        .map_err(|e| OAuthError::Transport(redact_text(&e.without_url().to_string())))?;
    let list = read_json(response).await?;
    let wanted = site.trim().trim_end_matches('/').to_ascii_lowercase();
    let sites = list.as_array().cloned().unwrap_or_default();
    let found = sites
        .iter()
        .find(|s| s["url"].as_str().is_some_and(|u| u.trim_end_matches('/').to_ascii_lowercase() == wanted));
    let Some(id) =
        found.and_then(|s| s["id"].as_str()).filter(|id| id.chars().all(|c| c.is_ascii_alphanumeric() || c == '-'))
    else {
        let seen: Vec<&str> = sites.iter().filter_map(|s| s["url"].as_str()).take(5).collect();
        return Err(OAuthError::Protocol(format!(
            "the sign-in does not reach {site}{}",
            if seen.is_empty() { String::new() } else { format!(" (it reaches {})", seen.join(", ")) }
        )));
    };
    let api = resources.join("/ex/jira/").map_err(|e| OAuthError::Protocol(e.to_string()))?;
    Ok(format!("{}{id}", api))
}
