//! The memory store against a real SQLite file: scopes, tenants, search, vectors, knowledge, snapshots and deletion.

use std::path::PathBuf;

use serde_json::json;
use zorix_core::{
    ConversationId, Corpus, DocState, DocStatus, DocumentWrite, ExecutionId, KnowledgeRootId, MemoryFilter, MemoryId,
    MemoryKind, MemoryOrigin, MemoryPatch, MemorySettings, MemoryStatus, MemoryStore, NewChunk, NewMemory, NewRoot,
    OperationRecord, RecallSnapshot, Remembered, Scope, TenantId, Timestamp, Visibility, chunk_key, memory_key,
    text_hash,
};
use zorix_store::{SqliteMemoryStore, SqliteRepository};

const NOW: Timestamp = Timestamp(1_000_000);

struct Fixture {
    _dir: tempfile::TempDir,
    path: PathBuf,
    store: SqliteMemoryStore,
}

fn fixture() -> Fixture {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("zorix.sqlite");
    let store = SqliteMemoryStore::open(&path).unwrap();
    Fixture { _dir: dir, path, store }
}

fn tenant(name: &str) -> TenantId {
    TenantId::new(name).unwrap()
}

fn vis(name: &str) -> Visibility {
    Visibility::tenant(tenant(name))
}

fn new(scope: Scope, content: &str, subject: Option<&str>) -> NewMemory {
    NewMemory {
        scope,
        kind: MemoryKind::Fact,
        subject: subject.map(str::to_string),
        content: content.into(),
        valid_until: None,
        origin: MemoryOrigin::User { name: "test".into() },
        replaces: vec![],
    }
}

fn terms(words: &[&str]) -> Vec<String> {
    words.iter().map(|w| w.to_string()).collect()
}

fn chunk(text: &str, line: u32) -> NewChunk {
    NewChunk {
        text: text.into(),
        text_hash: text_hash(text),
        heading: None,
        line_start: Some(line),
        line_end: Some(line + 5),
        page: None,
        tokens: 10,
    }
}

async fn add_root(store: &SqliteMemoryStore, t: &str, scope: Scope, path: &str) -> KnowledgeRootId {
    store
        .add_root(
            &tenant(t),
            NewRoot {
                scope,
                path: path.into(),
                include: vec![],
                exclude: vec![],
                max_file_bytes: 2_000_000,
                auto_sync: false,
            },
            NOW,
        )
        .await
        .unwrap()
        .id
}

fn doc(root: KnowledgeRootId, rel: &str, chunks: Vec<NewChunk>) -> DocumentWrite {
    DocumentWrite {
        root,
        rel_path: rel.into(),
        kind: "markdown".into(),
        state: DocState { size: 10, mtime_ns: 5, content_hash: text_hash(rel), status: DocStatus::Indexed },
        error: None,
        chunks,
        indexed_at: NOW,
    }
}

fn unit(v: &[f32]) -> Vec<f32> {
    let n = v.iter().map(|x| x * x).sum::<f32>().sqrt();
    v.iter().map(|x| x / n).collect()
}

#[tokio::test]
async fn remembering_the_same_text_twice_changes_nothing_and_a_subject_replaces_the_old_item() {
    let f = fixture();
    let t = tenant("a");
    let first =
        f.store.remember(&t, new(Scope::Global, "The project uses Postgres 15.", Some("database")), NOW).await.unwrap();
    let Remembered::Added(first) = first else { panic!("added") };
    assert!(matches!(
        f.store.remember(&t, new(Scope::Global, "  The project uses Postgres 15.  ", None), NOW).await.unwrap(),
        Remembered::Duplicate(_)
    ));
    let second = f
        .store
        .remember(&t, new(Scope::Global, "The project uses Postgres 16.", Some("database")), Timestamp(NOW.0 + 1))
        .await
        .unwrap();
    let Remembered::Replaced { new: second, old } = second else { panic!("replaced") };
    assert_eq!(old, vec![first.id]);
    let old_item = f.store.memory(&t, first.id).await.unwrap().unwrap();
    assert_eq!((old_item.status, old_item.superseded_by), (MemoryStatus::Superseded, Some(second.id)));
    // Only the new one is listed; the old one is kept for the record.
    let in_force = f.store.list_memory(&vis("a"), &MemoryFilter::default(), NOW).await.unwrap();
    assert_eq!(in_force.iter().map(|i| i.id).collect::<Vec<_>>(), vec![second.id]);
    let all = f
        .store
        .list_memory(&vis("a"), &MemoryFilter { include_inactive: true, ..Default::default() }, NOW)
        .await
        .unwrap();
    assert_eq!(all.len(), 2);
    // A superseded item cannot be edited, and the same text in another scope is a different memory.
    assert!(
        f.store.update_memory(&t, first.id, MemoryPatch { confirm: true, ..Default::default() }, NOW).await.is_err()
    );
    assert!(matches!(
        f.store
            .remember(&t, new(Scope::Project("/p".into()), "The project uses Postgres 16.", None), NOW)
            .await
            .unwrap(),
        Remembered::Added(_)
    ));
}

