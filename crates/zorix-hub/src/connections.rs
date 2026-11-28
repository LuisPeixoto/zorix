//! Adding, testing and removing connections: what a person types, checked, and the secret put in the keychain.

use std::sync::Arc;

use serde_json::{Value, json};
use tokio_util::sync::CancellationToken;
use zorix_core::{
    Clock, Connection, ConnectionKind, ConnectionTest, IntegrationStore, NewConnection, Secret, SystemClock,
};
use zorix_integrations::signin::{self, App, Endpoints, SignInUi};
use zorix_integrations::{secret_name, test_connection};

use crate::{Hub, HubError};

/// What a person gave for a new connection.
#[derive(Clone, Debug)]
pub struct ConnectRequest {
    /// What it connects to.
    pub kind: ConnectionKind,
    /// Its name, used in grants (`db:<name>`) and by the model to pick one.
    pub name: String,
    /// The settings that are not secret, as the kind documents them.
    pub settings: Value,
    /// The token, password or signing key.
    pub secret: Option<Secret>,
}

/// The app a person signs in with: their own, or ZORIX's when it was built with one.
#[derive(Clone, Debug, Default)]
pub struct SignInApp {
    /// The client id; `None` uses ZORIX's.
    pub client_id: Option<String>,
    /// The client secret, for services that need one.
    pub client_secret: Option<Secret>,
}

fn text<'a>(settings: &'a Value, key: &str) -> Option<&'a str> {
    settings[key].as_str().map(str::trim).filter(|s| !s.is_empty())
}

fn need<'a>(settings: &'a Value, key: &str, what: &str) -> Result<&'a str, HubError> {
    text(settings, key).ok_or_else(|| HubError(format!("{what} is needed.")))
}

/// Check what a person gave and return the settings to store.
pub fn validate(request: &ConnectRequest) -> Result<Value, HubError> {
    let s = &request.settings;
    let base = |s: &Value, key: &str| -> Result<(), HubError> {
        if let Some(url) = text(s, key) {
            zorix_integrations::api::validate_base(url).map_err(|e| HubError(e.to_string().trim().to_string()))?;
        }
        Ok(())
    };
    let secret_needed = |what: &str| -> Result<(), HubError> {
        if request.secret.is_none() { Err(HubError(format!("{what} is needed."))) } else { Ok(()) }
    };
    let mut out = s.clone();
    // A connection signed in to gets its credential from the sign-in, so none is asked for here.
    let oauth = text(s, "auth") == Some("oauth");
    if oauth && !signin::can_sign_in(request.kind) {
        return Err(HubError(format!("{} connections are not signed in to; give a token.", request.kind.as_str())));
    }
    match request.kind {
        ConnectionKind::Github | ConnectionKind::Gitlab => base(s, "base_url")?,
        ConnectionKind::Jira => {
            need(s, "base_url", "The site address (for example https://acme.atlassian.net)")?;
            base(s, "base_url")?;
            if !oauth {
                if text(s, "auth") != Some("bearer") {
                    need(s, "email", "The account email")?;
                }
                secret_needed("An API token")?;
            }
        }
        ConnectionKind::Slack => {
            base(s, "base_url")?;
            secret_needed("A bot token")?;
        }
        ConnectionKind::Discord => {
            base(s, "base_url")?;
            secret_needed("A bot token")?;
        }
        ConnectionKind::Http => {
            let host = need(s, "host", "The host (for example api.example.com)")?.to_ascii_lowercase();
            zorix_core::validate_host_pattern(&host).map_err(HubError)?;
            if host.contains('*') {
                return Err(HubError("A connection names one host; wildcards belong in grants.".into()));
            }
            out["host"] = json!(host);
            match text(s, "auth").unwrap_or("bearer") {
                "bearer" => {}
                "header" => {
                    let header = text(s, "header").unwrap_or("X-Api-Key");
                    let ok = !header.is_empty()
                        && header.len() <= 64
                        && header.chars().all(|c| c.is_ascii_alphanumeric() || c == '-');
                    if !ok || header.eq_ignore_ascii_case("host") || header.eq_ignore_ascii_case("content-length") {
                        return Err(HubError("That header name cannot carry a credential.".into()));
                    }
                }
                "basic" => {
                    need(s, "username", "The user name")?;
                }
                other => {
                    return Err(HubError(format!(
                        "The sign-in kind “{other}” is not supported; use bearer, header or basic."
                    )));
                }
            }
            secret_needed("The credential")?;
        }
        ConnectionKind::Webhook => {
            let url = need(s, "url", "The webhook address")?;
            let parsed = url::Url::parse(url).map_err(|_| HubError("The webhook address is not valid.".into()))?;
            if parsed.scheme() != "https" && !parsed.host_str().is_some_and(zorix_providers::is_loopback) {
                return Err(HubError(
                    "A webhook must be reached over HTTPS (plain HTTP is only for this computer).".into(),
                ));
            }
            if !parsed.username().is_empty() || parsed.password().is_some() {
                return Err(HubError("The webhook address must not contain a user name or password.".into()));
            }
            secret_needed("A signing key")?;
        }
        ConnectionKind::Postgres => {
            need(s, "host", "The host")?;
            need(s, "database", "The database name")?;
            need(s, "user", "The user")?;
            if let Some(mode) = text(s, "sslmode")
                && !matches!(mode, "verify-full" | "disable")
            {
                return Err(HubError(
                    "sslmode is verify-full (the default) or disable (only for this computer).".into(),
                ));
            }
            if let Some(port) = s.get("port").filter(|p| !p.is_null())
                && !port.as_u64().is_some_and(|p| (1..=65535).contains(&p))
            {
                return Err(HubError("The port is a number from 1 to 65535.".into()));
            }
        }
        ConnectionKind::Sqlite => {
            let path = need(s, "path", "The path of the SQLite file")?;
            let real =
                std::fs::canonicalize(path).map_err(|_| HubError(format!("There is no SQLite file at {path}.")))?;
            if !real.is_file() {
                return Err(HubError(format!("{path} is not a file.")));
            }
            out["path"] = json!(real.to_string_lossy());
        }
    }
    Ok(out)
}

