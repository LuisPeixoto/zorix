//! The GitHub and GitLab tools against mock APIs: what is read, what asks first, what a grant does and does not cover, and
//! that a token never shows up in anything the model sees (ADR-0021).

use std::sync::Arc;

use base64::Engine;
use serde_json::{Value, json};
use tokio_util::sync::CancellationToken;
use wiremock::matchers::{body_json, header, method, path, query_param};
use wiremock::{Mock, MockServer, ResponseTemplate};
use zorix_core::{
    Action, ConnectionKind, ExecutionId, Grants, IntegrationStore, NewConnection, Permission, Secret, TenantId,
    Timestamp, Tool, ToolContext, ToolError,
};
use zorix_integrations::{Services, forge_tools, secret_name};
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
    let tools =
        forge_tools(Arc::new(Services { store: store.clone(), secrets: secrets.clone(), signins: Default::default() }));
    Rig { store, secrets, tools }
}

impl Rig {
    async fn connect(&self, kind: ConnectionKind, name: &str, base: &str, token: Option<&str>) {
        let connection = self
            .store
            .add_connection(
                &TenantId::local(),
                NewConnection {
                    kind,
                    name: name.into(),
                    settings: json!({"base_url": base}),
                    has_secret: token.is_some(),
                },
                Timestamp(1),
            )
            .await
            .unwrap();
        if let Some(token) = token {
            self.secrets.set(&secret_name(connection.id), &Secret::new(token)).unwrap();
        }
    }

    fn tool(&self, id: &str) -> &Arc<dyn Tool> {
        self.tools.iter().find(|t| t.spec().id == id).unwrap_or_else(|| panic!("no tool {id}"))
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
            agent: None,
            host: None,
        };
        self.tool(id).call(&ctx, args).await
    }
}

fn read(prefix: &str, repo: &str) -> Grants {
    Grants::none().with(Permission::new(Action::Read, format!("{prefix}:{repo}")))
}

fn write(prefix: &str, repo: &str) -> Grants {
    Grants::none().with(Permission::new(Action::Write, format!("{prefix}:{repo}")))
}

fn message(error: ToolError) -> String {
    match error {
        ToolError::Failed(m) | ToolError::Retryable(m) => m,
    }
}

fn issue_json() -> Value {
    json!({"number": 7, "title": "Crash on start", "state": "open", "user": {"login": "ana"}, "labels": [{"name": "bug"}, {"name": "p1"}], "assignees": [{"login": "bo"}],
           "comments": 2, "created_at": "2026-11-01T00:00:00Z", "updated_at": "2026-11-02T00:00:00Z", "html_url": "https://github.com/o/r/issues/7", "body": "It crashes. ".repeat(1000)})
}

#[tokio::test]
async fn the_tools_say_which_ones_change_things_and_those_always_ask() {
    let rig = rig();
    assert_eq!(rig.tools.len(), 20, "ten operations on each of two forges");
    for tool in &rig.tools {
        let spec = tool.spec();
        let writes = ["create_issue", "comment", "update_issue"].iter().any(|w| spec.id.ends_with(w));
        assert_eq!(spec.requires_approval, writes, "{}", spec.id);
        assert_eq!(spec.annotations.read_only, !writes, "{}", spec.id);
        assert!(spec.id.starts_with("github.") || spec.id.starts_with("gitlab."));
    }
}

#[tokio::test]
async fn reading_an_issue_needs_a_read_grant_for_that_repository_and_returns_a_short_clean_record() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/repos/o/r/issues/7"))
        .and(header("authorization", "Bearer ghp_token"))
        .and(header("x-github-api-version", "2022-11-28"))
        .respond_with(ResponseTemplate::new(200).set_body_json(issue_json()))
        .mount(&server)
        .await;
    let rig = rig();
    rig.connect(ConnectionKind::Github, "work", &server.uri(), Some("ghp_token")).await;
    let args = json!({"repo": "o/r", "number": 7});
    // Another repository, a write grant, or nothing: refused before anything is sent.
    for grants in [Grants::none(), read("github", "o/other"), write("github", "o/r")] {
        let error = message(rig.call("github.get_issue", grants, args.clone()).await.unwrap_err());
        assert!(error.contains("no permission to read github:o/r") && error.contains("--allow-repo o/r"), "{error}");
    }
    assert!(server.received_requests().await.unwrap().is_empty());
    let got = rig.call("github.get_issue", read("github", "o/r"), args.clone()).await.unwrap();
    assert_eq!(
        (got["number"].as_u64(), got["title"].as_str(), got["author"].as_str()),
        (Some(7), Some("Crash on start"), Some("ana"))
    );
    assert_eq!((got["labels"].clone(), got["assignees"].clone()), (json!(["bug", "p1"]), json!(["bo"])));
    assert!(got["body"].as_str().unwrap().contains("[cut: "), "a long body is cut");
    assert!(!got.to_string().contains("ghp_token"), "the token is nowhere in the result");
    // A pattern grant covers every repository under it.
    assert!(rig.call("github.get_issue", read("github", "o/*"), args).await.is_ok());
    // Names that could wander are refused.
    for bad in ["o", "o/r/x", "../r", "o/..", "o r/x", ""] {
        assert!(
            rig.call("github.get_issue", read("github", "*"), json!({"repo": bad, "number": 1})).await.is_err(),
            "{bad:?}"
        );
    }
}