#[tokio::test]
async fn replaces_must_name_visible_items_in_force() {
    let f = fixture();
    let (a, b) = (tenant("a"), tenant("b"));
    let mine = f.store.remember(&a, new(Scope::Global, "mine", None), NOW).await.unwrap().item().id;
    let mut theirs = new(Scope::Global, "theirs", None);
    theirs.replaces = vec![mine];
    assert!(f.store.remember(&b, theirs, NOW).await.is_err(), "another tenant's item cannot be replaced");
    let mut gone = new(Scope::Global, "x", None);
    gone.replaces = vec![MemoryId::new()];
    assert!(f.store.remember(&a, gone, NOW).await.is_err());
    let mut ok = new(Scope::Global, "mine, better", None);
    ok.replaces = vec![mine];
    assert!(matches!(f.store.remember(&a, ok, NOW).await.unwrap(), Remembered::Replaced { .. }));
}

#[tokio::test]
async fn what_a_run_sees_depends_on_tenant_project_and_conversation() {
    let f = fixture();
    let chat = ConversationId::new("chat-1").unwrap();
    for (t, scope, text) in [
        ("a", Scope::Global, "global fact"),
        ("a", Scope::Project("/work/one".into()), "project one fact"),
        ("a", Scope::Project("/work/two".into()), "project two fact"),
        ("a", Scope::Conversation(chat.clone()), "chat fact"),
        ("a", Scope::Conversation(ConversationId::new("chat-2").unwrap()), "other chat fact"),
        ("b", Scope::Global, "other tenant global"),
    ] {
        f.store.remember(&tenant(t), new(scope, text, None), NOW).await.unwrap();
    }
    let texts = |items: Vec<zorix_core::MemoryItem>| {
        let mut t: Vec<String> = items.into_iter().map(|i| i.content).collect();
        t.sort();
        t
    };
    let full =
        Visibility { tenant: tenant("a"), project: Some("/work/one".into()), conversation: Some(chat), agent: None };
    assert_eq!(
        texts(f.store.list_memory(&full, &MemoryFilter::default(), NOW).await.unwrap()),
        ["chat fact", "global fact", "project one fact"]
    );
    assert_eq!(texts(f.store.list_memory(&vis("a"), &MemoryFilter::default(), NOW).await.unwrap()), ["global fact"]);
    assert_eq!(
        texts(f.store.list_memory(&vis("b"), &MemoryFilter::default(), NOW).await.unwrap()),
        ["other tenant global"]
    );
    // Search follows the same rules.
    let hits = f.store.text_search(&full, Corpus::Memory, &terms(&["fact"]), 10).await.unwrap();
    assert_eq!(hits.len(), 3);
    assert!(f.store.text_search(&vis("b"), Corpus::Memory, &terms(&["chat"]), 10).await.unwrap().is_empty());
}

#[tokio::test]
async fn expired_items_are_not_in_force_and_can_still_be_listed_on_request() {
    let f = fixture();
    let t = tenant("a");
    let mut temp = new(Scope::Global, "The freeze ends soon.", None);
    temp.valid_until = Some(Timestamp(NOW.0 + 100));
    f.store.remember(&t, temp, NOW).await.unwrap();
    assert_eq!(f.store.list_memory(&vis("a"), &MemoryFilter::default(), NOW).await.unwrap().len(), 1);
    let later = Timestamp(NOW.0 + 100);
    assert!(f.store.list_memory(&vis("a"), &MemoryFilter::default(), later).await.unwrap().is_empty());
    assert_eq!(
        f.store
            .list_memory(&vis("a"), &MemoryFilter { include_inactive: true, ..Default::default() }, later)
            .await
            .unwrap()
            .len(),
        1
    );
}

#[tokio::test]
async fn editing_confirming_and_using_an_item() {
    let f = fixture();
    let t = tenant("a");
    let id =
        f.store.remember(&t, new(Scope::Global, "Reply in English.", Some("language")), NOW).await.unwrap().item().id;
    let later = Timestamp(NOW.0 + 50);
    let edited = f
        .store
        .update_memory(
            &t,
            id,
            MemoryPatch { content: Some("Reply in Portuguese.".into()), confirm: true, ..Default::default() },
            later,
        )
        .await
        .unwrap();
    assert_eq!(
        (edited.content.as_str(), edited.confirmed_at, edited.updated_at),
        ("Reply in Portuguese.", later, later)
    );
    // The full-text index follows the edit.
    assert!(f.store.text_search(&vis("a"), Corpus::Memory, &terms(&["english"]), 5).await.unwrap().is_empty());
    assert_eq!(f.store.text_search(&vis("a"), Corpus::Memory, &terms(&["portuguese"]), 5).await.unwrap().len(), 1);
    // Clearing and moving.
    let moved = f
        .store
        .update_memory(
            &t,
            id,
            MemoryPatch { subject: Some(None), scope: Some(Scope::Project("/p".into())), ..Default::default() },
            later,
        )
        .await
        .unwrap();
    assert_eq!((moved.subject, moved.scope), (None, Scope::Project("/p".into())));
    assert!(
        f.store
            .update_memory(&t, id, MemoryPatch { content: Some(" ".into()), ..Default::default() }, later)
            .await
            .is_err()
    );
    f.store.mark_used(&t, &[id, id], later).await.unwrap();
    let used = f.store.memory(&t, id).await.unwrap().unwrap();
    assert_eq!((used.use_count, used.last_used_at), (2, Some(later)));
    assert!(
        f.store.update_memory(&tenant("b"), id, MemoryPatch::default(), later).await.is_err(),
        "another tenant cannot edit it"
    );
}

