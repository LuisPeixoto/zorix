//! Jira, Slack and Discord against mock APIs: scoping by grant, what asks first, and what the model never gets to do.

use std::sync::Arc;

use serde_json::{Value, json};
use tokio_util::sync::CancellationToken;
use wiremock::matchers::{body_json, body_partial_json, header, method, path, query_param};
use wiremock::{Mock, MockServer, Request, ResponseTemplate};
use zorix_core::{
    Action, ConnectionKind, ExecutionId, Grants, IntegrationStore, NewConnection, Permission, Secret, TenantId,
    Timestamp, Tool, ToolContext, ToolError,
};
use zorix_integrations::{Services, discord_tools, jira_tools, secret_name, slack_tools};
use zorix_providers::{MemoryStore, SecretStore};
use zorix_store::SqliteIntegrationStore;

struct Rig {
    store: Arc<SqliteIntegrationStore>,
    secrets: Arc<MemoryStore>,
    tools: Vec<Arc<dyn Tool>>,
}

fn rig(make: fn(Arc<Services>) -> Vec<Arc<dyn Tool>>) -> Rig {
    let store = Arc::new(SqliteIntegrationStore::open_in_memory().unwrap());
    let secrets = Arc::new(MemoryStore::new());
    let tools = make(Arc::new(Services { store: store.clone(), secrets: secrets.clone() }));
    Rig { store, secrets, tools }
}

