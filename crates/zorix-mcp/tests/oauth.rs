//! Signing in to a remote server (ADR-0019) against a mock that is both the MCP server and its authorization server.

use std::sync::{Arc, Mutex};
use std::time::Duration;

use base64::Engine;
use base64::engine::general_purpose::URL_SAFE_NO_PAD;
use reqwest::Url;
use serde_json::{Value, json};
use sha2::{Digest, Sha256};
use tokio_util::sync::CancellationToken;
use wiremock::matchers::{method, path};
use wiremock::{Mock, MockServer, Request, ResponseTemplate};
use zorix_core::{ManualClock, Timestamp};
use zorix_mcp::{AuthProvider, BrowserOpener, McpError, OAuthClient, OAuthSession, OAuthSettings, Vault};
use zorix_providers::{MemoryStore, SecretStore};

/// What the mock authorization server does, so one test can make it misbehave.
#[derive(Clone)]
struct Behaviour {
    iss_advertised: bool,
    pkce: bool,
    registration: bool,
    issuer_in_metadata: Option<String>,
    resource_in_metadata: Option<String>,
    /// How the server names itself: appended to its address (`""` is the origin).
    resource_style: Option<&'static str>,
    refresh_fails: bool,
}

impl Default for Behaviour {
    fn default() -> Self {
        Self {
            iss_advertised: true,
            pkce: true,
            registration: true,
            issuer_in_metadata: None,
            resource_in_metadata: None,
            resource_style: None,
            refresh_fails: false,
        }
    }
}

async fn mock(b: Behaviour) -> MockServer {
    let server = MockServer::start().await;
    let base = server.uri();
    Mock::given(method("POST"))
        .and(path("/mcp"))
        .respond_with(
            ResponseTemplate::new(401).insert_header(
                "WWW-Authenticate",
                format!("Bearer resource_metadata=\"{base}/.well-known/oauth-protected-resource/mcp\", scope=\"read\"")
                    .as_str(),
            ),
        )
        .mount(&server)
        .await;
    Mock::given(method("GET"))
        .and(path("/.well-known/oauth-protected-resource/mcp"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "resource": match b.resource_style { Some(style) => format!("{base}{style}"), None => b.resource_in_metadata.clone().unwrap_or(format!("{base}/mcp")) },
            "authorization_servers": [base],
            "scopes_supported": ["read", "write"],
        })))
        .mount(&server)
        .await;
    let mut meta = json!({
        "issuer": b.issuer_in_metadata.clone().unwrap_or(base.clone()),
        "authorization_endpoint": format!("{base}/authorize"),
        "token_endpoint": format!("{base}/token"),
        "revocation_endpoint": format!("{base}/revoke"),
        "authorization_response_iss_parameter_supported": b.iss_advertised,
    });
    if b.pkce {
        meta["code_challenge_methods_supported"] = json!(["S256"]);
    }
    if b.registration {
        meta["registration_endpoint"] = json!(format!("{base}/register"));
    }
    Mock::given(method("GET"))
        .and(path("/.well-known/oauth-authorization-server"))
        .respond_with(ResponseTemplate::new(200).set_body_json(meta))
        .mount(&server)
        .await;
    Mock::given(method("POST"))
        .and(path("/register"))
        .respond_with(ResponseTemplate::new(201).set_body_json(json!({"client_id": "cid-1"})))
        .mount(&server)
        .await;
    let refresh_fails = b.refresh_fails;
    Mock::given(method("POST"))
        .and(path("/token"))
        .respond_with(move |request: &Request| {
            let body = String::from_utf8_lossy(&request.body).into_owned();
            if body.contains("grant_type=refresh_token") {
                if refresh_fails {
                    return ResponseTemplate::new(400).set_body_json(json!({"error": "invalid_grant", "error_description": "token revoked"}));
                }
                let n = if body.contains("refresh_token=rt-1") { 2 } else { 3 };
                return ResponseTemplate::new(200).set_body_json(json!({"access_token": format!("at-{n}"), "token_type": "bearer", "expires_in": 3600, "refresh_token": format!("rt-{n}")}));
            }
            ResponseTemplate::new(200).set_body_json(json!({"access_token": "at-1", "token_type": "Bearer", "expires_in": 3600, "refresh_token": "rt-1", "scope": "read"}))
        })
        .mount(&server)
        .await;
    Mock::given(method("POST")).and(path("/revoke")).respond_with(ResponseTemplate::new(200)).mount(&server).await;
    server
}

