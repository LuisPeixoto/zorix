//! A server run in a container tries to get out (ADR-0050). Needs Docker or Podman and pulls a pinned image, so it runs only when
//! `ZORIX_TEST_CONTAINER=1` (CI sets it on Linux).
#![cfg(unix)]

use std::net::TcpListener;
use std::process::Command;
use std::sync::Arc;
use std::time::Duration;

use serde_json::json;
use tokio_util::sync::CancellationToken;
use zorix_core::{SandboxFolder, SandboxMode, ServerSandbox};
use zorix_mcp::{Confinement, McpClient, StdioConfig, StdioTransport, container_runtime};

fn wanted() -> bool {
    std::env::var("ZORIX_TEST_CONTAINER").is_ok_and(|v| v == "1")
}

#[tokio::test]
async fn a_server_in_a_container_sees_only_what_it_was_given_and_leaves_no_container_behind() {
    if !wanted() {
        return;
    }
    let runtime = container_runtime().expect("docker or podman");
    let dir = tempfile::tempdir().unwrap();
    let root = std::fs::canonicalize(dir.path()).unwrap();
    for folder in ["code", "rw", "secret"] {
        std::fs::create_dir_all(root.join(folder)).unwrap();
    }
    std::fs::copy(concat!(env!("CARGO_MANIFEST_DIR"), "/tests/fixtures/probe-server.js"), root.join("code/server.js"))
        .unwrap();
    std::fs::write(root.join("secret/key"), "SECRET").unwrap();
    let tcp = TcpListener::bind("127.0.0.1:0").unwrap();
    let path = |part: &str| root.join(part).display().to_string();
    let sandbox = ServerSandbox {
        mode: SandboxMode::Container,
        network: false,
        folders: vec![
            SandboxFolder { path: path("code"), write: false },
            SandboxFolder { path: path("rw"), write: true },
        ],
    };
    let confinement = Confinement::of(&sandbox, "node", root.join("own"), None).unwrap().unwrap();
    let (_, remove) = confinement.cleanup().unwrap();
    let name = remove.last().unwrap().clone();
    let transport = StdioTransport::spawn_confined(
        StdioConfig {
            command: "node".into(),
            args: vec![path("code/server.js")],
            env: vec![("ZORIX_PROBE".into(), "handed in".into())],
            cwd: None,
        },
        Some(confinement),
    )
    .unwrap();
    // The first start pulls the image.
    let client = McpClient::connect_with(
        Arc::new(transport),
        &CancellationToken::new(),
        Duration::from_secs(240),
        Duration::from_secs(60),
    )
    .await
    .expect("the server starts in its container");
    let said = client
        .call_tool(
            "probe",
            json!({
                "read": [path("secret/key"), path("code/server.js")],
                "write": [path("code/new"), path("rw/new"), "/etc/new"],
                "connect": [tcp.local_addr().unwrap().to_string(), "1.1.1.1:443"],
            }),
            Duration::from_secs(30),
            &CancellationToken::new(),
        )
        .await
        .unwrap();
    let said = said.content[0]["text"].as_str().unwrap().to_string();
    for line in [
        format!("read {}=denied", path("secret/key")),
        format!("read {}=ok", path("code/server.js")),
        format!("write {}=denied", path("code/new")),
        format!("write {}=ok", path("rw/new")),
        "write /etc/new=denied".to_string(),
        format!("connect {}=denied", tcp.local_addr().unwrap()),
        "connect 1.1.1.1:443=denied".to_string(),
        "home=/home/server".to_string(),
        "given=handed in".to_string(),
    ] {
        assert!(said.lines().any(|l| l == line), "{line} is not in:\n{said}");
    }
    assert!(root.join("rw/new").exists() && !root.join("code/new").exists());
    client.close().await;
    // Nothing is left running or stopped under the server's name.
    for _ in 0..50 {
        let listed = Command::new(&runtime).args(["ps", "-a", "--format", "{{.Names}}"]).output().unwrap();
        if !String::from_utf8_lossy(&listed.stdout).contains(&name) {
            return;
        }
        tokio::time::sleep(Duration::from_millis(200)).await;
    }
    panic!("the container {name} was left behind");
}
