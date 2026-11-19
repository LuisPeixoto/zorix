//! The hub: what a person adds is checked, secrets go to the keychain and leave with what they belonged to.

use std::sync::Arc;

use serde_json::json;
use zorix_core::{ConnectionKind, EnvValue, EnvVar, HttpAuth, McpTransport, NewMcpServer, Secret, TenantId};
use zorix_hub::Hub;
use zorix_hub::connections::ConnectRequest;
use zorix_providers::{MemoryStore, SecretStore};
use zorix_runtime::InMemoryToolRegistry;

fn hub() -> (Hub, Arc<MemoryStore>, tempfile::TempDir) {
    let dir = tempfile::tempdir().unwrap();
    let secrets = Arc::new(MemoryStore::new());
    let hub = Hub::open(dir.path(), TenantId::local(), secrets.clone(), Arc::new(InMemoryToolRegistry::new())).unwrap();
    (hub, secrets, dir)
}

fn request(kind: ConnectionKind, name: &str, settings: serde_json::Value, secret: Option<&str>) -> ConnectRequest {
    ConnectRequest { kind, name: name.into(), settings, secret: secret.map(Secret::new) }
}

#[tokio::test]
async fn connections_are_checked_before_they_are_kept() {
    let (hub, _secrets, dir) = hub();
    for (bad, why) in [
        (request(ConnectionKind::Jira, "j", json!({"base_url": "https://a.atlassian.net"}), Some("t")), "email"),
        (
            request(
                ConnectionKind::Jira,
                "j",
                json!({"base_url": "http://a.atlassian.net", "email": "a@b.c"}),
                Some("t"),
            ),
            "HTTPS",
        ),
        (request(ConnectionKind::Slack, "s", json!({}), None), "bot token"),
        (request(ConnectionKind::Http, "h", json!({"host": "api.example.*"}), Some("t")), "wildcards"),
        (
            request(
                ConnectionKind::Http,
                "h",
                json!({"host": "a.example.com", "auth": "header", "header": "Host"}),
                Some("t"),
            ),
            "cannot carry",
        ),
        (request(ConnectionKind::Webhook, "w", json!({"url": "http://example.com/x"}), Some("k")), "HTTPS"),
        (
            request(
                ConnectionKind::Postgres,
                "p",
                json!({"host": "h", "database": "d", "user": "u", "sslmode": "prefer"}),
                None,
            ),
            "sslmode",
        ),
        (
            request(ConnectionKind::Sqlite, "q", json!({"path": dir.path().join("none.db").to_str().unwrap()}), None),
            "no SQLite file",
        ),
        (request(ConnectionKind::Github, "Bad Name", json!({}), None), ""),
    ] {
        let error = hub.add_connection(bad).await.unwrap_err().to_string();
        assert!(error.contains(why), "{error} should mention {why}");
    }
    assert!(hub.connections().await.unwrap().is_empty());
}

#[tokio::test]
async fn a_connection_keeps_its_secret_in_the_keychain_and_takes_it_away_when_removed() {
    let (hub, secrets, _dir) = hub();
    let added = hub
        .add_connection(request(ConnectionKind::Slack, "team", json!({}), Some("xoxb-1111111111-abcdefghij")))
        .await
        .unwrap();
    assert!(added.has_secret);
    let key = zorix_integrations::secret_name(added.id);
    assert_eq!(secrets.get(&key).unwrap().unwrap().expose(), "xoxb-1111111111-abcdefghij");
    let stored = serde_json::to_string(&hub.connection("team").await.unwrap()).unwrap();
    assert!(!stored.contains("xoxb-"), "the record never holds the secret: {stored}");
    hub.remove_connection("team").await.unwrap();
    assert!(secrets.get(&key).unwrap().is_none());
    assert!(hub.connection("team").await.is_err());
}

#[tokio::test]
async fn a_sqlite_file_is_stored_by_its_real_path_and_can_be_tested() {
    let (hub, _secrets, dir) = hub();
    let file = dir.path().join("app.db");
    rusqlite_free_file(&file);
    let added = hub
        .add_connection(request(ConnectionKind::Sqlite, "app", json!({"path": file.to_str().unwrap()}), None))
        .await
        .unwrap();
    assert_eq!(added.settings["path"].as_str().unwrap(), std::fs::canonicalize(&file).unwrap().to_str().unwrap());
    let test = hub.test_connection("app").await.unwrap();
    assert!(test.ok, "{test:?}");
    assert!(hub.connection("app").await.unwrap().last_test.unwrap().ok, "the result is remembered");
}

fn rusqlite_free_file(path: &std::path::Path) {
    // An empty file is not a database SQLite can read tables from, so write the smallest valid one: the header is made by SQLite
    // itself, which is why the hub's own store (a SQLite file) is borrowed here.
    let store = zorix_store::SqliteIntegrationStore::open(path).unwrap();
    drop(store);
}

#[tokio::test]
async fn servers_start_disabled_and_every_secret_leaves_with_them() {
    let (hub, secrets, _dir) = hub();
    let stdio = hub
        .add_server(
            NewMcpServer {
                name: "tools".into(),
                transport: McpTransport::Stdio {
                    command: "some-server".into(),
                    args: vec![],
                    env: vec![EnvVar { name: "API_KEY".into(), value: EnvValue::Secret }],
                    cwd: None,
                },
            },
            vec![(Some("API_KEY".into()), Secret::new("sk-abcdefghijklmnopqrstuvwxyz"))],
        )
        .await
        .unwrap();
    assert!(!stdio.enabled, "a server is enabled by a person, after seeing what it is");
    let key = zorix_mcp::env_secret_name(stdio.id, "API_KEY");
    assert!(secrets.get(&key).unwrap().is_some());
    assert!(hub.set_server_secret(&stdio, Some("OTHER"), Secret::new("x")).await.is_err(), "only declared secrets");

    let remote = hub
        .add_server(
            NewMcpServer {
                name: "remote".into(),
                transport: McpTransport::Http { url: "https://mcp.example.com/mcp".into(), auth: HttpAuth::Bearer },
            },
            vec![(None, Secret::new("tok-abcdefghijklmnopqrst"))],
        )
        .await
        .unwrap();
    let token = zorix_mcp::token_secret_name(remote.id);
    assert!(secrets.get(&token).unwrap().is_some());
    assert!(hub.set_enabled("remote", true).await.unwrap().enabled);

    hub.remove_server("tools").await.unwrap();
    hub.remove_server("remote").await.unwrap();
    assert!(secrets.get(&key).unwrap().is_none() && secrets.get(&token).unwrap().is_none());
    assert!(hub.server("tools").await.is_err());
}
