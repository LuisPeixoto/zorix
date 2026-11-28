//! Signing in to GitHub, GitLab and Jira against a mock service: what is sent, what is refused, where the tokens go and how
//! they are renewed (ADR-0053).

use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};

use reqwest::Url;
use serde_json::{Value, json};
use tokio_util::sync::CancellationToken;
use wiremock::matchers::{body_string_contains, header, method, path};
use wiremock::{Mock, MockServer, Request, Respond, ResponseTemplate};
use zorix_core::{ConnectionKind, IntegrationStore, NewConnection, Secret, TenantId, Timestamp};
use zorix_integrations::Services;
use zorix_integrations::signin::{self, App, Endpoints, SignInUi};
use zorix_providers::MemoryStore;
use zorix_store::SqliteIntegrationStore;

fn endpoints(server: &MockServer) -> Endpoints {
    let at = |p: &str| Url::parse(&format!("{}{p}", server.uri())).unwrap();
    Endpoints {
        web: at(""),
        start: at("/start"),
        token: at("/token"),
        revoke: Some(at("/revoke")),
        resources: Some(at("/resources")),
        port: 0,
    }
}

fn app(secret: Option<&str>) -> App {
    App { client_id: "app-1".into(), client_secret: secret.map(Secret::new) }
}

/// What the person's side saw, and what their browser does when it is asked to open a page.
#[derive(Default)]
struct Browser {
    codes: parking_lot::Mutex<Vec<String>>,
    pages: parking_lot::Mutex<Vec<Url>>,
    /// The `state` the browser sends back first, to try one that is not the sign-in's.
    wrong_state_first: bool,
}

impl SignInUi for Browser {
    fn device_code(&self, code: &str, url: &str) {
        self.codes.lock().push(format!("{code} {url}"));
    }

    fn open(&self, url: &str) -> Result<(), String> {
        let page = Url::parse(url).map_err(|e| e.to_string())?;
        let query: std::collections::HashMap<_, _> = page.query_pairs().into_owned().collect();
        self.pages.lock().push(page);
        let redirect = query["redirect_uri"].clone();
        let state = query["state"].clone();
        let wrong = self.wrong_state_first;
        tokio::spawn(async move {
            if wrong {
                let _ = reqwest::get(format!("{redirect}?code=stolen&state=not-ours")).await;
            }
            let _ = reqwest::get(format!("{redirect}?code=the-code&state={state}")).await;
        });
        Ok(())
    }
}

/// GitHub's answers in turn: pending, then what `then` says.
struct PendingThen(AtomicUsize, Value);

impl Respond for PendingThen {
    fn respond(&self, _: &Request) -> ResponseTemplate {
        let body = if self.0.fetch_add(1, Ordering::SeqCst) == 0 {
            json!({"error": "authorization_pending"})
        } else {
            self.1.clone()
        };
        ResponseTemplate::new(200).set_body_json(body)
    }
}

async fn github(server: &MockServer, page_host: Option<&str>, answer: Value) {
    let page = format!("{}/login/device", page_host.map_or_else(|| server.uri(), str::to_string));
    Mock::given(method("POST"))
        .and(path("/start"))
        .and(body_string_contains("client_id=app-1"))
        .and(body_string_contains("scope=repo"))
        .respond_with(ResponseTemplate::new(200).set_body_json(
            json!({"device_code": "dev-1", "user_code": "WDJB-MJHT", "verification_uri": page, "interval": 1, "expires_in": 60}),
        ))
        .mount(server)
        .await;
    Mock::given(method("POST"))
        .and(path("/token"))
        .and(body_string_contains("device_code=dev-1"))
        .respond_with(PendingThen(AtomicUsize::new(0), answer))
        .mount(server)
        .await;
}

#[tokio::test]
async fn github_shows_the_code_waits_and_keeps_the_token() {
    let server = MockServer::start().await;
    github(&server, None, json!({"access_token": "gho_x", "token_type": "bearer", "scope": "repo"})).await;
    let ui = Browser::default();
    let signed =
        signin::sign_in(ConnectionKind::Github, &endpoints(&server), &app(None), None, &ui, &CancellationToken::new())
            .await
            .unwrap();
    assert_eq!(signed.scope(), Some("repo"));
    assert_eq!(ui.codes.lock()[0], format!("WDJB-MJHT {}/login/device", server.uri()));
    assert!(ui.pages.lock().is_empty(), "the person opens the page; nothing is opened for them");
}

