//! The Streamable HTTP transport against a mock server of either era (ADR-0018).

use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::time::Duration;

use async_trait::async_trait;
use serde_json::{Value, json};
use tokio_util::sync::CancellationToken;
use wiremock::matchers::method;
use wiremock::{Mock, MockServer, Request, Respond, ResponseTemplate};
use zorix_core::Secret;
use zorix_mcp::{AuthProvider, Era, HttpTransport, McpClient, McpError, NoAuth, ServerEvent, StaticBearer, Transport};

type Handler = dyn Fn(&str, &Value, &Value, &Request) -> ResponseTemplate + Send + Sync;

struct Rpc(Arc<Handler>);

impl Respond for Rpc {
    fn respond(&self, request: &Request) -> ResponseTemplate {
        let body: Value = serde_json::from_slice(&request.body).unwrap_or(Value::Null);
        let method = body["method"].as_str().unwrap_or("");
        (self.0)(method, &body["params"], &body["id"], request)
    }
}

fn answer(id: &Value, result: Value) -> ResponseTemplate {
    ResponseTemplate::new(200).set_body_json(json!({"jsonrpc": "2.0", "id": id, "result": result}))
}

fn error(status: u16, id: &Value, code: i64, message: &str, data: Value) -> ResponseTemplate {
    ResponseTemplate::new(status)
        .set_body_json(json!({"jsonrpc": "2.0", "id": id, "error": {"code": code, "message": message, "data": data}}))
}

async fn serve(
    handler: impl Fn(&str, &Value, &Value, &Request) -> ResponseTemplate + Send + Sync + 'static,
) -> MockServer {
    let server = MockServer::start().await;
    Mock::given(method("POST")).respond_with(Rpc(Arc::new(handler))).mount(&server).await;
    server
}

fn header<'a>(request: &'a Request, name: &str) -> Option<&'a str> {
    request.headers.get(name).and_then(|v| v.to_str().ok())
}

fn transport(server: &MockServer, auth: Arc<dyn AuthProvider>) -> Arc<HttpTransport> {
    Arc::new(HttpTransport::new(&format!("{}/mcp", server.uri()), auth).unwrap())
}

async fn connect(t: Arc<HttpTransport>) -> Result<McpClient, McpError> {
    McpClient::connect_with(t, &CancellationToken::new(), Duration::from_secs(2), Duration::from_secs(5)).await
}

fn modern_server() -> impl Fn(&str, &Value, &Value, &Request) -> ResponseTemplate + Send + Sync + 'static {
    |m, params, id, _| match m {
        "server/discover" => answer(
            id,
            json!({"resultType": "complete", "supportedVersions": ["2026-07-28"], "capabilities": {"tools": {}}, "_meta": {"io.modelcontextprotocol/serverInfo": {"name": "remote", "version": "2"}}}),
        ),
        "tools/list" => answer(
            id,
            json!({"resultType": "complete", "tools": [
                {"name": "echo", "inputSchema": {"type": "object"}},
                {"name": "query", "inputSchema": {"type": "object", "properties": {"region": {"type": "string", "x-mcp-header": "Region"}, "q": {"type": "string"}}}},
                {"name": "bad", "inputSchema": {"type": "object", "properties": {"n": {"type": "number", "x-mcp-header": "N"}}}}
            ]}),
        ),
        "tools/call" => answer(
            id,
            json!({"resultType": "complete", "content": [{"type": "text", "text": format!("called {}", params["name"].as_str().unwrap_or(""))}], "isError": false}),
        ),
        _ => error(404, id, -32601, "Method not found", Value::Null),
    }
}

