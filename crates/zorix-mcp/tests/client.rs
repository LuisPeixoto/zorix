//! The client against a fake server that plays well-behaved and hostile servers of both eras (ADR-0018).

use std::sync::Arc;
use std::time::Duration;

use serde_json::json;
use tokio_util::sync::CancellationToken;
use zorix_mcp::{Era, McpClient, McpError, ServerEvent, StdioConfig, StdioTransport};

fn config(args: &[&str]) -> StdioConfig {
    StdioConfig {
        command: env!("CARGO_BIN_EXE_zorix-fake-mcp").into(),
        args: args.iter().map(|a| a.to_string()).collect(),
        env: vec![],
        cwd: None,
    }
}

async fn connect_with(args: &[&str], env: Vec<(String, String)>) -> Result<McpClient, McpError> {
    let mut c = config(args);
    c.env = env;
    let transport = Arc::new(StdioTransport::spawn(c)?);
    McpClient::connect_with(transport, &CancellationToken::new(), Duration::from_millis(600), Duration::from_secs(10))
        .await
}

async fn connect(args: &[&str]) -> McpClient {
    connect_with(args, vec![]).await.unwrap_or_else(|e| panic!("{args:?}: {e}"))
}

fn text_of(output: &zorix_mcp::CallOutput) -> String {
    output.content.iter().filter_map(|c| c["text"].as_str()).collect::<Vec<_>>().join("")
}

async fn call(client: &McpClient, name: &str, args: serde_json::Value) -> Result<zorix_mcp::CallOutput, McpError> {
    client.call_tool(name, args, Duration::from_secs(10), &CancellationToken::new()).await
}

#[tokio::test]
async fn a_modern_server_needs_no_handshake_and_lists_and_calls_tools() {
    let client = connect(&["--era", "modern"]).await;
    assert_eq!(client.era(), &Era::Modern);
    assert_eq!(client.info().name.as_deref(), Some("fake"));
    assert!(client.info().instructions.is_some());
    let listing = client.list_tools(&CancellationToken::new()).await.unwrap();
    assert!(listing.tools.iter().any(|t| t.name == "echo") && listing.skipped.is_empty());
    assert_eq!(text_of(&call(&client, "echo", json!({"text": "hello"})).await.unwrap()), "hello");
    client.close().await;
    assert!(!client.alive());
}

#[tokio::test]
async fn a_legacy_server_is_found_out_by_the_probe_and_gets_an_initialize_handshake() {
    let client = connect(&["--era", "legacy"]).await;
    assert_eq!(client.era(), &Era::Legacy { version: "2025-11-25".into() });
    assert_eq!(client.era().label(), "legacy");
    let listing = client.list_tools(&CancellationToken::new()).await.unwrap();
    assert!(!listing.tools.is_empty());
    assert_eq!(text_of(&call(&client, "echo", json!({"text": "old"})).await.unwrap()), "old");
}

#[tokio::test]
async fn a_server_that_speaks_both_is_treated_as_modern() {
    let client = connect(&["--era", "dual"]).await;
    assert_eq!(client.era(), &Era::Modern);
}

#[tokio::test]
async fn a_legacy_server_that_ignores_unknown_methods_is_still_found_after_the_probe_times_out() {
    let started = std::time::Instant::now();
    let client = connect(&["--era", "legacy", "--silent-discover"]).await;
    assert_eq!(client.era().label(), "legacy");
    assert!(started.elapsed() < Duration::from_secs(5), "it waited for the probe, not for the handshake");
    assert!(!client.list_tools(&CancellationToken::new()).await.unwrap().tools.is_empty());
}

#[tokio::test]
async fn a_server_that_asks_for_models_input_or_folders_is_refused_and_the_call_goes_on() {
    let client = connect(&["--era", "legacy", "--ask"]).await;
    let mut events = client.events();
    let output = call(&client, "echo", json!({"text": "done"})).await.unwrap();
    let said = text_of(&output);
    assert!(said.contains("p1=ok"), "ping is answered: {said}");
    for refused in ["s1=refused", "e1=refused", "r1=refused"] {
        assert!(said.contains(refused), "{refused} in {said}");
    }
    let mut methods = Vec::new();
    while let Ok(ServerEvent::Refused { method }) = events.try_recv() {
        methods.push(method);
    }
    methods.sort();
    assert_eq!(methods, ["elicitation/create", "roots/list", "sampling/createMessage"], "each refusal is reported");
}

