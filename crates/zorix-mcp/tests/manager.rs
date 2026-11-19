//! The manager against the fake server: what a person has to approve before a tool is offered, what happens when a server
//! changes, crashes or announces new tools, and a whole run through the engine (ADR-0018, ADR-0020).

use std::sync::Arc;
use std::time::Duration;

use serde_json::json;
use zorix_core::{
    EnvValue, EnvVar, ExecutionEngine, ExecutionState, Grants, IntegrationStore, McpServer, McpTransport, NewExecution,
    NewMcpServer, PinAction, PinStatus, Secret, TaskRequest, TenantId, Timestamp, ToolContext, ToolRegistry,
    ToolSource, ToolTrust,
};
use zorix_mcp::{ManagerOptions, McpManager, ServerState, env_secret_name};
use zorix_providers::{MemoryStore, SecretStore};
use zorix_runtime::{Engine, EngineConfig, InMemoryToolRegistry, MemoryRepository};
use zorix_sim::{ScriptedModel, Turn};
use zorix_store::SqliteIntegrationStore;

struct Rig {
    store: Arc<SqliteIntegrationStore>,
    registry: Arc<InMemoryToolRegistry>,
    secrets: Arc<MemoryStore>,
    manager: McpManager,
    tenant: TenantId,
}

fn rig() -> Rig {
    let store = Arc::new(SqliteIntegrationStore::open_in_memory().unwrap());
    let registry = Arc::new(InMemoryToolRegistry::new());
    let secrets = Arc::new(MemoryStore::new());
    let tenant = TenantId::local();
    let options = ManagerOptions {
        backoff: vec![Duration::from_millis(50); 3],
        probe_timeout: Duration::from_millis(800),
        work_dir: None,
    };
    let manager = McpManager::with(
        store.clone(),
        registry.clone(),
        secrets.clone(),
        tenant.clone(),
        Arc::new(zorix_mcp::DefaultConnector),
        Arc::new(zorix_core::SystemClock),
        options,
    );
    Rig { store, registry, secrets, manager, tenant }
}

async fn add(rig: &Rig, name: &str, args: &[&str], env: Vec<EnvVar>) -> McpServer {
    let transport = McpTransport::Stdio {
        command: env!("CARGO_BIN_EXE_zorix-fake-mcp").into(),
        args: args.iter().map(|a| a.to_string()).collect(),
        env,
        cwd: None,
    };
    let server = rig
        .store
        .add_mcp_server(&rig.tenant, NewMcpServer { name: name.into(), transport }, Timestamp(1))
        .await
        .unwrap();
    rig.store.update_mcp_server(&rig.tenant, McpServer { enabled: true, ..server }, Timestamp(2)).await.unwrap()
}

async fn until<F: Fn(&zorix_mcp::ServerStatus) -> bool>(
    rig: &Rig,
    id: zorix_core::McpServerId,
    what: &str,
    check: F,
) -> zorix_mcp::ServerStatus {
    for _ in 0..200 {
        if let Some(status) = rig.manager.status(id).await.unwrap()
            && check(&status)
        {
            return status;
        }
        tokio::time::sleep(Duration::from_millis(25)).await;
    }
    panic!("never reached: {what}; now {:?}", rig.manager.status(id).await.unwrap());
}

fn mcp_ids(rig: &Rig) -> Vec<String> {
    rig.registry.list().into_iter().filter(|s| s.id.starts_with("mcp.")).map(|s| s.id).collect()
}

fn ctx() -> ToolContext {
    ToolContext {
        execution_id: zorix_core::ExecutionId::new(),
        tenant: TenantId::local(),
        call_id: zorix_core::ToolCallId("c1".into()),
        grants: Grants::none(),
        cancel: tokio_util::sync::CancellationToken::new(),
        workspace_root: None,
        conversation: None,
    }
}