#[tokio::test]
async fn search_never_lets_query_syntax_through_and_counts_matched_terms() {
    let f = fixture();
    let t = tenant("a");
    f.store.remember(&t, new(Scope::Global, "Deploys go through the staging cluster first.", None), NOW).await.unwrap();
    f.store.remember(&t, new(Scope::Global, "Use snake_case names in Rust code.", None), NOW).await.unwrap();
    for hostile in ["NEAR(", "\"", "*", "a OR", "-x", "col:foo", "(", "'; DROP TABLE memory_items; --", "\u{0}"] {
        f.store
            .text_search(&vis("a"), Corpus::Memory, &[hostile.to_string()], 5)
            .await
            .unwrap_or_else(|e| panic!("{hostile:?}: {e}"));
    }
    assert!(f.store.text_search(&vis("a"), Corpus::Memory, &[], 5).await.unwrap().is_empty());
    assert!(f.store.text_search(&vis("a"), Corpus::Memory, &terms(&[" ", ""]), 5).await.unwrap().is_empty());
    let hits =
        f.store.text_search(&vis("a"), Corpus::Memory, &terms(&["deploys", "staging", "unrelated"]), 5).await.unwrap();
    assert_eq!(hits.len(), 1);
    assert_eq!((hits[0].rank, hits[0].matched_terms), (1, 2));
    assert!(hits[0].score > 0.0);
    // Identifiers with underscores are one word, and accents do not matter.
    assert_eq!(f.store.text_search(&vis("a"), Corpus::Memory, &terms(&["snake_case"]), 5).await.unwrap().len(), 1);
    f.store.remember(&t, new(Scope::Global, "A decisão foi adiada.", None), NOW).await.unwrap();
    assert_eq!(f.store.text_search(&vis("a"), Corpus::Memory, &terms(&["decisao"]), 5).await.unwrap().len(), 1);
    // Superseded items are not found.
    let id =
        f.store.remember(&t, new(Scope::Global, "Old plan: waterfall.", Some("plan")), NOW).await.unwrap().item().id;
    f.store.remember(&t, new(Scope::Global, "New plan: kanban.", Some("plan")), NOW).await.unwrap();
    assert!(f.store.text_search(&vis("a"), Corpus::Memory, &terms(&["waterfall"]), 5).await.unwrap().is_empty());
    assert!(f.store.memory(&t, id).await.unwrap().is_some());
}

#[tokio::test]
async fn vectors_are_keyed_by_text_and_searched_by_cosine() {
    let f = fixture();
    let t = tenant("a");
    let cats = f.store.remember(&t, new(Scope::Global, "Cats purr.", None), NOW).await.unwrap().item().clone();
    let cars = f.store.remember(&t, new(Scope::Global, "Cars have engines.", None), NOW).await.unwrap().item().clone();
    let (h1, h2) = (text_hash("Cats purr."), text_hash("Cars have engines."));
    assert_eq!(
        f.store.missing_vectors("m", &[h1.clone(), h2.clone(), h1.clone()]).await.unwrap(),
        vec![h1.clone(), h2.clone()]
    );
    assert_eq!(f.store.vector_coverage(&t, "m").await.unwrap(), (0, 2));
    f.store.put_vectors("m", vec![(h1.clone(), unit(&[1.0, 0.1])), (h2.clone(), unit(&[0.1, 1.0]))]).await.unwrap();
    assert!(f.store.missing_vectors("m", std::slice::from_ref(&h1)).await.unwrap().is_empty());
    assert_eq!(
        f.store.missing_vectors("other", std::slice::from_ref(&h1)).await.unwrap(),
        vec![h1.clone()],
        "vectors belong to a model"
    );
    assert_eq!(f.store.vector_coverage(&t, "m").await.unwrap(), (2, 2));
    let hits = f.store.vector_search(&vis("a"), Corpus::Memory, "m", &unit(&[1.0, 0.0]), 5).await.unwrap();
    assert_eq!(hits.iter().map(|h| h.id.clone()).collect::<Vec<_>>(), vec![cats.id.to_string(), cars.id.to_string()]);
    assert!(hits[0].cosine > 0.99 && hits[0].cosine > hits[1].cosine && hits[0].rank == 1);
    assert_eq!(
        f.store.vector_search(&vis("a"), Corpus::Memory, "m", &unit(&[1.0, 0.0]), 1).await.unwrap().len(),
        1,
        "limit applies"
    );
    // A query of another size matches nothing; another tenant sees nothing.
    assert!(f.store.vector_search(&vis("a"), Corpus::Memory, "m", &[1.0, 0.0, 0.0], 5).await.unwrap().is_empty());
    assert!(f.store.vector_search(&vis("b"), Corpus::Memory, "m", &unit(&[1.0, 0.0]), 5).await.unwrap().is_empty());
    // Bad vectors are refused.
    assert!(f.store.put_vectors("m", vec![("x".into(), vec![])]).await.is_err());
    assert!(f.store.put_vectors("m", vec![("x".into(), vec![f32::NAN])]).await.is_err());
    // Switching models: the old model's vectors go, the rest stay.
    f.store.put_vectors("old", vec![(h1.clone(), unit(&[1.0, 0.0]))]).await.unwrap();
    assert_eq!(f.store.gc_vectors(Some("m")).await.unwrap(), 1);
    assert_eq!(f.store.vector_coverage(&t, "m").await.unwrap(), (2, 2));
}

