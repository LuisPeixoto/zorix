//! Contract tests for the agent store: versions are immutable and never duplicated, rollback adds a version, tenants never mix,
//! skills keep every hash an agent still names, and a run remembers which agent made it.

use zorix_core::{
    AgentBinding, AgentSlug, AgentSpec, AgentStore, ConversationId, EvalCase, EvalCheck, EvalRun, EvalRunId, Execution,
    ExecutionId, ExecutionRepository, ListFilter, NewExecution, SkillFile, SkillPin, SkillRecord, TaskRequest,
    TenantId, Timestamp,
};
use zorix_store::{SqliteAgentStore, SqliteRepository};

fn tenant(name: &str) -> TenantId {
    TenantId::new(name).unwrap()
}

fn spec(slug: &str) -> AgentSpec {
    let mut spec = AgentSpec::new(AgentSlug::new(slug).unwrap(), "Researcher", "demo");
    spec.instructions = "Find things out.".into();
    spec
}

const NOW: Timestamp = Timestamp(1_000);

fn skill(name: &str, hash: &str) -> SkillRecord {
    SkillRecord {
        name: name.into(),
        hash: hash.into(),
        description: "Does something useful. Use when asked.".into(),
        files: vec![SkillFile { path: "SKILL.md".into(), size: 10, sha256: hash.into() }],
        license: None,
        compatibility: None,
        metadata: Default::default(),
        allowed_tools: vec![],
        source: Some("test".into()),
        imported_at: NOW,
    }
}

#[tokio::test]
async fn an_agent_starts_at_version_one_and_a_slug_is_taken_once_per_tenant() {
    let store = SqliteAgentStore::open_in_memory().unwrap();
    let (agent, v1) = store.create_agent(&tenant("a"), spec("researcher"), Some("first".into()), NOW).await.unwrap();
    assert_eq!((agent.current_version, v1.version, v1.note.as_deref()), (1, 1, Some("first")));
    assert_eq!(v1.hash, spec("researcher").hash());
    let again = store.create_agent(&tenant("a"), spec("researcher"), None, NOW).await.unwrap_err().to_string();
    assert!(again.contains("already an agent called researcher"), "{again}");
    // Another tenant may use the same name and cannot see this agent.
    let (other, _) = store.create_agent(&tenant("b"), spec("researcher"), None, NOW).await.unwrap();
    assert_ne!(other.id, agent.id);
    assert!(store.agent(&tenant("b"), agent.id).await.unwrap().is_none());
    assert!(store.versions(&tenant("b"), agent.id).await.is_err());
    assert_eq!(store.agents(&tenant("a"), false).await.unwrap().len(), 1);
    let found = store.agent_by_slug(&tenant("a"), &AgentSlug::new("researcher").unwrap()).await.unwrap().unwrap();
    assert_eq!(found.id, agent.id);
    // An invalid spec never reaches the database.
    let mut bad = spec("bad");
    bad.access = vec!["ftp:x".into()];
    assert!(store.create_agent(&tenant("a"), bad, None, NOW).await.is_err());
    assert_eq!(store.agents(&tenant("a"), true).await.unwrap().len(), 1);
}

#[tokio::test]
async fn saving_the_same_content_writes_nothing_and_new_content_writes_the_next_version() {
    let store = SqliteAgentStore::open_in_memory().unwrap();
    let who = tenant("a");
    let (agent, _) = store.create_agent(&who, spec("writer"), None, NOW).await.unwrap();
    let (same, changed) = store.save_version(&who, agent.id, spec("writer"), None, Timestamp(2_000)).await.unwrap();
    assert!(!changed && same.version == 1);

    let mut better = spec("writer");
    better.instructions = "Write clearly.".into();
    let (v2, changed) =
        store.save_version(&who, agent.id, better.clone(), Some("clearer".into()), Timestamp(3_000)).await.unwrap();
    assert!(changed && v2.version == 2 && v2.created_at == Timestamp(3_000));
    assert_eq!(store.agent(&who, agent.id).await.unwrap().unwrap().current_version, 2);

    // Every version is kept exactly as it was, newest first.
    let all = store.versions(&who, agent.id).await.unwrap();
    assert_eq!(all.iter().map(|v| v.version).collect::<Vec<_>>(), vec![2, 1]);
    assert_eq!(all[1].spec.instructions, "Find things out.", "version 1 is untouched by what came after");
    assert_eq!(all[0].spec.instructions, "Write clearly.");
}