#[tokio::test]
async fn a_search_is_scoped_to_the_granted_repository_and_cannot_widen_itself() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/search/issues"))
        .and(query_param("per_page", "20"))
        .respond_with(|request: &wiremock::Request| {
            let q = request.url.query_pairs().find(|(k, _)| k == "q").map(|(_, v)| v.into_owned()).unwrap_or_default();
            assert!(q.contains("repo:o/r") && q.contains("crash") && q.contains("state:open"), "{q}");
            ResponseTemplate::new(200)
                .insert_header("Link", "<https://api.github.com/x?page=2>; rel=\"next\"")
                .set_body_json(json!({"items": [issue_json()]}))
        })
        .mount(&server)
        .await;
    let rig = rig();
    rig.connect(ConnectionKind::Github, "work", &server.uri(), None).await;
    let got = rig
        .call("github.search_issues", read("github", "o/r"), json!({"repo": "o/r", "query": "crash"}))
        .await
        .unwrap();
    assert_eq!(
        (got["issues"].as_array().unwrap().len(), got["has_more"].as_bool(), got["page"].as_u64()),
        (1, Some(true), Some(1))
    );
    for sneaky in ["crash repo:other/secret", "org:acme crash", "USER:someone x", "owner:x y"] {
        let error = message(
            rig.call("github.search_issues", read("github", "o/r"), json!({"repo": "o/r", "query": sneaky}))
                .await
                .unwrap_err(),
        );
        assert!(error.contains("the repository is set by `repo`"), "{sneaky}: {error}");
    }
}

#[tokio::test]
async fn files_are_decoded_folders_listed_binaries_withheld_and_big_files_cut() {
    let server = MockServer::start().await;
    let b64 = |bytes: &[u8]| {
        let all = base64::engine::general_purpose::STANDARD.encode(bytes);
        all.as_bytes().chunks(60).map(|c| String::from_utf8_lossy(c).into_owned()).collect::<Vec<_>>().join("\n")
    };
    Mock::given(method("GET"))
        .and(path("/repos/o/r/contents/README.md"))
        .and(query_param("ref", "dev"))
        .respond_with(
            ResponseTemplate::new(200)
                .set_body_json(json!({"type": "file", "content": b64(b"# Title\n\nHello"), "size": 14})),
        )
        .mount(&server)
        .await;
    Mock::given(method("GET"))
        .and(path("/repos/o/r/contents/logo.png"))
        .respond_with(
            ResponseTemplate::new(200)
                .set_body_json(json!({"type": "file", "content": b64(&[137, 80, 78, 0, 1, 2]), "size": 6})),
        )
        .mount(&server)
        .await;
    Mock::given(method("GET"))
        .and(path("/repos/o/r/contents/big.txt"))
        .respond_with(
            ResponseTemplate::new(200)
                .set_body_json(json!({"type": "file", "content": b64("x".repeat(300_000).as_bytes()), "size": 300000})),
        )
        .mount(&server)
        .await;
    Mock::given(method("GET")).and(path("/repos/o/r/contents/src")).respond_with(ResponseTemplate::new(200).set_body_json(json!([{"name": "main.rs", "path": "src/main.rs", "type": "file", "size": 10}, {"name": "util", "path": "src/util", "type": "dir"}]))).mount(&server).await;
    Mock::given(method("GET"))
        .and(path("/repos/o/r/contents/missing"))
        .respond_with(ResponseTemplate::new(404).set_body_json(json!({"message": "Not Found"})))
        .mount(&server)
        .await;
    let rig = rig();
    rig.connect(ConnectionKind::Github, "work", &server.uri(), None).await;
    let g = read("github", "o/r");
    let file = rig
        .call("github.get_file", g.clone(), json!({"repo": "o/r", "path": "README.md", "ref": "dev"}))
        .await
        .unwrap();
    assert_eq!((file["content"].as_str(), file["size"].as_u64()), (Some("# Title\n\nHello"), Some(14)));
    assert!(
        rig.call("github.get_file", g.clone(), json!({"repo": "o/r", "path": "logo.png"})).await.unwrap()["content"]
            .is_null()
    );
    let big = rig.call("github.get_file", g.clone(), json!({"repo": "o/r", "path": "big.txt"})).await.unwrap();
    assert!(big["truncated"].as_bool().unwrap() && big["content"].as_str().unwrap().len() == 200_000);
    let dir = rig.call("github.get_file", g.clone(), json!({"repo": "o/r", "path": "src"})).await.unwrap();
    assert_eq!((dir["type"].as_str(), dir["entries"][1]["type"].as_str()), (Some("dir"), Some("dir")));
    let error =
        message(rig.call("github.get_file", g.clone(), json!({"repo": "o/r", "path": "missing"})).await.unwrap_err());
    assert!(error.contains("no “missing”"), "{error}");
    assert!(rig.call("github.get_file", g, json!({"repo": "o/r", "path": "../etc/passwd"})).await.is_err());
}

