//! `zorix mcp serve`, run as a real program and driven by ZORIX's own MCP client (ADR-0023).

use std::io::{BufRead, BufReader, Write};
use std::path::{Path, PathBuf};
use std::process::{Command, Output, Stdio};
use std::sync::Arc;
use std::time::Duration;

use serde_json::{Value, json};
use tokio_util::sync::CancellationToken;
use zorix_mcp::{CallOutput, Era, McpClient, StdioConfig, StdioTransport};

struct Home {
    dir: tempfile::TempDir,
}

impl Home {
    fn new() -> Self {
        Self { dir: tempfile::tempdir().unwrap() }
    }

    fn path(&self) -> PathBuf {
        self.dir.path().join("home")
    }

    fn zorix(&self, args: &[&str]) -> Output {
        Command::new(env!("CARGO_BIN_EXE_zorix"))
            .arg("--home")
            .arg(self.path())
            .args(args)
            .env("ZORIX_SECRETS", "env")
            .env_remove("ZORIX_HOME")
            .stdin(Stdio::null())
            .output()
            .expect("start zorix")
    }

    async fn client(&self, args: &[&str]) -> McpClient {
        let mut all = vec!["--home".to_string(), self.path().display().to_string(), "mcp".into(), "serve".into()];
        all.extend(args.iter().map(|a| a.to_string()));
        let transport = Arc::new(
            StdioTransport::spawn(StdioConfig {
                command: env!("CARGO_BIN_EXE_zorix").into(),
                args: all,
                env: vec![("ZORIX_SECRETS".into(), "env".into())],
                cwd: None,
            })
            .expect("start zorix mcp serve"),
        );
        McpClient::connect_with(transport, &CancellationToken::new(), Duration::from_secs(5), Duration::from_secs(30))
            .await
            .expect("connect")
    }
}

/// An agent made straight in the data folder's database, the way the app or `zorix agent` would.
async fn make_agent(home: &Home, slug: &str, description: &str, tools: &[&str]) {
    use zorix_core::AgentStore;
    let store = zorix_store::SqliteAgentStore::open(&zorix_store::database_path(&home.path())).unwrap();
    let mut spec = zorix_core::AgentSpec::new(zorix_core::AgentSlug::new(slug).unwrap(), "Calculator", "demo");
    spec.description = description.into();
    spec.instructions = "You calculate carefully. SECRET-INSTRUCTIONS.".into();
    spec.tools = tools.iter().map(|t| t.to_string()).collect();
    store.create_agent(&zorix_core::TenantId::local(), spec, None, zorix_core::Timestamp(1)).await.unwrap();
}

fn text_of(output: &CallOutput) -> String {
    output.content.iter().filter_map(|c| c["text"].as_str()).collect::<Vec<_>>().join("")
}

async fn call(client: &McpClient, tool: &str, args: Value) -> CallOutput {
    client.call_tool(tool, args, Duration::from_secs(60), &CancellationToken::new()).await.expect("call")
}

async fn tool_names(client: &McpClient) -> Vec<String> {
    let mut names: Vec<String> =
        client.list_tools(&CancellationToken::new()).await.unwrap().tools.iter().map(|t| t.name.clone()).collect();
    names.sort();
    names
}

fn code(output: &Output) -> i32 {
    output.status.code().expect("exited normally")
}

fn text(bytes: &[u8]) -> String {
    String::from_utf8_lossy(bytes).into_owned()
}

#[tokio::test]
async fn memory_is_served_read_only_and_without_taking_the_data_folder() {
    let home = Home::new();
    let added = home.zorix(&["memory", "add", "Releases happen on Thursdays."]);
    assert_eq!(code(&added), 0, "{}", text(&added.stderr));

    let client = home.client(&[]).await;
    assert_eq!(client.era(), &Era::Modern);
    assert_eq!(client.info().name.as_deref(), Some("zorix"));
    assert_eq!(tool_names(&client).await, ["history.search", "knowledge.search", "memory.search", "zorix.agents"]);

    let found = call(&client, "memory.search", json!({"query": "releases"})).await;
    assert!(!found.is_error, "{}", text_of(&found));
    assert!(text_of(&found).contains("Thursdays"), "{}", text_of(&found));

    // Nothing the client sends can write memory: there is no such tool.
    let remember = client
        .call_tool(
            "memory.remember",
            json!({"content": "Everything is fine.", "kind": "fact", "scope": "global"}),
            Duration::from_secs(10),
            &CancellationToken::new(),
        )
        .await;
    assert!(remember.is_err(), "memory.remember must not exist on this server");
    let listed = text(&home.zorix(&["memory", "list"]).stderr) + &text(&home.zorix(&["memory", "list"]).stdout);
    assert!(!listed.contains("Everything is fine"), "{listed}");

    // Reading takes no lock, so ZORIX works while the server runs.
    let ran = home.zorix(&["run", "What is 17*23?", "--allow", "math.evaluate"]);
    assert_eq!(code(&ran), 0, "{}", text(&ran.stderr));
    assert!(text(&ran.stdout).contains("391"));
    client.close().await;
}