/// A browser that, instead of showing anything, comes straight back to the redirect address.
type Answer = Box<dyn Fn(&str, &str) -> String + Send + Sync>;

struct Consent {
    seen: Mutex<Option<Url>>,
    /// Builds the query the "browser" brings back, from the state and the issuer.
    answer: Answer,
    knock_first: bool,
}

impl Consent {
    fn approve(issuer: &str) -> Arc<Self> {
        let issuer = issuer.to_string();
        Arc::new(Self {
            seen: Mutex::new(None),
            answer: Box::new(move |state, _| format!("code=abc&state={state}&iss={issuer}")),
            knock_first: true,
        })
    }

    fn with(answer: impl Fn(&str, &str) -> String + Send + Sync + 'static) -> Arc<Self> {
        Arc::new(Self { seen: Mutex::new(None), answer: Box::new(answer), knock_first: false })
    }

    fn param(&self, name: &str) -> Option<String> {
        self.seen
            .lock()
            .unwrap()
            .as_ref()
            .and_then(|u| u.query_pairs().find(|(k, _)| k == name).map(|(_, v)| v.into_owned()))
    }
}

impl BrowserOpener for Consent {
    fn open(&self, url: &str) -> Result<(), String> {
        let url = Url::parse(url).unwrap();
        let get = |n: &str| url.query_pairs().find(|(k, _)| k == n).map(|(_, v)| v.into_owned()).unwrap();
        let (redirect, state) = (get("redirect_uri"), get("state"));
        *self.seen.lock().unwrap() = Some(url.clone());
        let target = format!("{redirect}?{}", (self.answer)(&state, &redirect));
        let knock = self.knock_first;
        let origin = redirect.trim_end_matches("/callback").to_string();
        tokio::spawn(async move {
            if knock {
                // Something else on this computer knocks first; it is turned away and the sign-in goes on waiting.
                let _ = reqwest::get(format!("{origin}/favicon.ico")).await;
            }
            let _ = reqwest::get(target).await;
        });
        Ok(())
    }
}

struct Rig {
    server: MockServer,
    secrets: Arc<MemoryStore>,
    clock: Arc<ManualClock>,
}

async fn rig(b: Behaviour) -> Rig {
    Rig {
        server: mock(b).await,
        secrets: Arc::new(MemoryStore::new()),
        clock: Arc::new(ManualClock::new(Timestamp(1_000_000))),
    }
}

impl Rig {
    fn url(&self) -> Url {
        Url::parse(&format!("{}/mcp", self.server.uri())).unwrap()
    }

    fn vault(&self) -> Vault {
        Vault::new(self.secrets.clone(), "mcp.abc123")
    }

    fn client(&self) -> OAuthClient {
        OAuthClient::new(self.vault(), self.clock.clone()).unwrap()
    }

    fn session(&self) -> OAuthSession {
        OAuthSession::new(self.vault(), self.clock.clone(), "zorix mcp login github").unwrap()
    }

    async fn login(
        &self,
        browser: &Arc<Consent>,
        settings: OAuthSettings,
    ) -> Result<zorix_mcp::LoginSummary, McpError> {
        let browser: Arc<dyn BrowserOpener> = browser.clone();
        tokio::time::timeout(
            Duration::from_secs(20),
            self.client().login(&self.url(), &settings, browser.as_ref(), &CancellationToken::new()),
        )
        .await
        .expect("login finishes")
    }

    async fn requests_to(&self, p: &str) -> Vec<Request> {
        self.server.received_requests().await.unwrap().into_iter().filter(|r| r.url.path() == p).collect()
    }
}