#[tokio::test]
async fn a_server_that_starts_offers_nothing_until_a_person_approves_what_it_lists() {
    let rig = rig();
    let server = add(&rig, "fake", &["--era", "modern"], vec![]).await;
    rig.manager.start(server.id).await.unwrap();
    let status = until(&rig, server.id, "ready", |s| s.state == ServerState::Ready).await;
    assert_eq!((status.era.as_deref(), status.server_name.as_deref()), (Some("modern"), Some("fake")));
    assert_eq!((status.tools_listed, status.tools_ready, status.tools_to_review), (8, 0, 8));
    assert!(mcp_ids(&rig).is_empty(), "nothing the server lists is offered before a person says so");

    rig.manager.review(server.id, "echo", PinAction::Approve, "luis").await.unwrap();
    assert_eq!(mcp_ids(&rig), ["mcp.fake.echo"]);
    let spec = rig.registry.get("mcp.fake.echo").unwrap().spec().clone();
    assert_eq!(spec.source, ToolSource::Mcp { server: "fake".into() });
    assert!(spec.requires_approval && spec.description.starts_with("[MCP server “fake”]"));
    let ready = rig.manager.status(server.id).await.unwrap().unwrap();
    assert_eq!((ready.tools_ready, ready.tools_to_review), (1, 7));

    // And it works.
    let tool = rig.registry.get("mcp.fake.echo").unwrap();
    let said = tool.call(&ctx(), json!({"text": "hi"})).await.unwrap();
    assert_eq!(said["content"][0]["text"], "hi");
    assert_eq!(rig.manager.approve_all(server.id, "luis").await.unwrap(), 7);
    assert_eq!(mcp_ids(&rig).len(), 8);
    rig.manager.shutdown().await;
    assert!(mcp_ids(&rig).is_empty(), "stopping a server takes its tools away");
}

#[tokio::test]
async fn a_server_that_rewrites_a_tool_loses_it_until_a_person_reviews_the_change() {
    let rig = rig();
    let server = add(&rig, "fake", &["--era", "modern", "--mutate"], vec![]).await;
    rig.manager.start(server.id).await.unwrap();
    until(&rig, server.id, "ready", |s| s.state == ServerState::Ready).await;
    rig.manager.approve_all(server.id, "luis").await.unwrap();
    assert!(mcp_ids(&rig).contains(&"mcp.fake.echo".to_string()));

    // The second listing has a poisoned description for echo.
    rig.manager.refresh(server.id).await.unwrap();
    assert!(!mcp_ids(&rig).contains(&"mcp.fake.echo".to_string()), "the changed tool is no longer offered");
    assert!(mcp_ids(&rig).contains(&"mcp.fake.env".to_string()), "the others are");
    let pins = rig.store.tool_pins(&rig.tenant, server.id).await.unwrap();
    let echo = pins.iter().find(|p| p.tool == "echo").unwrap();
    assert_eq!(echo.status, PinStatus::Changed);
    assert!(echo.approved.as_ref().unwrap()["description"].as_str().unwrap().starts_with("Echo the text back"));
    assert!(echo.seen["description"].as_str().unwrap().contains("ssh"), "the review can show what changed");
    assert_eq!(rig.manager.status(server.id).await.unwrap().unwrap().tools_to_review, 1);

    // A person approves the new text, and it is offered again.
    rig.manager.review(server.id, "echo", PinAction::Approve, "luis").await.unwrap();
    assert!(mcp_ids(&rig).contains(&"mcp.fake.echo".to_string()));
    rig.manager.shutdown().await;
}

#[tokio::test]
async fn new_tools_announced_while_running_wait_for_approval() {
    let rig = rig();
    let server = add(&rig, "fake", &["--era", "modern", "--list-changed"], vec![]).await;
    rig.manager.start(server.id).await.unwrap();
    until(&rig, server.id, "ready", |s| s.state == ServerState::Ready).await;
    rig.manager.approve_all(server.id, "luis").await.unwrap();
    rig.registry.get("mcp.fake.echo").unwrap().call(&ctx(), json!({"text": "x"})).await.unwrap();
    let status = until(&rig, server.id, "the new tool", |s| s.tools_listed == 9).await;
    assert_eq!(status.tools_to_review, 1);
    assert!(
        !mcp_ids(&rig).contains(&"mcp.fake.extra".to_string()),
        "a tool that appears later is not offered unreviewed"
    );
    rig.manager.shutdown().await;
}