impl Rig {
    async fn connect(&self, kind: ConnectionKind, settings: Value, token: Option<&str>) {
        let connection = self
            .store
            .add_connection(
                &TenantId::local(),
                NewConnection { kind, name: "main".into(), settings, has_secret: token.is_some() },
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

fn grant(action: Action, resource: &str) -> Grants {
    Grants::none().with(Permission::new(action, resource))
}

fn message(error: ToolError) -> String {
    match error {
        ToolError::Failed(m) | ToolError::Retryable(m) => m,
    }
}

// ---------------------------------------------------------------------------------------------------------------------
// Jira

fn jira_issue() -> Value {
    json!({"key": "ABC-12", "fields": {"summary": "Login fails", "status": {"name": "In Progress"}, "issuetype": {"name": "Bug"}, "priority": {"name": "High"},
        "assignee": {"displayName": "Ana"}, "reporter": {"displayName": "Bo"}, "labels": ["auth"], "created": "c", "updated": "u",
        "description": {"type": "doc", "version": 1, "content": [{"type": "paragraph", "content": [{"type": "text", "text": "Steps to reproduce"}]}]}}})
}

#[tokio::test]
async fn jira_reads_are_limited_to_the_granted_project_and_a_search_cannot_leave_it() {
    let server = MockServer::start().await;
    // "luis@example.com:tok" in Basic auth.
    let basic =
        format!("Basic {}", base64::Engine::encode(&base64::engine::general_purpose::STANDARD, "luis@example.com:tok"));
    Mock::given(method("POST")).and(path("/rest/api/3/search/jql")).and(header("authorization", basic.as_str()))
        .and(body_partial_json(json!({"jql": "project = \"ABC\" AND (status = Open OR project = OTHER) ORDER BY updated DESC", "maxResults": 20})))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({"issues": [jira_issue()], "nextPageToken": "tok2"}))).mount(&server).await;
    Mock::given(method("GET"))
        .and(path("/rest/api/3/issue/ABC-12"))
        .respond_with(ResponseTemplate::new(200).set_body_json(jira_issue()))
        .mount(&server)
        .await;
    let rig = rig(jira_tools);
    rig.connect(ConnectionKind::Jira, json!({"base_url": server.uri(), "email": "luis@example.com"}), Some("tok"))
        .await;
    let read = grant(Action::Read, "jira:ABC");
    let found = rig
        .call(
            "jira.search",
            read.clone(),
            json!({"project": "abc", "jql": "status = Open OR project = OTHER ORDER BY updated DESC"}),
        )
        .await
        .unwrap();
    assert_eq!(
        (found["issues"][0]["key"].as_str(), found["issues"][0]["status"].as_str(), found["next_page_token"].as_str()),
        (Some("ABC-12"), Some("In Progress"), Some("tok2"))
    );
    let got = rig.call("jira.get_issue", read.clone(), json!({"key": "ABC-12"})).await.unwrap();
    assert_eq!(
        (got["description"].as_str(), got["assignee"].as_str()),
        (Some("Steps to reproduce"), Some("Ana")),
        "the Atlassian document became plain text"
    );
    assert!(!got.to_string().contains("tok\""));
    // Another project, no grant, or a write grant: refused before anything is sent.
    for grants in [Grants::none(), grant(Action::Read, "jira:XYZ"), grant(Action::Write, "jira:ABC")] {
        assert!(
            message(rig.call("jira.get_issue", grants, json!({"key": "ABC-12"})).await.unwrap_err())
                .contains("no permission to read jira:ABC")
        );
    }
    assert!(rig.call("jira.get_issue", read.clone(), json!({"key": "not-a-key"})).await.is_err());
    // A search that closes the wrapping group early to reach another project is refused before anything is sent, and so is a
    // project that is not a key.
    for hostile in [
        json!({"project": "ABC", "jql": "x = 1) OR project = OTHER OR (x = 1"}),
        json!({"project": "ABC", "jql": "x = 1 ORDER BY a) OR (project = OTHER"}),
        json!({"project": "ABC\" OR project = \"Z", "jql": ""}),
    ] {
        assert!(rig.call("jira.search", read.clone(), hostile.clone()).await.is_err(), "{hostile}");
    }
    assert_eq!(server.received_requests().await.unwrap().len(), 2);
}

#[tokio::test]
async fn jira_writes_ask_first_and_send_documents_the_service_accepts() {
    let server = MockServer::start().await;
    Mock::given(method("POST")).and(path("/rest/api/3/issue")).and(body_partial_json(json!({"fields": {"project": {"key": "ABC"}, "summary": "New", "issuetype": {"name": "Bug"}, "labels": ["x"]}})))
        .respond_with(|request: &Request| {
            let body: Value = serde_json::from_slice(&request.body).unwrap();
            assert_eq!(body["fields"]["description"]["type"], "doc", "plain text is sent as a document");
            ResponseTemplate::new(201).set_body_json(json!({"id": "10001", "key": "ABC-13"}))
        }).mount(&server).await;
    Mock::given(method("POST"))
        .and(path("/rest/api/3/issue/ABC-12/comment"))
        .and(body_partial_json(json!({"body": {"type": "doc"}})))
        .respond_with(ResponseTemplate::new(201).set_body_json(json!({"id": "555"})))
        .mount(&server)
        .await;
    Mock::given(method("GET")).and(path("/rest/api/3/issue/ABC-12/transitions"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({"transitions": [{"id": "31", "name": "Start", "to": {"name": "In Progress"}}, {"id": "41", "name": "Finish", "to": {"name": "Done"}}]}))).mount(&server).await;
    Mock::given(method("POST"))
        .and(path("/rest/api/3/issue/ABC-12/transitions"))
        .and(body_json(json!({"transition": {"id": "41"}})))
        .respond_with(ResponseTemplate::new(204))
        .mount(&server)
        .await;
    let rig = rig(jira_tools);
    for id in ["jira.create_issue", "jira.comment", "jira.transition"] {
        assert!(rig.tool(id).spec().requires_approval, "{id}");
    }
    assert!(
        !rig.tool("jira.search").spec().requires_approval
            && rig.tool("jira.list_transitions").spec().annotations.read_only
    );
    rig.connect(ConnectionKind::Jira, json!({"base_url": server.uri(), "email": "e@x.com"}), Some("tok")).await;
    let w = grant(Action::Write, "jira:ABC");
    assert!(
        message(
            rig.call("jira.create_issue", grant(Action::Read, "jira:ABC"), json!({"project": "ABC", "summary": "x"}))
                .await
                .unwrap_err()
        )
        .contains("no permission to write jira:ABC")
    );
    let created = rig.call("jira.create_issue", w.clone(), json!({"project": "ABC", "summary": "New", "description": "Line one\n\nLine two", "issue_type": "Bug", "labels": ["x"]})).await.unwrap();
    assert_eq!(created["key"], "ABC-13");
    assert_eq!(
        rig.call("jira.comment", w.clone(), json!({"key": "ABC-12", "body": "On it"})).await.unwrap()["id"],
        "555"
    );
    // A move can be named by the move or by where it leads, in any case; one that is not available says what is.
    let moved = rig.call("jira.transition", w.clone(), json!({"key": "ABC-12", "to": "done"})).await.unwrap();
    assert_eq!((moved["moved"].as_str(), moved["status"].as_str()), (Some("Finish"), Some("Done")));
    let error = message(rig.call("jira.transition", w, json!({"key": "ABC-12", "to": "Archived"})).await.unwrap_err());
    assert!(error.contains("cannot move to “archived” now") && error.contains("Start, Finish"), "{error}");
    assert_eq!(
        rig.tool("jira.transition").describe_call(&json!({"key": "ABC-12", "to": "Done"})).unwrap(),
        "Move Jira issue ABC-12 to “Done”"
    );
}

#[tokio::test]
async fn jira_needs_a_connection_with_a_token_and_says_how_to_fix_it() {
    let rig = rig(jira_tools);
    let read = grant(Action::Read, "jira:ABC");
    assert!(
        message(rig.call("jira.get_issue", read.clone(), json!({"key": "ABC-1"})).await.unwrap_err())
            .contains("zorix connect add jira")
    );
    rig.connect(ConnectionKind::Jira, json!({"base_url": "https://acme.atlassian.net", "email": "e@x.com"}), None)
        .await;
    assert!(
        message(rig.call("jira.get_issue", read, json!({"key": "ABC-1"})).await.unwrap_err())
            .contains("no token stored")
    );
}

// ---------------------------------------------------------------------------------------------------------------------
// Slack

fn slack_channels() -> Value {
    json!({"ok": true, "channels": [
        {"id": "C0001GENERAL", "name": "general", "is_private": false, "is_member": true, "topic": {"value": "everything"}, "num_members": 40},
        {"id": "C0002SECRETS1", "name": "secrets", "is_private": true, "is_member": true, "topic": {"value": ""}, "num_members": 3}]})
}

#[tokio::test]
async fn slack_shows_only_the_channels_a_run_may_read_and_resolves_names() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/conversations.list"))
        .and(header("authorization", "Bearer xoxb-token"))
        .respond_with(ResponseTemplate::new(200).set_body_json(slack_channels()))
        .mount(&server)
        .await;
    Mock::given(method("GET")).and(path("/conversations.history")).and(query_param("channel", "C0001GENERAL")).and(query_param("limit", "20"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({"ok": true, "has_more": false, "messages": [{"user": "U1", "ts": "1.1", "text": "hello", "reply_count": 2}]}))).mount(&server).await;
    let rig = rig(slack_tools);
    rig.connect(ConnectionKind::Slack, json!({"base_url": server.uri()}), Some("xoxb-token")).await;
    // A run granted `general` sees only it, and the private channel's name never reaches the model.
    let listed = rig.call("slack.list_channels", grant(Action::Read, "slack:general"), json!({})).await.unwrap();
    assert_eq!(listed["channels"].as_array().unwrap().len(), 1);
    assert!(!listed.to_string().contains("secrets"));
    // The grant may name the channel or its id, with or without the #.
    for grants in [
        grant(Action::Read, "slack:general"),
        grant(Action::Read, "slack:#general"),
        grant(Action::Read, "slack:C0001GENERAL"),
        grant(Action::Read, "slack:*"),
    ] {
        let got = rig.call("slack.read_channel", grants, json!({"channel": "#general"})).await.unwrap();
        assert_eq!(got["messages"][0]["text"], "hello");
    }
    let error = message(
        rig.call("slack.read_channel", grant(Action::Read, "slack:general"), json!({"channel": "secrets"}))
            .await
            .unwrap_err(),
    );
    assert!(error.contains("no permission to read slack:secrets"), "{error}");
    assert!(
        message(
            rig.call("slack.read_channel", grant(Action::Read, "slack:general"), json!({"channel": "nowhere"}))
                .await
                .unwrap_err()
        )
        .contains("no channel called nowhere")
    );
}

