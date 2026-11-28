//! A confined server tries to get out (ADR-0050). The same attempts without a sandbox succeed, which proves the attempts are real.
//!
//! Windows confines neither files nor the network natively, so these run on macOS and Linux (both in CI).
#![cfg(any(target_os = "macos", target_os = "linux"))]

use std::net::TcpListener;
use std::os::unix::net::UnixListener;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::Duration;

use serde_json::{Value, json};
use tokio_util::sync::CancellationToken;
use zorix_mcp::{Confinement, McpClient, StdioConfig, StdioTransport};

/// A pretend computer: a home with a secret and a note, a folder given for reading, one given for writing, and somewhere else.
struct World {
    _dir: tempfile::TempDir,
    _short: tempfile::TempDir,
    root: PathBuf,
    tcp: TcpListener,
    _unix: UnixListener,
    socket: PathBuf,
}

fn world() -> World {
    let dir = tempfile::tempdir().unwrap();
    let root = std::fs::canonicalize(dir.path()).unwrap();
    for folder in ["home/.ssh", "ro", "rw", "outside", "data/server"] {
        std::fs::create_dir_all(root.join(folder)).unwrap();
    }
    std::fs::write(root.join("home/.ssh/id"), "SECRET").unwrap();
    std::fs::write(root.join("home/notes.txt"), "NOTE").unwrap();
    std::fs::write(root.join("ro/data.txt"), "DATA").unwrap();
    std::fs::write(root.join("outside/other.txt"), "OTHER").unwrap();
    // ZORIX's data folder, which holds the server's own folder too.
    std::fs::write(root.join("data/zorix.db"), "RUNS").unwrap();
    let tcp = TcpListener::bind("127.0.0.1:0").unwrap();
    // A Unix socket path must be short; the temporary folder of a test often is not.
    let short = tempfile::Builder::new().prefix("zx").tempdir_in("/tmp").unwrap();
    let socket = std::fs::canonicalize(short.path()).unwrap().join("s");
    let unix = UnixListener::bind(&socket).unwrap();
    World { _dir: dir, _short: short, root, tcp, _unix: unix, socket }
}

impl World {
    fn path(&self, part: &str) -> String {
        self.root.join(part).display().to_string()
    }

    fn confinement(&self, network: bool) -> Confinement {
        Confinement {
            server_dir: self.root.join("data/server"),
            folders: vec![(self.root.join("ro"), false), (self.root.join("rw"), true)],
            network,
            private: vec![self.root.join("home/.ssh"), self.root.join("data")],
            user_data: vec![self.root.clone()],
            home: Some(self.root.join("home")),
            container: None,
        }
    }

    fn attempts(&self) -> Value {
        let paths = |parts: &[&str]| parts.iter().map(|p| self.path(p)).collect::<Vec<_>>();
        json!({
            "read": paths(&["home/.ssh/id", "home/notes.txt", "outside/other.txt", "ro/data.txt", "data/zorix.db"]),
            "write": paths(&["outside/new", "ro/new", "home/new", "rw/new", "data/server/new", "data/new"]),
            "resolve": paths(&["data/server", "rw"]),
            "connect": [self.tcp.local_addr().unwrap().to_string()],
            "unix": [self.socket.display().to_string()],
            "home": true,
        })
    }

    /// What the server said about each attempt.
    async fn tried(&self, confinement: Option<Confinement>) -> Vec<String> {
        let transport = StdioTransport::spawn_confined(
            StdioConfig {
                command: env!("CARGO_BIN_EXE_zorix-fake-mcp").into(),
                args: vec!["--era".into(), "modern".into()],
                env: vec![],
                cwd: None,
            },
            confinement,
        )
        .unwrap();
        let client = McpClient::connect_with(
            Arc::new(transport),
            &CancellationToken::new(),
            Duration::from_secs(5),
            Duration::from_secs(10),
        )
        .await
        .expect("a confined server starts and speaks");
        let said = client
            .call_tool("probe", self.attempts(), Duration::from_secs(15), &CancellationToken::new())
            .await
            .unwrap();
        client.close().await;
        said.content[0]["text"].as_str().unwrap().lines().map(str::to_string).collect()
    }
}

fn result(lines: &[String], attempt: &str, of: &str) -> String {
    let start = format!("{attempt} {of}=");
    lines.iter().find_map(|l| l.strip_prefix(&start)).unwrap_or_else(|| panic!("no {start} in {lines:#?}")).to_string()
}

fn check(lines: &[String], world: &World, expected: &[(&str, &str, &str)]) {
    for (attempt, path, want) in expected {
        let of = match *attempt {
            "connect" => world.tcp.local_addr().unwrap().to_string(),
            "unix" => world.socket.display().to_string(),
            _ => world.path(path),
        };
        assert_eq!(result(lines, attempt, &of), *want, "{attempt} {of}: {lines:#?}");
    }
}

#[tokio::test]
async fn without_a_sandbox_every_attempt_succeeds_so_the_attempts_are_real() {
    let world = world();
    let lines = world.tried(None).await;
    assert!(lines.iter().filter(|l| !l.starts_with("home=")).all(|l| l.ends_with("=ok")), "{lines:#?}");
}

#[tokio::test]
async fn a_confined_server_reads_and_writes_only_where_it_was_allowed_and_reaches_nothing() {
    let world = world();
    let lines = world.tried(Some(world.confinement(false))).await;
    check(
        &lines,
        &world,
        &[
            ("read", "home/.ssh/id", "denied"),
            ("read", "home/notes.txt", "denied"),
            ("read", "outside/other.txt", "denied"),
            ("read", "ro/data.txt", "ok"),
            ("write", "outside/new", "denied"),
            ("write", "ro/new", "denied"),
            ("write", "home/new", "denied"),
            ("write", "rw/new", "ok"),
            ("write", "data/server/new", "ok"),
            ("write", "data/new", "denied"),
            ("read", "data/zorix.db", "denied"),
            // Resolving the way to what is allowed works, as a program that canonicalises its folder needs (Node does).
            ("resolve", "data/server", "ok"),
            ("resolve", "rw", "ok"),
            ("connect", "", "denied"),
            ("unix", "", "denied"),
        ],
    );
    assert!(lines.contains(&format!("home={}", world.path("data/server"))), "its home is its own folder: {lines:#?}");
    assert!(!Path::new(&world.path("outside/new")).exists() && !Path::new(&world.path("ro/new")).exists());
}

#[tokio::test]
async fn the_network_is_a_choice_and_local_unix_sockets_stay_refused_either_way() {
    let world = world();
    let lines = world.tried(Some(world.confinement(true))).await;
    check(&lines, &world, &[("connect", "", "ok"), ("unix", "", "denied"), ("read", "home/.ssh/id", "denied")]);
}

#[tokio::test]
async fn a_credential_folder_stays_refused_even_inside_a_folder_that_was_given() {
    let world = world();
    let mut confinement = world.confinement(false);
    // A person gave the whole home for reading: its keys are still not readable (macOS; on Linux the sandbox is chosen folder by
    // folder and the store refuses such a folder before it gets here).
    if cfg!(target_os = "macos") {
        confinement.folders.push((world.root.join("home"), true));
        // And the whole of everything, ZORIX's data included.
        confinement.folders.push((world.root.clone(), true));
        let lines = world.tried(Some(confinement)).await;
        check(
            &lines,
            &world,
            &[
                ("read", "home/.ssh/id", "denied"),
                ("read", "data/zorix.db", "denied"),
                ("write", "data/new", "denied"),
                ("read", "home/notes.txt", "ok"),
                ("write", "data/server/new", "ok"),
            ],
        );
    }
}