#[tokio::test]
async fn writes_need_a_write_grant_and_a_token_and_send_exactly_what_was_approved() {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/repos/o/r/issues"))
        .and(header("authorization", "Bearer ghp_token"))
        .and(body_json(json!({"title": "Bug", "body": "Details", "labels": ["bug"]})))
        .respond_with(ResponseTemplate::new(201).set_body_json(issue_json()))
        .mount(&server)
        .await;
    Mock::given(method("POST"))
        .and(path("/repos/o/r/issues/7/comments"))
        .and(body_json(json!({"body": "Thanks!"})))
        .respond_with(ResponseTemplate::new(201).set_body_json(
            json!({"id": 99, "html_url": "https://github.com/o/r/issues/7#issuecomment-99", "created_at": "t"}),
        ))
        .mount(&server)
        .await;
    Mock::given(method("PATCH"))
        .and(path("/repos/o/r/issues/7"))
        .and(body_json(json!({"state": "closed"})))
        .respond_with(
            ResponseTemplate::new(200)
                .set_body_json(json!({"number": 7, "state": "closed", "title": "Crash", "user": {"login": "a"}})),
        )
        .mount(&server)
        .await;
    let rig = rig();
    // No connection with a token: writing is refused with the way to fix it.
    rig.connect(ConnectionKind::Github, "anon", &server.uri(), None).await;
    let error = message(
        rig.call("github.create_issue", write("github", "o/r"), json!({"repo": "o/r", "title": "Bug"}))
            .await
            .unwrap_err(),
    );
    assert!(error.contains("needs a connection with a token"), "{error}");
    let rig = self::rig();
    rig.connect(ConnectionKind::Github, "work", &server.uri(), Some("ghp_token")).await;
    // A read grant does not allow writing.
    assert!(
        message(
            rig.call("github.create_issue", read("github", "o/r"), json!({"repo": "o/r", "title": "Bug"}))
                .await
                .unwrap_err()
        )
        .contains("no permission to write github:o/r")
    );
    let created = rig
        .call(
            "github.create_issue",
            write("github", "o/r"),
            json!({"repo": "o/r", "title": "Bug", "body": "Details", "labels": ["bug"]}),
        )
        .await
        .unwrap();
    assert_eq!(created["number"], 7);
    let said = rig
        .call("github.comment", write("github", "o/r"), json!({"repo": "o/r", "number": 7, "body": "Thanks!"}))
        .await
        .unwrap();
    assert_eq!(said["id"], 99);
    let closed = rig
        .call("github.update_issue", write("github", "o/r"), json!({"repo": "o/r", "number": 7, "state": "closed"}))
        .await
        .unwrap();
    assert_eq!(closed["state"], "closed");
    assert!(
        rig.call("github.update_issue", write("github", "o/r"), json!({"repo": "o/r", "number": 7})).await.is_err(),
        "nothing to change"
    );
    // The approval card gets a plain sentence.
    let card =
        rig.tool("github.update_issue").describe_call(&json!({"repo": "o/r", "number": 7, "state": "closed"})).unwrap();
    assert_eq!(card, "Close #7 in o/r on GitHub");
    assert!(
        rig.tool("github.create_issue")
            .describe_call(&json!({"repo": "o/r", "title": "Bug"}))
            .unwrap()
            .contains("“Bug”")
    );
}

