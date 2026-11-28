//! ZORIX as an MCP server: both protocol eras, the limits, and what it will not do (ADR-0023).

use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};

use async_trait::async_trait;
use serde_json::{Value, json};
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader, DuplexStream};
use tokio_util::sync::CancellationToken;
use zorix_core::{Action, Grants, Permission, TenantId, Tool, ToolAnnotations, ToolContext, ToolError, ToolSpec};
use zorix_mcp::{MAX_IN_FLIGHT, MAX_REQUEST_BYTES, McpServer, ServerConfig};

struct Probe {
    spec: ToolSpec,
    calls: Arc<AtomicUsize>,
    behaviour: Behaviour,
}

enum Behaviour {
    Echo,
    Hang,
    Fail,
    Long,
    Sleep(u64),
}

#[async_trait]
impl Tool for Probe {
    fn spec(&self) -> &ToolSpec {
        &self.spec
    }

    async fn call(&self, ctx: &ToolContext, args: Value) -> Result<Value, ToolError> {
        self.calls.fetch_add(1, Ordering::SeqCst);
        match self.behaviour {
            Behaviour::Echo => Ok(json!({"echo": args["text"], "tenant": ctx.tenant.to_string()})),
            Behaviour::Hang => {
                ctx.cancel.cancelled().await;
                Err(ToolError::Failed("cancelled".into()))
            }
            Behaviour::Fail => Err(ToolError::Failed("the thing is broken".into())),
            Behaviour::Long => Ok(json!("é".repeat(5000))),
            Behaviour::Sleep(ms) => {
                tokio::time::sleep(std::time::Duration::from_millis(ms)).await;
                Ok(json!("late"))
            }
        }
    }
}

fn probe(id: &str, behaviour: Behaviour, tweak: impl FnOnce(&mut ToolSpec)) -> (Arc<dyn Tool>, Arc<AtomicUsize>) {
    let mut spec = ToolSpec::new(
        id,
        id,
        format!("{id} for tests"),
        json!({"type": "object", "properties": {"text": {"type": "string"}}, "additionalProperties": false}),
    );
    spec.annotations = ToolAnnotations::safe_read();
    tweak(&mut spec);
    let calls = Arc::new(AtomicUsize::new(0));
    (Arc::new(Probe { spec, calls: calls.clone(), behaviour }), calls)
}

struct Rig {
    to_server: DuplexStream,
    from_server: BufReader<DuplexStream>,
    done: tokio::task::JoinHandle<std::io::Result<()>>,
    cancel: CancellationToken,
    counters: std::collections::HashMap<&'static str, Arc<AtomicUsize>>,
}

async fn rig() -> Rig {
    let mut counters = std::collections::HashMap::new();
    let mut tools = Vec::new();
    let mut add = |name: &'static str, behaviour: Behaviour, tweak: Box<dyn FnOnce(&mut ToolSpec)>| {
        let (tool, calls) = probe(name, behaviour, tweak);
        counters.insert(name, calls);
        tools.push(tool);
    };
    add("test.echo", Behaviour::Echo, Box::new(|_| {}));
    add("test.hang", Behaviour::Hang, Box::new(|s| s.timeout_ms = 60_000));
    add("test.fail", Behaviour::Fail, Box::new(|_| {}));
    add("test.long", Behaviour::Long, Box::new(|s| s.max_output_bytes = 100));
    add("test.slow", Behaviour::Sleep(2000), Box::new(|s| s.timeout_ms = 100));
    add("test.writes", Behaviour::Echo, Box::new(|s| s.requires_approval = true));
    add("test.person", Behaviour::Echo, Box::new(|s| s.approval_by_person_only = true));
    add(
        "test.files",
        Behaviour::Echo,
        Box::new(|s| s.required_permissions.push(Permission::new(Action::Read, "workspace:*"))),
    );
    add("test.bad_schema", Behaviour::Echo, Box::new(|s| s.input_schema = json!({"type": "wrong"})));
    let mut grants = Grants::none();
    for id in
        ["test.echo", "test.hang", "test.fail", "test.long", "test.slow", "test.writes", "test.person", "test.files"]
    {
        grants = grants.with(Permission::use_tool(id));
    }
    let config = ServerConfig {
        name: "zorix".into(),
        version: "9.9.9".into(),
        instructions: Some("Test instructions.".into()),
        tenant: TenantId::local(),
        grants,
        workspace_root: None,
    };
    let server = Arc::new(McpServer::new(config, tools));
    let (to_server, server_in) = tokio::io::duplex(1 << 22);
    let (server_out, from_server) = tokio::io::duplex(1 << 22);
    let cancel = CancellationToken::new();
    let done = tokio::spawn(server.serve(server_in, server_out, cancel.clone()));
    Rig { to_server, from_server: BufReader::new(from_server), done, cancel, counters }
}