#[tokio::test]
async fn knowledge_files_are_indexed_searched_cited_and_replaced_when_they_change() {
    let f = fixture();
    let root = add_root(&f.store, "a", Scope::Global, "/docs").await;
    f.store
        .write_document(doc(
            root,
            "guide/deploy.md",
            vec![
                chunk("Deploys go through staging then production.", 1),
                chunk("Rollbacks use the previous image.", 10),
            ],
        ))
        .await
        .unwrap();
    f.store.write_document(doc(root, "notes.txt", vec![chunk("Lunch is at noon.", 1)])).await.unwrap();
    let hits = f.store.text_search(&vis("a"), Corpus::Knowledge, &terms(&["staging", "production"]), 5).await.unwrap();
    assert_eq!(hits.len(), 1);
    let id: i64 = hits[0].id.parse().unwrap();
    let chunks = f.store.chunks(&tenant("a"), &[id]).await.unwrap();
    assert_eq!((chunks[0].citation(), chunks[0].root_path.as_str()), ("guide/deploy.md:L1-L6".to_string(), "/docs"));
    assert!(f.store.chunks(&tenant("b"), &[id]).await.unwrap().is_empty(), "another tenant cannot read a chunk");
    // The file changes: old pieces go, new ones come, and search follows.
    f.store
        .write_document(doc(root, "guide/deploy.md", vec![chunk("Deploys now use blue green releases.", 1)]))
        .await
        .unwrap();
    assert!(f.store.text_search(&vis("a"), Corpus::Knowledge, &terms(&["staging"]), 5).await.unwrap().is_empty());
    assert_eq!(f.store.text_search(&vis("a"), Corpus::Knowledge, &terms(&["blue"]), 5).await.unwrap().len(), 1);
    let states = f.store.document_states(root).await.unwrap();
    assert_eq!(states.len(), 2);
    assert_eq!(states["notes.txt"].status, DocStatus::Indexed);
    // A failed file is reported with its reason and is not searched.
    let mut bad = doc(root, "broken.pdf", vec![]);
    bad.state.status = DocStatus::Failed;
    bad.error = Some("no text found".into());
    f.store.write_document(bad).await.unwrap();
    assert_eq!(
        f.store.problem_documents(root).await.unwrap(),
        vec![("broken.pdf".to_string(), DocStatus::Failed, "no text found".to_string())]
    );
    // Vanished files are forgotten.
    f.store.remove_documents(root, &["notes.txt".into(), "missing.txt".into()]).await.unwrap();
    assert!(f.store.text_search(&vis("a"), Corpus::Knowledge, &terms(&["lunch"]), 5).await.unwrap().is_empty());
}

#[tokio::test]
async fn project_folders_are_visible_only_to_their_project_and_limits_hold() {
    let f = fixture();
    let g = add_root(&f.store, "a", Scope::Global, "/shared").await;
    let p = add_root(&f.store, "a", Scope::Project("/work/one".into()), "/work/one/docs").await;
    f.store.write_document(doc(g, "g.md", vec![chunk("shared handbook text", 1)])).await.unwrap();
    f.store.write_document(doc(p, "p.md", vec![chunk("project handbook text", 1)])).await.unwrap();
    let one = Visibility { tenant: tenant("a"), project: Some("/work/one".into()), conversation: None, agent: None };
    let two = Visibility { tenant: tenant("a"), project: Some("/work/two".into()), conversation: None, agent: None };
    assert_eq!(f.store.text_search(&one, Corpus::Knowledge, &terms(&["handbook"]), 5).await.unwrap().len(), 2);
    assert_eq!(f.store.text_search(&two, Corpus::Knowledge, &terms(&["handbook"]), 5).await.unwrap().len(), 1);
    assert_eq!(f.store.roots(&one).await.unwrap().len(), 2);
    assert_eq!(f.store.roots(&two).await.unwrap().len(), 1);
    assert!(f.store.roots(&vis("b")).await.unwrap().is_empty());
    // A disabled folder is not searched.
    let mut root = f.store.all_roots(&tenant("a")).await.unwrap().into_iter().find(|r| r.id == g).unwrap();
    root.enabled = false;
    f.store.update_root(&tenant("a"), root).await.unwrap();
    assert_eq!(f.store.text_search(&one, Corpus::Knowledge, &terms(&["handbook"]), 5).await.unwrap().len(), 1);
    // Rules: no duplicate paths, no conversation-scoped folders, a cap on pieces of one file.
    assert!(
        f.store
            .add_root(
                &tenant("a"),
                NewRoot {
                    scope: Scope::Global,
                    path: "/shared".into(),
                    include: vec![],
                    exclude: vec![],
                    max_file_bytes: 1,
                    auto_sync: false
                },
                NOW
            )
            .await
            .is_err()
    );
    assert!(
        f.store
            .add_root(
                &tenant("a"),
                NewRoot {
                    scope: Scope::Conversation(ConversationId::new("c").unwrap()),
                    path: "/c".into(),
                    include: vec![],
                    exclude: vec![],
                    max_file_bytes: 1,
                    auto_sync: false
                },
                NOW
            )
            .await
            .is_err()
    );
    let many: Vec<NewChunk> = (0..2_001).map(|i| chunk(&format!("piece {i}"), i)).collect();
    assert!(f.store.write_document(doc(g, "huge.md", many)).await.is_err());
    // Another tenant cannot remove or change it.
    assert!(!f.store.remove_root(&tenant("b"), p).await.unwrap());
    assert!(f.store.finish_sync(&tenant("b"), p, NOW, json!({})).await.is_ok());
    assert_eq!(
        f.store.all_roots(&tenant("a")).await.unwrap().iter().filter(|r| r.last_sync_at.is_some()).count(),
        0,
        "a stranger's finish_sync changed nothing"
    );
}

