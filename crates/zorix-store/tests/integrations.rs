//! Contract tests for the integration store: tenants never mix, a tool's approved definition is remembered exactly, a
//! changed definition stops being offered, and no secret ends up in the database file.

use serde_json::{Value, json};
use zorix_core::{
    Connection, ConnectionKind, ConnectionPatch, ConnectionTest, EnvValue, EnvVar, HttpAuth, IntegrationStore,
    McpServer, McpTransport, NewConnection, NewMcpServer, ObservedTool, PinAction, PinStatus, SandboxFolder,
    SandboxMode, ServerSandbox, TenantId, Timestamp, ToolTrust,
};
use zorix_store::SqliteIntegrationStore;

fn tenant(name: &str) -> TenantId {
    TenantId::new(name).unwrap()
}

fn stdio(command: &str) -> McpTransport {
    McpTransport::Stdio {
        command: command.into(),
        args: vec!["--stdio".into()],
        env: vec![
            EnvVar { name: "API_TOKEN".into(), value: EnvValue::Secret },
            EnvVar { name: "MODE".into(), value: EnvValue::Literal("fast".into()) },
        ],
        cwd: None,
    }
}

fn tool(name: &str, hash: &str, annotations: Value) -> ObservedTool {
    ObservedTool {
        name: name.into(),
        hash: hash.into(),
        definition: json!({"name": name, "description": format!("tool {hash}"), "annotations": annotations}),
    }
}

const NOW: Timestamp = Timestamp(1_000);

async fn server(store: &SqliteIntegrationStore, who: &TenantId, name: &str) -> zorix_core::McpServer {
    store.add_mcp_server(who, NewMcpServer { name: name.into(), transport: stdio("npx") }, NOW).await.unwrap()
}

#[tokio::test]
async fn servers_start_disabled_have_unique_plain_names_and_belong_to_one_tenant() {
    let store = SqliteIntegrationStore::open_in_memory().unwrap();
    let (a, b) = (tenant("acme"), tenant("globex"));
    let github = server(&store, &a, "github").await;
    assert!(!github.enabled, "a person enables it after seeing what it is");
    assert!(
        store.add_mcp_server(&a, NewMcpServer { name: "github".into(), transport: stdio("x") }, NOW).await.is_err(),
        "names are unique per tenant"
    );
    server(&store, &b, "github").await;
    for bad in ["GitHub", "a b", "", "a.b", "-x"] {
        assert!(
            store.add_mcp_server(&a, NewMcpServer { name: bad.into(), transport: stdio("x") }, NOW).await.is_err(),
            "{bad}"
        );
    }
    assert_eq!(store.mcp_servers(&a).await.unwrap().len(), 1);
    // Another tenant cannot read, change, remove or see the tools of it.
    assert!(store.mcp_server(&b, github.id).await.unwrap().is_none());
    assert!(!store.remove_mcp_server(&b, github.id).await.unwrap());
    assert!(store.tool_pins(&b, github.id).await.is_err());
    assert!(store.update_mcp_server(&b, zorix_core::McpServer { enabled: true, ..github.clone() }, NOW).await.is_err());
    let enabled = store
        .update_mcp_server(&a, zorix_core::McpServer { enabled: true, ..github.clone() }, Timestamp(2_000))
        .await
        .unwrap();
    assert!(enabled.enabled && enabled.updated_at == Timestamp(2_000));
    assert!(store.mcp_server_named(&a, "github").await.unwrap().unwrap().enabled);
}