#[tokio::test]
async fn a_modern_server_gets_the_metadata_headers_and_the_token_only_in_the_authorization_header() {
    let server = serve(modern_server()).await;
    let client = connect(transport(&server, Arc::new(StaticBearer(Secret::new("tok-abc"))))).await.unwrap();
    assert_eq!(client.era(), &Era::Modern);
    assert_eq!(client.info().name.as_deref(), Some("remote"));
    let listing = client.list_tools(&CancellationToken::new()).await.unwrap();
    assert_eq!(listing.skipped.len(), 1, "a tool with a bad x-mcp-header is left out: {:?}", listing.skipped);
    let out = client
        .call_tool("query", json!({"region": "us-west1", "q": "x"}), Duration::from_secs(5), &CancellationToken::new())
        .await
        .unwrap();
    assert_eq!(out.content[0]["text"], "called query");

    let requests = server.received_requests().await.unwrap();
    let call = requests.iter().find(|r| String::from_utf8_lossy(&r.body).contains("tools/call")).unwrap();
    assert_eq!(header(call, "mcp-protocol-version"), Some("2026-07-28"));
    assert_eq!(header(call, "mcp-method"), Some("tools/call"));
    assert_eq!(header(call, "mcp-name"), Some("query"));
    assert_eq!(header(call, "mcp-param-region"), Some("us-west1"), "the parameter the server asked for is mirrored");
    assert!(
        header(call, "accept").unwrap().contains("application/json")
            && header(call, "accept").unwrap().contains("text/event-stream")
    );
    for request in &requests {
        assert_eq!(header(request, "authorization"), Some("Bearer tok-abc"), "every request is authorised");
        assert!(!request.url.as_str().contains("tok-abc"), "never in the address");
        assert!(!String::from_utf8_lossy(&request.body).contains("tok-abc"), "never in the body");
    }
}

#[tokio::test]
async fn an_answer_that_arrives_as_a_stream_delivers_its_notifications_first() {
    let server = serve(|m, _, id, _| match m {
        "server/discover" => answer(id, json!({"supportedVersions": ["2026-07-28"], "capabilities": {}})),
        "tools/call" => {
            let progress = json!({"jsonrpc": "2.0", "method": "notifications/progress", "params": {"progress": 1}});
            let done = json!({"jsonrpc": "2.0", "id": id, "result": {"resultType": "complete", "content": [{"type": "text", "text": "streamed"}]}});
            ResponseTemplate::new(200).set_body_raw(format!(": keep-alive\n\ndata: {progress}\n\ndata: {done}\n\n"), "text/event-stream")
        }
        _ => error(404, id, -32601, "no", Value::Null),
    })
    .await;
    let client = connect(transport(&server, Arc::new(NoAuth))).await.unwrap();
    let mut events = client.events();
    let out = client.call_tool("t", json!({}), Duration::from_secs(5), &CancellationToken::new()).await.unwrap();
    assert_eq!(out.content[0]["text"], "streamed");
    tokio::time::sleep(Duration::from_millis(50)).await;
    let mut heard = false;
    while let Ok(event) = events.try_recv() {
        heard |= matches!(event, ServerEvent::Notification { method, .. } if method == "notifications/progress");
    }
    assert!(heard, "the progress notification was passed on");
}

fn legacy_server(
    calls: Arc<AtomicUsize>,
) -> impl Fn(&str, &Value, &Value, &Request) -> ResponseTemplate + Send + Sync + 'static {
    move |m, _, id, request| {
        match m {
        // A legacy server does not know the probe and says so without a JSON-RPC body.
        "server/discover" => ResponseTemplate::new(400),
        "initialize" => answer(id, json!({"protocolVersion": "2025-11-25", "capabilities": {"tools": {}}, "serverInfo": {"name": "old", "version": "1"}})).insert_header("Mcp-Session-Id", "sess-42"),
        "" => ResponseTemplate::new(202),
        "notifications/initialized" => ResponseTemplate::new(202),
        "tools/list" => {
            calls.fetch_add(1, Ordering::SeqCst);
            assert_eq!(header(request, "mcp-session-id"), Some("sess-42"), "the session id is sent back");
            assert_eq!(header(request, "mcp-protocol-version"), Some("2025-11-25"));
            answer(id, json!({"tools": [{"name": "echo", "inputSchema": {"type": "object"}}]}))
        }
        _ => error(404, id, -32601, "no", Value::Null),
    }
    }
}

#[tokio::test]
async fn a_legacy_server_is_found_by_the_probe_and_its_session_is_kept_and_ended() {
    let calls = Arc::new(AtomicUsize::new(0));
    let server = serve(legacy_server(calls.clone())).await;
    let deletes = Arc::new(AtomicUsize::new(0));
    let counted = deletes.clone();
    Mock::given(method("DELETE"))
        .respond_with(move |request: &Request| {
            assert_eq!(header(request, "mcp-session-id"), Some("sess-42"));
            counted.fetch_add(1, Ordering::SeqCst);
            ResponseTemplate::new(200)
        })
        .mount(&server)
        .await;
    let client = connect(transport(&server, Arc::new(NoAuth))).await.unwrap();
    assert_eq!(client.era(), &Era::Legacy { version: "2025-11-25".into() });
    assert_eq!(client.list_tools(&CancellationToken::new()).await.unwrap().tools.len(), 1);
    assert_eq!(calls.load(Ordering::SeqCst), 1);
    client.close().await;
    assert_eq!(deletes.load(Ordering::SeqCst), 1, "the session is ended politely");
    assert!(!client.alive());
}