#[tokio::test]
async fn a_tool_that_needs_more_input_fails_with_an_explanation() {
    let client = connect(&["--era", "modern", "--input-required"]).await;
    let error = call(&client, "echo", json!({})).await.unwrap_err();
    match error {
        McpError::Refused(message) => {
            assert!(message.contains("elicitation") && message.contains("does not let servers ask"), "{message}")
        }
        other => panic!("{other:?}"),
    }
}

#[tokio::test]
async fn a_message_larger_than_the_limit_closes_the_connection() {
    let client = connect(&["--era", "modern"]).await;
    let error = call(&client, "huge", json!({})).await.unwrap_err();
    assert!(matches!(error, McpError::Closed | McpError::TooLarge { .. }), "{error:?}");
    tokio::time::sleep(Duration::from_millis(200)).await;
    assert!(!client.alive(), "the server was ended, not left running");
    assert!(matches!(call(&client, "echo", json!({})).await, Err(McpError::Closed)));
}

#[tokio::test]
async fn cancelling_tells_the_server_and_a_slow_call_times_out() {
    let client = connect(&["--era", "modern"]).await;
    let token = CancellationToken::new();
    let stopper = token.clone();
    tokio::spawn(async move {
        tokio::time::sleep(Duration::from_millis(150)).await;
        stopper.cancel();
    });
    let started = std::time::Instant::now();
    let error = client.call_tool("hang", json!({}), Duration::from_secs(30), &token).await.unwrap_err();
    assert_eq!(error, McpError::Cancelled);
    assert!(started.elapsed() < Duration::from_secs(5));
    let error =
        client.call_tool("hang", json!({}), Duration::from_millis(200), &CancellationToken::new()).await.unwrap_err();
    assert!(matches!(error, McpError::Timeout(_)) && error.is_retryable(), "{error:?}");
    // The server heard both cancellations.
    assert_eq!(text_of(&call(&client, "cancelled", json!({})).await.unwrap()), "2");
}

#[tokio::test]
async fn a_server_that_crashes_fails_the_call_and_stays_closed() {
    let client = connect(&["--era", "modern"]).await;
    let mut events = client.events();
    let error = call(&client, "crash", json!({})).await.unwrap_err();
    assert_eq!(error, McpError::Closed);
    assert!(!client.alive());
    let mut closed = false;
    for _ in 0..20 {
        if let Ok(ServerEvent::Closed(_)) = events.try_recv() {
            closed = true;
            break;
        }
        tokio::time::sleep(Duration::from_millis(20)).await;
    }
    assert!(closed, "the end of the connection is announced");
}

#[tokio::test]
async fn tools_with_bad_names_or_schemas_are_skipped_and_the_rest_still_work() {
    let client = connect(&["--era", "modern", "--invalid-tools"]).await;
    let listing = client.list_tools(&CancellationToken::new()).await.unwrap();
    assert!(listing.tools.iter().any(|t| t.name == "echo"));
    let skipped: Vec<&str> = listing.skipped.iter().map(|(n, _)| n.as_str()).collect();
    assert_eq!(skipped, ["bad name", "bad_schema"]);
    assert!(listing.skipped.iter().all(|(_, why)| !why.is_empty()));
}

#[tokio::test]
async fn lists_of_tools_are_followed_across_pages_and_bounded() {
    let client = connect(&["--era", "modern", "--many", "120"]).await;
    let listing = client.list_tools(&CancellationToken::new()).await.unwrap();
    assert_eq!(listing.tools.len(), 8 + 120, "three pages were followed");
    let client = connect(&["--era", "modern", "--many", "250"]).await;
    let error = client.list_tools(&CancellationToken::new()).await.unwrap_err();
    assert!(matches!(&error, McpError::Protocol(m) if m.contains("more than 200")), "{error:?}");
}

#[tokio::test]
async fn a_changed_description_changes_the_hash_which_is_what_pinning_watches() {
    let client = connect(&["--era", "modern", "--mutate"]).await;
    let first = client.list_tools(&CancellationToken::new()).await.unwrap();
    let second = client.list_tools(&CancellationToken::new()).await.unwrap();
    let hash = |l: &zorix_mcp::ToolListing, n: &str| l.tools.iter().find(|t| t.name == n).unwrap().hash();
    assert_ne!(hash(&first, "echo"), hash(&second, "echo"));
    assert_eq!(hash(&first, "env"), hash(&second, "env"), "only the tool that changed has a new hash");
}