#[tokio::test]
async fn chunks_without_a_vector_are_listed_until_embedded() {
    let f = fixture();
    let root = add_root(&f.store, "a", Scope::Global, "/docs").await;
    f.store
        .write_document(doc(root, "a.md", vec![chunk("alpha", 1), chunk("beta", 5), chunk("alpha", 9)]))
        .await
        .unwrap();
    let todo = f.store.chunk_hashes_without_vector(root, "m", 10).await.unwrap();
    assert_eq!(todo.len(), 2, "identical text is embedded once");
    // Counting says the same without reading the texts (a sync of a large folder used to load every one of them to count them).
    assert_eq!(f.store.count_chunks_without_vector(root, "m").await.unwrap(), 2);
    f.store.put_vectors("m", vec![(text_hash("alpha"), unit(&[1.0, 0.0]))]).await.unwrap();
    assert_eq!(f.store.count_chunks_without_vector(root, "m").await.unwrap(), 1);
    assert_eq!(f.store.count_chunks_without_vector(root, "another-model").await.unwrap(), 2, "per model");
    assert_eq!(
        f.store.chunk_hashes_without_vector(root, "m", 10).await.unwrap(),
        vec![(text_hash("beta"), "beta".to_string())]
    );
    assert_eq!(f.store.chunk_hashes_without_vector(root, "m", 0).await.unwrap().len(), 0);
    f.store.put_vectors("m", vec![(text_hash("beta"), unit(&[0.0, 1.0]))]).await.unwrap();
    assert_eq!(f.store.count_chunks_without_vector(root, "m").await.unwrap(), 0);
}

fn snapshot(execution: ExecutionId, t: &str, items: &[&str]) -> RecallSnapshot {
    RecallSnapshot {
        execution_id: execution,
        tenant: tenant(t),
        created_at: NOW,
        items: items.iter().map(|s| s.to_string()).collect(),
        doc: json!({"query": "q", "items": items.iter().map(|i| json!({"id": i, "text": format!("text of {i}")})).collect::<Vec<_>>()}),
    }
}

#[tokio::test]
async fn the_first_snapshot_of_an_execution_stands() {
    let f = fixture();
    let e = ExecutionId::new();
    let first = f.store.save_snapshot(snapshot(e, "a", &["m:1", "k:2"])).await.unwrap();
    let second = f.store.save_snapshot(snapshot(e, "a", &["m:9"])).await.unwrap();
    assert_eq!(second, first, "the second writer gets the first one back");
    assert_eq!(second.items, vec!["k:2".to_string(), "m:1".to_string()]);
    assert_eq!(f.store.snapshot(e).await.unwrap().unwrap(), first);
    assert_eq!(f.store.snapshots_using(&tenant("a"), "m:1").await.unwrap(), vec![e]);
    assert!(f.store.snapshots_using(&tenant("b"), "m:1").await.unwrap().is_empty());
    assert!(f.store.snapshot(ExecutionId::new()).await.unwrap().is_none());
}

#[tokio::test]
async fn forgetting_removes_the_item_from_every_place_it_was_kept() {
    let f = fixture();
    let t = tenant("a");
    let secret = "zebracorn-quartz-7731";
    let id = f
        .store
        .remember(&t, new(Scope::Global, &format!("The launch codename is {secret}."), None), NOW)
        .await
        .unwrap()
        .item()
        .id;
    f.store
        .put_vectors("m", vec![(text_hash(&format!("The launch codename is {secret}.")), unit(&[1.0, 0.0]))])
        .await
        .unwrap();
    let e = ExecutionId::new();
    f.store.save_snapshot(snapshot(e, "a", &[&memory_key(id), "m:other"])).await.unwrap();
    assert_eq!(f.store.snapshots_using(&t, &memory_key(id)).await.unwrap(), vec![e]);

    // The check below can see the text while it is stored (otherwise it would prove nothing).
    assert!(file_holds(&f.path, secret), "the text should be in the database before it is forgotten");

    assert!(!f.store.forget(&tenant("b"), id).await.unwrap(), "another tenant cannot forget it");
    assert!(f.store.forget(&t, id).await.unwrap());
    assert!(!f.store.forget(&t, id).await.unwrap());

    assert!(f.store.memory(&t, id).await.unwrap().is_none());
    assert!(f.store.text_search(&vis("a"), Corpus::Memory, &terms(&["zebracorn"]), 5).await.unwrap().is_empty());
    assert_eq!(f.store.vector_coverage(&t, "m").await.unwrap(), (0, 0));
    let snap = f.store.snapshot(e).await.unwrap().unwrap();
    assert_eq!(snap.items, vec!["m:other".to_string()]);
    assert!(!snap.doc.to_string().contains("zebracorn") && snap.doc["scrubbed"] == json!(true));
    assert!(f.store.snapshots_using(&t, &memory_key(id)).await.unwrap().is_empty());

    // Not even the file holds it any more (the log is checkpointed and freed pages are overwritten).
    assert!(!file_holds(&f.path, secret), "the text is still in the database file");
}