fn form_value(request: &Request, name: &str) -> Option<String> {
    Url::parse(&format!("http://x/?{}", String::from_utf8_lossy(&request.body)))
        .unwrap()
        .query_pairs()
        .find(|(k, _)| k == name)
        .map(|(_, v)| v.into_owned())
}

#[tokio::test]
async fn a_person_signs_in_with_pkce_the_resource_and_a_client_the_server_registered() {
    let rig = rig(Behaviour::default()).await;
    let browser = Consent::approve(&rig.server.uri());
    let summary = rig.login(&browser, OAuthSettings::default()).await.unwrap();
    assert_eq!(
        (summary.issuer.as_str(), summary.scope.as_deref(), summary.refreshable),
        (rig.server.uri().as_str(), Some("read"), true)
    );

    // What the person's browser was sent to.
    assert_eq!(browser.param("response_type").as_deref(), Some("code"));
    assert_eq!(browser.param("client_id").as_deref(), Some("cid-1"));
    assert_eq!(browser.param("code_challenge_method").as_deref(), Some("S256"));
    assert_eq!(
        browser.param("resource"),
        Some(format!("{}/mcp", rig.server.uri())),
        "the server's address, with no trailing slash"
    );
    assert_eq!(browser.param("scope").as_deref(), Some("read"), "the scope the server's challenge named");
    assert!(browser.param("redirect_uri").unwrap().starts_with("http://127.0.0.1:"));
    assert!(browser.param("state").unwrap().len() >= 24);

    // The client registered itself as a public client on a loopback address.
    let registered: Value = serde_json::from_slice(&rig.requests_to("/register").await[0].body).unwrap();
    assert_eq!(registered["token_endpoint_auth_method"], "none");
    assert_eq!(registered["client_name"], "ZORIX");
    assert!(registered["redirect_uris"][0].as_str().unwrap().starts_with("http://127.0.0.1:"));

    // The code was exchanged with a verifier that matches the challenge, and the resource.
    let exchange = &rig.requests_to("/token").await[0];
    let verifier = form_value(exchange, "code_verifier").unwrap();
    assert_eq!(URL_SAFE_NO_PAD.encode(Sha256::digest(verifier.as_bytes())), browser.param("code_challenge").unwrap());
    assert!(verifier.len() >= 43, "RFC 7636 asks for at least 43 characters");
    assert_eq!(form_value(exchange, "resource"), browser.param("resource"));
    assert_eq!(form_value(exchange, "grant_type").as_deref(), Some("authorization_code"));
    assert_eq!(form_value(exchange, "code").as_deref(), Some("abc"));

    // The tokens are in the keychain under names with no secret in them, and the session hands them out.
    assert!(rig.vault().signed_in().await);
    assert!(
        rig.secrets.get("mcp.abc123.access").unwrap().is_some()
            && rig.secrets.get("mcp.abc123.oauth").unwrap().is_some()
    );
    assert_eq!(rig.session().authorization().await.unwrap().as_deref(), Some("Bearer at-1"));
}

#[tokio::test]
async fn a_token_that_ran_out_is_refreshed_with_the_resource_and_the_new_refresh_token_is_kept() {
    let rig = rig(Behaviour::default()).await;
    rig.login(&Consent::approve(&rig.server.uri()), OAuthSettings::default()).await.unwrap();
    let session = rig.session();
    assert_eq!(session.authorization().await.unwrap().as_deref(), Some("Bearer at-1"));
    rig.clock.advance(3_600_000 - 30_000); // less than a minute left: renewed before it fails
    assert_eq!(session.authorization().await.unwrap().as_deref(), Some("Bearer at-2"));
    let refresh = rig.requests_to("/token").await.pop().unwrap();
    assert_eq!(form_value(&refresh, "grant_type").as_deref(), Some("refresh_token"));
    assert_eq!(form_value(&refresh, "refresh_token").as_deref(), Some("rt-1"));
    assert_eq!(form_value(&refresh, "resource").unwrap(), format!("{}/mcp", rig.server.uri()));
    rig.clock.advance(3_600_000);
    assert_eq!(session.authorization().await.unwrap().as_deref(), Some("Bearer at-3"));
    let second = rig.requests_to("/token").await.pop().unwrap();
    assert_eq!(form_value(&second, "refresh_token").as_deref(), Some("rt-2"), "the rotated refresh token was kept");
    // A fresh session (a restart) finds the same tokens in the keychain.
    assert_eq!(rig.session().authorization().await.unwrap().as_deref(), Some("Bearer at-3"));
}

