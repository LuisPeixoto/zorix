use std::time::Duration;

use base64::Engine;
use base64::engine::general_purpose::URL_SAFE_NO_PAD;
use reqwest::Url;
use serde_json::Value;
use sha2::{Digest, Sha256};
use zorix_core::redact_text;

use crate::error::OAuthError;
use crate::reach::{Reach, read_json};

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

/// The answer of a token endpoint.
pub struct TokenResponse {
    /// The access token.
    pub access_token: String,
    /// A new refresh token, when one came.
    pub refresh_token: Option<String>,
    /// How long the access token lasts, in seconds.
    pub expires_in: Option<i64>,
    /// The scopes granted, when the server said.
    pub scope: Option<String>,
}

/// A form body, `application/x-www-form-urlencoded`.
pub fn form_body(pairs: &[(&str, String)]) -> String {
    let mut url = Url::parse("http://form.invalid/").expect("a constant address");
    url.query_pairs_mut().extend_pairs(pairs.iter().map(|(k, v)| (*k, v.as_str())));
    url.query().unwrap_or("").to_string()
}

/// Exchange a code or a refresh token for tokens.
pub async fn token_request(
    reach: &Reach,
    endpoint: &Url,
    form: &[(&str, String)],
) -> Result<TokenResponse, OAuthError> {
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
        .map_err(|e| OAuthError::Transport(redact_text(&e.without_url().to_string())))?;
    let ok = response.status().is_success();
    let body = read_json(response).await?;
    if !ok {
        let error = body.get("error").and_then(Value::as_str).unwrap_or("error");
        let why = body.get("error_description").and_then(Value::as_str).map(redact_text).unwrap_or_default();
        return Err(if error == "invalid_grant" {
            OAuthError::NeedsLogin("the sign-in expired or was revoked; sign in again".into())
        } else {
            OAuthError::Protocol(format!(
                "the authorization server refused the request ({error}{}{why})",
                if why.is_empty() { "" } else { ": " }
            ))
        });
    }
    if !body.get("token_type").and_then(Value::as_str).is_some_and(|t| t.eq_ignore_ascii_case("bearer")) {
        return Err(OAuthError::Protocol(
            "the authorization server returned a token that is not a bearer token".into(),
        ));
    }
    Ok(TokenResponse {
        access_token: body
            .get("access_token")
            .and_then(Value::as_str)
            .ok_or_else(|| OAuthError::Protocol("the token response had no access token".into()))?
            .to_string(),
        refresh_token: body.get("refresh_token").and_then(Value::as_str).map(str::to_string),
        expires_in: body.get("expires_in").and_then(Value::as_i64),
        scope: body.get("scope").and_then(Value::as_str).map(str::to_string),
    })
}