/// True when the text appears in the database file or its log.
fn file_holds(path: &std::path::Path, text: &str) -> bool {
    let mut bytes = std::fs::read(path).unwrap_or_default();
    if let Ok(wal) = std::fs::read(path.with_extension("sqlite-wal")) {
        bytes.extend(wal);
    }
    bytes.windows(text.len()).any(|w| w == text.as_bytes())
}

#[tokio::test]
async fn removing_a_folder_removes_its_index_but_nothing_else() {
    let f = fixture();
    let keep = add_root(&f.store, "a", Scope::Global, "/keep").await;
    let drop = add_root(&f.store, "a", Scope::Global, "/drop").await;
    f.store.write_document(doc(keep, "k.md", vec![chunk("kept handbook", 1)])).await.unwrap();
    f.store.write_document(doc(drop, "d.md", vec![chunk("dropped handbook zebracorn", 1)])).await.unwrap();
    let hit = f.store.text_search(&vis("a"), Corpus::Knowledge, &terms(&["zebracorn"]), 5).await.unwrap();
    let key = chunk_key(hit[0].id.parse().unwrap());
    let e = ExecutionId::new();
    f.store.save_snapshot(snapshot(e, "a", &[&key])).await.unwrap();
    f.store
        .put_vectors(
            "m",
            vec![
                (text_hash("dropped handbook zebracorn"), unit(&[1.0, 0.0])),
                (text_hash("kept handbook"), unit(&[0.0, 1.0])),
            ],
        )
        .await
        .unwrap();

    assert!(f.store.remove_root(&tenant("a"), drop).await.unwrap());
    assert!(f.store.text_search(&vis("a"), Corpus::Knowledge, &terms(&["zebracorn"]), 5).await.unwrap().is_empty());
    assert_eq!(f.store.text_search(&vis("a"), Corpus::Knowledge, &terms(&["handbook"]), 5).await.unwrap().len(), 1);
    assert_eq!(f.store.vector_coverage(&tenant("a"), "m").await.unwrap(), (1, 1));
    assert!(f.store.snapshot(e).await.unwrap().unwrap().items.is_empty());
    assert_eq!(f.store.all_roots(&tenant("a")).await.unwrap().len(), 1);
    assert!(f.store.document_states(drop).await.unwrap().is_empty());
}

/// The vector search over documents honours the scope of the folder like the text search does: a run in one project never gets the
/// documents of another project, nor those that belong to an agent that is not its own (finding F12 of the Phase 11 audit).
#[tokio::test]
async fn the_vector_search_over_documents_keeps_projects_and_agents_apart() {
    let f = fixture();
    let (mine, theirs) = (zorix_core::AgentId::new(), zorix_core::AgentId::new());
    let roots = [
        ("global", Scope::Global, "The deploy guide everyone may read.", [1.0, 0.0, 0.0]),
        ("one", Scope::Project("/work/one".into()), "The deploy notes of project one.", [0.9, 0.1, 0.0]),
        ("two", Scope::Project("/work/two".into()), "The deploy notes of project two.", [0.8, 0.2, 0.0]),
        ("mine", Scope::Agent(mine), "The deploy notes of my agent.", [0.7, 0.3, 0.0]),
        ("theirs", Scope::Agent(theirs), "The deploy notes of another agent.", [0.6, 0.4, 0.0]),
    ];
    let mut by_text = std::collections::HashMap::new();
    for (name, scope, text, vector) in roots {
        let root = add_root(&f.store, "a", scope, &format!("/docs/{name}")).await;
        f.store.write_document(doc(root, "deploy.md", vec![chunk(text, 1)])).await.unwrap();
        f.store.put_vectors("m", vec![(text_hash(text), unit(&vector))]).await.unwrap();
        by_text.insert(text, name);
    }
    let search = |visibility: Visibility| {
        let store = f.store.clone();
        let by_text = by_text.clone();
        async move {
            let hits =
                store.vector_search(&visibility, Corpus::Knowledge, "m", &unit(&[1.0, 0.0, 0.0]), 10).await.unwrap();
            let ids: Vec<i64> = hits.iter().map(|h| h.id.parse().unwrap()).collect();
            let chunks = store.chunks(&visibility.tenant, &ids).await.unwrap();
            let mut names: Vec<&str> = chunks.iter().map(|c| by_text[c.text.as_str()]).collect();
            names.sort_unstable();
            names
        }
    };
    let project_one = Visibility { project: Some("/work/one".into()), agent: Some(mine), ..vis("a") };
    assert_eq!(search(project_one).await, ["global", "mine", "one"], "its project, its agent, and what is global");
    assert_eq!(search(vis("a")).await, ["global"], "a run with no project and no agent sees only what is global");
    let project_two = Visibility { project: Some("/work/two".into()), ..vis("a") };
    assert_eq!(search(project_two).await, ["global", "two"]);
    assert!(
        f.store.vector_search(&vis("b"), Corpus::Knowledge, "m", &unit(&[1.0, 0.0, 0.0]), 10).await.unwrap().is_empty(),
        "another tenant finds nothing"
    );
}