#[tokio::test]
async fn the_stored_transport_names_secrets_but_never_holds_them() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("zorix.sqlite");
    let store = SqliteIntegrationStore::open(&path).unwrap();
    let me = tenant("local");
    let http = NewMcpServer {
        name: "remote".into(),
        transport: McpTransport::Http {
            url: "https://mcp.example.com/mcp".into(),
            auth: HttpAuth::OAuth { client_id: None, scopes: vec!["read".into()] },
        },
    };
    store.add_mcp_server(&me, http, NOW).await.unwrap();
    server(&store, &me, "local-one").await;
    let back = store.mcp_server_named(&me, "local-one").await.unwrap().unwrap();
    assert_eq!(back.transport, stdio("npx"), "the transport round-trips exactly");
    store
        .add_connection(
            &me,
            NewConnection {
                kind: ConnectionKind::Github,
                name: "gh".into(),
                settings: json!({"base_url": "https://api.github.com"}),
                has_secret: true,
            },
            NOW,
        )
        .await
        .unwrap();
    drop(store);
    // Whatever the user typed as a secret could not be here: only names and the fact that one exists are stored.
    let bytes = std::fs::read(&path).unwrap();
    let text = String::from_utf8_lossy(&bytes);
    assert!(text.contains("API_TOKEN") && text.contains("\"kind\":\"secret\""));
    assert!(!text.contains("ghp_") && !text.contains("sk-"));
}

#[tokio::test]
async fn a_tool_is_new_until_a_person_approves_exactly_what_the_server_lists() {
    let store = SqliteIntegrationStore::open_in_memory().unwrap();
    let me = tenant("local");
    let s = server(&store, &me, "svc").await;
    let pins = store
        .observe_tools(
            &me,
            s.id,
            vec![tool("read", "h1", json!({"readOnlyHint": true})), tool("write", "h2", json!({}))],
            NOW,
        )
        .await
        .unwrap();
    assert!(pins.iter().all(|p| p.status == PinStatus::New && !p.usable() && p.trust == ToolTrust::Ask));
    let approved = store.review_tool(&me, s.id, "read", PinAction::approve(), "luis", Timestamp(5)).await.unwrap();
    assert_eq!(
        (approved.status, approved.approved_hash.as_deref(), approved.approved_by.as_deref()),
        (PinStatus::Pinned, Some("h1"), Some("luis"))
    );
    assert!(approved.usable() && approved.approved == Some(approved.seen.clone()));
    assert!(
        store.review_tool(&me, s.id, "read", PinAction::approve(), "  ", Timestamp(5)).await.is_err(),
        "someone has to be named"
    );
    // Observing the same definition again changes nothing.
    let again = store
        .observe_tools(
            &me,
            s.id,
            vec![tool("read", "h1", json!({"readOnlyHint": true})), tool("write", "h2", json!({}))],
            Timestamp(9),
        )
        .await
        .unwrap();
    assert_eq!(again.iter().find(|p| p.tool == "read").unwrap().status, PinStatus::Pinned);
    assert_eq!(again.iter().find(|p| p.tool == "write").unwrap().status, PinStatus::New);
}

#[tokio::test]
async fn a_changed_definition_stops_being_offered_until_a_person_reviews_it() {
    let store = SqliteIntegrationStore::open_in_memory().unwrap();
    let me = tenant("local");
    let s = server(&store, &me, "svc").await;
    let ro = json!({"readOnlyHint": true, "openWorldHint": false});
    store.observe_tools(&me, s.id, vec![tool("read", "h1", ro.clone())], NOW).await.unwrap();
    store.review_tool(&me, s.id, "read", PinAction::approve(), "luis", NOW).await.unwrap();
    store.set_tool_trust(&me, s.id, "read", ToolTrust::ReadOnlyNoAsk).await.unwrap();

    // The server quietly rewrites the description.
    let pins = store.observe_tools(&me, s.id, vec![tool("read", "EVIL", ro.clone())], NOW).await.unwrap();
    let pin = &pins[0];
    assert_eq!(pin.status, PinStatus::Changed);
    assert!(!pin.usable());
    assert_eq!(pin.trust, ToolTrust::Ask, "a changed tool asks again, whatever it was trusted to do");
    assert_eq!(
        pin.approved_hash.as_deref(),
        Some("h1"),
        "what was approved is kept, so the change can be shown as a diff"
    );
    assert_eq!(pin.seen_hash, "EVIL");
    assert_ne!(pin.approved, Some(pin.seen.clone()));

    // If the server goes back to what was approved, it is simply pinned again.
    let back = store.observe_tools(&me, s.id, vec![tool("read", "h1", ro.clone())], NOW).await.unwrap();
    assert_eq!(back[0].status, PinStatus::Pinned);

    // Or a person approves the new definition.
    store.observe_tools(&me, s.id, vec![tool("read", "h3", ro)], NOW).await.unwrap();
    let reviewed = store.review_tool(&me, s.id, "read", PinAction::approve(), "luis", NOW).await.unwrap();
    assert_eq!((reviewed.status, reviewed.approved_hash.as_deref()), (PinStatus::Pinned, Some("h3")));
}