#[tokio::test]
async fn slack_posts_only_with_a_write_grant_never_to_everyone_and_with_clear_errors() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/conversations.list"))
        .respond_with(ResponseTemplate::new(200).set_body_json(slack_channels()))
        .mount(&server)
        .await;
    Mock::given(method("POST"))
        .and(path("/chat.postMessage"))
        .and(body_partial_json(
            json!({"channel": "C0001GENERAL", "text": "Deploy done", "unfurl_links": false, "link_names": false}),
        ))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({"ok": true, "ts": "1700000000.000100"})))
        .mount(&server)
        .await;
    Mock::given(method("POST"))
        .and(path("/chat.postMessage"))
        .and(body_partial_json(json!({"channel": "C0002SECRETS1"})))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({"ok": false, "error": "not_in_channel"})))
        .mount(&server)
        .await;
    let rig = rig(slack_tools);
    assert!(
        rig.tool("slack.send_message").spec().requires_approval
            && !rig.tool("slack.read_channel").spec().requires_approval
    );
    rig.connect(ConnectionKind::Slack, json!({"base_url": server.uri()}), Some("xoxb-token")).await;
    let w = grant(Action::Write, "slack:general");
    let sent =
        rig.call("slack.send_message", w.clone(), json!({"channel": "#general", "text": "Deploy done"})).await.unwrap();
    assert_eq!(sent["ts"], "1700000000.000100");
    // A read grant is not a write grant.
    assert!(
        message(
            rig.call(
                "slack.send_message",
                grant(Action::Read, "slack:general"),
                json!({"channel": "#general", "text": "x"})
            )
            .await
            .unwrap_err()
        )
        .contains("no permission to write")
    );
    for loud in ["<!channel> look", "hey @here", "@everyone!"] {
        let error = message(
            rig.call("slack.send_message", w.clone(), json!({"channel": "#general", "text": loud})).await.unwrap_err(),
        );
        assert!(error.contains("may not notify the whole channel"), "{loud}");
    }
    let error = message(
        rig.call(
            "slack.send_message",
            grant(Action::Write, "slack:secrets"),
            json!({"channel": "secrets", "text": "hi"}),
        )
        .await
        .unwrap_err(),
    );
    assert!(error.contains("has not been invited"), "{error}");
    let card =
        rig.tool("slack.send_message").describe_call(&json!({"channel": "#general", "text": "Deploy done"})).unwrap();
    assert_eq!(card, "Post a message to #general on Slack: “Deploy done”");
}

