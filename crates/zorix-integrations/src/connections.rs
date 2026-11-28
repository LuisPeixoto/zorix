//! Finding the connection a tool should use, and its secret.

use std::sync::Arc;

use zorix_core::{Connection, ConnectionKind, IntegrationStore, TenantId, ToolError};
use zorix_providers::{SecretStore, get_async};

use crate::secret_name;
use crate::signin::SignIns;

/// What integration tools need from the rest of ZORIX.
pub struct Services {
    /// Where connections are kept.
    pub store: Arc<dyn IntegrationStore>,
    /// Where their secrets are kept.
    pub secrets: Arc<dyn SecretStore>,
    /// The sessions of the connections that were signed in to.
    pub signins: SignIns,
}

/// Whether a connection was signed in to, rather than given a token.
pub fn signed_in(connection: &Connection) -> bool {
    connection.settings["auth"] == "oauth"
}

fn failed(message: impl Into<String>) -> ToolError {
    ToolError::Failed(message.into())
}

impl Services {
    /// The connection of `kind` a call should use: the one named, or the only one there is.
    pub async fn pick(
        &self,
        tenant: &TenantId,
        kind: ConnectionKind,
        name: Option<&str>,
    ) -> Result<Option<Connection>, ToolError> {
        let all: Vec<Connection> = self
            .store
            .connections(tenant)
            .await
            .map_err(|e| failed(e.to_string()))?
            .into_iter()
            .filter(|c| c.kind == kind)
            .collect();
        match name {
            Some(name) => all
                .into_iter()
                .find(|c| c.name == name)
                .map(Some)
                .ok_or_else(|| failed(format!("There is no {} connection called {name}.", kind.as_str()))),
            None => match all.len() {
                0 | 1 => Ok(all.into_iter().next()),
                _ => Err(failed(format!(
                    "There is more than one {} connection ({}). Say which one with `connection`.",
                    kind.as_str(),
                    all.iter().map(|c| c.name.as_str()).collect::<Vec<_>>().join(", ")
                ))),
            },
        }
    }

    /// The credential of a connection and, for one signed in through a service's gateway, where its API answers.
    pub async fn credential(&self, connection: &Connection) -> Result<(Option<String>, Option<String>), ToolError> {
        if signed_in(connection) {
            let (token, base) =
                self.signins.current(self.secrets.clone(), connection.id, &connection.name).await.map_err(
                    |e| match e {
                        zorix_oauth::OAuthError::Transport(_) | zorix_oauth::OAuthError::Timeout(_) => {
                            ToolError::Retryable(e.to_string())
                        }
                        other => failed(format!("The {} connection needs a new sign-in: {other}", connection.name)),
                    },
                )?;
            return Ok((Some(token), base));
        }
        Ok((self.secret(connection).await?, None))
    }

    /// The secret of a connection, if it has one.
    pub async fn secret(&self, connection: &Connection) -> Result<Option<String>, ToolError> {
        if signed_in(connection) {
            return Ok(Box::pin(self.credential(connection)).await?.0);
        }
        if !connection.has_secret {
            return Ok(None);
        }
        let found =
            get_async(self.secrets.clone(), secret_name(connection.id)).await.map_err(|e| failed(e.to_string()))?;
        Ok(found.map(|s| s.expose().to_string()))
    }
}

/// Percent-encode text for a path segment or a query value.
pub fn encode(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    for byte in text.bytes() {
        if byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'.' | b'_' | b'~') {
            out.push(byte as char);
        } else {
            out.push_str(&format!("%{byte:02X}"));
        }
    }
    out
}

/// `?a=1&b=2` with every value encoded; pairs with no value are left out.
pub fn query(pairs: &[(&str, Option<String>)]) -> String {
    let parts: Vec<String> =
        pairs.iter().filter_map(|(k, v)| v.as_ref().map(|v| format!("{k}={}", encode(v)))).collect();
    if parts.is_empty() { String::new() } else { format!("?{}", parts.join("&")) }
}

/// At most `max` characters of `text`, with a mark when it was cut.
pub fn clip(text: &str, max: usize) -> String {
    if text.chars().count() <= max {
        return text.to_string();
    }
    let kept: String = text.chars().take(max).collect();
    format!("{kept}… [cut: {} more characters]", text.chars().count() - max)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn values_are_encoded_for_addresses_and_empty_ones_are_left_out() {
        assert_eq!(encode("group/sub project"), "group%2Fsub%20project");
        assert_eq!(encode("a.b-c_d~e"), "a.b-c_d~e");
        assert_eq!(encode("é&=?"), "%C3%A9%26%3D%3F");
        assert_eq!(
            query(&[("q", Some("a b".into())), ("page", None), ("state", Some("open".into()))]),
            "?q=a%20b&state=open"
        );
        assert_eq!(query(&[("page", None)]), "");
    }

    #[test]
    fn long_text_is_cut_on_a_character_boundary_and_says_so() {
        assert_eq!(clip("short", 10), "short");
        assert_eq!(clip("éééééé", 3), "ééé… [cut: 3 more characters]");
    }
}