#[tokio::test]
async fn rollback_writes_a_new_version_that_copies_an_old_one_and_the_name_never_changes() {
    let store = SqliteAgentStore::open_in_memory().unwrap();
    let who = tenant("a");
    let mut first = spec("analyst");
    first.instructions = "Version one.".into();
    let (agent, _) = store.create_agent(&who, first.clone(), None, NOW).await.unwrap();
    let mut second = first.clone();
    second.instructions = "Version two.".into();
    store.save_version(&who, agent.id, second, None, Timestamp(2_000)).await.unwrap();

    let back = store.rollback(&who, agent.id, 1, Timestamp(3_000)).await.unwrap();
    assert_eq!(back.version, 3);
    assert_eq!(back.spec.instructions, "Version one.");
    assert_eq!(back.note.as_deref(), Some("rollback to v1"));
    assert_eq!(store.versions(&who, agent.id).await.unwrap().len(), 3, "history is never rewritten");
    // Rolling back to what is already current changes nothing.
    assert_eq!(store.rollback(&who, agent.id, 3, Timestamp(4_000)).await.unwrap().version, 3);
    assert!(store.rollback(&who, agent.id, 99, NOW).await.is_err());

    let renamed = spec("another");
    assert!(
        store
            .save_version(&who, agent.id, renamed, None, NOW)
            .await
            .unwrap_err()
            .to_string()
            .contains("does not change")
    );
}

#[tokio::test]
async fn archiving_hides_and_removing_deletes_with_everything_that_belongs_to_it() {
    let store = SqliteAgentStore::open_in_memory().unwrap();
    let who = tenant("a");
    let (agent, _) = store.create_agent(&who, spec("temp"), None, NOW).await.unwrap();
    let case = EvalCase {
        name: "greets".into(),
        prompt: "Say hello".into(),
        input: None,
        tags: vec![],
        checks: vec![EvalCheck::Contains { text: "hello".into(), ignore_case: true }],
        judge: None,
        approve: vec![],
        allow_writes: false,
    };
    store.put_eval_case(&who, agent.id, case, NOW).await.unwrap();
    store.set_archived(&who, agent.id, true).await.unwrap();
    assert!(store.agents(&who, false).await.unwrap().is_empty());
    assert_eq!(store.agents(&who, true).await.unwrap().len(), 1);
    assert!(!store.remove_agent(&tenant("b"), agent.id).await.unwrap(), "another tenant cannot remove it");
    assert!(store.remove_agent(&who, agent.id).await.unwrap());
    assert!(store.agent(&who, agent.id).await.unwrap().is_none());
    assert!(store.eval_cases(&who, agent.id).await.is_err());
    assert!(!store.remove_agent(&who, agent.id).await.unwrap());
}

#[tokio::test]
async fn a_skill_keeps_every_version_and_a_hash_stays_referenced_while_an_agent_names_it() {
    let store = SqliteAgentStore::open_in_memory().unwrap();
    let who = tenant("a");
    let (h1, h2) = ("1".repeat(64), "2".repeat(64));
    store.record_skill(&who, skill("pdf-tools", &h1)).await.unwrap();
    let mut pinned = spec("reader");
    pinned.skills = vec![SkillPin { name: "pdf-tools".into(), hash: h1.clone() }];
    store.create_agent(&who, pinned, None, NOW).await.unwrap();
    // The skill is updated: the old content is still there for the agent that pinned it.
    store.record_skill(&who, skill("pdf-tools", &h2)).await.unwrap();
    assert_eq!(store.skill(&who, "pdf-tools").await.unwrap().unwrap().hash, h2);
    assert_eq!(store.skill_version(&who, "pdf-tools", &h1).await.unwrap().unwrap().hash, h1);
    assert_eq!(store.skills(&who).await.unwrap().len(), 1);
    let referenced = store.referenced_skill_hashes(&who).await.unwrap();
    assert!(referenced.contains(&h1) && referenced.contains(&h2));
    assert!(store.skill(&tenant("b"), "pdf-tools").await.unwrap().is_none());
    assert!(store.record_skill(&who, skill("Bad Name", &h1)).await.is_err());
    assert!(store.remove_skill(&who, "pdf-tools").await.unwrap());
    let still = store.referenced_skill_hashes(&who).await.unwrap();
    assert!(still.contains(&h1), "the agent still names it, so its files must not be cleaned up");
    assert!(!still.contains(&h2));
}

