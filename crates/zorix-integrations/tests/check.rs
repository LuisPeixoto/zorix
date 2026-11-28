//! Testing connections: one small read each, with a clear answer and no secret in it.

use std::sync::Arc;

use serde_json::{Value, json};
use wiremock::matchers::{header, method, path};
use wiremock::{Mock, MockServer, ResponseTemplate};
use zorix_core::{ConnectionKind, IntegrationStore, NewConnection, Secret, TenantId, Timestamp};
use zorix_integrations::{Checked, Services, secret_name, test_connection};
use zorix_providers::{MemoryStore, SecretStore};
use zorix_store::SqliteIntegrationStore;

async fn check(kind: ConnectionKind, settings: Value, token: Option<&str>) -> Checked {
    let store = Arc::new(SqliteIntegrationStore::open_in_memory().unwrap());
    let secrets = Arc::new(MemoryStore::new());
    let connection = store
        .add_connection(
            &TenantId::local(),
            NewConnection { kind, name: "main".into(), settings, has_secret: token.is_some() },
            Timestamp(1),
        )
        .await
        .unwrap();
    if let Some(token) = token {
        secrets.set(&secret_name(connection.id), &Secret::new(token)).unwrap();
    }
    test_connection(Arc::new(Services { store, secrets, signins: Default::default() }), &connection).await
}

#[tokio::test]
async fn github_says_who_signed_in_or_that_only_public_data_is_open() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/user"))
        .and(header("authorization", "Bearer ghp_abcdefghijklmnopqrstuvwxyz0123456789"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({"login": "octocat"})))
        .mount(&server)
        .await;
    Mock::given(method("GET"))
        .and(path("/rate_limit"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({"resources": {"core": {"remaining": 57}}})))
        .mount(&server)
        .await;
    let settings = json!({"base_url": server.uri()});
    let signed =
        check(ConnectionKind::Github, settings.clone(), Some("ghp_abcdefghijklmnopqrstuvwxyz0123456789")).await;
    assert!(signed.ok && signed.note.contains("octocat"), "{signed:?}");
    assert!(!signed.note.contains("ghp_"));
    let public = check(ConnectionKind::Github, settings, None).await;
    assert!(public.ok && public.note.contains("public data") && public.note.contains("57"), "{public:?}");
}

#[tokio::test]
async fn a_rejected_token_is_reported_without_the_token() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/user"))
        .respond_with(ResponseTemplate::new(401).set_body_json(json!({"message": "Bad credentials"})))
        .mount(&server)
        .await;
    let result = check(
        ConnectionKind::Github,
        json!({"base_url": server.uri()}),
        Some("ghp_secretsecretsecretsecretsecret123456"),
    )
    .await;
    assert!(!result.ok, "{result:?}");
    assert!(!result.note.contains("ghp_secret"), "{}", result.note);
}

#[tokio::test]
async fn gitlab_jira_slack_and_discord_each_make_one_read() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/api/v4/user"))
        .and(header("private-token", "glpat-abcdefghijklmnopqrst"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({"username": "tanuki"})))
        .mount(&server)
        .await;
    Mock::given(method("GET"))
        .and(path("/rest/api/3/myself"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({"displayName": "Ada"})))
        .mount(&server)
        .await;
    Mock::given(method("GET"))
        .and(path("/auth.test"))
        .and(header("authorization", "Bearer xoxb-1111111111-abcdefghij"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({"ok": true, "team": "Acme", "user": "zorix"})))
        .mount(&server)
        .await;
    Mock::given(method("GET"))
        .and(path("/users/@me"))
        .and(header("authorization", "Bot discord-token-abcdefghijklmnop"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({"username": "zorix-bot"})))
        .mount(&server)
        .await;
    let base = json!({"base_url": server.uri()});
    let gitlab = check(ConnectionKind::Gitlab, base.clone(), Some("glpat-abcdefghijklmnopqrst")).await;
    assert!(gitlab.ok && gitlab.note.contains("tanuki"), "{gitlab:?}");
    let jira = check(
        ConnectionKind::Jira,
        json!({"base_url": server.uri(), "email": "a@b.co"}),
        Some("jira-token-abcdefghijkl"),
    )
    .await;
    assert!(jira.ok && jira.note.contains("Ada"), "{jira:?}");
    let slack = check(ConnectionKind::Slack, base.clone(), Some("xoxb-1111111111-abcdefghij")).await;
    assert!(slack.ok && slack.note.contains("Acme"), "{slack:?}");
    let discord = check(ConnectionKind::Discord, base, Some("discord-token-abcdefghijklmnop")).await;
    assert!(discord.ok && discord.note.contains("zorix-bot"), "{discord:?}");
}

#[tokio::test]
async fn slack_errors_and_missing_tokens_are_plain_sentences() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/auth.test"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({"ok": false, "error": "invalid_auth"})))
        .mount(&server)
        .await;
    let slack =
        check(ConnectionKind::Slack, json!({"base_url": server.uri()}), Some("xoxb-1111111111-abcdefghij")).await;
    assert!(!slack.ok && slack.note.contains("invalid_auth"), "{slack:?}");
    let none = check(ConnectionKind::Slack, json!({"base_url": server.uri()}), None).await;
    assert!(!none.ok && none.note.contains("No token"), "{none:?}");
}

#[tokio::test]
async fn a_database_file_is_opened_and_a_webhook_is_never_sent() {
    let dir = tempfile::tempdir().unwrap();
    let file = dir.path().join("app.db");
    {
        let db = rusqlite::Connection::open(&file).unwrap();
        db.execute("CREATE TABLE t (a INTEGER)", []).unwrap();
    }
    let db = check(ConnectionKind::Sqlite, json!({"path": file.to_str().unwrap()}), None).await;
    assert!(db.ok && db.note.contains("1 table"), "{db:?}");
    let missing =
        check(ConnectionKind::Sqlite, json!({"path": dir.path().join("none.db").to_str().unwrap()}), None).await;
    assert!(!missing.ok, "{missing:?}");

    let server = MockServer::start().await;
    Mock::given(method("POST")).respond_with(ResponseTemplate::new(200)).expect(0).mount(&server).await;
    let hook = check(
        ConnectionKind::Webhook,
        json!({"url": format!("{}/hook", server.uri())}),
        Some("signing-key-abcdefghijklmnop"),
    )
    .await;
    assert!(hook.ok && hook.note.contains("Nothing was sent"), "{hook:?}");
    let unsigned = check(ConnectionKind::Webhook, json!({"url": format!("{}/hook", server.uri())}), None).await;
    assert!(!unsigned.ok);
}
