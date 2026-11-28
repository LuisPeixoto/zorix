use std::time::Duration;

use reqwest::{Client, Url};
use serde_json::Value;
use zorix_core::redact_text;
use zorix_net::{NetError, forbidden, resolve};
use zorix_providers::is_loopback;

use crate::error::OAuthError;

/// The most of a discovery or token response that is read.
const MAX_BODY: usize = 1024 * 1024;

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
    pub async fn client(&self, url: &Url) -> Result<Client, OAuthError> {
        let host = url.host_str().ok_or_else(|| OAuthError::Protocol("a sign-in address has no host".into()))?;
        let port = url.port_or_known_default().unwrap_or(443);
        let addresses = resolve(host, port, self.private_ok).await.map_err(|e| match e {
            NetError::Private { host, why, .. } => OAuthError::Protocol(format!(
                "the server asked ZORIX to contact {host}, which is {why}, and a server that is not on your own network may not send it there, so the sign-in was stopped"
            )),
            NetError::NotFound(host) => OAuthError::Transport(format!("{host} could not be found")),
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
        builder.build().map_err(|e| OAuthError::Spawn(format!("could not start the HTTP client: {e}")))
    }
}

/// Whether the environment names a proxy for web requests.
fn through_a_proxy() -> bool {
    ["HTTPS_PROXY", "https_proxy", "ALL_PROXY", "all_proxy"]
        .iter()
        .any(|v| std::env::var(v).is_ok_and(|p| !p.is_empty()))
}

/// An address of an authorization server: `https`, or `http` on this computer.
pub fn endpoint(text: &str, what: &str) -> Result<Url, OAuthError> {
    let url = Url::parse(text)
        .map_err(|_| OAuthError::Protocol(format!("the authorization server's {what} is not a valid address")))?;
    match (url.scheme(), url.host_str()) {
        ("https", Some(_)) => Ok(url),
        ("http", Some(host)) if is_loopback(host) => Ok(url),
        _ => Err(OAuthError::Protocol(format!("the authorization server's {what} does not use HTTPS"))),
    }
}

/// A bounded JSON body.
pub async fn read_json(response: reqwest::Response) -> Result<Value, OAuthError> {
    let mut body = Vec::new();
    let mut response = response;
    while let Some(chunk) =
        response.chunk().await.map_err(|e| OAuthError::Transport(redact_text(&e.without_url().to_string())))?
    {
        if body.len() + chunk.len() > MAX_BODY {
            return Err(OAuthError::TooLarge { limit: MAX_BODY });
        }
        body.extend_from_slice(&chunk);
    }
    serde_json::from_slice(&body).map_err(|_| OAuthError::Protocol("a sign-in response was not JSON".into()))
}

/// A JSON document at `url`, or `None` when it is not there.
pub async fn get_json(reach: &Reach, url: &Url) -> Result<Option<Value>, OAuthError> {
    let response = reach
        .client(url)
        .await?
        .get(url.clone())
        .header("Accept", "application/json")
        .timeout(Duration::from_secs(15))
        .send()
        .await
        .map_err(|e| OAuthError::Transport(redact_text(&e.without_url().to_string())))?;
    if !response.status().is_success() {
        return Ok(None);
    }
    read_json(response).await.map(Some)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::error::OAuthError;

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
                matches!(&error, OAuthError::Protocol(m) if m.contains("may not send it there")),
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