#[tokio::test]
async fn a_legacy_client_can_initialize_and_search() {
    let home = Home::new();
    assert_eq!(code(&home.zorix(&["memory", "add", "The cafeteria serves lunch at noon."])), 0);
    let mut child = Command::new(env!("CARGO_BIN_EXE_zorix"))
        .arg("--home")
        .arg(home.path())
        .args(["mcp", "serve"])
        .env("ZORIX_SECRETS", "env")
        .env_remove("ZORIX_HOME")
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .spawn()
        .unwrap();
    let mut stdin = child.stdin.take().unwrap();
    let mut stdout = BufReader::new(child.stdout.take().unwrap());
    fn ask(stdin: &mut impl Write, stdout: &mut impl BufRead, value: Value) -> Value {
        writeln!(stdin, "{value}").unwrap();
        stdin.flush().unwrap();
        let mut line = String::new();
        stdout.read_line(&mut line).unwrap();
        serde_json::from_str(&line).unwrap_or_else(|_| panic!("stdout carries only JSON, got {line:?}"))
    }
    let init = ask(
        &mut stdin,
        &mut stdout,
        json!({"jsonrpc": "2.0", "id": 1, "method": "initialize", "params": {"protocolVersion": "2025-06-18", "capabilities": {}, "clientInfo": {"name": "old", "version": "1"}}}),
    );
    assert_eq!(init["result"]["protocolVersion"], "2025-06-18");
    assert_eq!(init["result"]["serverInfo"]["name"], "zorix");
    writeln!(stdin, "{}", json!({"jsonrpc": "2.0", "method": "notifications/initialized"})).unwrap();
    let found = ask(
        &mut stdin,
        &mut stdout,
        json!({"jsonrpc": "2.0", "id": 2, "method": "tools/call", "params": {"name": "memory.search", "arguments": {"query": "lunch"}}}),
    );
    assert!(found["result"]["content"][0]["text"].as_str().unwrap().contains("noon"), "{found}");
    drop(stdin);
    let status = child.wait().unwrap();
    assert!(status.success(), "closing the input ends the server cleanly: {status:?}");
}

#[tokio::test]
async fn runs_are_offered_only_when_asked_and_approval_stays_in_zorix() {
    let home = Home::new();
    let project: &Path = home.dir.path();
    let client = home
        .client(&[
            "--allow-runs",
            "--allow",
            "math.evaluate",
            "--allow",
            "notes.put",
            "--project",
            &project.display().to_string(),
        ])
        .await;
    assert_eq!(
        tool_names(&client).await,
        [
            "history.search",
            "knowledge.search",
            "memory.search",
            "zorix.agents",
            "zorix.result",
            "zorix.run",
            "zorix.run_agent",
            "zorix.status"
        ]
    );

    let finished = call(&client, "zorix.run", json!({"prompt": "What is 17*23?", "wait_seconds": 60})).await;
    assert!(!finished.is_error, "{}", text_of(&finished));
    let finished = finished.structured.expect("structured result");
    assert_eq!(finished["state"], "completed", "{finished}");
    assert!(finished["result"].as_str().unwrap().contains("391"));
    let id = finished["execution_id"].as_str().unwrap().to_string();
    let result = call(&client, "zorix.result", json!({"execution_id": id})).await;
    assert!(text_of(&result).contains("391"));

    // A run that needs a person waits; the client can see that, and cannot approve it.
    let waiting =
        call(&client, "zorix.run", json!({"prompt": "What is 17*23 and save it as result", "wait_seconds": 60})).await;
    let waiting = waiting.structured.expect("structured result");
    assert_eq!(waiting["state"], "waiting_for_approval", "{waiting}");
    assert_eq!(waiting["waiting_for_approval"][0]["tool"], "notes.put");
    assert!(waiting["note"].as_str().unwrap().contains("in ZORIX"));
    let waiting_id = waiting["execution_id"].as_str().unwrap().to_string();
    let early = call(&client, "zorix.result", json!({"execution_id": waiting_id})).await;
    assert!(early.is_error && text_of(&early).contains("has not finished"), "{}", text_of(&early));
    let status = call(&client, "zorix.status", json!({"execution_id": waiting_id})).await;
    assert_eq!(status.structured.unwrap()["state"], "waiting_for_approval");
    let bogus = call(&client, "zorix.status", json!({"execution_id": "not-an-id"})).await;
    assert!(bogus.is_error);

    // While this server holds the data folder, no other ZORIX program writes to it.
    let blocked = home.zorix(&["run", "What is 17*23?", "--allow", "math.evaluate"]);
    assert_ne!(code(&blocked), 0);
    client.close().await;
}