#[tokio::test]
async fn the_server_gets_a_clean_environment_plus_what_it_was_given() {
    // SAFETY: tests in this file do not read this variable concurrently except through the child process, and nothing else sets it.
    unsafe { std::env::set_var("ZORIX_TEST_LEAK", "do-not-pass-on") };
    let client = connect_with(&["--era", "modern"], vec![("GITHUB_TOKEN".into(), "granted".into())]).await.unwrap();
    let keys = text_of(&call(&client, "env_keys", json!({})).await.unwrap());
    assert!(keys.contains("PATH") && keys.contains("GITHUB_TOKEN"), "{keys}");
    assert!(!keys.contains("ZORIX_TEST_LEAK"), "nothing else of ours is passed on: {keys}");
    assert_eq!(text_of(&call(&client, "env", json!({"name": "GITHUB_TOKEN"})).await.unwrap()), "granted");
}

#[tokio::test]
async fn what_the_server_writes_to_stderr_is_kept_bounded_and_scrubbed() {
    let client = connect(&["--era", "modern", "--noise"]).await;
    tokio::time::sleep(Duration::from_millis(500)).await;
    call(&client, "echo", json!({"text": "x"})).await.unwrap();
    let log = client.log();
    assert!(!log.is_empty());
    assert!(log.iter().all(|l| !l.contains("sk-ant-api03-abcdefghijklmnopqrstuvwxyz")), "secrets are removed");
    assert!(log.iter().map(String::len).sum::<usize>() <= 1_100_000, "the log is bounded");
    assert!(log.last().unwrap().contains("noise line"), "the newest lines are kept");
}

#[tokio::test]
async fn a_command_that_does_not_exist_says_so() {
    let error = StdioTransport::spawn(StdioConfig {
        command: "zorix-no-such-program".into(),
        args: vec![],
        env: vec![],
        cwd: None,
    })
    .err()
    .unwrap();
    assert!(matches!(&error, McpError::Spawn(m) if m.contains("was not found on the PATH")), "{error:?}");
}

#[tokio::test]
async fn a_server_that_announces_new_tools_is_heard() {
    let client = connect(&["--era", "modern", "--list-changed"]).await;
    let mut events = client.events();
    call(&client, "echo", json!({"text": "x"})).await.unwrap();
    let mut heard = false;
    for _ in 0..20 {
        if let Ok(ServerEvent::Notification { method, .. }) = events.try_recv() {
            heard |= method == "notifications/tools/list_changed";
        }
        if heard {
            break;
        }
        tokio::time::sleep(Duration::from_millis(20)).await;
    }
    assert!(heard);
    assert!(client.list_tools(&CancellationToken::new()).await.unwrap().tools.iter().any(|t| t.name == "extra"));
}

#[tokio::test]
async fn a_server_that_writes_the_id_of_its_answers_as_a_string_is_still_understood() {
    // JSON-RPC lets a server echo an id as a string; the answer to request 7 is still the answer to request 7.
    let client = connect(&["--era", "modern", "--string-ids"]).await;
    assert_eq!(text_of(&call(&client, "echo", json!({"text": "hello"})).await.unwrap()), "hello");
    assert!(!client.list_tools(&CancellationToken::new()).await.unwrap().tools.is_empty());
    client.close().await;
}

#[tokio::test]
async fn a_request_for_initialize_that_is_given_up_on_is_not_cancelled() {
    use zorix_mcp::Transport;
    // The server only starts reading after a delay, so the first request times out before it is read. A request that is given up on
    // is cancelled with a notification, except `initialize`, which the specification says must never be cancelled.
    let transport = StdioTransport::spawn(config(&["--era", "modern", "--delay", "700"])).unwrap();
    let gave_up = transport
        .request(
            "initialize",
            json!({"protocolVersion": "2025-11-25"}),
            Duration::from_millis(150),
            &CancellationToken::new(),
        )
        .await;
    assert!(matches!(gave_up, Err(McpError::Timeout(_))), "{gave_up:?}");
    let gave_up = transport
        .request(
            "tools/list",
            zorix_mcp::protocol::with_modern_meta(json!({})),
            Duration::from_millis(100),
            &CancellationToken::new(),
        )
        .await;
    assert!(matches!(gave_up, Err(McpError::Timeout(_))), "{gave_up:?}");
    tokio::time::sleep(Duration::from_millis(1000)).await;
    let counted = transport
        .request(
            "tools/call",
            zorix_mcp::protocol::with_modern_meta(json!({"name": "cancelled", "arguments": {}})),
            Duration::from_secs(5),
            &CancellationToken::new(),
        )
        .await
        .unwrap();
    // Exactly one cancellation reached the server: the one for tools/list, not the one for initialize.
    assert_eq!(counted["content"][0]["text"], "1", "{counted}");
    transport.close().await;
}