#[tokio::test]
async fn not_asking_is_only_possible_for_an_approved_read_only_tool() {
    let store = SqliteIntegrationStore::open_in_memory().unwrap();
    let me = tenant("local");
    let s = server(&store, &me, "svc").await;
    store
        .observe_tools(
            &me,
            s.id,
            vec![
                tool("reader", "r", json!({"readOnlyHint": true, "openWorldHint": false})),
                tool("liar", "l", json!({"readOnlyHint": true, "openWorldHint": true})),
                // Read-only, and silent about whether it reaches outside: the specification's default is that it does.
                tool("unsaid", "u", json!({"readOnlyHint": true})),
                tool("destroyer", "d", json!({"readOnlyHint": true, "destructiveHint": true})),
                tool("silent", "s", json!({})),
            ],
            NOW,
        )
        .await
        .unwrap();
    // Not approved yet.
    assert!(store.set_tool_trust(&me, s.id, "reader", ToolTrust::ReadOnlyNoAsk).await.is_err());
    for name in ["reader", "liar", "unsaid", "destroyer", "silent"] {
        store.review_tool(&me, s.id, name, PinAction::approve(), "luis", NOW).await.unwrap();
    }
    assert_eq!(
        store.set_tool_trust(&me, s.id, "reader", ToolTrust::ReadOnlyNoAsk).await.unwrap().trust,
        ToolTrust::ReadOnlyNoAsk
    );
    for name in ["liar", "unsaid", "destroyer", "silent"] {
        assert!(store.set_tool_trust(&me, s.id, name, ToolTrust::ReadOnlyNoAsk).await.is_err(), "{name} always asks");
    }
    assert_eq!(store.set_tool_trust(&me, s.id, "reader", ToolTrust::Ask).await.unwrap().trust, ToolTrust::Ask);
    // Disabling a tool also resets it to asking.
    store.set_tool_trust(&me, s.id, "reader", ToolTrust::ReadOnlyNoAsk).await.unwrap();
    let off = store.review_tool(&me, s.id, "reader", PinAction::Disable, "luis", NOW).await.unwrap();
    assert_eq!((off.status, off.trust), (PinStatus::Disabled, ToolTrust::Ask));
    // A disabled tool stays disabled when the server lists it again.
    let still = store
        .observe_tools(&me, s.id, vec![tool("reader", "r", json!({"readOnlyHint": true, "openWorldHint": false}))], NOW)
        .await
        .unwrap();
    assert_eq!(still.iter().find(|p| p.tool == "reader").unwrap().status, PinStatus::Disabled);
}

#[tokio::test]
async fn tools_that_vanish_are_marked_and_unapproved_ones_are_forgotten() {
    let store = SqliteIntegrationStore::open_in_memory().unwrap();
    let me = tenant("local");
    let s = server(&store, &me, "svc").await;
    store
        .observe_tools(&me, s.id, vec![tool("kept", "k", json!({})), tool("junk", "j", json!({}))], NOW)
        .await
        .unwrap();
    store.review_tool(&me, s.id, "kept", PinAction::approve(), "luis", NOW).await.unwrap();
    let pins = store.observe_tools(&me, s.id, vec![], NOW).await.unwrap();
    assert_eq!(pins.len(), 1, "the never-approved tool is gone");
    assert!(!pins[0].listed && !pins[0].usable(), "an approved tool the server no longer lists is not offered");
    assert!(
        store.review_tool(&me, s.id, "kept", PinAction::approve(), "luis", NOW).await.is_err(),
        "nothing to approve"
    );
    // Removing the server removes what is known about its tools.
    assert!(store.remove_mcp_server(&me, s.id).await.unwrap());
    assert!(store.tool_pins(&me, s.id).await.is_err());
}