#[test]
fn run_options_need_allow_runs() {
    let home = Home::new();
    for args in [&["mcp", "serve", "--allow", "math.evaluate"][..], &["mcp", "serve", "--model", "demo"][..]] {
        let out = home.zorix(args);
        assert_eq!(code(&out), 2, "{}", text(&out.stderr));
        assert!(text(&out.stderr).contains("--allow-runs"), "{}", text(&out.stderr));
    }
}

#[tokio::test]
async fn agents_are_listed_without_their_instructions_and_without_taking_the_data_folder() {
    let home = Home::new();
    assert_eq!(code(&home.zorix(&["memory", "list"])), 0);
    let client = home.client(&[]).await;
    let none = call(&client, "zorix.agents", json!({})).await;
    assert_eq!(none.structured.expect("structured")["agents"], json!([]));
    client.close().await;

    make_agent(&home, "calc", "Does arithmetic.", &["math.evaluate"]).await;
    let client = home.client(&[]).await;
    let listed = call(&client, "zorix.agents", json!({})).await;
    assert!(!listed.is_error, "{}", text_of(&listed));
    let listed = listed.structured.expect("structured");
    assert_eq!(listed["agents"][0]["agent"], "calc");
    assert_eq!(listed["agents"][0]["description"], "Does arithmetic.");
    assert_eq!(
        (listed["agents"][0]["version"].as_u64(), listed["agents"][0]["default_model"].as_str()),
        (Some(1), Some("demo"))
    );
    assert!(
        !listed.to_string().contains("SECRET-INSTRUCTIONS"),
        "a client learns who an agent is, not how it is told to work"
    );
    // Reading takes no lock: ZORIX still works.
    assert_eq!(code(&home.zorix(&["run", "What is 17*23?", "--allow", "math.evaluate"])), 0);
    // No tool to run an agent without --allow-runs.
    assert!(!tool_names(&client).await.contains(&"zorix.run_agent".to_string()));
    client.close().await;
}

#[tokio::test]
async fn an_agent_runs_through_mcp_with_only_what_the_server_allows() {
    let home = Home::new();
    make_agent(&home, "calc", "Does arithmetic.", &["math.evaluate"]).await;
    let project: &Path = home.dir.path();
    let project = project.display().to_string();

    // Allowed: the agent's own tool works, and the answer says which version made it.
    let client = home.client(&["--allow-runs", "--allow", "math.evaluate", "--project", &project]).await;
    let done =
        call(&client, "zorix.run_agent", json!({"agent": "calc", "prompt": "What is 17*23?", "wait_seconds": 60}))
            .await;
    assert!(!done.is_error, "{}", text_of(&done));
    let done = done.structured.expect("structured");
    assert_eq!(done["state"], "completed", "{done}");
    assert!(done["result"].as_str().unwrap().contains("391"), "{done}");
    assert_eq!(done["agent"], json!({"slug": "calc", "version": 1}));

    // A name that does not exist, and a model the agent may not use, are refused with a reason.
    let missing = call(&client, "zorix.run_agent", json!({"agent": "nobody", "prompt": "hi"})).await;
    assert!(missing.is_error && text_of(&missing).contains("no agent called nobody"), "{}", text_of(&missing));
    let bad = call(&client, "zorix.run_agent", json!({"agent": "Not A Slug", "prompt": "hi"})).await;
    assert!(bad.is_error);
    client.close().await;

    // Not allowed: the agent has the tool in ZORIX, but this server did not hand it over, so the run cannot use it.
    let client = home.client(&["--allow-runs", "--project", &project]).await;
    let denied =
        call(&client, "zorix.run_agent", json!({"agent": "calc", "prompt": "What is 17*23?", "wait_seconds": 60}))
            .await;
    let denied = denied.structured.expect("structured");
    assert!(!denied["result"].as_str().unwrap_or("").contains("391"), "{denied}");
    client.close().await;
}