#[tokio::test]
async fn github_refusals_end_the_sign_in() {
    let server = MockServer::start().await;
    github(&server, None, json!({"error": "access_denied"})).await;
    let ui = Browser::default();
    let denied =
        signin::sign_in(ConnectionKind::Github, &endpoints(&server), &app(None), None, &ui, &CancellationToken::new())
            .await;
    assert!(denied.err().unwrap().to_string().contains("access_denied"));

    // A code page on another site is never shown to the person.
    let server = MockServer::start().await;
    github(&server, Some("https://github.evil.example"), json!({})).await;
    let ui = Browser::default();
    let elsewhere =
        signin::sign_in(ConnectionKind::Github, &endpoints(&server), &app(None), None, &ui, &CancellationToken::new())
            .await;
    assert!(elsewhere.err().unwrap().to_string().contains("another site"));
    assert!(ui.codes.lock().is_empty());

    let cancel = CancellationToken::new();
    cancel.cancel();
    let server = MockServer::start().await;
    github(&server, None, json!({})).await;
    let cancelled =
        signin::sign_in(ConnectionKind::Github, &endpoints(&server), &app(None), None, &Browser::default(), &cancel)
            .await;
    assert!(matches!(cancelled, Err(zorix_oauth::OAuthError::Cancelled)));
}

struct Store {
    services: Arc<Services>,
    secrets: Arc<MemoryStore>,
}

async fn kept(kind: ConnectionKind, signed: &signin::SignedIn) -> (Store, zorix_core::Connection) {
    let store = Arc::new(SqliteIntegrationStore::open_in_memory().unwrap());
    let secrets = Arc::new(MemoryStore::new());
    let connection = store
        .add_connection(
            &TenantId::local(),
            NewConnection { kind, name: "svc".into(), settings: json!({"auth": "oauth"}), has_secret: true },
            Timestamp(1),
        )
        .await
        .unwrap();
    signin::keep(&signin::vault(secrets.clone(), connection.id), signed).await.unwrap();
    let services = Arc::new(Services { store: store.clone(), secrets: secrets.clone(), signins: Default::default() });
    (Store { services, secrets }, connection)
}

#[tokio::test]
async fn gitlab_uses_pkce_ignores_a_strangers_answer_and_renews_the_token() {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/token"))
        .and(body_string_contains("grant_type=authorization_code"))
        .and(body_string_contains("code=the-code"))
        .and(body_string_contains("code_verifier="))
        .respond_with(ResponseTemplate::new(200).set_body_json(
            json!({"access_token": "first", "token_type": "Bearer", "refresh_token": "r1", "expires_in": 1}),
        ))
        .expect(1)
        .mount(&server)
        .await;
    Mock::given(method("POST"))
        .and(path("/token"))
        .and(body_string_contains("grant_type=refresh_token"))
        .and(body_string_contains("refresh_token=r1"))
        .respond_with(ResponseTemplate::new(200).set_body_json(
            json!({"access_token": "second", "token_type": "Bearer", "refresh_token": "r2", "expires_in": 3600}),
        ))
        .expect(1)
        .mount(&server)
        .await;
    Mock::given(method("POST"))
        .and(path("/revoke"))
        .and(body_string_contains("token=r2"))
        .respond_with(ResponseTemplate::new(200))
        .expect(1)
        .mount(&server)
        .await;
    let ui = Browser { wrong_state_first: true, ..Browser::default() };
    let signed =
        signin::sign_in(ConnectionKind::Gitlab, &endpoints(&server), &app(None), None, &ui, &CancellationToken::new())
            .await
            .unwrap();
    let page = ui.pages.lock()[0].clone();
    let query: std::collections::HashMap<_, _> = page.query_pairs().into_owned().collect();
    assert_eq!((query["code_challenge_method"].as_str(), query["scope"].as_str()), ("S256", "api"));
    assert!(query["redirect_uri"].starts_with("http://127.0.0.1:"));

    // The first token runs out at once: two callers together renew it once, and the rotated refresh token is kept.
    let (store, connection) = kept(ConnectionKind::Gitlab, &signed).await;
    let (a, b) = tokio::join!(store.services.secret(&connection), store.services.secret(&connection));
    assert_eq!((a.unwrap().unwrap(), b.unwrap().unwrap()), ("second".into(), "second".into()));

    let vault = signin::vault(store.secrets.clone(), connection.id);
    signin::revoke(&vault).await;
    assert!(!vault.signed_in().await, "nothing is kept after revoking");
}