#[tokio::test]
async fn cases_are_replaced_by_name_and_eval_runs_keep_their_results() {
    let store = SqliteAgentStore::open_in_memory().unwrap();
    let who = tenant("a");
    let (agent, _) = store.create_agent(&who, spec("tested"), None, NOW).await.unwrap();
    let mut case = EvalCase {
        name: "math".into(),
        prompt: "What is 2+2?".into(),
        input: None,
        tags: vec!["basic".into()],
        checks: vec![EvalCheck::Contains { text: "4".into(), ignore_case: false }],
        judge: None,
        approve: vec![],
        allow_writes: false,
    };
    let first = store.put_eval_case(&who, agent.id, case.clone(), NOW).await.unwrap();
    case.prompt = "What is 3+3?".into();
    let replaced = store.put_eval_case(&who, agent.id, case, Timestamp(2_000)).await.unwrap();
    assert_eq!(first.id, replaced.id, "same name, same case");
    let cases = store.eval_cases(&who, agent.id).await.unwrap();
    assert_eq!(cases.len(), 1);
    assert_eq!(cases[0].case.prompt, "What is 3+3?");
    assert!(
        store.put_eval_case(&who, agent.id, EvalCase { checks: vec![], ..cases[0].case.clone() }, NOW).await.is_err()
    );

    let run = EvalRun {
        id: EvalRunId::new(),
        agent: agent.id,
        version: 1,
        model: "demo".into(),
        status: "running".into(),
        started_at: NOW,
        finished_at: None,
        results: vec![],
    };
    store.save_eval_run(&who, &run).await.unwrap();
    let mut done = run.clone();
    done.status = "finished".into();
    done.finished_at = Some(Timestamp(5_000));
    store.save_eval_run(&who, &done).await.unwrap();
    assert_eq!(store.eval_run(&who, run.id).await.unwrap().unwrap().status, "finished");
    assert!(store.eval_run(&tenant("b"), run.id).await.unwrap().is_none());
    assert_eq!(store.eval_runs(&who, agent.id, 10).await.unwrap().len(), 1);
    assert!(store.remove_eval_case(&who, agent.id, "math").await.unwrap());
}

#[tokio::test]
async fn a_run_remembers_its_agent_and_the_agent_has_a_list_of_conversations() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("zorix.sqlite");
    let agents = SqliteAgentStore::open(&path).unwrap();
    let repository = SqliteRepository::open(&path).unwrap();
    let who = TenantId::local();
    let (agent, version) = agents.create_agent(&who, spec("chatty"), None, NOW).await.unwrap();
    let binding = AgentBinding::of(&agent, &version);

    let make = |prompt: &str, conversation: &str, agent: Option<AgentBinding>, at: i64| {
        let new = NewExecution {
            conversation: Some(ConversationId::new(conversation).unwrap()),
            request: TaskRequest { prompt: prompt.into(), model: "demo".into(), ..TaskRequest::default() },
            agent,
            ..NewExecution::default()
        };
        Execution::new(ExecutionId::new(), new, Timestamp(at))
    };
    for execution in [
        make("Hello there", "chat-a", Some(binding.clone()), 10),
        make("And another thing", "chat-a", Some(binding.clone()), 20),
        make("A different chat", "chat-b", Some(binding.clone()), 30),
        make("Not an agent's run", "chat-c", None, 40),
    ] {
        repository.create(execution, vec![]).await.unwrap();
    }

    let by_agent = repository.list(&who, &ListFilter { agent: Some(agent.id), ..ListFilter::default() }).await.unwrap();
    assert_eq!(by_agent.len(), 3);
    assert!(by_agent.iter().all(|e| e.agent.as_ref().map(|a| a.version) == Some(1)), "the version is recorded");
    assert_eq!(repository.list(&who, &ListFilter::default()).await.unwrap().len(), 4);

    let conversations = agents.conversations(&who, agent.id, 10).await.unwrap();
    assert_eq!(conversations.len(), 2, "chat-c is not this agent's");
    assert_eq!(conversations[0].conversation.as_str(), "chat-b", "newest first");
    assert_eq!(conversations[1].title, "Hello there", "the first prompt names the conversation");
    assert_eq!(conversations[1].runs, 2);
}