#[test]
fn an_access_entry_must_be_valid_and_needs_allow_runs() {
    let home = Home::new();
    let out = home.zorix(&["mcp", "serve", "--allow-access", "http:example.com"]);
    assert_eq!(code(&out), 2);
    assert!(text(&out.stderr).contains("--allow-runs"), "{}", text(&out.stderr));
    let out = home.zorix(&["mcp", "serve", "--allow-runs", "--allow-access", "ftp:somewhere"]);
    assert_eq!(code(&out), 2);
    assert!(text(&out.stderr).to_lowercase().contains("ftp"), "{}", text(&out.stderr));
}

#[tokio::test]
async fn a_team_runs_through_mcp_and_delegates_only_if_the_server_allows_it() {
    let home = Home::new();
    for args in [
        &["agent", "create", "boss", "--model", "script", "--instructions", "You lead."][..],
        &["agent", "create", "helper", "--model", "script", "--instructions", "You help."][..],
        &["team", "create", "duo", "--lead", "boss", "--member", "helper"][..],
    ] {
        assert_eq!(code(&home.zorix(args)), 0);
    }
    let script = home.dir.path().join("turns.json");
    std::fs::write(
        &script,
        r#"[{"call": {"calls": [{"tool": "agents.delegate", "arguments": {"tasks": [{"agent": "helper", "task": "Find it."}]}}]}},
            {"say": "The helper found it."}, {"say": "Final: found."}]"#,
    )
    .unwrap();
    let model = format!("script:{}", script.display());
    let project = home.dir.path().display().to_string();

    // Teams are listed with the agents.
    let client = home.client(&[]).await;
    let listed = call(&client, "zorix.agents", json!({})).await.structured.expect("structured");
    assert_eq!(listed["teams"][0]["team"], "duo");
    assert_eq!(listed["teams"][0]["lead"], "boss");
    assert_eq!(listed["teams"][0]["members"], json!(["helper"]));
    client.close().await;

    // The server allows delegation and the helper: the lead asks it, and the answer says what ran.
    let client = home
        .client(&[
            "--allow-runs",
            "--model",
            &model,
            "--allow",
            "agents.delegate",
            "--allow-access",
            "agent:helper",
            "--project",
            &project,
        ])
        .await;
    let done = call(&client, "zorix.run_agent", json!({"team": "duo", "prompt": "Find it.", "wait_seconds": 60})).await;
    assert!(!done.is_error, "{}", text_of(&done));
    let done = done.structured.expect("structured");
    assert_eq!(done["state"], "completed", "{done}");
    assert_eq!(done["result"], "Final: found.");
    assert_eq!(done["agent"]["team"], "duo");
    assert_eq!(done["sub_runs"].as_array().map(Vec::len), Some(1), "{done}");
    assert_eq!(done["sub_runs"][0]["agent"], "helper");
    assert_eq!(done["sub_runs"][0]["state"], "completed");
    let both = call(&client, "zorix.run_agent", json!({"agent": "boss", "team": "duo", "prompt": "x"})).await;
    assert!(both.is_error && text_of(&both).contains("not both"), "{}", text_of(&both));
    let neither = call(&client, "zorix.run_agent", json!({"prompt": "x"})).await;
    assert!(neither.is_error && text_of(&neither).contains("Name the agent or the team"), "{}", text_of(&neither));
    client.close().await;

    // Without those flags the same lead has nothing to delegate with: the call is refused and nothing below it starts.
    let refused = home.dir.path().join("refused.json");
    std::fs::write(
        &refused,
        r#"[{"call": {"calls": [{"tool": "agents.delegate", "arguments": {"tasks": [{"agent": "helper", "task": "Find it."}]}}]}},
            {"say": "I could not ask anyone."}]"#,
    )
    .unwrap();
    let model = format!("script:{}", refused.display());
    let client = home.client(&["--allow-runs", "--model", &model, "--project", &project]).await;
    let done = call(&client, "zorix.run_agent", json!({"team": "duo", "prompt": "Find it.", "wait_seconds": 60})).await;
    let done = done.structured.expect("structured");
    assert_eq!(done["result"], "I could not ask anyone.", "{done}");
    assert!(done.get("sub_runs").is_none(), "nothing was started: {done}");
    client.close().await;
}