impl Rig {
    async fn send(&mut self, value: Value) {
        self.to_server.write_all(format!("{value}\n").as_bytes()).await.unwrap();
    }

    async fn read(&mut self) -> Value {
        let mut line = String::new();
        let n = tokio::time::timeout(std::time::Duration::from_secs(10), self.from_server.read_line(&mut line))
            .await
            .expect("the server answered in time")
            .unwrap();
        assert!(n > 0, "the server closed");
        serde_json::from_str(&line).unwrap_or_else(|_| panic!("stdout carries only JSON, got {line:?}"))
    }

    async fn ask(&mut self, id: i64, method: &str, params: Value) -> Value {
        self.send(json!({"jsonrpc": "2.0", "id": id, "method": method, "params": params})).await;
        self.read().await
    }

    fn calls(&self, name: &str) -> usize {
        self.counters[name].load(Ordering::SeqCst)
    }
}

fn modern(params: Value) -> Value {
    let mut params = params;
    params["_meta"] = json!({
        "io.modelcontextprotocol/protocolVersion": "2026-07-28",
        "io.modelcontextprotocol/clientInfo": {"name": "t", "version": "1"},
        "io.modelcontextprotocol/clientCapabilities": {},
    });
    params
}

#[tokio::test]
async fn a_modern_client_discovers_lists_and_calls_without_a_handshake() {
    let mut rig = rig().await;
    let discovered = rig.ask(1, "server/discover", modern(json!({}))).await;
    assert_eq!(discovered["result"]["resultType"], "complete");
    assert_eq!(discovered["result"]["supportedVersions"][0], "2026-07-28");
    assert_eq!(discovered["result"]["_meta"]["io.modelcontextprotocol/serverInfo"]["name"], "zorix");
    assert_eq!(discovered["result"]["capabilities"]["tools"]["listChanged"], false);
    assert_eq!(discovered["result"]["instructions"], "Test instructions.");

    let listed = rig.ask(2, "tools/list", modern(json!({}))).await;
    let names: Vec<&str> =
        listed["result"]["tools"].as_array().unwrap().iter().map(|t| t["name"].as_str().unwrap()).collect();
    assert!(names.contains(&"test.echo") && !names.contains(&"test.bad_schema"), "{names:?}");
    let echo = listed["result"]["tools"].as_array().unwrap().iter().find(|t| t["name"] == "test.echo").unwrap();
    assert_eq!(echo["annotations"]["readOnlyHint"], true);
    assert_eq!(listed["result"]["resultType"], "complete");

    let called = rig.ask(3, "tools/call", modern(json!({"name": "test.echo", "arguments": {"text": "hi"}}))).await;
    assert_eq!(called["result"]["isError"], false);
    assert_eq!(called["result"]["resultType"], "complete");
    assert_eq!(called["result"]["structuredContent"]["echo"], "hi");
    let text: Value = serde_json::from_str(called["result"]["content"][0]["text"].as_str().unwrap()).unwrap();
    assert_eq!(text["echo"], "hi", "the text carries the same result for clients that ignore structured content");
    assert_eq!(rig.ask(4, "ping", modern(json!({}))).await["result"], json!({}));
}