impl Hub {
    /// Sign in to a service and add the connection with what came of it. Nothing is added unless the sign-in succeeds.
    pub async fn sign_in(
        &self,
        mut request: ConnectRequest,
        app: SignInApp,
        ui: &dyn SignInUi,
        cancel: &CancellationToken,
    ) -> Result<Connection, HubError> {
        if request.secret.is_some() {
            return Err(HubError("A connection takes a token or a sign-in, not both.".into()));
        }
        request.settings["auth"] = json!("oauth");
        let settings = validate(&request)?;
        if self.store.connection_named(&self.tenant, &request.name).await?.is_some() {
            return Err(HubError(format!("There is already a connection called {}.", request.name)));
        }
        let client_id = app
            .client_id
            .filter(|id| !id.trim().is_empty())
            .or_else(|| signin::builtin_client_id(request.kind).map(str::to_string))
            .ok_or_else(|| {
                HubError(format!(
                    "This build of ZORIX has no {} app of its own: register one and give its client id.",
                    request.kind.as_str()
                ))
            })?;
        let endpoints =
            Endpoints::of(request.kind, text(&settings, "base_url")).map_err(|e| HubError(e.to_string()))?;
        let app = App { client_id, client_secret: app.client_secret };
        let signed = signin::sign_in(request.kind, &endpoints, &app, text(&settings, "base_url"), ui, cancel)
            .await
            .map_err(|e| HubError(e.to_string()))?;
        let connection = self
            .store
            .add_connection(
                &self.tenant,
                NewConnection { kind: request.kind, name: request.name.clone(), settings, has_secret: true },
                SystemClock.now(),
            )
            .await?;
        if let Err(e) = signin::keep(&signin::vault(self.secrets.clone(), connection.id), &signed).await {
            let _ = self.store.remove_connection(&self.tenant, connection.id).await;
            return Err(HubError(e.to_string()));
        }
        Ok(connection)
    }

    /// Add a connection and keep its secret. If the secret cannot be kept, nothing is added.
    pub async fn add_connection(&self, request: ConnectRequest) -> Result<Connection, HubError> {
        if text(&request.settings, "auth") == Some("oauth") {
            return Err(HubError("A connection is signed in to by signing in, not by giving settings.".into()));
        }
        let settings = validate(&request)?;
        let connection = self
            .store
            .add_connection(
                &self.tenant,
                NewConnection {
                    kind: request.kind,
                    name: request.name.clone(),
                    settings,
                    has_secret: request.secret.is_some(),
                },
                SystemClock.now(),
            )
            .await?;
        if let Some(secret) = request.secret
            && let Err(e) = self.keep(secret_name(connection.id), secret).await
        {
            let _ = self.store.remove_connection(&self.tenant, connection.id).await;
            return Err(e);
        }
        Ok(connection)
    }

    /// One connection by name.
    pub async fn connection(&self, name: &str) -> Result<Connection, HubError> {
        self.store
            .connection_named(&self.tenant, name)
            .await?
            .ok_or_else(|| HubError(format!("There is no connection called {name}. `zorix connect list` shows them.")))
    }

    /// Every connection, by name.
    pub async fn connections(&self) -> Result<Vec<Connection>, HubError> {
        Ok(self.store.connections(&self.tenant).await?)
    }

    /// Remove a connection and its secret.
    pub async fn remove_connection(&self, name: &str) -> Result<(), HubError> {
        let connection = self.connection(name).await?;
        if zorix_integrations::connections::signed_in(&connection) {
            signin::revoke(&signin::vault(self.secrets.clone(), connection.id)).await;
            self.services.signins.forget(connection.id);
        }
        self.store.remove_connection(&self.tenant, connection.id).await?;
        let secrets = self.secrets.clone();
        let key = secret_name(connection.id);
        let _ = tokio::task::spawn_blocking(move || secrets.remove(&key)).await;
        Ok(())
    }

    /// Test a connection and remember the result.
    pub async fn test_connection(&self, name: &str) -> Result<ConnectionTest, HubError> {
        let connection = self.connection(name).await?;
        let checked = test_connection(Arc::clone(&self.services), &connection).await;
        let test = ConnectionTest { at: SystemClock.now(), ok: checked.ok, note: checked.note };
        self.store.record_connection_test(&self.tenant, connection.id, test.clone()).await?;
        Ok(test)
    }
}