#[tokio::test]
async fn errors_from_the_service_are_turned_into_things_a_person_can_act_on() {
    let server = MockServer::start().await;
    let reply = |status: u16, body: Value| ResponseTemplate::new(status).set_body_json(body);
    Mock::given(method("GET"))
        .and(path("/repos/o/r/issues/401"))
        .respond_with(reply(401, json!({"message": "Bad credentials"})))
        .mount(&server)
        .await;
    Mock::given(method("GET"))
        .and(path("/repos/o/r/issues/403"))
        .respond_with(reply(403, json!({"message": "Resource not accessible"})))
        .mount(&server)
        .await;
    Mock::given(method("GET"))
        .and(path("/repos/o/r/issues/429"))
        .respond_with(
            reply(403, json!({"message": "API rate limit exceeded"}))
                .insert_header("x-ratelimit-remaining", "0")
                .insert_header("x-ratelimit-reset", "1800000000"),
        )
        .mount(&server)
        .await;
    Mock::given(method("GET"))
        .and(path("/repos/o/r/issues/500"))
        .respond_with(reply(502, json!({})))
        .mount(&server)
        .await;
    Mock::given(method("POST"))
        .and(path("/repos/o/r/issues"))
        .respond_with(reply(422, json!({"message": "Validation Failed"})))
        .mount(&server)
        .await;
    let rig = rig();
    rig.connect(ConnectionKind::Github, "work", &server.uri(), Some("ghp_token")).await;
    let g = read("github", "o/r");
    let get = |n: u64| rig.call("github.get_issue", g.clone(), json!({"repo": "o/r", "number": n}));
    assert!(message(get(401).await.unwrap_err()).contains("did not accept the connection's token"));
    assert!(message(get(403).await.unwrap_err()).contains("may not be allowed"));
    let limited = get(429).await.unwrap_err();
    assert!(
        matches!(&limited, ToolError::Retryable(m) if m.contains("limiting requests") && m.contains("1800000000")),
        "{limited:?}"
    );
    assert!(matches!(get(500).await.unwrap_err(), ToolError::Retryable(_)), "a service problem may pass");
    let invalid = message(
        rig.call("github.create_issue", write("github", "o/r"), json!({"repo": "o/r", "title": "x"}))
            .await
            .unwrap_err(),
    );
    assert!(invalid.contains("rejected creating the issue") && invalid.contains("Validation Failed"), "{invalid}");
    for error in [get(401).await.unwrap_err(), get(403).await.unwrap_err()] {
        assert!(!message(error).contains("ghp_token"));
    }
}

