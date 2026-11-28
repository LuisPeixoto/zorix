use std::sync::Arc;

use serde::Serialize;
use serde::de::DeserializeOwned;
use tokio::sync::Mutex;
use zorix_core::Clock;

use crate::error::OAuthError;
use crate::token::{TokenResponse, token_request, token_request_json};
use crate::vault::{Access, Vault};
use crate::{Reach, endpoint};

/// A token this close to expiring is refreshed first.
const REFRESH_MARGIN_MS: i64 = 60_000;

/// What is kept about a sign-in so that it can be renewed.
pub trait Renewable: Serialize + DeserializeOwned + Send + Sync {
    /// Where refresh tokens are exchanged.
    fn token_endpoint(&self) -> &str;
    /// The form of a renewal, or `None` when the sign-in has no way to renew itself.
    fn renewal(&self) -> Option<Vec<(&'static str, String)>>;
    /// Keep what the renewal brought (a rotated refresh token, the scopes).
    fn renewed(&mut self, tokens: &TokenResponse);
    /// The address of the thing signed in to, which decides what a token call may reach.
    fn resource(&self) -> &str;
    /// Whether the token endpoint takes JSON rather than a form.
    fn json(&self) -> bool {
        false
    }
}

/// One sign-in's tokens: hands out the access token and renews it when it runs out. Renewals are one at a time, so two callers
/// never spend the same refresh token twice.
pub struct Session<R> {
    vault: Vault,
    clock: Arc<dyn Clock>,
    cache: Mutex<Option<Access>>,
    /// How to sign in again, said in messages ("zorix mcp login github").
    hint: String,
    kept: std::marker::PhantomData<fn() -> R>,
}

impl<R: Renewable> Session<R> {
    /// A session over the tokens in `vault`.
    pub fn new(vault: Vault, clock: Arc<dyn Clock>, hint: &str) -> Self {
        Self { vault, clock, cache: Mutex::new(None), hint: hint.to_string(), kept: std::marker::PhantomData }
    }

    /// The vault the tokens are in.
    pub fn vault(&self) -> &Vault {
        &self.vault
    }

    /// An error saying to sign in again, and how.
    pub fn needs_login(&self, why: &str) -> OAuthError {
        OAuthError::NeedsLogin(format!("{why} (sign in again: {})", self.hint))
    }

    async fn renew(&self, cache: &mut Option<Access>) -> Result<Access, OAuthError> {
        let Some(mut kept) = self.vault.read::<R>(self.vault.registration()).await? else {
            return Err(self.needs_login("you are not signed in"));
        };
        let Some(form) = kept.renewal() else {
            *cache = None;
            return Err(self.needs_login("the sign-in expired and has no way to renew itself"));
        };
        let url = endpoint(kept.token_endpoint(), "token endpoint")
            .map_err(|_| self.needs_login("the stored sign-in is damaged"))?;
        let reach = match reqwest::Url::parse(kept.resource()) {
            Ok(resource) => Reach::of(&resource).await,
            Err(_) => Reach::internet_only(),
        };
        let asked = if kept.json() {
            token_request_json(&reach, &url, &form).await
        } else {
            token_request(&reach, &url, &form).await
        };
        let tokens = match asked {
            Ok(tokens) => tokens,
            Err(OAuthError::NeedsLogin(why)) => {
                self.vault.forget().await;
                *cache = None;
                return Err(self.needs_login(&why));
            }
            Err(other) => return Err(other),
        };
        kept.renewed(&tokens);
        let access = Access {
            token: tokens.access_token,
            expires_at_ms: tokens.expires_in.map(|s| self.clock.now().millis() + s * 1000),
        };
        self.vault.write(self.vault.registration(), &kept).await?;
        self.vault.write(self.vault.access(), &access).await?;
        *cache = Some(access.clone());
        Ok(access)
    }

    /// The access token to use now, renewed first when it is about to run out.
    pub async fn current(&self) -> Result<Access, OAuthError> {
        let mut cache = self.cache.lock().await;
        if cache.is_none() {
            *cache = self.vault.read::<Access>(self.vault.access()).await?;
        }
        let now = self.clock.now().millis();
        match cache.clone() {
            Some(access) if access.expires_at_ms.is_none_or(|at| at - REFRESH_MARGIN_MS > now) => Ok(access),
            Some(_) => self.renew(&mut cache).await,
            None if self.vault.signed_in().await => self.renew(&mut cache).await,
            None => Err(self.needs_login("you are not signed in")),
        }
    }

    /// Renew now, whatever the clock says (the server refused the token).
    pub async fn refresh(&self) -> Result<(), OAuthError> {
        let mut cache = self.cache.lock().await;
        self.renew(&mut cache).await.map(|_| ())
    }
}