#[tokio::test]
async fn a_refresh_the_server_refuses_forgets_the_sign_in_and_says_how_to_sign_in_again() {
    let rig = rig(Behaviour { refresh_fails: true, ..Behaviour::default() }).await;
    rig.login(&Consent::approve(&rig.server.uri()), OAuthSettings::default()).await.unwrap();
    rig.clock.advance(3_700_000);
    let error = rig.session().authorization().await.unwrap_err();
    match error {
        McpError::NeedsLogin(why) => {
            assert!(why.contains("zorix mcp login github") && why.contains("expired or was revoked"), "{why}")
        }
        other => panic!("{other:?}"),
    }
    assert!(!rig.vault().signed_in().await, "the dead sign-in is gone");
    assert!(matches!(rig.session().authorization().await, Err(McpError::NeedsLogin(_))));
}

#[tokio::test]
async fn an_answer_that_does_not_match_what_was_asked_is_ignored_and_no_code_is_spent() {
    let issuer_of = |rig: &Rig| rig.server.uri();
    // The wrong state.
    let rig_a = rig(Behaviour::default()).await;
    let iss = issuer_of(&rig_a);
    let error = rig_a
        .login(&Consent::with(move |_, _| format!("code=abc&state=forged&iss={iss}")), OAuthSettings::default())
        .await
        .unwrap_err();
    assert!(matches!(&error, McpError::Protocol(m) if m.contains("did not match")), "{error:?}");
    assert!(rig_a.requests_to("/token").await.is_empty(), "the code was never sent anywhere");
    // The wrong issuer (a mix-up attack).
    let rig_b = rig(Behaviour::default()).await;
    let error = rig_b
        .login(
            &Consent::with(|state, _| format!("code=abc&state={state}&iss=https://evil.example")),
            OAuthSettings::default(),
        )
        .await
        .unwrap_err();
    assert!(matches!(&error, McpError::Protocol(m) if m.contains("different authorization server")), "{error:?}");
    assert!(rig_b.requests_to("/token").await.is_empty());
    // No issuer, though the server said it would send one.
    let rig_c = rig(Behaviour::default()).await;
    let error = rig_c
        .login(&Consent::with(|state, _| format!("code=abc&state={state}")), OAuthSettings::default())
        .await
        .unwrap_err();
    assert!(matches!(&error, McpError::Protocol(m) if m.contains("did not say who it is")), "{error:?}");
    // No issuer and none promised: allowed.
    let rig_d = rig(Behaviour { iss_advertised: false, ..Behaviour::default() }).await;
    rig_d.login(&Consent::with(|state, _| format!("code=abc&state={state}")), OAuthSettings::default()).await.unwrap();
}

#[tokio::test]
async fn a_refusal_is_reported_but_what_a_stranger_wrote_in_an_answer_is_not_shown() {
    let rig = rig(Behaviour::default()).await;
    let iss = rig.server.uri();
    let error = rig
        .login(
            &Consent::with(move |state, _| format!("error=access_denied&state={state}&iss={iss}")),
            OAuthSettings::default(),
        )
        .await
        .unwrap_err();
    assert!(
        matches!(&error, McpError::Protocol(m) if m.contains("not completed") && m.contains("access_denied")),
        "{error:?}"
    );
    let rig = self::rig(Behaviour::default()).await;
    let error = rig
        .login(
            &Consent::with(|state, _| format!("error=Send%20money&state={state}&iss=https://evil.example")),
            OAuthSettings::default(),
        )
        .await
        .unwrap_err();
    assert!(!error.to_string().contains("Send money"), "an answer from the wrong issuer is not shown: {error}");
}

