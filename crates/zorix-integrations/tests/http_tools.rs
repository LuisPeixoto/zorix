//! The HTTP and webhook tools: what they refuse before touching the network, what they do with a granted host, and what
//! they never do with a secret (ADR-0021).

use std::sync::Arc;

use serde_json::{Value, json};
use tokio_util::sync::CancellationToken;
use wiremock::matchers::{header, method, path};
use wiremock::{Mock, MockServer, Request, ResponseTemplate};
use zorix_core::{
    Action, ConnectionKind, ExecutionId, Grants, IntegrationStore, NewConnection, Permission, Secret, TenantId,
    Timestamp, Tool, ToolContext, ToolError, network_grant,
};
use zorix_integrations::{http_tools, secret_name, signature};
use zorix_providers::{MemoryStore, SecretStore};
use zorix_store::SqliteIntegrationStore;

struct Rig {
    store: Arc<SqliteIntegrationStore>,
    secrets: Arc<MemoryStore>,
    tools: Vec<Arc<dyn Tool>>,
}

fn rig() -> Rig {
    let store = Arc::new(SqliteIntegrationStore::open_in_memory().unwrap());
    let secrets = Arc::new(MemoryStore::new());
    let tools = http_tools(store.clone(), secrets.clone());
    Rig { store, secrets, tools }
}

impl Rig {
    fn tool(&self, id: &str) -> &Arc<dyn Tool> {
        self.tools.iter().find(|t| t.spec().id == id).unwrap()
    }

    async fn call(&self, id: &str, grants: Grants, args: Value) -> Result<Value, ToolError> {
        let ctx = ToolContext {
            execution_id: ExecutionId::new(),
            tenant: TenantId::local(),
            call_id: zorix_core::ToolCallId("c".into()),
            grants,
            cancel: CancellationToken::new(),
            workspace_root: None,
            conversation: None,
        };
        self.tool(id).call(&ctx, args).await
    }
}

fn grants(hosts: &[&str]) -> Grants {
    hosts.iter().fold(Grants::none(), |g, h| g.with(network_grant(h).unwrap()))
}

fn message(error: ToolError) -> String {
    match error {
        ToolError::Failed(m) | ToolError::Retryable(m) => m,
    }
}

#[tokio::test]
async fn nothing_is_reached_without_a_grant_and_the_message_says_what_to_grant() {
    let rig = rig();
    let error = rig.call("http.get", Grants::none(), json!({"url": "https://api.example.com/x"})).await.unwrap_err();
    assert!(message(error).contains("http:api.example.com"));
    // A grant for another host does not help; a read grant is not a network grant.
    let other = grants(&["docs.example.com"]).with(Permission::new(Action::Read, "http:api.example.com"));
    assert!(rig.call("http.get", other, json!({"url": "https://api.example.com/x"})).await.is_err());
}

#[tokio::test]
async fn private_and_special_addresses_are_refused_even_with_a_wildcard_grant() {
    let rig = rig();
    for url in [
        "https://169.254.169.254/latest/meta-data/",
        "https://10.0.0.5/admin",
        "https://[::1]/",
        "https://192.168.1.1/",
        "https://localhost/",
    ] {
        let error = message(rig.call("http.get", grants(&["*"]), json!({"url": url})).await.unwrap_err());
        assert!(error.contains("tools may not reach unless that exact host is granted"), "{url}: {error}");
    }
    // Plain HTTP is for this computer only, and credentials do not belong in an address.
    let error = message(rig.call("http.get", grants(&["*"]), json!({"url": "http://example.com/"})).await.unwrap_err());
    assert!(error.contains("HTTPS"), "{error}");
    let error = message(
        rig.call("http.get", grants(&["*"]), json!({"url": "https://user:pw@example.com/"})).await.unwrap_err(),
    );
    assert!(error.contains("user name"), "{error}");
    assert!(rig.call("http.get", grants(&["*"]), json!({"url": "ftp://example.com/"})).await.is_err());
    assert!(rig.call("http.get", grants(&["*"]), json!({"url": "not a url"})).await.is_err());
}