#[tokio::test]
async fn a_server_that_crashes_is_restarted_and_its_approved_tools_come_back() {
    let rig = rig();
    let server = add(&rig, "fake", &["--era", "modern"], vec![]).await;
    rig.manager.start(server.id).await.unwrap();
    until(&rig, server.id, "ready", |s| s.state == ServerState::Ready).await;
    rig.manager.approve_all(server.id, "luis").await.unwrap();
    let mut states = rig.manager.subscribe();
    let crash = rig.registry.get("mcp.fake.crash").unwrap();
    assert!(crash.call(&ctx(), json!({})).await.is_err(), "the call that crashed it fails");
    until(&rig, server.id, "restarting or ready again", |s| {
        matches!(s.state, ServerState::Restarting { .. }) || s.state == ServerState::Ready
    })
    .await;
    until(&rig, server.id, "ready again", |s| s.state == ServerState::Ready).await;
    assert!(states.try_recv().is_ok(), "state changes are announced");
    assert_eq!(mcp_ids(&rig).len(), 8, "tools approved before the crash are offered again, with no new approval");
    let echo = rig.registry.get("mcp.fake.echo").unwrap().call(&ctx(), json!({"text": "back"})).await.unwrap();
    assert_eq!(echo["content"][0]["text"], "back");
    rig.manager.shutdown().await;
}

#[tokio::test]
async fn a_server_that_cannot_start_is_tried_a_few_times_and_then_reported_with_the_reason() {
    let rig = rig();
    let transport =
        McpTransport::Stdio { command: "zorix-no-such-program".into(), args: vec![], env: vec![], cwd: None };
    let server = rig
        .store
        .add_mcp_server(&rig.tenant, NewMcpServer { name: "broken".into(), transport }, Timestamp(1))
        .await
        .unwrap();
    let server =
        rig.store.update_mcp_server(&rig.tenant, McpServer { enabled: true, ..server }, Timestamp(2)).await.unwrap();
    rig.manager.start(server.id).await.unwrap();
    let status = until(&rig, server.id, "failed", |s| matches!(s.state, ServerState::Failed { .. })).await;
    match status.state {
        ServerState::Failed { reason } => assert!(reason.contains("was not found on the PATH"), "{reason}"),
        other => panic!("{other:?}"),
    }
    // A disabled server cannot be started.
    let off = add(&rig, "off", &["--era", "modern"], vec![]).await;
    rig.store.update_mcp_server(&rig.tenant, McpServer { enabled: false, ..off.clone() }, Timestamp(3)).await.unwrap();
    assert!(rig.manager.start(off.id).await.is_err());
}