#[tokio::test]
async fn a_server_that_cannot_prove_its_metadata_or_pkce_is_not_signed_in_to() {
    for (behaviour, expect) in [
        (Behaviour { pkce: false, ..Behaviour::default() }, "PKCE"),
        (
            Behaviour { issuer_in_metadata: Some("https://other.example".into()), ..Behaviour::default() },
            "different issuer",
        ),
        (
            Behaviour {
                resource_in_metadata: Some("https://another-server.example/mcp".into()),
                ..Behaviour::default()
            },
            "not for",
        ),
    ] {
        let rig = rig(behaviour).await;
        let error = rig.login(&Consent::approve(&rig.server.uri()), OAuthSettings::default()).await.unwrap_err();
        assert!(matches!(&error, McpError::Protocol(m) if m.contains(expect)), "{expect}: {error:?}");
        assert!(rig.requests_to("/token").await.is_empty());
    }
}

#[tokio::test]
async fn a_server_may_name_itself_by_its_origin_or_with_a_trailing_slash_and_that_name_is_what_is_asked_for() {
    // Seen in the wild: one server names itself `https://host`, another `https://host/mcp/`.
    for style in ["", "/mcp/"] {
        let rig = rig(Behaviour { resource_style: Some(style), ..Behaviour::default() }).await;
        let browser = Consent::approve(&rig.server.uri());
        rig.login(&browser, OAuthSettings::default()).await.unwrap();
        assert_eq!(
            browser.param("resource"),
            Some(format!("{}{style}", rig.server.uri())),
            "the server's own name for itself"
        );
        assert_eq!(form_value(&rig.requests_to("/token").await[0], "resource"), browser.param("resource"));
    }
    // A name that is not this server or one of its parents is still refused.
    let rig = rig(Behaviour { resource_style: Some("/other"), ..Behaviour::default() }).await;
    assert!(rig.login(&Consent::approve(&rig.server.uri()), OAuthSettings::default()).await.is_err());
}

#[tokio::test]
async fn a_server_that_does_not_register_clients_needs_a_client_id_from_the_person() {
    let rig = rig(Behaviour { registration: false, ..Behaviour::default() }).await;
    let error = rig.login(&Consent::approve(&rig.server.uri()), OAuthSettings::default()).await.unwrap_err();
    assert!(matches!(&error, McpError::Protocol(m) if m.contains("--client-id")), "{error:?}");
    let browser = Consent::approve(&rig.server.uri());
    rig.login(&browser, OAuthSettings { client_id: Some("mine".into()), scopes: vec!["write".into(), "read".into()] })
        .await
        .unwrap();
    assert_eq!(browser.param("client_id").as_deref(), Some("mine"));
    assert_eq!(browser.param("scope").as_deref(), Some("write read"), "the person's scopes win");
    assert!(rig.requests_to("/register").await.is_empty());
}

#[tokio::test]
async fn a_server_that_asks_for_no_sign_in_is_not_given_one() {
    let server = MockServer::start().await;
    Mock::given(method("POST")).respond_with(ResponseTemplate::new(200).set_body_json(json!({}))).mount(&server).await;
    let secrets = Arc::new(MemoryStore::new());
    let client = OAuthClient::new(Vault::new(secrets, "mcp.x"), Arc::new(ManualClock::new(Timestamp(0)))).unwrap();
    let browser: Arc<dyn BrowserOpener> = Consent::approve("x");
    let error = client
        .login(
            &Url::parse(&format!("{}/mcp", server.uri())).unwrap(),
            &OAuthSettings::default(),
            browser.as_ref(),
            &CancellationToken::new(),
        )
        .await
        .unwrap_err();
    assert!(matches!(&error, McpError::Protocol(m) if m.contains("did not ask for a sign-in")), "{error:?}");
}

