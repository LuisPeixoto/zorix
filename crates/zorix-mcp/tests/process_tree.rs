//! A stdio server starts helpers of its own (the real program behind `npx.cmd` is one). Stopping the server stops them (Phase 12).
//!
//! The fake server can start a grandchild that writes a growing number into a file ten times a second: while the number grows the
//! grandchild is alive, and when it stops growing it is gone. That works the same on every system.

use std::path::Path;
use std::sync::Arc;
use std::time::Duration;

use serde_json::json;
use tokio_util::sync::CancellationToken;
use zorix_mcp::{McpClient, StdioConfig, StdioTransport};

async fn connect(args: &[&str]) -> McpClient {
    let transport = StdioTransport::spawn(StdioConfig {
        command: env!("CARGO_BIN_EXE_zorix-fake-mcp").into(),
        args: args.iter().map(|a| a.to_string()).collect(),
        env: vec![],
        cwd: None,
    })
    .unwrap();
    McpClient::connect_with(
        Arc::new(transport),
        &CancellationToken::new(),
        Duration::from_millis(600),
        Duration::from_secs(10),
    )
    .await
    .unwrap()
}

fn beat(file: &Path) -> Option<u64> {
    std::fs::read_to_string(file).ok()?.trim().parse().ok()
}

/// The grandchild is running: its number grows.
async fn alive(file: &Path) -> bool {
    for _ in 0..50 {
        if let Some(first) = beat(file) {
            tokio::time::sleep(Duration::from_millis(350)).await;
            if beat(file).is_some_and(|second| second > first) {
                return true;
            }
        } else {
            tokio::time::sleep(Duration::from_millis(100)).await;
        }
    }
    false
}

/// The grandchild is gone: its number stops growing and stays where it is.
async fn gone(file: &Path) -> bool {
    for _ in 0..40 {
        let first = beat(file);
        tokio::time::sleep(Duration::from_millis(500)).await;
        if beat(file) == first {
            return true;
        }
    }
    false
}

fn folder() -> (tempfile::TempDir, std::path::PathBuf) {
    let dir = tempfile::tempdir().unwrap();
    let file = dir.path().join("heartbeat");
    (dir, file)
}

#[tokio::test]
async fn what_a_server_started_does_not_outlive_a_server_that_is_closed() {
    let (_keep, file) = folder();
    let client = connect(&["--era", "modern", "--child", &file.display().to_string()]).await;
    assert!(alive(&file).await, "the helper the server started is running");
    // The polite way: standard input is closed, the server leaves by itself, and the helper it left behind must go too.
    client.close().await;
    assert!(gone(&file).await, "the helper was left running after its server was closed");
}

#[tokio::test]
async fn what_a_server_started_does_not_outlive_a_server_that_crashes() {
    let (_keep, file) = folder();
    let client = connect(&["--era", "modern", "--child", &file.display().to_string()]).await;
    assert!(alive(&file).await);
    let crashed = client.call_tool("crash", json!({}), Duration::from_secs(5), &CancellationToken::new()).await;
    assert!(crashed.is_err(), "the server exits in the middle of the call");
    assert!(gone(&file).await, "the helper was left running after its server crashed");
}

#[tokio::test]
async fn what_a_server_started_does_not_outlive_the_client_that_is_dropped() {
    let (_keep, file) = folder();
    let client = connect(&["--era", "modern", "--child", &file.display().to_string()]).await;
    assert!(alive(&file).await);
    drop(client);
    assert!(gone(&file).await, "the helper was left running after nothing held its server any more");
}