#[tokio::test]
async fn a_server_cannot_list_more_tools_than_the_limit() {
    let store = SqliteIntegrationStore::open_in_memory().unwrap();
    let me = tenant("local");
    let s = server(&store, &me, "svc").await;
    let many: Vec<ObservedTool> =
        (0..=zorix_core::integrations::MAX_TOOLS_PER_SERVER).map(|i| tool(&format!("t{i}"), "h", json!({}))).collect();
    assert!(store.observe_tools(&me, s.id, many, NOW).await.is_err());
    assert!(store.tool_pins(&me, s.id).await.unwrap().is_empty(), "nothing was half-written");
}

#[tokio::test]
async fn at_most_fifty_servers_per_tenant() {
    let store = SqliteIntegrationStore::open_in_memory().unwrap();
    let me = tenant("local");
    for i in 0..zorix_core::integrations::MAX_MCP_SERVERS {
        server(&store, &me, &format!("s{i}")).await;
    }
    assert!(
        store.add_mcp_server(&me, NewMcpServer { name: "one-more".into(), transport: stdio("x") }, NOW).await.is_err()
    );
    server(&store, &tenant("other"), "fine").await;
}

#[tokio::test]
async fn connections_are_named_tested_changed_and_kept_per_tenant() {
    let store = SqliteIntegrationStore::open_in_memory().unwrap();
    let (a, b) = (tenant("acme"), tenant("globex"));
    let new = |name: &str| NewConnection {
        kind: ConnectionKind::Postgres,
        name: name.into(),
        settings: json!({"host": "db.internal", "database": "app"}),
        has_secret: true,
    };
    let prod: Connection = store.add_connection(&a, new("prod"), NOW).await.unwrap();
    assert!(prod.last_test.is_none());
    assert!(store.add_connection(&a, new("prod"), NOW).await.is_err(), "unique per tenant");
    assert!(store.add_connection(&a, new("Prod!"), NOW).await.is_err());
    store.add_connection(&b, new("prod"), NOW).await.unwrap();
    assert!(store.connection(&b, prod.id).await.unwrap().is_none());
    assert!(!store.remove_connection(&b, prod.id).await.unwrap());
    assert!(
        store
            .record_connection_test(&b, prod.id, ConnectionTest { at: NOW, ok: true, note: "x".into() })
            .await
            .is_err()
    );

    store
        .record_connection_test(
            &a,
            prod.id,
            ConnectionTest { at: Timestamp(7), ok: false, note: "password rejected".into() },
        )
        .await
        .unwrap();
    let tested = store.connection_named(&a, "prod").await.unwrap().unwrap();
    assert_eq!(
        tested.last_test,
        Some(ConnectionTest { at: Timestamp(7), ok: false, note: "password rejected".into() })
    );
    let changed = store
        .update_connection(
            &a,
            prod.id,
            ConnectionPatch { settings: Some(json!({"host": "db2.internal"})), has_secret: None },
            Timestamp(9),
        )
        .await
        .unwrap();
    assert_eq!(
        (changed.settings["host"].as_str(), changed.has_secret, changed.updated_at),
        (Some("db2.internal"), true, Timestamp(9))
    );
    assert_eq!(store.connections(&a).await.unwrap().len(), 1);
    assert!(store.remove_connection(&a, prod.id).await.unwrap());
    assert!(store.connections(&a).await.unwrap().is_empty());
}

