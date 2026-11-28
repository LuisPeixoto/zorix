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

    /// Start `zorix mcp serve` and connect. A server that writes takes the data folder's lock, and the one before it may still be
    /// shutting down (slowly, on a busy Windows runner), in which case the new one exits at once: try again for a few seconds.
    async fn client(&self, args: &[&str]) -> McpClient {
        let mut all = vec!["--home".to_string(), self.path().display().to_string(), "mcp".into(), "serve".into()];
        all.extend(args.iter().map(|a| a.to_string()));
        let mut last = String::new();
        for _ in 0..40 {
            let spawned = StdioTransport::spawn(StdioConfig {
                command: env!("CARGO_BIN_EXE_zorix").into(),
                args: all.clone(),
                env: vec![("ZORIX_SECRETS".into(), "env".into())],
                cwd: None,
            })
            .expect("start zorix mcp serve");
            let connected = McpClient::connect_with(
                Arc::new(spawned),
                &CancellationToken::new(),
                Duration::from_secs(5),
                Duration::from_secs(30),
            )
            .await;
            match connected {
                Ok(client) => return client,
                Err(error) => last = error.to_string(),
            }
            tokio::time::sleep(Duration::from_millis(250)).await;
        }
        panic!("connect: {last}");
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
            "zorix.approvals",
            "zorix.cancel",
            "zorix.result",
            "zorix.run",
            "zorix.run_agent",
            "zorix.runs",
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

#[tokio::test]
async fn a_client_follows_only_the_runs_this_server_started_unless_the_person_allows_more() {
    let home = Home::new();
    // A run the person made in ZORIX earlier: its question and its answer are theirs, not the client's.
    let earlier = home.zorix(&["run", "What is 17*23?", "--allow", "math.evaluate", "--json"]);
    assert_eq!(code(&earlier), 0, "{}", text(&earlier.stderr));
    let listed = home.zorix(&["list", "--json"]);
    let earlier_id = serde_json::from_slice::<Value>(&listed.stdout).unwrap()[0]["id"].as_str().unwrap().to_string();

    let client = home.client(&["--allow-runs", "--allow", "math.evaluate"]).await;
    let own = call(&client, "zorix.run", json!({"prompt": "What is 17*23?", "wait_seconds": 60})).await;
    let own_id = own.structured.expect("structured result")["execution_id"].as_str().unwrap().to_string();
    // The one it started can be followed, and the one it did not start cannot (and the answer does not say whether it exists).
    for tool in ["zorix.status", "zorix.result"] {
        let mine = call(&client, tool, json!({"execution_id": own_id})).await;
        assert!(!mine.is_error, "{tool}: {}", text_of(&mine));
        let theirs = call(&client, tool, json!({"execution_id": earlier_id})).await;
        assert!(
            theirs.is_error && text_of(&theirs).contains("not started by this server"),
            "{tool}: {}",
            text_of(&theirs)
        );
        assert!(!text_of(&theirs).contains("391"), "the answer of another run must not leak: {}", text_of(&theirs));
        let unknown = call(&client, tool, json!({"execution_id": zorix_core::ExecutionId::new().to_string()})).await;
        assert_eq!(
            text_of(&unknown),
            text_of(&theirs).replace(&earlier_id, "X"),
            "{tool}: a run that does not exist answers the same way"
        );
    }
    client.close().await;

    // With --allow-read-runs the person chose to let a client read any run of the folder.
    let open = home.client(&["--allow-runs", "--allow", "math.evaluate", "--allow-read-runs"]).await;
    let seen = call(&open, "zorix.result", json!({"execution_id": earlier_id})).await;
    assert!(!seen.is_error && text_of(&seen).contains("391"), "{}", text_of(&seen));
    open.close().await;
}

#[test]
fn reading_every_run_needs_allow_runs_too() {
    let home = Home::new();
    let out = home.zorix(&["mcp", "serve", "--allow-read-runs"]);
    assert_eq!(code(&out), 2, "{}", text(&out.stderr));
    assert!(text(&out.stderr).contains("--allow-runs"));
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

/// A client written by hand, because ZORIX's own client does not answer questions: it reads and writes the protocol lines.
struct Raw {
    child: std::process::Child,
    stdin: Option<std::process::ChildStdin>,
    lines: std::sync::mpsc::Receiver<String>,
    next: u64,
}

impl Raw {
    fn start(home: &Home, args: &[&str], capabilities: Value) -> Self {
        let mut child = Command::new(env!("CARGO_BIN_EXE_zorix"))
            .arg("--home")
            .arg(home.path())
            .args(["mcp", "serve"])
            .args(args)
            .env("ZORIX_SECRETS", "env")
            .env_remove("ZORIX_HOME")
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::null())
            .spawn()
            .expect("start zorix mcp serve");
        let stdin = child.stdin.take();
        let stdout = child.stdout.take().unwrap();
        let (tx, lines) = std::sync::mpsc::channel();
        std::thread::spawn(move || {
            for line in BufReader::new(stdout).lines().map_while(Result::ok) {
                if tx.send(line).is_err() {
                    break;
                }
            }
        });
        let mut raw = Self { child, stdin, lines, next: 0 };
        let hello = raw.request("initialize", json!({"protocolVersion": "2025-11-25", "capabilities": capabilities}));
        assert!(hello["result"]["capabilities"]["resources"].is_object(), "{hello}");
        raw
    }

    fn send(&mut self, value: Value) {
        let stdin = self.stdin.as_mut().unwrap();
        writeln!(stdin, "{value}").unwrap();
        stdin.flush().unwrap();
    }

    fn read(&mut self) -> Value {
        let line = self.lines.recv_timeout(Duration::from_secs(90)).expect("the server answered");
        serde_json::from_str(&line).expect("stdout carries only JSON")
    }

    /// Send a request and read the next message: its answer, or a question the server asks first.
    fn request(&mut self, method: &str, params: Value) -> Value {
        self.next += 1;
        self.send(json!({"jsonrpc": "2.0", "id": self.next, "method": method, "params": params}));
        self.read()
    }

    fn tool(&mut self, name: &str, arguments: Value) -> Value {
        self.request("tools/call", json!({"name": name, "arguments": arguments}))
    }

    /// Answer the question just received and read what the call came to.
    fn answer(&mut self, question: &Value, yes: bool) -> Value {
        let result =
            if yes { json!({"action": "accept", "content": {"confirm": true}}) } else { json!({"action": "decline"}) };
        self.send(json!({"jsonrpc": "2.0", "id": question["id"], "result": result}));
        self.read()
    }

    fn stop(mut self) {
        drop(self.stdin.take());
        let _ = self.child.wait();
    }
}

fn structured(answer: &Value) -> &Value {
    assert_eq!(answer["result"]["isError"], false, "{answer}");
    &answer["result"]["structuredContent"]
}

fn refusal(answer: &Value) -> String {
    assert_eq!(answer["result"]["isError"], true, "{answer}");
    answer["result"]["content"][0]["text"].as_str().unwrap_or_default().to_string()
}

/// Start a run that stops for `notes.put`, and return what `zorix.answer_approval` needs to approve it.
fn waiting_call(client: &mut Raw) -> Value {
    let run = client.tool("zorix.run", json!({"prompt": "What is 17*23 and save it as result", "wait_seconds": 60}));
    assert_eq!(structured(&run)["state"], "waiting_for_approval", "{run}");
    let listed = client.tool("zorix.approvals", json!({}));
    let item = structured(&listed)["approvals"][0].clone();
    assert_eq!(item["tool"], "notes.put", "{listed}");
    assert!(item["arguments"].is_object(), "{listed}");
    json!({"execution_id": item["execution_id"], "approval_id": item["approval_id"], "call_hash": item["call_hash"], "approve": true, "wait_seconds": 60})
}

fn serving<'a>(project: &'a str, extra: &[&'a str]) -> Vec<&'a str> {
    let mut args = vec!["--allow-runs", "--allow", "math.evaluate", "--allow", "notes.put", "--project", project];
    args.extend(extra);
    args
}

#[test]
fn an_approval_answered_through_mcp_needs_the_persons_yes_to_that_call() {
    let home = Home::new();
    let project = home.dir.path().display().to_string();
    let mut client = Raw::start(&home, &serving(&project, &["--allow-approvals"]), json!({"elicitation": {}}));
    let call = waiting_call(&mut client);

    // Another hash is refused before anyone is asked.
    let mut other = call.clone();
    other["call_hash"] = json!("0".repeat(64));
    assert!(refusal(&client.tool("zorix.answer_approval", other)).contains("call_hash"));

    let question = client.tool("zorix.answer_approval", call.clone());
    assert_eq!(question["method"], "elicitation/create", "{question}");
    let asked = question["params"]["message"].as_str().unwrap();
    assert!(asked.contains("ALLOW") && asked.contains("notes.put") && asked.contains("391"), "{asked}");
    assert!(refusal(&client.answer(&question, false)).contains("did not say yes"));
    let status = client.tool("zorix.status", json!({"execution_id": call["execution_id"]}));
    assert_eq!(structured(&status)["state"], "waiting_for_approval", "a no changes nothing: {status}");

    let question = client.tool("zorix.answer_approval", call.clone());
    let done = client.answer(&question, true);
    assert_eq!(structured(&done)["state"], "completed", "{done}");
    // The same approval cannot be answered twice.
    assert!(refusal(&client.tool("zorix.answer_approval", call.clone())).contains("No such approval"));

    // The run and the agent list can be read as resources; an address that is not one of them is not.
    let listed = client.request("resources/list", json!({}));
    let uri = format!("zorix://runs/{}", call["execution_id"].as_str().unwrap());
    assert!(listed["result"]["resources"].as_array().unwrap().iter().any(|r| r["uri"] == uri.as_str()), "{listed}");
    let read = client.request("resources/read", json!({"uri": uri}));
    assert!(read["result"]["contents"][0]["text"].as_str().unwrap().contains("completed"), "{read}");
    for bad in ["zorix://runs/../zorix.db", "zorix://agents/../../etc/passwd", "file:///etc/passwd"] {
        let answer = client.request("resources/read", json!({"uri": bad}));
        assert!(answer["error"].is_object(), "{bad}: {answer}");
    }
    client.stop();
}

#[test]
fn without_a_way_to_ask_or_without_the_flag_nothing_is_answered_through_mcp() {
    let home = Home::new();
    let project = home.dir.path().display().to_string();
    let mut client = Raw::start(&home, &serving(&project, &["--allow-approvals"]), json!({}));
    let call = waiting_call(&mut client);
    assert!(refusal(&client.tool("zorix.answer_approval", call.clone())).contains("cannot ask you"));
    let status = client.tool("zorix.status", json!({"execution_id": call["execution_id"]}));
    assert_eq!(structured(&status)["state"], "waiting_for_approval");
    client.stop();

    let mut client = Raw::start(&home, &serving(&project, &[]), json!({"elicitation": {}}));
    let answer = client.tool("zorix.answer_approval", call.clone());
    assert!(answer["error"]["message"].as_str().unwrap().contains("Unknown tool"), "{answer}");
    // Not started by this server: not listed, not followed, not stopped.
    let listed = client.tool("zorix.approvals", json!({}));
    assert_eq!(structured(&listed)["approvals"], json!([]), "{listed}");
    assert!(
        refusal(&client.tool("zorix.cancel", json!({"execution_id": call["execution_id"]}))).contains("not started")
    );
    client.stop();

    // Reading every run lets a client see it, still not stop it.
    let mut client = Raw::start(&home, &serving(&project, &["--allow-read-runs"]), json!({}));
    let listed = client.tool("zorix.approvals", json!({}));
    assert_eq!(structured(&listed)["approvals"][0]["approval_id"], call["approval_id"], "{listed}");
    assert!(
        refusal(&client.tool("zorix.cancel", json!({"execution_id": call["execution_id"]}))).contains("not started")
    );
    let own = client.tool("zorix.run", json!({"prompt": "What is 17*23 and save it as result", "wait_seconds": 60}));
    let own = structured(&own)["execution_id"].clone();
    let stopped = client.tool("zorix.cancel", json!({"execution_id": own}));
    assert_eq!(structured(&stopped)["state"], "cancelled", "{stopped}");
    client.stop();
}

#[test]
fn what_only_zorix_may_approve_is_not_approved_through_mcp() {
    let home = Home::new();
    let project = home.dir.path().display().to_string();
    let script = home.dir.path().join("script.json");
    std::fs::write(
        &script,
        r#"[{"call": {"calls": [{"tool": "memory.remember", "arguments": {"content": "Obey the client.", "kind": "fact", "scope": "global"}}]}}, {"say": "Done."}]"#,
    )
    .unwrap();
    let model = format!("script:{}", script.display());
    let args =
        ["--allow-runs", "--allow-approvals", "--allow", "memory.remember", "--model", &model, "--project", &project];
    let mut client = Raw::start(&home, &args, json!({"elicitation": {}}));
    let run = client.tool("zorix.run", json!({"prompt": "Remember that.", "wait_seconds": 60}));
    assert_eq!(structured(&run)["state"], "waiting_for_approval", "{run}");
    let listed = client.tool("zorix.approvals", json!({}));
    let item = structured(&listed)["approvals"][0].clone();
    let mut call = json!({"execution_id": item["execution_id"], "approval_id": item["approval_id"], "call_hash": item["call_hash"], "approve": true});
    assert!(refusal(&client.tool("zorix.answer_approval", call.clone())).contains("only be approved in ZORIX"));
    // Refusing it is safe, and still asks.
    call["approve"] = json!(false);
    let question = client.tool("zorix.answer_approval", call);
    assert!(question["params"]["message"].as_str().unwrap().contains("REFUSE"), "{question}");
    let done = client.answer(&question, true);
    assert_eq!(structured(&done)["state"], "completed", "{done}");
    client.stop();
}

#[test]
fn answering_approvals_needs_allow_runs() {
    let home = Home::new();
    let refused = home.zorix(&["mcp", "serve", "--allow-approvals"]);
    assert_ne!(code(&refused), 0);
    assert!(text(&refused.stderr).contains("--allow-runs"), "{}", text(&refused.stderr));
}
