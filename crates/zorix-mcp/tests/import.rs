//! Importing servers from another app's config: nothing is added until a person chooses, secrets go to the keychain and not
//! to the database, and a secret that cannot be kept leaves nothing behind.

use std::sync::Arc;

use zorix_core::{HttpAuth, IntegrationStore, McpTransport, TenantId, Timestamp};
use zorix_mcp::import::{Source, apply, parse};
use zorix_mcp::{env_secret_name, token_secret_name};
use zorix_providers::{MemoryStore, SecretStore};
use zorix_store::SqliteIntegrationStore;

const CONFIG: &str = r#"{"mcpServers": {
  "github": {"command": "npx", "args": ["-y", "server-github"], "env": {"GITHUB_TOKEN": "ghp_abcdefghijklmnopqrstuvwxyz0123456789", "MODE": "fast"}},
  "remote": {"url": "https://mcp.example.com/mcp", "headers": {"Authorization": "Bearer tok-remote-123"}},
  "spacey": {"command": "x", "env": {"API_KEY": "has a space inside"}},
  "old": {"type": "sse", "url": "https://old.example.com/sse"}
}}"#;

#[tokio::test]
async fn chosen_servers_are_added_disabled_with_their_secrets_in_the_keychain_only() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("zorix.sqlite");
    let store: Arc<dyn IntegrationStore> = Arc::new(SqliteIntegrationStore::open(&path).unwrap());
    let secrets_impl = Arc::new(MemoryStore::new());
    let secrets: Arc<dyn SecretStore> = secrets_impl.clone();
    let tenant = TenantId::local();
    let preview = parse(Source::Claude, CONFIG).unwrap();

    // Looking at a config adds nothing.
    assert!(store.mcp_servers(&tenant).await.unwrap().is_empty());

    let results = apply(
        &preview,
        &["github".to_string(), "remote".to_string(), "spacey".to_string(), "old".to_string()],
        &store,
        &secrets,
        &tenant,
        Timestamp(5),
    )
    .await;
    assert_eq!(results.len(), 3, "the one that cannot be imported is not attempted");
    let github = results.iter().find_map(|r| r.as_ref().ok().filter(|i| i.server.name == "github")).unwrap();
    assert!(!github.server.enabled, "a person turns it on after seeing what it is");
    assert_eq!(github.secrets_moved, 1);
    assert_eq!(
        secrets_impl.get(&env_secret_name(github.server.id, "GITHUB_TOKEN")).unwrap().unwrap().expose(),
        "ghp_abcdefghijklmnopqrstuvwxyz0123456789"
    );
    assert!(
        secrets_impl.get(&env_secret_name(github.server.id, "MODE")).unwrap().is_none(),
        "a plain value is not a secret"
    );

    let remote = results.iter().find_map(|r| r.as_ref().ok().filter(|i| i.server.name == "remote")).unwrap();
    assert!(matches!(remote.server.transport, McpTransport::Http { auth: HttpAuth::Bearer, .. }));
    assert_eq!(secrets_impl.get(&token_secret_name(remote.server.id)).unwrap().unwrap().expose(), "tok-remote-123");

    // A secret that cannot be kept (the keychain refuses spaces) leaves no server behind, and says so.
    let failed = results.iter().find_map(|r| r.as_ref().err()).unwrap();
    assert_eq!(failed.0, "spacey");
    assert!(failed.1.contains("keychain"), "{failed:?}");
    assert!(store.mcp_server_named(&tenant, "spacey").await.unwrap().is_none());

    // The database file never saw a secret.
    drop(store);
    let bytes = std::fs::read(&path).unwrap();
    let text = String::from_utf8_lossy(&bytes);
    assert!(!text.contains("ghp_abcdef") && !text.contains("tok-remote-123") && !text.contains("has a space"));
    assert!(text.contains("GITHUB_TOKEN"), "only the name is there");
}

#[tokio::test]
async fn importing_twice_does_not_replace_what_is_there() {
    let store: Arc<dyn IntegrationStore> = Arc::new(SqliteIntegrationStore::open_in_memory().unwrap());
    let secrets: Arc<dyn SecretStore> = Arc::new(MemoryStore::new());
    let tenant = TenantId::local();
    let preview = parse(Source::Claude, r#"{"mcpServers": {"files": {"command": "npx"}}}"#).unwrap();
    let first = apply(&preview, &["files".to_string()], &store, &secrets, &tenant, Timestamp(1)).await;
    let second = apply(&preview, &["files".to_string()], &store, &secrets, &tenant, Timestamp(2)).await;
    assert_eq!(first[0].as_ref().unwrap().server.name, "files");
    assert_eq!(second[0].as_ref().unwrap().server.name, "files-2", "the earlier one is untouched");
    assert_eq!(store.mcp_servers(&tenant).await.unwrap().len(), 2);
}