#[tokio::test]
async fn a_revoked_refresh_token_asks_for_a_new_sign_in() {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/token"))
        .and(body_string_contains("grant_type=authorization_code"))
        .respond_with(ResponseTemplate::new(200).set_body_json(
            json!({"access_token": "first", "token_type": "Bearer", "refresh_token": "r1", "expires_in": 1}),
        ))
        .mount(&server)
        .await;
    Mock::given(method("POST"))
        .and(path("/token"))
        .and(body_string_contains("grant_type=refresh_token"))
        .respond_with(ResponseTemplate::new(400).set_body_json(json!({"error": "invalid_grant"})))
        .mount(&server)
        .await;
    let ui = Browser::default();
    let signed =
        signin::sign_in(ConnectionKind::Gitlab, &endpoints(&server), &app(None), None, &ui, &CancellationToken::new())
            .await
            .unwrap();
    let (store, connection) = kept(ConnectionKind::Gitlab, &signed).await;
    let error = format!("{:?}", store.services.secret(&connection).await.unwrap_err());
    assert!(error.contains("needs a new sign-in") && error.contains("remove svc"), "{error}");
    assert!(!signin::vault(store.secrets.clone(), connection.id).signed_in().await);
}

#[tokio::test]
async fn jira_sends_json_and_finds_its_site_behind_atlassians_gateway() {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/token"))
        .and(header("content-type", "application/json"))
        .and(body_string_contains("\"client_secret\":\"s3cret\""))
        .respond_with(ResponseTemplate::new(200).set_body_json(
            json!({"access_token": "jira-token", "token_type": "Bearer", "refresh_token": "r", "expires_in": 3600}),
        ))
        .mount(&server)
        .await;
    Mock::given(method("GET"))
        .and(path("/resources"))
        .and(header("authorization", "Bearer jira-token"))
        .respond_with(ResponseTemplate::new(200).set_body_json(
            json!([{"id": "abc-123", "url": "https://acme.atlassian.net"}, {"id": "../x", "url": "https://bad.atlassian.net"}]),
        ))
        .mount(&server)
        .await;
    let sign = |site: &'static str| {
        let endpoints = endpoints(&server);
        async move {
            let ui = Browser::default();
            signin::sign_in(
                ConnectionKind::Jira,
                &endpoints,
                &app(Some("s3cret")),
                Some(site),
                &ui,
                &CancellationToken::new(),
            )
            .await
        }
    };
    let signed = sign("https://Acme.atlassian.net/").await.unwrap();
    let (store, connection) = kept(ConnectionKind::Jira, &signed).await;
    let (token, base) = store.services.credential(&connection).await.unwrap();
    assert_eq!(token.as_deref(), Some("jira-token"));
    assert_eq!(base.as_deref(), Some(format!("{}/ex/jira/abc-123", server.uri()).as_str()));

    let other = sign("https://other.atlassian.net").await.err().unwrap().to_string();
    assert!(other.contains("does not reach") && other.contains("acme.atlassian.net"), "{other}");
    assert!(sign("https://bad.atlassian.net").await.is_err(), "a site id is never read as a path");

    let ui = Browser::default();
    let no_secret = signin::sign_in(
        ConnectionKind::Jira,
        &endpoints(&server),
        &app(None),
        Some("x"),
        &ui,
        &CancellationToken::new(),
    )
    .await;
    assert!(no_secret.err().unwrap().to_string().contains("client secret"));
}

#[test]
fn a_sign_in_goes_only_where_the_connection_says_and_never_in_the_clear() {
    let github = Endpoints::of(ConnectionKind::Github, None).unwrap();
    assert_eq!(github.start.as_str(), "https://github.com/login/device/code");
    let enterprise = Endpoints::of(ConnectionKind::Github, Some("https://ghe.corp.example/api/v3/")).unwrap();
    assert_eq!(enterprise.token.as_str(), "https://ghe.corp.example/login/oauth/access_token");
    let gitlab = Endpoints::of(ConnectionKind::Gitlab, Some("https://git.corp.example/api/v4")).unwrap();
    assert_eq!(gitlab.start.as_str(), "https://git.corp.example/oauth/authorize");
    assert_eq!(gitlab.port, signin::REDIRECT_PORT);
    assert!(Endpoints::of(ConnectionKind::Gitlab, Some("http://git.corp.example")).is_err());
    assert!(Endpoints::of(ConnectionKind::Slack, None).is_err());
    assert!(!signin::can_sign_in(ConnectionKind::Slack) && signin::can_sign_in(ConnectionKind::Jira));
}
