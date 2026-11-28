use std::sync::Arc;

use base64::Engine;
use base64::engine::general_purpose::URL_SAFE_NO_PAD;
use serde::{Deserialize, Serialize};
use zorix_core::Secret;
use zorix_providers::{SecretStore, get_async};

use crate::error::OAuthError;

/// A short-lived access token and when it runs out.
#[derive(Clone, Serialize, Deserialize)]
pub struct Access {
    /// The token.
    pub token: String,
    /// When it stops working, in Unix milliseconds, when the server said.
    pub expires_at_ms: Option<i64>,
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

    /// The name of the entry that holds what renews the sign-in.
    pub fn registration(&self) -> &str {
        &self.registration
    }

    /// The name of the entry that holds the access token.
    pub fn access(&self) -> &str {
        &self.access
    }

    /// What is kept under `name`.
    pub async fn read<T: for<'de> Deserialize<'de>>(&self, name: &str) -> Result<Option<T>, OAuthError> {
        let Some(secret) = get_async(self.secrets.clone(), name.to_string())
            .await
            .map_err(|e| OAuthError::NeedsLogin(e.to_string()))?
        else {
            return Ok(None);
        };
        let bytes = URL_SAFE_NO_PAD
            .decode(secret.expose())
            .map_err(|_| OAuthError::NeedsLogin("the stored sign-in is damaged; sign in again".into()))?;
        serde_json::from_slice(&bytes)
            .map(Some)
            .map_err(|_| OAuthError::NeedsLogin("the stored sign-in is damaged; sign in again".into()))
    }

    /// Keep `value` under `name`.
    pub async fn write<T: Serialize>(&self, name: &str, value: &T) -> Result<(), OAuthError> {
        let text = URL_SAFE_NO_PAD.encode(serde_json::to_vec(value).map_err(|e| OAuthError::Protocol(e.to_string()))?);
        let secrets = self.secrets.clone();
        let name = name.to_string();
        tokio::task::spawn_blocking(move || secrets.set(&name, &Secret::new(text)))
            .await
            .map_err(|e| OAuthError::Protocol(e.to_string()))?
            .map_err(|e| OAuthError::Protocol(format!("the sign-in could not be kept: {e}")))
    }

    /// Whether anything is kept.
    pub async fn signed_in(&self) -> bool {
        matches!(self.read::<serde_json::Value>(&self.registration).await, Ok(Some(_)))
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