#[tokio::test]
async fn a_legacy_client_must_initialize_first_and_gets_the_version_it_asked_for() {
    let mut rig = rig().await;
    let early = rig.ask(1, "tools/list", json!({})).await;
    assert_eq!(early["error"]["code"], -32600, "{early}");

    let init = rig.ask(2, "initialize", json!({"protocolVersion": "2025-06-18", "capabilities": {}})).await;
    assert_eq!(init["result"]["protocolVersion"], "2025-06-18");
    assert_eq!(init["result"]["serverInfo"]["version"], "9.9.9");
    assert_eq!(init["result"]["capabilities"]["tools"]["listChanged"], false);
    rig.send(json!({"jsonrpc": "2.0", "method": "notifications/initialized"})).await;

    let listed = rig.ask(3, "tools/list", json!({})).await;
    assert!(listed["result"]["tools"].as_array().unwrap().len() >= 5);
    assert!(listed["result"].get("resultType").is_none(), "legacy results carry no resultType");
    let called = rig.ask(4, "tools/call", json!({"name": "test.echo", "arguments": {"text": "old"}})).await;
    assert_eq!(called["result"]["structuredContent"]["echo"], "old");

    // An unknown revision gets the newest one ZORIX speaks, and the client decides.
    let mut again = self::rig().await;
    let other = again.ask(1, "initialize", json!({"protocolVersion": "1999-01-01"})).await;
    assert_eq!(other["result"]["protocolVersion"], "2025-11-25");
}

#[tokio::test]
async fn an_unknown_modern_version_is_refused_with_the_versions_that_work() {
    let mut rig = rig().await;
    let mut params = modern(json!({}));
    params["_meta"]["io.modelcontextprotocol/protocolVersion"] = json!("2030-01-01");
    let answer = rig.ask(1, "tools/list", params).await;
    assert_eq!(answer["error"]["code"], -32022);
    assert!(answer["error"]["data"]["supported"].as_array().unwrap().iter().any(|v| v == "2026-07-28"));
}

#[tokio::test]
async fn bad_requests_get_errors_and_never_reach_a_tool() {
    let mut rig = rig().await;
    let unknown = rig.ask(1, "tools/call", modern(json!({"name": "test.nope"}))).await;
    assert_eq!(unknown["error"]["code"], -32602);
    let bad = rig.ask(2, "tools/call", modern(json!({"name": "test.echo", "arguments": {"text": 5}}))).await;
    assert_eq!(bad["error"]["code"], -32602);
    let extra = rig.ask(3, "tools/call", modern(json!({"name": "test.echo", "arguments": {"other": 1}}))).await;
    assert_eq!(extra["error"]["code"], -32602);
    assert_eq!(rig.ask(4, "resources/list", modern(json!({}))).await["error"]["code"], -32601);
    // Sampling and the like are not something a client can ask of a server, and a client's answers are ignored.
    rig.send(json!({"jsonrpc": "2.0", "id": 9, "result": {}})).await;
    // Rubbish is answered, and the server carries on.
    rig.to_server.write_all(b"this is not json\n").await.unwrap();
    assert_eq!(rig.read().await["error"]["code"], -32700);
    assert_eq!(rig.ask(5, "ping", modern(json!({}))).await["result"], json!({}));
    assert_eq!(rig.calls("test.echo"), 0);
}

#[tokio::test]
async fn a_tool_that_needs_a_person_or_a_grant_is_not_run() {
    let mut rig = rig().await;
    for name in ["test.writes", "test.person"] {
        let answer = rig.ask(1, "tools/call", modern(json!({"name": name, "arguments": {}}))).await;
        assert_eq!(answer["result"]["isError"], true, "{name}");
        let text = answer["result"]["content"][0]["text"].as_str().unwrap();
        assert!(text.contains("approval") && text.contains("in ZORIX"), "{text}");
        assert_eq!(rig.calls(name), 0, "{name} must not run");
    }
    let files = rig.ask(2, "tools/call", modern(json!({"name": "test.files", "arguments": {}}))).await;
    assert_eq!(files["result"]["isError"], true);
    assert!(files["result"]["content"][0]["text"].as_str().unwrap().contains("did not allow"));
    assert_eq!(rig.calls("test.files"), 0);
}