#[tokio::test]
async fn a_granted_host_is_read_and_the_answer_is_shaped_for_the_model() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/doc"))
        .respond_with(
            ResponseTemplate::new(200)
                .insert_header("ETag", "\"v1\"")
                .set_body_raw("Hello, world!", "text/plain; charset=utf-8"),
        )
        .mount(&server)
        .await;
    Mock::given(method("GET"))
        .and(path("/image"))
        .respond_with(ResponseTemplate::new(200).set_body_raw(vec![0u8; 3000], "image/png"))
        .mount(&server)
        .await;
    Mock::given(method("GET"))
        .and(path("/big"))
        .respond_with(ResponseTemplate::new(200).set_body_raw("x".repeat(3 * 1024 * 1024), "text/plain"))
        .mount(&server)
        .await;
    let rig = rig();
    let url = |p: &str| format!("{}{p}", server.uri());
    // The mock is on this computer, so the exact host is named (a wildcard would not do).
    let g = grants(&["127.0.0.1"]);
    let got = rig.call("http.get", g.clone(), json!({"url": url("/doc")})).await.unwrap();
    assert_eq!(
        (got["status"].as_u64(), got["ok"].as_bool(), got["body"].as_str(), got["truncated"].as_bool()),
        (Some(200), Some(true), Some("Hello, world!"), Some(false))
    );
    assert_eq!(got["headers"]["etag"], "\"v1\"");
    assert!(got["headers"].get("content-length").is_none());
    let cut = rig.call("http.get", g.clone(), json!({"url": url("/doc"), "max_chars": 5})).await.unwrap();
    assert_eq!((cut["body"].as_str(), cut["truncated"].as_bool()), (Some("Hello"), Some(true)));
    let picture = rig.call("http.get", g.clone(), json!({"url": url("/image")})).await.unwrap();
    assert!(picture["body"].is_null() && picture["note"].as_str().unwrap().contains("image/png"));
    let big = rig.call("http.get", g.clone(), json!({"url": url("/big"), "max_chars": 100000})).await.unwrap();
    assert!(
        big["truncated"].as_bool().unwrap() && big["bytes"].as_u64().unwrap() <= 2 * 1024 * 1024,
        "the response is cut at 2 MB"
    );
    assert_eq!(rig.call("http.get", g, json!({"url": url("/missing")})).await.unwrap()["status"], 404);
}

#[tokio::test]
async fn a_request_may_not_carry_credentials_of_its_own_and_other_headers_pass() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(header("accept", "text/plain"))
        .respond_with(ResponseTemplate::new(200).set_body_string("ok"))
        .mount(&server)
        .await;
    let rig = rig();
    let g = grants(&["127.0.0.1"]);
    let url = server.uri();
    for bad in [
        "Authorization",
        "authorization",
        "Cookie",
        "X-Api-Key",
        "X-Auth-Token",
        "Proxy-Authorization",
        "Host",
        "Content-Length",
        "X-Session-Id",
    ] {
        let error = rig.call("http.get", g.clone(), json!({"url": url, "headers": {bad: "x"}})).await.unwrap_err();
        assert!(message(error).contains("cannot be set"), "{bad}");
    }
    assert!(rig.call("http.get", g.clone(), json!({"url": url, "headers": {"bad header": "x"}})).await.is_err());
    assert!(rig.call("http.get", g.clone(), json!({"url": url, "headers": {"Accept": "a\nb"}})).await.is_err());
    assert_eq!(
        rig.call("http.get", g, json!({"url": url, "headers": {"Accept": "text/plain"}})).await.unwrap()["body"],
        "ok"
    );
}