#[tokio::test]
async fn secrets_for_a_server_come_from_the_keychain_and_a_missing_one_is_named() {
    let rig = rig();
    let env = vec![
        EnvVar { name: "API_TOKEN".into(), value: EnvValue::Secret },
        EnvVar { name: "MODE".into(), value: EnvValue::Literal("fast".into()) },
    ];
    let server = add(&rig, "fake", &["--era", "modern"], env).await;
    // Missing first: the reason says where to put it.
    let only_once = ManagerOptions { backoff: vec![], probe_timeout: Duration::from_millis(500), work_dir: None };
    let strict = McpManager::with(
        rig.store.clone(),
        rig.registry.clone(),
        rig.secrets.clone(),
        rig.tenant.clone(),
        Arc::new(zorix_mcp::DefaultConnector),
        Arc::new(zorix_core::SystemClock),
        only_once,
    );
    strict.start(server.id).await.unwrap();
    let status = until_manager(&strict, server.id, |s| matches!(s.state, ServerState::Failed { .. })).await;
    match status.state {
        ServerState::Failed { reason } => assert!(
            reason.contains("API_TOKEN") && reason.contains(&env_secret_name(server.id, "API_TOKEN")),
            "{reason}"
        ),
        other => panic!("{other:?}"),
    }
    rig.secrets.set(&env_secret_name(server.id, "API_TOKEN"), &Secret::new("tok-123")).unwrap();
    rig.manager.start(server.id).await.unwrap();
    until(&rig, server.id, "ready", |s| s.state == ServerState::Ready).await;
    rig.manager.approve_all(server.id, "luis").await.unwrap();
    let got = rig.registry.get("mcp.fake.env").unwrap().call(&ctx(), json!({"name": "API_TOKEN"})).await.unwrap();
    assert_eq!(got["content"][0]["text"], "tok-123");
    let mode = rig.registry.get("mcp.fake.env").unwrap().call(&ctx(), json!({"name": "MODE"})).await.unwrap();
    assert_eq!(mode["content"][0]["text"], "fast");
    // The database holds the names, not the value.
    let stored = rig.store.mcp_server(&rig.tenant, server.id).await.unwrap().unwrap();
    assert!(!format!("{stored:?}").contains("tok-123"));
    rig.manager.shutdown().await;
}

async fn until_manager(
    manager: &McpManager,
    id: zorix_core::McpServerId,
    check: impl Fn(&zorix_mcp::ServerStatus) -> bool,
) -> zorix_mcp::ServerStatus {
    for _ in 0..200 {
        if let Some(status) = manager.status(id).await.unwrap()
            && check(&status)
        {
            return status;
        }
        tokio::time::sleep(Duration::from_millis(25)).await;
    }
    panic!("never reached the state");
}

#[tokio::test]
async fn not_asking_is_a_persons_choice_for_one_tool_and_is_withdrawn_when_the_tool_changes() {
    let rig = rig();
    let server = add(&rig, "fake", &["--era", "modern", "--mutate"], vec![]).await;
    rig.manager.start(server.id).await.unwrap();
    until(&rig, server.id, "ready", |s| s.state == ServerState::Ready).await;
    rig.manager.approve_all(server.id, "luis").await.unwrap();
    assert!(
        rig.manager.set_trust(server.id, "hang", ToolTrust::ReadOnlyNoAsk).await.is_err(),
        "the server does not call it read-only"
    );
    rig.manager.set_trust(server.id, "echo", ToolTrust::ReadOnlyNoAsk).await.unwrap();
    assert!(!rig.registry.get("mcp.fake.echo").unwrap().spec().requires_approval);
    rig.manager.refresh(server.id).await.unwrap(); // echo is rewritten
    rig.manager.review(server.id, "echo", PinAction::Approve, "luis").await.unwrap();
    assert!(
        rig.registry.get("mcp.fake.echo").unwrap().spec().requires_approval,
        "a changed tool asks again, even after it is re-approved"
    );
    rig.manager.shutdown().await;
}

#[tokio::test]
async fn removing_a_server_removes_its_tools_and_what_was_known_about_them() {
    let rig = rig();
    let server = add(&rig, "fake", &["--era", "modern"], vec![]).await;
    rig.manager.start(server.id).await.unwrap();
    until(&rig, server.id, "ready", |s| s.state == ServerState::Ready).await;
    rig.manager.approve_all(server.id, "luis").await.unwrap();
    assert!(rig.manager.remove(server.id).await.unwrap());
    assert!(mcp_ids(&rig).is_empty());
    assert!(rig.store.mcp_server(&rig.tenant, server.id).await.unwrap().is_none());
    assert!(rig.manager.statuses().await.unwrap().is_empty());
}