#[tokio::test]
async fn a_tool_that_fails_or_is_too_slow_says_so_as_a_result() {
    let mut rig = rig().await;
    let failed = rig.ask(1, "tools/call", modern(json!({"name": "test.fail", "arguments": {}}))).await;
    assert_eq!(failed["result"]["isError"], true);
    assert_eq!(failed["result"]["content"][0]["text"], "the thing is broken");
    let slow = rig.ask(2, "tools/call", modern(json!({"name": "test.slow", "arguments": {}}))).await;
    assert_eq!(slow["result"]["isError"], true);
    assert!(slow["result"]["content"][0]["text"].as_str().unwrap().contains("was stopped"));
}

#[tokio::test]
async fn a_long_result_is_cut_on_a_character_boundary_and_marked() {
    let mut rig = rig().await;
    let long = rig.ask(1, "tools/call", modern(json!({"name": "test.long", "arguments": {}}))).await;
    let text = long["result"]["content"][0]["text"].as_str().unwrap();
    assert!(text.ends_with("[ZORIX cut the rest of this result]"), "{text}");
    assert!(text.len() < 200);
}

#[tokio::test]
async fn a_cancelled_call_stops_and_gets_no_answer() {
    let mut rig = rig().await;
    rig.send(json!({"jsonrpc": "2.0", "id": 7, "method": "tools/call", "params": modern(json!({"name": "test.hang", "arguments": {}}))})).await;
    while rig.calls("test.hang") == 0 {
        tokio::time::sleep(std::time::Duration::from_millis(10)).await;
    }
    rig.send(json!({"jsonrpc": "2.0", "method": "notifications/cancelled", "params": {"requestId": 7}})).await;
    // The very next answer is the ping's: the cancelled call said nothing.
    let answer = rig.ask(8, "ping", modern(json!({}))).await;
    assert_eq!(answer["id"], 8);
}

#[tokio::test]
async fn only_a_few_calls_run_at_once() {
    let mut rig = rig().await;
    for id in 0..MAX_IN_FLIGHT as i64 {
        rig.send(json!({"jsonrpc": "2.0", "id": 100 + id, "method": "tools/call", "params": modern(json!({"name": "test.hang", "arguments": {}}))})).await;
    }
    while rig.calls("test.hang") < MAX_IN_FLIGHT {
        tokio::time::sleep(std::time::Duration::from_millis(10)).await;
    }
    let refused = rig.ask(1, "tools/call", modern(json!({"name": "test.hang", "arguments": {}}))).await;
    assert_eq!(refused["error"]["code"], -32000, "{refused}");
    assert_eq!(rig.calls("test.hang"), MAX_IN_FLIGHT);
}

#[tokio::test]
async fn an_oversized_line_is_refused_without_stopping_the_server() {
    let mut rig = rig().await;
    let mut huge = vec![b'x'; MAX_REQUEST_BYTES + 10];
    huge.push(b'\n');
    rig.to_server.write_all(&huge).await.unwrap();
    assert_eq!(rig.read().await["error"]["code"], -32700);
    assert_eq!(rig.ask(1, "ping", modern(json!({}))).await["result"], json!({}));
}

#[tokio::test]
async fn closing_the_input_or_cancelling_ends_the_server_and_its_calls() {
    let mut rig = rig().await;
    rig.send(json!({"jsonrpc": "2.0", "id": 1, "method": "tools/call", "params": modern(json!({"name": "test.hang", "arguments": {}}))})).await;
    while rig.calls("test.hang") == 0 {
        tokio::time::sleep(std::time::Duration::from_millis(10)).await;
    }
    drop(rig.to_server);
    let ended = tokio::time::timeout(std::time::Duration::from_secs(10), rig.done).await;
    assert!(ended.expect("the server ended").unwrap().is_ok());

    let rig = self::rig().await;
    rig.cancel.cancel();
    let ended = tokio::time::timeout(std::time::Duration::from_secs(10), rig.done).await;
    assert!(ended.expect("the server ended").unwrap().is_ok());
}

