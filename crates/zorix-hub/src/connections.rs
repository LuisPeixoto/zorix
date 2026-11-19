//! Adding, testing and removing connections: what a person types, checked, and the secret put in the keychain.

use std::sync::Arc;

use serde_json::{Value, json};
use zorix_core::{
    Clock, Connection, ConnectionKind, ConnectionTest, IntegrationStore, NewConnection, Secret, SystemClock,
};
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
    match request.kind {
        ConnectionKind::Github | ConnectionKind::Gitlab => base(s, "base_url")?,
        ConnectionKind::Jira => {
            need(s, "base_url", "The site address (for example https://acme.atlassian.net)")?;
            base(s, "base_url")?;
            if text(s, "auth") != Some("bearer") {
                need(s, "email", "The account email")?;
            }
            secret_needed("An API token")?;
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
    /// Add a connection and keep its secret. If the secret cannot be kept, nothing is added.
    pub async fn add_connection(&self, request: ConnectRequest) -> Result<Connection, HubError> {
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