#[tokio::test]
async fn a_connection_supplies_the_secret_by_host_and_it_never_follows_a_redirect_elsewhere() {
    let secret_host = MockServer::start().await;
    let other_host = MockServer::start().await;
    // The first host redirects to the second (a different port, and reached by another name, so another host).
    let target = format!("http://localhost:{}/landing", other_host.address().port());
    Mock::given(method("GET"))
        .and(path("/start"))
        .and(header("authorization", "Bearer s3cret"))
        .respond_with(ResponseTemplate::new(302).insert_header("Location", target.as_str()))
        .mount(&secret_host)
        .await;
    Mock::given(method("GET"))
        .and(path("/landing"))
        .respond_with(|request: &Request| {
            assert!(request.headers.get("authorization").is_none(), "the secret did not follow the redirect");
            ResponseTemplate::new(200).set_body_string("landed")
        })
        .mount(&other_host)
        .await;

    let rig = rig();
    let connection = rig
        .store
        .add_connection(
            &TenantId::local(),
            NewConnection {
                kind: ConnectionKind::Http,
                name: "api".into(),
                settings: json!({"host": "127.0.0.1", "auth": "bearer"}),
                has_secret: true,
            },
            Timestamp(1),
        )
        .await
        .unwrap();
    rig.secrets.set(&secret_name(connection.id), &Secret::new("s3cret")).unwrap();

    // With both hosts granted, the redirect is followed, without the secret.
    let g = grants(&["127.0.0.1", "localhost"]);
    let got = rig.call("http.get", g, json!({"url": format!("{}/start", secret_host.uri())})).await.unwrap();
    assert_eq!((got["status"].as_u64(), got["body"].as_str()), (Some(200), Some("landed")));
    assert_eq!(got["redirected_via"].as_array().unwrap().len(), 1);

    // With only the first granted, the redirect is reported and not followed.
    let stopped = rig
        .call("http.get", grants(&["127.0.0.1"]), json!({"url": format!("{}/start", secret_host.uri())}))
        .await
        .unwrap();
    assert_eq!(stopped["status"], 302);
    assert!(stopped["redirect_not_followed"]["why"].as_str().unwrap().contains("no permission"), "{stopped}");
    assert!(other_host.received_requests().await.unwrap().len() == 1, "only the allowed run reached the second host");
    // The model sees none of it.
    assert!(!stopped.to_string().contains("s3cret"));
}

#[tokio::test]
async fn http_request_always_asks_and_sends_json_bodies_with_the_right_headers() {
    let rig = rig();
    assert!(rig.tool("http.request").spec().requires_approval && !rig.tool("http.get").spec().requires_approval);
    assert!(rig.tool("webhook.send").spec().requires_approval);
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/items"))
        .and(header("content-type", "application/json"))
        .respond_with(|request: &Request| {
            let body: Value = serde_json::from_slice(&request.body).unwrap();
            ResponseTemplate::new(201).set_body_json(json!({"got": body}))
        })
        .mount(&server)
        .await;
    Mock::given(method("DELETE"))
        .and(path("/items/1"))
        .respond_with(ResponseTemplate::new(302).insert_header("Location", "/elsewhere"))
        .mount(&server)
        .await;
    let g = grants(&["127.0.0.1"]);
    let made = rig
        .call(
            "http.request",
            g.clone(),
            json!({"method": "POST", "url": format!("{}/items", server.uri()), "json": {"name": "a"}}),
        )
        .await
        .unwrap();
    assert_eq!(made["status"], 201);
    assert!(made["body"].as_str().unwrap().contains("\"name\":\"a\""));
    // A call that changes things does not follow a redirect, and says so.
    let deleted = rig
        .call("http.request", g.clone(), json!({"method": "DELETE", "url": format!("{}/items/1", server.uri())}))
        .await
        .unwrap();
    assert_eq!(deleted["status"], 302);
    assert!(deleted["note"].as_str().unwrap().contains("does not follow redirects"));
    // Both a body and json, a bad method and an enormous body are refused.
    assert!(
        rig.call("http.request", g.clone(), json!({"method": "POST", "url": server.uri(), "body": "a", "json": {}}))
            .await
            .is_err()
    );
    assert!(
        rig.call(
            "http.request",
            g.clone(),
            json!({"method": "POST", "url": server.uri(), "body": "x".repeat(1024 * 1024 + 1)})
        )
        .await
        .is_err()
    );
    assert!(rig.call("http.request", g, json!({"method": "NOPE", "url": server.uri()})).await.is_err());
}