/// Asks about the text a call names, and refuses "forbidden" without asking.
struct AskAbout;

#[async_trait]
impl zorix_mcp::Confirmation for AskAbout {
    async fn question(&self, args: &Value) -> Result<String, String> {
        match args["text"].as_str() {
            Some("forbidden") => Err("That is never done here.".into()),
            text => Ok(format!("Do the thing with {}?", text.unwrap_or("nothing"))),
        }
    }
}

async fn confirming() -> (Rig, Arc<AtomicUsize>) {
    let (tool, calls) = probe("test.confirmed", Behaviour::Echo, |_| {});
    let config = ServerConfig {
        name: "zorix".into(),
        version: "9.9.9".into(),
        instructions: None,
        tenant: TenantId::local(),
        grants: Grants::none().with(Permission::use_tool("test.confirmed")),
        workspace_root: None,
    };
    let server = Arc::new(McpServer::new(config, vec![tool]).confirming("test.confirmed", Arc::new(AskAbout)));
    let (to_server, server_in) = tokio::io::duplex(1 << 20);
    let (server_out, from_server) = tokio::io::duplex(1 << 20);
    let cancel = CancellationToken::new();
    let done = tokio::spawn(server.serve(server_in, server_out, cancel.clone()));
    (Rig { to_server, from_server: BufReader::new(from_server), done, cancel, counters: Default::default() }, calls)
}

fn said(answer: &Value) -> String {
    answer["result"]["content"][0]["text"].as_str().unwrap_or_default().to_string()
}

#[tokio::test]
async fn a_legacy_client_that_cannot_ask_is_refused_and_one_that_can_is_asked_first() {
    let (mut rig, calls) = confirming().await;
    rig.ask(1, "initialize", json!({"protocolVersion": "2025-11-25", "capabilities": {}})).await;
    let refused = rig.ask(2, "tools/call", json!({"name": "test.confirmed", "arguments": {"text": "x"}})).await;
    assert!(said(&refused).contains("cannot ask you"), "{refused}");
    assert_eq!(calls.load(Ordering::SeqCst), 0);

    let (mut rig, calls) = confirming().await;
    rig.ask(1, "initialize", json!({"protocolVersion": "2025-11-25", "capabilities": {"elicitation": {}}})).await;
    for (answer, runs) in [
        (json!({"action": "decline"}), 0),
        (json!({"action": "accept", "content": {"confirm": false}}), 0),
        (json!({"action": "accept", "content": {"confirm": true}}), 1),
    ] {
        rig.send(json!({"jsonrpc": "2.0", "id": 3, "method": "tools/call", "params": {"name": "test.confirmed", "arguments": {"text": "x"}}})).await;
        let question = rig.read().await;
        assert_eq!(question["method"], "elicitation/create", "{question}");
        assert_eq!(question["params"]["message"], "Do the thing with x?");
        assert_eq!(calls.load(Ordering::SeqCst), 0, "nothing happens before the answer");
        rig.send(json!({"jsonrpc": "2.0", "id": question["id"], "result": answer})).await;
        let result = rig.read().await;
        assert_eq!(result["id"], 3);
        assert_eq!(calls.load(Ordering::SeqCst), runs, "{answer}: {result}");
        if runs == 0 {
            assert!(said(&result).contains("did not say yes"), "{result}");
        }
    }
    // A call refused before asking is not asked about.
    let never = rig.ask(4, "tools/call", json!({"name": "test.confirmed", "arguments": {"text": "forbidden"}})).await;
    assert_eq!(said(&never), "That is never done here.");
    rig.cancel.cancel();
    rig.done.await.unwrap().unwrap();
}