#[tokio::test]
async fn slack_errors_that_come_with_status_200_are_understood() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/conversations.info"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({"ok": false, "error": "invalid_auth"})))
        .mount(&server)
        .await;
    Mock::given(method("GET"))
        .and(path("/conversations.history"))
        .respond_with(ResponseTemplate::new(429).insert_header("retry-after", "12"))
        .mount(&server)
        .await;
    let rig = rig(slack_tools);
    rig.connect(ConnectionKind::Slack, json!({"base_url": server.uri()}), Some("xoxb-bad")).await;
    // An id is used as given; its name is looked up only for grants, and a failed lookup is not fatal.
    let error = rig
        .call("slack.read_channel", grant(Action::Read, "slack:C0001GENERAL"), json!({"channel": "C0001GENERAL"}))
        .await
        .unwrap_err();
    assert!(matches!(&error, ToolError::Retryable(m) if m.contains("12 seconds")), "{error:?}");
}

// ---------------------------------------------------------------------------------------------------------------------
// Discord

#[tokio::test]
async fn discord_lists_only_readable_text_channels_reads_and_never_mentions_anyone() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/guilds/123456/channels"))
        .and(header("authorization", "Bot bot-token"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!([
            {"id": "1000001", "name": "general", "type": 0, "topic": "chat"},
            {"id": "1000002", "name": "voice", "type": 2},
            {"id": "1000003", "name": "private", "type": 0}])))
        .mount(&server)
        .await;
    Mock::given(method("GET")).and(path("/channels/1000001/messages")).and(query_param("limit", "5"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!([{"id": "9", "author": {"username": "ana", "bot": false}, "timestamp": "t", "content": "hi @everyone"}]))).mount(&server).await;
    Mock::given(method("POST")).and(path("/channels/1000001/messages"))
        .and(body_json(json!({"content": "Deploy done @everyone", "allowed_mentions": {"parse": []}, "message_reference": {"message_id": "123456789", "fail_if_not_exists": false}})))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({"id": "10"}))).mount(&server).await;
    Mock::given(method("POST"))
        .and(path("/channels/1000004/messages"))
        .respond_with(
            ResponseTemplate::new(403).set_body_json(json!({"message": "Missing Permissions", "code": 50013})),
        )
        .mount(&server)
        .await;
    let rig = rig(discord_tools);
    assert!(rig.tool("discord.send_message").spec().requires_approval);
    rig.connect(ConnectionKind::Discord, json!({"base_url": server.uri()}), Some("bot-token")).await;
    let listed = rig
        .call("discord.list_channels", grant(Action::Read, "discord:1000001"), json!({"guild_id": "123456"}))
        .await
        .unwrap();
    assert_eq!(listed["channels"].as_array().unwrap().len(), 1, "no voice channels, no unreadable ones");
    let read = rig
        .call(
            "discord.read_channel",
            grant(Action::Read, "discord:1000001"),
            json!({"channel_id": "1000001", "limit": 5}),
        )
        .await
        .unwrap();
    assert_eq!(read["messages"][0]["author"], "ana");
    assert!(
        message(
            rig.call("discord.read_channel", grant(Action::Read, "discord:1000001"), json!({"channel_id": "1000003"}))
                .await
                .unwrap_err()
        )
        .contains("no permission to read discord:1000003")
    );
    // The text may say @everyone, but the message is sent with mentions switched off.
    let sent = rig
        .call(
            "discord.send_message",
            grant(Action::Write, "discord:1000001"),
            json!({"channel_id": "1000001", "content": "Deploy done @everyone", "reply_to": "123456789"}),
        )
        .await
        .unwrap();
    assert_eq!(sent["id"], "10");
    let denied = message(
        rig.call(
            "discord.send_message",
            grant(Action::Write, "discord:1000004"),
            json!({"channel_id": "1000004", "content": "x"}),
        )
        .await
        .unwrap_err(),
    );
    assert!(denied.contains("not allowed to do that in this channel"), "{denied}");
    assert!(
        rig.call(
            "discord.send_message",
            grant(Action::Read, "discord:1000001"),
            json!({"channel_id": "1000001", "content": "x"})
        )
        .await
        .is_err()
    );
}

#[tokio::test]
async fn a_discord_id_never_carries_anything_into_the_path_of_a_request() {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/channels/123456789/messages"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({"id": "42"})))
        .mount(&server)
        .await;
    let rig = rig(discord_tools);
    rig.connect(ConnectionKind::Discord, json!({"base_url": server.uri()}), Some("bot-token")).await;
    let write = grant(Action::Write, "discord:*");
    let sent = rig
        .call("discord.send_message", write.clone(), json!({"channel_id": "123456789", "content": "hi"}))
        .await
        .unwrap();
    assert_eq!(sent["id"], "42");
    // Called with no schema check in front (as a workflow action once was), a hostile id still goes nowhere.
    for hostile in ["123456789/../../guilds/1", "12345?x=1", "../123456789", "abc", "1234", "123456789 "] {
        let error = rig
            .call("discord.send_message", write.clone(), json!({"channel_id": hostile, "content": "hi"}))
            .await
            .unwrap_err();
        assert!(matches!(&error, ToolError::Failed(m) if m.contains("is not an id")), "{hostile}: {error:?}");
    }
    assert_eq!(server.received_requests().await.unwrap().len(), 1, "only the good one was sent");
}