/// Writing a file again replaces its pieces, and the old ones must leave the frozen recall of past runs like a removed file's do:
/// otherwise removing the folder later would still leave an old version of its text in them (finding F5 of the Phase 11 audit).
#[tokio::test]
async fn writing_a_file_again_scrubs_its_old_pieces_from_frozen_snapshots() {
    let f = fixture();
    let root = add_root(&f.store, "a", Scope::Global, "/docs").await;
    f.store.write_document(doc(root, "plan.md", vec![chunk("the zebracorn launch plan", 1)])).await.unwrap();
    let hit = f.store.text_search(&vis("a"), Corpus::Knowledge, &terms(&["zebracorn"]), 5).await.unwrap();
    let old_key = chunk_key(hit[0].id.parse().unwrap());
    let e = ExecutionId::new();
    f.store.save_snapshot(snapshot(e, "a", &[&old_key, "m:other"])).await.unwrap();

    // The file changes and is indexed again.
    f.store.write_document(doc(root, "plan.md", vec![chunk("the revised plan", 1)])).await.unwrap();

    assert!(f.store.text_search(&vis("a"), Corpus::Knowledge, &terms(&["zebracorn"]), 5).await.unwrap().is_empty());
    let snap = f.store.snapshot(e).await.unwrap().unwrap();
    assert_eq!(snap.items, vec!["m:other".to_string()], "the old piece left the snapshot");
    assert_eq!(snap.doc["scrubbed"], json!(true));
    assert!(
        f.store.snapshots_using(&tenant("a"), &old_key).await.unwrap().is_empty(),
        "and nothing refers to it any more"
    );
}

fn record(e: ExecutionId, t: &str, project: Option<&str>, text: &str) -> OperationRecord {
    OperationRecord {
        execution_id: e,
        tenant: tenant(t),
        project: project.map(str::to_string),
        conversation: None,
        agent: None,
        state: "completed".into(),
        finished_at: NOW,
        text: text.into(),
        doc: json!({"tools": ["math.evaluate"]}),
    }
}

#[tokio::test]
async fn past_work_that_belongs_to_an_agent_is_seen_by_that_agent_and_by_no_one_else() {
    let f = fixture();
    let (mine, theirs, nobodys) = (ExecutionId::new(), ExecutionId::new(), ExecutionId::new());
    let (me, them) = (zorix_core::AgentId::new(), zorix_core::AgentId::new());
    for (id, agent, text) in [
        (mine, Some(me), "billing work of my agent"),
        (theirs, Some(them), "billing work of another agent"),
        (nobodys, None, "billing work of a plain run"),
    ] {
        f.store.record_operation(OperationRecord { agent, ..record(id, "a", None, text) }).await.unwrap();
    }
    assert_eq!(f.store.operation(&tenant("a"), mine).await.unwrap().unwrap().agent, Some(me), "the agent is kept");
    let ids = |hits: Vec<zorix_core::TextHit>| {
        let mut ids: Vec<String> = hits.into_iter().map(|h| h.id).collect();
        ids.sort();
        ids
    };
    let sorted = |mut v: Vec<ExecutionId>| {
        v.sort_by_key(ToString::to_string);
        v.into_iter().map(|id| id.to_string()).collect::<Vec<_>>()
    };
    let as_agent = Visibility { agent: Some(me), ..vis("a") };
    let search = |v: Visibility| {
        let store = f.store.clone();
        async move { ids(store.text_search(&v, Corpus::Operations, &terms(&["billing"]), 10).await.unwrap()) }
    };
    assert_eq!(search(as_agent).await, sorted(vec![mine, nobodys]), "an agent sees its own work and what nobody owns");
    assert_eq!(search(vis("a")).await, sorted(vec![nobodys]), "a plain run sees only what nobody owns");
    let other = Visibility { agent: Some(them), ..vis("a") };
    assert_eq!(search(other).await, sorted(vec![theirs, nobodys]), "another agent does not see mine");
}

#[tokio::test]
async fn operational_records_are_searchable_by_project_and_replaced_when_recorded_again() {
    let f = fixture();
    let (e1, e2, e3) = (ExecutionId::new(), ExecutionId::new(), ExecutionId::new());
    f.store.record_operation(record(e1, "a", Some("/work/one"), "migrated the billing database")).await.unwrap();
    f.store.record_operation(record(e2, "a", None, "billing report without a workspace")).await.unwrap();
    f.store.record_operation(record(e3, "b", None, "billing for another tenant")).await.unwrap();
    let one = Visibility { tenant: tenant("a"), project: Some("/work/one".into()), conversation: None, agent: None };
    let ids = |hits: Vec<zorix_core::TextHit>| hits.into_iter().map(|h| h.id).collect::<Vec<_>>();
    assert_eq!(
        ids(f.store.text_search(&one, Corpus::Operations, &terms(&["billing"]), 5).await.unwrap()),
        vec![e1.to_string()]
    );
    assert_eq!(
        ids(f.store.text_search(&vis("a"), Corpus::Operations, &terms(&["billing"]), 5).await.unwrap()),
        vec![e2.to_string()]
    );
    f.store.record_operation(record(e1, "a", Some("/work/one"), "rewrote the invoices module")).await.unwrap();
    assert!(
        f.store.text_search(&one, Corpus::Operations, &terms(&["billing"]), 5).await.unwrap().is_empty(),
        "the old text left the index"
    );
    assert_eq!(f.store.text_search(&one, Corpus::Operations, &terms(&["invoices"]), 5).await.unwrap().len(), 1);
    assert_eq!(f.store.operation(&tenant("a"), e1).await.unwrap().unwrap().doc, json!({"tools": ["math.evaluate"]}));
    assert!(f.store.operation(&tenant("b"), e1).await.unwrap().is_none());
    assert!(!f.store.forget_operation(&tenant("b"), e1).await.unwrap());
    assert!(f.store.forget_operation(&tenant("a"), e1).await.unwrap());
    assert!(f.store.text_search(&one, Corpus::Operations, &terms(&["invoices"]), 5).await.unwrap().is_empty());
}