#[tokio::test]
async fn a_run_uses_an_mcp_tool_only_after_a_person_approves_the_call() {
    let rig = rig();
    let server = add(&rig, "fake", &["--era", "legacy"], vec![]).await;
    rig.manager.start(server.id).await.unwrap();
    until(&rig, server.id, "ready", |s| s.state == ServerState::Ready).await;
    rig.manager.review(server.id, "echo", PinAction::Approve, "luis").await.unwrap();

    let model = Arc::new(ScriptedModel::new([
        Turn::call("mcp.fake.echo", json!({"text": "from the model"})),
        Turn::Say("it said hello".into()),
    ]));
    let engine = Engine::builder(Arc::new(MemoryRepository::new()), model.clone())
        .tools(rig.registry.clone())
        .config(EngineConfig { workers: 2, lease_ttl_ms: 1_000, ..EngineConfig::default() })
        .build();
    let mut grants = Grants::none();
    grants = grants.with(zorix_core::Permission::use_tool("mcp.fake.*"));
    let created = engine
        .create(NewExecution {
            request: TaskRequest {
                prompt: "say hello through the server".into(),
                model: "sim".into(),
                ..TaskRequest::default()
            },
            grants,
            ..NewExecution::default()
        })
        .await
        .unwrap();
    engine.start(&created.tenant, created.id).await.unwrap();
    let waiting = tokio::time::timeout(Duration::from_secs(10), engine.settle(&created.tenant, created.id))
        .await
        .unwrap()
        .unwrap();
    assert_eq!(waiting.state, ExecutionState::WaitingForApproval, "an MCP tool asks first");
    let events = engine.events(&created.tenant, created.id, 0).await.unwrap();
    let card = events.iter().find(|e| e.kind.name() == "execution.waiting_approval").unwrap();
    assert_eq!(card.payload["source"], "mcp:fake");
    assert!(card.payload["summary"].as_str().unwrap().contains("Run “echo” on the MCP server “fake”"));
    assert!(card.payload["definition_hash"].as_str().is_some());
    assert_eq!(card.payload["person_only"], false);

    let detail = engine.detail(&created.tenant, created.id).await.unwrap();
    let approval = detail.approvals.iter().find(|a| a.status == zorix_core::ApprovalStatus::Pending).unwrap();
    engine
        .decide(
            &created.tenant,
            created.id,
            approval.id,
            zorix_core::ApprovalDecision {
                approve: true,
                decided_by: "luis".into(),
                note: None,
                call_hash: Some(approval.call_hash.clone()),
                automatic: false,
            },
        )
        .await
        .unwrap();
    let done = tokio::time::timeout(Duration::from_secs(10), engine.settle(&created.tenant, created.id))
        .await
        .unwrap()
        .unwrap();
    assert_eq!(done.state, ExecutionState::Completed, "{:?}", done.failure);
    let second = &model.requests()[1];
    let result = second.messages.iter().find(|m| m.tool_call_id.is_some()).unwrap();
    assert!(
        result.content.contains("from the model") && result.content.contains("untrusted"),
        "the result is data: {}",
        result.content
    );
    engine.shutdown().await;
    rig.manager.shutdown().await;
}

mod remote {
    use super::*;
    use wiremock::matchers::method;
    use wiremock::{Mock, MockServer, Request, Respond, ResponseTemplate};
    use zorix_core::HttpAuth;

    struct Modern;

    impl Respond for Modern {
        fn respond(&self, request: &Request) -> ResponseTemplate {
            if request.headers.get("authorization").and_then(|v| v.to_str().ok()) != Some("Bearer pasted-token") {
                return ResponseTemplate::new(401).insert_header("WWW-Authenticate", "Bearer");
            }
            let body: serde_json::Value = serde_json::from_slice(&request.body).unwrap();
            let result = match body["method"].as_str().unwrap() {
                "server/discover" => {
                    json!({"supportedVersions": ["2026-07-28"], "capabilities": {"tools": {}}, "_meta": {"io.modelcontextprotocol/serverInfo": {"name": "remote", "version": "1"}}})
                }
                "tools/list" => {
                    json!({"tools": [{"name": "search", "description": "Search.", "inputSchema": {"type": "object"}, "annotations": {"readOnlyHint": true}}]})
                }
                _ => return ResponseTemplate::new(404),
            };
            ResponseTemplate::new(200).set_body_json(json!({"jsonrpc": "2.0", "id": body["id"], "result": result}))
        }
    }