#[tokio::test]
async fn more_permission_is_a_new_sign_in_and_logout_revokes_and_forgets() {
    let rig = rig(Behaviour::default()).await;
    rig.login(&Consent::approve(&rig.server.uri()), OAuthSettings::default()).await.unwrap();
    let session = rig.session();
    let error =
        session.challenged(403, Some("Bearer error=\"insufficient_scope\", scope=\"files:write\"")).await.unwrap_err();
    assert!(matches!(&error, McpError::NeedsLogin(why) if why.contains("files:write")), "{error:?}");
    rig.client().logout().await;
    assert!(!rig.vault().signed_in().await);
    assert!(rig.secrets.get("mcp.abc123.access").unwrap().is_none());
    let revoked = rig.requests_to("/revoke").await;
    assert_eq!(form_value(&revoked[0], "token").as_deref(), Some("rt-1"), "the authorization server was told");
}

#[tokio::test]
async fn the_signed_in_session_authorises_real_requests_to_the_server() {
    use zorix_mcp::{HttpTransport, Transport};
    let rig = rig(Behaviour::default()).await;
    rig.login(&Consent::approve(&rig.server.uri()), OAuthSettings::default()).await.unwrap();
    let ok = MockServer::start().await;
    Mock::given(method("POST"))
        .respond_with(|request: &Request| {
            if request.headers.get("authorization").and_then(|v| v.to_str().ok()) == Some("Bearer at-1") {
                ResponseTemplate::new(200)
                    .set_body_json(json!({"jsonrpc": "2.0", "id": 1, "result": {"supportedVersions": ["2026-07-28"]}}))
            } else {
                ResponseTemplate::new(401)
            }
        })
        .mount(&ok)
        .await;
    let transport = HttpTransport::new(&format!("{}/mcp", ok.uri()), Arc::new(rig.session())).unwrap();
    let answer = transport
        .request(
            "server/discover",
            json!({"_meta": {"io.modelcontextprotocol/protocolVersion": "2026-07-28"}}),
            Duration::from_secs(5),
            &CancellationToken::new(),
        )
        .await
        .unwrap();
    assert_eq!(answer["supportedVersions"][0], "2026-07-28");
    // With nothing stored, the request does not even leave: a person has to sign in.
    let nobody =
        OAuthSession::new(Vault::new(Arc::new(MemoryStore::new()), "mcp.none"), rig.clock.clone(), "zorix mcp login x")
            .unwrap();
    let error = nobody.authorization().await.unwrap_err();
    assert!(matches!(error, McpError::NeedsLogin(_)));
}

#[test]
fn challenges_are_read_quoted_unquoted_and_with_escapes() {
    use zorix_mcp::oauth::{Challenge, parse_challenge};
    assert_eq!(
        parse_challenge(
            "Bearer resource_metadata=\"https://m.example/.well-known/oauth-protected-resource\", scope=\"files:read files:write\""
        ),
        Challenge {
            resource_metadata: Some("https://m.example/.well-known/oauth-protected-resource".into()),
            scope: Some("files:read files:write".into()),
            error: None
        }
    );
    let c = parse_challenge("Bearer error=insufficient_scope, scope=\"a\\\"b\", realm=\"x\"");
    assert_eq!((c.error.as_deref(), c.scope.as_deref()), (Some("insufficient_scope"), Some("a\"b")));
    assert_eq!(parse_challenge("Basic realm=x"), Challenge::default());
    assert_eq!(parse_challenge(""), Challenge::default());
}

#[test]
fn the_resource_is_the_servers_address_without_a_trailing_slash_fragment_or_query() {
    use zorix_mcp::oauth::canonical_resource;
    for (given, want) in [
        ("https://mcp.example.com/mcp", "https://mcp.example.com/mcp"),
        ("https://mcp.example.com/", "https://mcp.example.com"),
        ("https://mcp.example.com", "https://mcp.example.com"),
        ("https://mcp.example.com:8443/server/mcp/?x=1", "https://mcp.example.com:8443/server/mcp"),
    ] {
        assert_eq!(canonical_resource(&Url::parse(given).unwrap()), want, "{given}");
    }
}