#[tokio::test]
async fn finished_executions_without_a_record_are_listed_oldest_first() {
    use zorix_core::{Execution, ExecutionRepository, NewEvent, NewExecution, TaskRequest};
    let f = fixture();
    let repository = SqliteRepository::open(&f.path).unwrap();
    let mut ids = Vec::new();
    for (n, state) in [
        (1, zorix_core::ExecutionState::Completed),
        (2, zorix_core::ExecutionState::Running),
        (3, zorix_core::ExecutionState::Failed),
    ] {
        let new = NewExecution {
            request: TaskRequest { prompt: format!("task {n}"), ..TaskRequest::default() },
            ..NewExecution::default()
        };
        let mut execution = Execution::new(ExecutionId::new(), new, Timestamp(n));
        execution.state = state;
        repository.create(execution.clone(), Vec::<NewEvent>::new()).await.unwrap();
        ids.push(execution.id);
    }
    let missing = f.store.operations_missing(10).await.unwrap();
    assert_eq!(missing, vec![ids[0], ids[2]], "only finished ones, oldest first");
    f.store.record_operation(record(ids[0], "local", None, "done")).await.unwrap();
    assert_eq!(f.store.operations_missing(10).await.unwrap(), vec![ids[2]]);
    assert_eq!(f.store.operations_missing(0).await.unwrap(), vec![]);
}

#[tokio::test]
async fn settings_are_per_tenant_and_default_to_keyword_search_only() {
    let f = fixture();
    assert_eq!(f.store.settings(&tenant("a")).await.unwrap(), MemorySettings::default());
    let s = MemorySettings {
        embedding_model: Some("ollama/all-minilm".into()),
        embedding_dimensions: None,
        allow_cloud_embeddings: false,
    };
    f.store.set_settings(&tenant("a"), s.clone()).await.unwrap();
    f.store.set_settings(&tenant("a"), MemorySettings { allow_cloud_embeddings: true, ..s.clone() }).await.unwrap();
    assert!(f.store.settings(&tenant("a")).await.unwrap().allow_cloud_embeddings);
    assert_eq!(f.store.settings(&tenant("b")).await.unwrap(), MemorySettings::default());
}

#[tokio::test]
async fn too_many_active_items_are_refused_but_replacing_one_is_allowed_at_the_limit() {
    let f = fixture();
    let t = tenant("a");
    // Fill to the limit cheaply with one transaction-sized loop.
    for i in 0..zorix_store::memory::MAX_ACTIVE_ITEMS {
        f.store.remember(&t, new(Scope::Global, &format!("fact number {i}"), None), NOW).await.unwrap();
    }
    assert!(f.store.remember(&t, new(Scope::Global, "one too many", None), NOW).await.is_err());
    let first =
        f.store.list_memory(&vis("a"), &MemoryFilter { limit: 1, ..Default::default() }, NOW).await.unwrap()[0].id;
    let mut swap = new(Scope::Global, "a replacement", None);
    swap.replaces = vec![first];
    assert!(matches!(f.store.remember(&t, swap, NOW).await.unwrap(), Remembered::Replaced { .. }));
}

#[tokio::test]
async fn everything_of_a_tenant_can_be_listed_and_the_texts_missing_a_vector_found() {
    let f = fixture();
    let t = tenant("a");
    f.store.remember(&t, new(Scope::Global, "global fact about invoices", None), NOW).await.unwrap();
    f.store.remember(&t, new(Scope::Project("/p".into()), "project fact about billing", None), NOW).await.unwrap();
    f.store.remember(&tenant("b"), new(Scope::Global, "someone else", None), NOW).await.unwrap();
    let old = f.store.remember(&t, new(Scope::Global, "old plan", Some("plan")), NOW).await.unwrap().item().id;
    f.store.remember(&t, new(Scope::Global, "new plan", Some("plan")), NOW).await.unwrap();
    let active = f.store.all_memory(&t, &MemoryFilter::default(), NOW).await.unwrap();
    assert_eq!(active.len(), 3, "every scope of the tenant, nothing of another, no replaced item");
    let all =
        f.store.all_memory(&t, &MemoryFilter { include_inactive: true, ..Default::default() }, NOW).await.unwrap();
    assert_eq!(all.len(), 4);
    assert!(all.iter().any(|i| i.id == old));
    f.store.record_operation(record(ExecutionId::new(), "a", None, "migrated the billing database")).await.unwrap();
    let missing = f.store.texts_without_vector(&t, "m", 100).await.unwrap();
    assert_eq!(missing.len(), 4, "three facts and one record: {missing:?}");
    f.store.put_vectors("m", vec![(text_hash("global fact about invoices"), unit(&[1.0, 0.0]))]).await.unwrap();
    assert_eq!(f.store.texts_without_vector(&t, "m", 100).await.unwrap().len(), 3);
    assert_eq!(f.store.texts_without_vector(&t, "m", 1).await.unwrap().len(), 1, "limit applies");
    assert!(f.store.texts_without_vector(&tenant("nobody"), "m", 10).await.unwrap().is_empty());
}