#[tokio::test]
async fn gitlab_projects_notes_and_states_are_mapped_and_system_notes_are_left_out() {
    let server = MockServer::start().await;
    let gl_issue = json!({"iid": 3, "title": "Slow", "state": "opened", "author": {"username": "cy"}, "labels": ["perf"], "assignees": [{"username": "di"}], "user_notes_count": 1,
                          "created_at": "a", "updated_at": "b", "web_url": "https://gitlab.com/g/s/p/-/issues/3", "description": "Too slow"});
    Mock::given(method("GET"))
        .and(path("/api/v4/projects/g%2Fs%2Fp/issues/3"))
        .and(header("private-token", "glpat-x"))
        .respond_with(ResponseTemplate::new(200).set_body_json(gl_issue.clone()))
        .mount(&server)
        .await;
    Mock::given(method("GET"))
        .and(path("/api/v4/projects/g%2Fs%2Fp/issues"))
        .and(query_param("state", "opened"))
        .and(query_param("search", "slow"))
        .respond_with(
            ResponseTemplate::new(200).insert_header("x-next-page", "2").set_body_json(json!([gl_issue.clone()])),
        )
        .mount(&server)
        .await;
    Mock::given(method("GET"))
        .and(path("/api/v4/projects/g%2Fs%2Fp/issues/3/notes"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!([
            {"author": {"username": "cy"}, "body": "me too", "created_at": "t", "system": false},
            {"author": {"username": "bot"}, "body": "changed the title", "created_at": "t", "system": true}])))
        .mount(&server)
        .await;
    Mock::given(method("GET")).and(path("/api/v4/projects/g%2Fs%2Fp/merge_requests/9")).respond_with(ResponseTemplate::new(200).set_body_json(json!({"iid": 9, "title": "Fix", "state": "merged", "draft": false, "author": {"username": "cy"}, "source_branch": "fix", "target_branch": "main", "web_url": "u", "description": "d", "changes_count": "3"}))).mount(&server).await;
    Mock::given(method("POST"))
        .and(path("/api/v4/projects/g%2Fs%2Fp/issues"))
        .and(body_json(json!({"title": "New", "description": "d", "labels": "a,b"})))
        .respond_with(ResponseTemplate::new(201).set_body_json(gl_issue.clone()))
        .mount(&server)
        .await;
    Mock::given(method("PUT"))
        .and(path("/api/v4/projects/g%2Fs%2Fp/issues/3"))
        .and(body_json(json!({"state_event": "close"})))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({"iid": 3, "state": "closed", "title": "Slow"})))
        .mount(&server)
        .await;
    let rig = rig();
    // GitLab needs a connection (there is no public default), and the base address may omit /api/v4.
    assert!(
        message(
            rig.call("gitlab.get_issue", read("gitlab", "g/s/p"), json!({"repo": "g/s/p", "number": 3}))
                .await
                .unwrap_err()
        )
        .contains("no GitLab connection")
    );
    rig.connect(ConnectionKind::Gitlab, "lab", &server.uri(), Some("glpat-x")).await;
    let g = read("gitlab", "g/s/*");
    let got = rig.call("gitlab.get_issue", g.clone(), json!({"repo": "g/s/p", "number": 3})).await.unwrap();
    assert_eq!(
        (got["state"].as_str(), got["number"].as_u64(), got["author"].as_str()),
        (Some("open"), Some(3), Some("cy")),
        "{got}"
    );
    let found = rig.call("gitlab.search_issues", g.clone(), json!({"repo": "g/s/p", "query": "slow"})).await.unwrap();
    assert_eq!((found["issues"].as_array().unwrap().len(), found["has_more"].as_bool()), (1, Some(true)));
    let notes = rig.call("gitlab.list_comments", g.clone(), json!({"repo": "g/s/p", "number": 3})).await.unwrap();
    assert_eq!(notes["comments"].as_array().unwrap().len(), 1, "system notes are not comments");
    let mr = rig.call("gitlab.get_pull", g.clone(), json!({"repo": "g/s/p", "number": 9})).await.unwrap();
    assert_eq!((mr["merged"].as_bool(), mr["source_branch"].as_str()), (Some(true), Some("fix")));
    rig.call(
        "gitlab.create_issue",
        write("gitlab", "g/*"),
        json!({"repo": "g/s/p", "title": "New", "body": "d", "labels": ["a", "b"]}),
    )
    .await
    .unwrap();
    assert_eq!(
        rig.call(
            "gitlab.update_issue",
            write("gitlab", "g/*"),
            json!({"repo": "g/s/p", "number": 3, "state": "closed"})
        )
        .await
        .unwrap()["state"],
        "closed"
    );
    // A GitLab project is at least group/project.
    assert!(rig.call("gitlab.get_issue", read("gitlab", "*"), json!({"repo": "solo", "number": 1})).await.is_err());
}

#[tokio::test]
async fn which_connection_is_used_is_clear_and_never_guessed() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .respond_with(ResponseTemplate::new(200).set_body_json(issue_json()))
        .mount(&server)
        .await;
    let rig = rig();
    rig.connect(ConnectionKind::Github, "one", &server.uri(), None).await;
    rig.connect(ConnectionKind::Github, "two", &server.uri(), None).await;
    let args = json!({"repo": "o/r", "number": 7});
    let error = message(rig.call("github.get_issue", read("github", "o/r"), args.clone()).await.unwrap_err());
    assert!(error.contains("more than one github connection (one, two)"), "{error}");
    let mut named = args.clone();
    named["connection"] = json!("two");
    assert!(rig.call("github.get_issue", read("github", "o/r"), named.clone()).await.is_ok());
    named["connection"] = json!("nope");
    assert!(
        message(rig.call("github.get_issue", read("github", "o/r"), named).await.unwrap_err())
            .contains("no github connection called nope")
    );
    // A connection whose address is not HTTPS is refused: a token must not go over plain HTTP to another machine.
    let rig = self::rig();
    rig.connect(ConnectionKind::Github, "plain", "http://ghe.example.com/api/v3", Some("tok")).await;
    assert!(message(rig.call("github.get_issue", read("github", "o/r"), args).await.unwrap_err()).contains("HTTPS"));
}