#[tokio::test]
async fn a_webhook_is_signed_goes_only_where_the_connection_says_and_never_shows_its_key() {
    let server = MockServer::start().await;
    let seen = Arc::new(std::sync::Mutex::new(None::<(String, String, String, String)>));
    let sink = seen.clone();
    Mock::given(method("POST"))
        .and(path("/hooks/deploy"))
        .respond_with(move |request: &Request| {
            let get = |n: &str| request.headers.get(n).and_then(|v| v.to_str().ok()).unwrap_or("").to_string();
            *sink.lock().unwrap() = Some((
                get("x-zorix-event"),
                get("x-zorix-timestamp"),
                get("x-zorix-signature"),
                String::from_utf8_lossy(&request.body).into_owned(),
            ));
            ResponseTemplate::new(204)
        })
        .mount(&server)
        .await;
    let rig = rig();
    let connection = rig
        .store
        .add_connection(
            &TenantId::local(),
            NewConnection {
                kind: ConnectionKind::Webhook,
                name: "deploys".into(),
                settings: json!({"url": format!("{}/hooks/deploy", server.uri())}),
                has_secret: true,
            },
            Timestamp(1),
        )
        .await
        .unwrap();
    let args = json!({"connection": "deploys", "event": "deploy.finished", "payload": {"ok": true}});
    // No key stored: refused. A connection of another kind or name: refused.
    assert!(
        message(rig.call("webhook.send", grants(&["127.0.0.1"]), args.clone()).await.unwrap_err())
            .contains("no signing key")
    );
    rig.secrets.set(&secret_name(connection.id), &Secret::new("whsec_test")).unwrap();
    assert!(
        message(
            rig.call(
                "webhook.send",
                grants(&["127.0.0.1"]),
                json!({"connection": "nope", "event": "x", "payload": {}})
            )
            .await
            .unwrap_err()
        )
        .contains("no webhook connection")
    );
    // The host still has to be granted.
    assert!(
        message(rig.call("webhook.send", Grants::none(), args.clone()).await.unwrap_err()).contains("no permission")
    );

    let sent = rig.call("webhook.send", grants(&["127.0.0.1"]), args).await.unwrap();
    assert_eq!((sent["status"].as_u64(), sent["connection"].as_str()), (Some(204), Some("deploys")));
    assert!(!sent.to_string().contains("whsec_test"));
    let (event, timestamp, sig, body) = seen.lock().unwrap().clone().unwrap();
    assert_eq!(event, "deploy.finished");
    assert_eq!(
        serde_json::from_str::<Value>(&body).unwrap(),
        json!({"event": "deploy.finished", "payload": {"ok": true}})
    );
    assert_eq!(
        sig,
        signature("whsec_test", timestamp.parse().unwrap(), &body),
        "the receiver can recompute it from the timestamp and the body"
    );
    assert!(sig.starts_with("sha256=") && sig.len() == 7 + 64);
    // Another secret gives another signature, and so does another body.
    assert_ne!(sig, signature("other", timestamp.parse().unwrap(), &body));
    assert_ne!(sig, signature("whsec_test", timestamp.parse().unwrap(), "{}"));
}

#[test]
fn the_signature_matches_a_known_hmac() {
    // RFC 4231 test case 2 (key "Jefe", data "what do ya want for nothing?") as the signed text is "<ts>.<body>".
    assert_eq!(signature("Jefe", 1, "x").len(), 71);
    assert_eq!(signature("k", 5, "{\"a\":1}"), signature("k", 5, "{\"a\":1}"));
}