#[tokio::test]
async fn a_modern_answer_counts_once_and_only_for_the_call_it_was_asked_about() {
    let (mut rig, calls) = confirming().await;
    let meta = |capabilities: Value| json!({"io.modelcontextprotocol/protocolVersion": "2026-07-28", "io.modelcontextprotocol/clientInfo": {"name": "t", "version": "1"}, "io.modelcontextprotocol/clientCapabilities": capabilities});
    let call = |text: &str, extra: Value, capabilities: Value| {
        let mut params = json!({"name": "test.confirmed", "arguments": {"text": text}, "_meta": meta(capabilities)});
        for (key, value) in extra.as_object().cloned().unwrap_or_default() {
            params[key] = value;
        }
        params
    };
    let cannot = rig.ask(1, "tools/call", call("x", json!({}), json!({}))).await;
    assert!(said(&cannot).contains("cannot ask you"), "{cannot}");
    let asked = rig.ask(2, "tools/call", call("x", json!({}), json!({"elicitation": {}}))).await;
    assert_eq!(asked["result"]["resultType"], "input_required", "{asked}");
    assert_eq!(asked["result"]["inputRequests"]["confirm"]["method"], "elicitation/create");
    let state = asked["result"]["requestState"].clone();
    let yes = json!({"confirm": {"action": "accept", "content": {"confirm": true}}});
    // The state of one call does not answer another, and a made-up state answers nothing.
    for (text, given) in [("y", state.clone()), ("x", json!("00".repeat(32)))] {
        let refused = rig
            .ask(
                3,
                "tools/call",
                call(text, json!({"inputResponses": yes, "requestState": given}), json!({"elicitation": {}})),
            )
            .await;
        assert!(said(&refused).contains("another question"), "{text}: {refused}");
    }
    assert_eq!(calls.load(Ordering::SeqCst), 0);
    let done = rig
        .ask(
            4,
            "tools/call",
            call("x", json!({"inputResponses": yes, "requestState": state}), json!({"elicitation": {}})),
        )
        .await;
    assert_eq!(done["result"]["isError"], false, "{done}");
    assert_eq!(calls.load(Ordering::SeqCst), 1);
    let again = rig
        .ask(
            5,
            "tools/call",
            call("x", json!({"inputResponses": yes, "requestState": state}), json!({"elicitation": {}})),
        )
        .await;
    assert!(said(&again).contains("another question"), "an answer is used once: {again}");
    assert_eq!(calls.load(Ordering::SeqCst), 1);
}

#[tokio::test]
async fn after_a_no_the_same_call_can_be_asked_again_and_questions_do_not_pile_up() {
    let (mut rig, calls) = confirming().await;
    let params = |extra: Value| {
        let mut params = json!({"name": "test.confirmed", "arguments": {"text": "x"}, "_meta": {
            "io.modelcontextprotocol/protocolVersion": "2026-07-28",
            "io.modelcontextprotocol/clientCapabilities": {"elicitation": {}}}});
        for (key, value) in extra.as_object().cloned().unwrap_or_default() {
            params[key] = value;
        }
        params
    };
    for (action, runs) in [("decline", 0), ("accept", 1)] {
        let asked = rig.ask(1, "tools/call", params(json!({}))).await;
        let answer = json!({"confirm": {"action": action, "content": {"confirm": true}}});
        let result = rig
            .ask(
                2,
                "tools/call",
                params(json!({"inputResponses": answer, "requestState": asked["result"]["requestState"]})),
            )
            .await;
        assert_eq!(calls.load(Ordering::SeqCst), runs, "{action}: {result}");
    }

    let (mut rig, _) = confirming().await;
    rig.ask(1, "initialize", json!({"protocolVersion": "2025-11-25", "capabilities": {"elicitation": {}}})).await;
    for id in 0..=MAX_IN_FLIGHT {
        rig.send(json!({"jsonrpc": "2.0", "id": 10 + id, "method": "tools/call", "params": {"name": "test.confirmed", "arguments": {}}})).await;
    }
    let mut questions = 0;
    let mut refused = 0;
    for _ in 0..=MAX_IN_FLIGHT {
        let line = rig.read().await;
        if line["method"] == "elicitation/create" {
            questions += 1;
        } else {
            assert!(said(&line).contains("waiting for your answer"), "{line}");
            refused += 1;
        }
    }
    assert_eq!((questions, refused), (MAX_IN_FLIGHT, 1));
}