#[tokio::test]
async fn a_modern_error_that_lists_older_versions_leads_to_a_legacy_handshake() {
    let server = serve(|m, _, id, _| match m {
        "server/discover" => error(
            400,
            id,
            -32022,
            "Unsupported protocol version",
            json!({"supported": ["2025-06-18"], "requested": "2026-07-28"}),
        ),
        "initialize" => answer(id, json!({"protocolVersion": "2025-06-18", "capabilities": {}})),
        _ => ResponseTemplate::new(202),
    })
    .await;
    let client = connect(transport(&server, Arc::new(NoAuth))).await.unwrap();
    assert_eq!(client.era(), &Era::Legacy { version: "2025-06-18".into() });
}

#[tokio::test]
async fn a_legacy_server_that_asks_things_in_its_stream_is_refused_and_pinged_back() {
    let answered = Arc::new(std::sync::Mutex::new(Vec::<Value>::new()));
    let seen = answered.clone();
    let server = serve(move |m, _, id, request| {
        let body: Value = serde_json::from_slice(&request.body).unwrap_or(Value::Null);
        if m.is_empty() && body.get("result").is_some() || body.get("error").is_some() {
            seen.lock().unwrap().push(body);
            return ResponseTemplate::new(202);
        }
        match m {
            "server/discover" => ResponseTemplate::new(404),
            "initialize" => answer(id, json!({"protocolVersion": "2025-11-25", "capabilities": {}})),
            "notifications/initialized" => ResponseTemplate::new(202),
            "tools/call" => {
                let ping = json!({"jsonrpc": "2.0", "id": "p", "method": "ping"});
                let sampling = json!({"jsonrpc": "2.0", "id": "s", "method": "sampling/createMessage", "params": {}});
                let done = json!({"jsonrpc": "2.0", "id": id, "result": {"content": [{"type": "text", "text": "ok"}]}});
                ResponseTemplate::new(200)
                    .set_body_raw(format!("data: {ping}\n\ndata: {sampling}\n\ndata: {done}\n\n"), "text/event-stream")
            }
            _ => ResponseTemplate::new(404),
        }
    })
    .await;
    let client = connect(transport(&server, Arc::new(NoAuth))).await.unwrap();
    let mut events = client.events();
    client.call_tool("t", json!({}), Duration::from_secs(5), &CancellationToken::new()).await.unwrap();
    let replies = answered.lock().unwrap().clone();
    assert!(replies.iter().any(|r| r["id"] == "p" && r.get("result").is_some()), "ping is answered: {replies:?}");
    assert!(replies.iter().any(|r| r["id"] == "s" && r["error"]["code"] == -32601), "sampling is refused: {replies:?}");
    let mut refused = false;
    while let Ok(event) = events.try_recv() {
        refused |= matches!(event, ServerEvent::Refused { method } if method == "sampling/createMessage");
    }
    assert!(refused);
}

struct Rotating {
    token: std::sync::Mutex<String>,
    refreshes: AtomicUsize,
    works: bool,
}

#[async_trait]
impl AuthProvider for Rotating {
    async fn authorization(&self) -> Result<Option<String>, McpError> {
        Ok(Some(format!("Bearer {}", self.token.lock().unwrap())))
    }

    async fn challenged(&self, status: u16, www: Option<&str>) -> Result<(), McpError> {
        assert_eq!(status, 401);
        assert!(www.unwrap().contains("resource_metadata"));
        self.refreshes.fetch_add(1, Ordering::SeqCst);
        if self.works {
            *self.token.lock().unwrap() = "fresh".into();
            Ok(())
        } else {
            Err(McpError::NeedsLogin("the sign-in expired".into()))
        }
    }
}