#[tokio::test]
async fn an_approval_is_for_the_definition_that_was_shown_and_for_no_other() {
    let store = SqliteIntegrationStore::open_in_memory().unwrap();
    let me = tenant("local");
    let s = server(&store, &me, "svc").await;
    store.observe_tools(&me, s.id, vec![tool("send", "h-shown", json!({}))], NOW).await.unwrap();
    // The server changes the tool after the person looked and before the click.
    store.observe_tools(&me, s.id, vec![tool("send", "h-swapped", json!({}))], NOW).await.unwrap();
    let refused = store.review_tool(&me, s.id, "send", PinAction::approve_seen("h-shown"), "luis", NOW).await;
    let message = refused.unwrap_err().to_string();
    assert!(message.contains("changed send after you looked at it"), "{message}");
    let pin = store.tool_pins(&me, s.id).await.unwrap().remove(0);
    assert_eq!((pin.status, pin.approved_hash.clone()), (PinStatus::New, None), "nothing was approved");
    // Approving what is shown now works, and records exactly that.
    let approved =
        store.review_tool(&me, s.id, "send", PinAction::approve_seen("h-swapped"), "luis", NOW).await.unwrap();
    assert_eq!((approved.status, approved.approved_hash.as_deref()), (PinStatus::Pinned, Some("h-swapped")));
    // A person who deliberately approves "what is pending" gives no hash and gets what is listed.
    store.observe_tools(&me, s.id, vec![tool("send", "h-third", json!({}))], NOW).await.unwrap();
    let blind = store.review_tool(&me, s.id, "send", PinAction::approve(), "luis", NOW).await.unwrap();
    assert_eq!(blind.approved_hash.as_deref(), Some("h-third"));
}

#[tokio::test]
async fn a_choice_not_to_ask_that_the_rule_no_longer_allows_is_taken_back() {
    // Before a server had to *say* that a tool stays inside it, a read-only tool with no such hint could be set to run without asking.
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("zorix.db");
    let me = tenant("local");
    let id;
    {
        let store = SqliteIntegrationStore::open(&path).unwrap();
        let s = server(&store, &me, "svc").await;
        id = s.id;
        store.observe_tools(&me, s.id, vec![tool("old", "h", json!({"readOnlyHint": true}))], NOW).await.unwrap();
        store.review_tool(&me, s.id, "old", PinAction::approve(), "luis", NOW).await.unwrap();
    }
    // The old behaviour, written straight into the file as it would have been.
    rusqlite::Connection::open(&path)
        .unwrap()
        .execute("UPDATE mcp_tool_pins SET trust = 'read_only_no_ask' WHERE tool = 'old'", [])
        .unwrap();
    let store = SqliteIntegrationStore::open(&path).unwrap();
    assert_eq!(store.tool_pins(&me, id).await.unwrap()[0].trust, ToolTrust::ReadOnlyNoAsk, "as it was left");
    // The next time the server's tools are read, the choice is taken back, and the tool stays approved.
    let pins = store.observe_tools(&me, id, vec![tool("old", "h", json!({"readOnlyHint": true}))], NOW).await.unwrap();
    assert_eq!((pins[0].status, pins[0].trust), (PinStatus::Pinned, ToolTrust::Ask));
    // One that does say so keeps its choice.
    store
        .observe_tools(&me, id, vec![tool("ok", "g", json!({"readOnlyHint": true, "openWorldHint": false}))], NOW)
        .await
        .unwrap();
    store.review_tool(&me, id, "ok", PinAction::approve(), "luis", NOW).await.unwrap();
    store.set_tool_trust(&me, id, "ok", ToolTrust::ReadOnlyNoAsk).await.unwrap();
    let again = store
        .observe_tools(&me, id, vec![tool("ok", "g", json!({"readOnlyHint": true, "openWorldHint": false}))], NOW)
        .await
        .unwrap();
    assert_eq!(again.iter().find(|p| p.tool == "ok").unwrap().trust, ToolTrust::ReadOnlyNoAsk);
}