    async fn remote(rig: &Rig, name: &str, url: &str, auth: HttpAuth) -> McpServer {
        let server = rig
            .store
            .add_mcp_server(
                &rig.tenant,
                NewMcpServer { name: name.into(), transport: McpTransport::Http { url: url.into(), auth } },
                Timestamp(1),
            )
            .await
            .unwrap();
        rig.store.update_mcp_server(&rig.tenant, McpServer { enabled: true, ..server }, Timestamp(2)).await.unwrap()
    }

    #[tokio::test]
    async fn a_remote_server_with_a_pasted_token_lists_its_tools_for_approval() {
        let rig = rig();
        let mock = MockServer::start().await;
        Mock::given(method("POST")).respond_with(Modern).mount(&mock).await;
        let server = remote(&rig, "remote", &format!("{}/mcp", mock.uri()), HttpAuth::Bearer).await;
        rig.secrets.set(&zorix_mcp::token_secret_name(server.id), &Secret::new("pasted-token")).unwrap();
        rig.manager.start(server.id).await.unwrap();
        let status = until(&rig, server.id, "ready", |s| s.state == ServerState::Ready).await;
        assert_eq!((status.era.as_deref(), status.tools_listed, status.tools_to_review), (Some("modern"), 1, 1));
        rig.manager.review(server.id, "search", PinAction::Approve, "luis").await.unwrap();
        assert_eq!(mcp_ids(&rig), ["mcp.remote.search"]);
        rig.manager.shutdown().await;
    }

    #[tokio::test]
    async fn a_remote_server_that_needs_a_sign_in_says_so_and_offers_nothing() {
        let rig = rig();
        let mock = MockServer::start().await;
        Mock::given(method("POST")).respond_with(Modern).mount(&mock).await;
        let oauth = HttpAuth::OAuth { client_id: None, scopes: vec![] };
        let server = remote(&rig, "signin", &format!("{}/mcp", mock.uri()), oauth).await;
        rig.manager.start(server.id).await.unwrap();
        let status = until(&rig, server.id, "needs login", |s| matches!(s.state, ServerState::NeedsLogin { .. })).await;
        match status.state {
            ServerState::NeedsLogin { reason } => assert!(reason.contains("zorix mcp login signin"), "{reason}"),
            other => panic!("{other:?}"),
        }
        assert!(mcp_ids(&rig).is_empty());
        // A wrong pasted token is the same: the person is told to paste another.
        let bad = remote(&rig, "badtoken", &format!("{}/mcp", mock.uri()), HttpAuth::Bearer).await;
        rig.secrets.set(&zorix_mcp::token_secret_name(bad.id), &Secret::new("wrong-token")).unwrap();
        rig.manager.start(bad.id).await.unwrap();
        until(&rig, bad.id, "needs login", |s| matches!(s.state, ServerState::NeedsLogin { .. })).await;
    }

    #[tokio::test]
    async fn an_address_that_is_not_https_is_refused_with_the_reason() {
        let rig = rig();
        let server = remote(&rig, "plain", "http://mcp.example.com/mcp", HttpAuth::None).await;
        let only_once = ManagerOptions { backoff: vec![], probe_timeout: Duration::from_millis(300), work_dir: None };
        let strict = McpManager::with(
            rig.store.clone(),
            rig.registry.clone(),
            rig.secrets.clone(),
            rig.tenant.clone(),
            Arc::new(zorix_mcp::DefaultConnector),
            Arc::new(zorix_core::SystemClock),
            only_once,
        );
        strict.start(server.id).await.unwrap();
        for _ in 0..100 {
            if let Some(ServerState::Failed { reason }) = strict.status(server.id).await.unwrap().map(|s| s.state) {
                assert!(reason.contains("plain HTTP"), "{reason}");
                return;
            }
            tokio::time::sleep(Duration::from_millis(25)).await;
        }
        panic!("never failed");
    }
}