#[tokio::test]
async fn a_401_is_retried_once_with_a_refreshed_token_and_a_failed_refresh_asks_for_a_sign_in() {
    let handler = |m: &str, _: &Value, id: &Value, request: &Request| {
        if header(request, "authorization") != Some("Bearer fresh") {
            return ResponseTemplate::new(401).insert_header(
                "WWW-Authenticate",
                "Bearer resource_metadata=\"https://x/.well-known/oauth-protected-resource\", scope=\"read\"",
            );
        }
        match m {
            "server/discover" => answer(id, json!({"supportedVersions": ["2026-07-28"], "capabilities": {}})),
            _ => error(404, id, -32601, "no", Value::Null),
        }
    };
    let server = serve(handler).await;
    let good = Arc::new(Rotating {
        token: std::sync::Mutex::new("stale".into()),
        refreshes: AtomicUsize::new(0),
        works: true,
    });
    let client = connect(transport(&server, good.clone())).await.unwrap();
    assert_eq!(client.era(), &Era::Modern);
    assert_eq!(good.refreshes.load(Ordering::SeqCst), 1);

    let bad = Arc::new(Rotating {
        token: std::sync::Mutex::new("stale".into()),
        refreshes: AtomicUsize::new(0),
        works: false,
    });
    let error = connect(transport(&server, bad)).await.err().unwrap();
    assert!(matches!(error, McpError::NeedsLogin(ref why) if why.contains("expired")), "{error:?}");
}

#[tokio::test]
async fn a_redirect_is_never_followed_so_a_token_cannot_be_bounced_to_another_host() {
    let elsewhere = MockServer::start().await;
    Mock::given(method("POST"))
        .respond_with(
            ResponseTemplate::new(200)
                .set_body_json(json!({"jsonrpc": "2.0", "id": 1, "result": {"supportedVersions": ["2026-07-28"]}})),
        )
        .mount(&elsewhere)
        .await;
    let target = format!("{}/steal", elsewhere.uri());
    let server = serve(move |_, _, _, _| ResponseTemplate::new(307).insert_header("Location", target.as_str())).await;
    let t = transport(&server, Arc::new(StaticBearer(Secret::new("tok-secret"))));
    let result = t
        .request(
            "server/discover",
            json!({"_meta": {"io.modelcontextprotocol/protocolVersion": "2026-07-28"}}),
            Duration::from_secs(5),
            &CancellationToken::new(),
        )
        .await;
    assert!(matches!(result, Err(McpError::Transport(ref m)) if m.contains("307")), "{result:?}");
    assert!(elsewhere.received_requests().await.unwrap().is_empty(), "nothing reached the other host");
}

#[tokio::test]
async fn a_body_over_the_limit_is_refused_and_a_slow_server_can_be_cancelled_or_timed_out() {
    let server = serve(|m, _, id, _| match m {
        "tools/call" => answer(id, json!({"content": [{"type": "text", "text": "x".repeat(5 * 1024 * 1024)}]})),
        "slow" => ResponseTemplate::new(200).set_delay(Duration::from_secs(5)),
        _ => error(404, id, -32601, "no", Value::Null),
    })
    .await;
    let t = transport(&server, Arc::new(NoAuth));
    let big = t.request("tools/call", json!({}), Duration::from_secs(10), &CancellationToken::new()).await;
    assert!(matches!(big, Err(McpError::TooLarge { .. })), "{big:?}");

    let token = CancellationToken::new();
    let stopper = token.clone();
    tokio::spawn(async move {
        tokio::time::sleep(Duration::from_millis(100)).await;
        stopper.cancel();
    });
    let started = std::time::Instant::now();
    assert_eq!(t.request("slow", json!({}), Duration::from_secs(30), &token).await, Err(McpError::Cancelled));
    assert!(started.elapsed() < Duration::from_secs(3));
    let timed = t.request("slow", json!({}), Duration::from_millis(200), &CancellationToken::new()).await;
    assert!(matches!(timed, Err(McpError::Timeout(_))), "{timed:?}");
}

#[test]
fn only_https_or_this_computer_is_accepted_and_never_with_credentials_in_the_address() {
    for ok in ["https://mcp.example.com/mcp", "http://127.0.0.1:8080/mcp", "http://localhost/mcp", "http://[::1]:9/mcp"]
    {
        assert!(HttpTransport::new(ok, Arc::new(NoAuth)).is_ok(), "{ok}");
    }
    for bad in [
        "http://mcp.example.com/mcp",
        "ftp://x/y",
        "https://user:pw@mcp.example.com/",
        "https://x/mcp#frag",
        "not a url",
    ] {
        assert!(HttpTransport::new(bad, Arc::new(NoAuth)).is_err(), "{bad}");
    }
}