#[tokio::test]
async fn the_variables_of_a_server_are_names_that_differ_by_more_than_case() {
    let store = SqliteIntegrationStore::open_in_memory().unwrap();
    let me = tenant("local");
    let with = |names: &[&str]| McpTransport::Stdio {
        command: "npx".into(),
        args: vec![],
        env: names.iter().map(|n| EnvVar { name: (*n).into(), value: EnvValue::Literal("1".into()) }).collect(),
        cwd: None,
    };
    let add = |name: &str, transport: McpTransport| NewMcpServer { name: name.into(), transport };
    // The same variable twice, and two that differ only by case (one secret would be kept for both).
    for (names, wanted) in [(&["FOO", "FOO"][..], "listed twice"), (&["FOO", "foo"][..], "differ only by case")] {
        let error = store.add_mcp_server(&me, add("bad", with(names)), NOW).await.unwrap_err().to_string();
        assert!(error.contains(wanted), "{names:?}: {error}");
    }
    for name in ["", "1ST", "A-B", "A B", "A=B"] {
        assert!(store.add_mcp_server(&me, add("bad", with(&[name])), NOW).await.is_err(), "{name:?}");
    }
    let ok = store.add_mcp_server(&me, add("fine", with(&["FOO", "BAR_2", "_x"])), NOW).await.unwrap();
    // Changing a server later is held to the same rule.
    let changed = McpServer { transport: with(&["FOO", "foo"]), ..ok };
    assert!(store.update_mcp_server(&me, changed, NOW).await.is_err());
}

#[tokio::test]
async fn a_new_local_server_is_confined_a_remote_one_has_no_sandbox_and_a_choice_is_kept() {
    let store = SqliteIntegrationStore::open_in_memory().unwrap();
    let me = tenant("local");
    let local =
        store.add_mcp_server(&me, NewMcpServer { name: "files".into(), transport: stdio("npx") }, NOW).await.unwrap();
    let sandbox = local.sandbox.clone().expect("a new local server starts confined");
    assert_eq!((sandbox.mode, sandbox.network, sandbox.folders.len()), (SandboxMode::Native, true, 0));
    let remote = NewMcpServer {
        name: "web".into(),
        transport: McpTransport::Http { url: "https://mcp.example.com/mcp".into(), auth: HttpAuth::None },
    };
    assert_eq!(store.add_mcp_server(&me, remote, NOW).await.unwrap().sandbox, None);
    // A choice is stored and read back; a folder that cannot be given is refused and changes nothing.
    let folder = if cfg!(windows) { "C:\\work" } else { "/work" };
    let chosen = ServerSandbox {
        mode: SandboxMode::Native,
        network: false,
        folders: vec![SandboxFolder { path: folder.into(), write: true }],
    };
    store.set_mcp_sandbox(&me, local.id, Some(chosen.clone()), NOW).await.unwrap();
    assert_eq!(store.mcp_server(&me, local.id).await.unwrap().unwrap().sandbox, Some(chosen.clone()));
    let bad = ServerSandbox {
        folders: vec![SandboxFolder { path: format!("{folder}/.ssh"), write: false }],
        ..chosen.clone()
    };
    assert!(store.set_mcp_sandbox(&me, local.id, Some(bad), NOW).await.is_err());
    assert_eq!(store.mcp_server(&me, local.id).await.unwrap().unwrap().sandbox, Some(chosen));
    // Turning a server on and off, or changing it, keeps its sandbox.
    let server = store.mcp_server(&me, local.id).await.unwrap().unwrap();
    store.update_mcp_server(&me, McpServer { enabled: true, ..server }, NOW).await.unwrap();
    assert!(store.mcp_server(&me, local.id).await.unwrap().unwrap().sandbox.is_some());
    // Another tenant cannot change it.
    assert!(store.set_mcp_sandbox(&tenant("other"), local.id, None, NOW).await.is_err());
}
