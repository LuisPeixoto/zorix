//! Contract tests for the project store: keys are unique and task numbers never reused, a task is written with a revision and its
//! events together, removing a task releases its dependants, history is numbered per project, and tenants never mix.

use zorix_core::{
    AgentSlug, Project, ProjectEventDraft, ProjectEventKind, ProjectKey, ProjectStore, Task, TaskKey, TaskState,
    TenantId, Timestamp, Who,
};
use zorix_store::SqliteProjectStore;

fn tenant(name: &str) -> TenantId {
    TenantId::new(name).unwrap()
}

const NOW: Timestamp = Timestamp(1_000);

fn key(text: &str) -> ProjectKey {
    ProjectKey::new(text).unwrap()
}

async fn project(store: &SqliteProjectStore, who: &TenantId, k: &str) -> Project {
    store
        .create_project(who, Project::new(who.clone(), key(k), format!("Project {k}"), NOW), Some(Who::You))
        .await
        .unwrap()
}

async fn task(store: &SqliteProjectStore, who: &TenantId, project: &Project, title: &str) -> Task {
    let number = store.next_task_number(who, project.id).await.unwrap();
    let t = Task::new(who.clone(), project.id, TaskKey::new(project.key.clone(), number).unwrap(), title, NOW);
    let event = ProjectEventDraft {
        task: Some(t.key.clone()),
        by: Some(Who::You),
        kind: ProjectEventKind::TaskCreated { title: title.into() },
        at: NOW,
    };
    assert!(store.put_task(who, &t, None, &[event]).await.unwrap());
    t
}

#[tokio::test]
async fn a_project_key_is_unique_and_the_project_is_found_by_it() {
    let store = SqliteProjectStore::open_in_memory().unwrap();
    let me = tenant("local");
    let p = project(&store, &me, "OPS").await;
    let again = store.create_project(&me, Project::new(me.clone(), key("OPS"), "Other", NOW), None).await;
    assert!(again.unwrap_err().to_string().contains("already used"));
    assert_eq!(store.project_by_key(&me, &key("OPS")).await.unwrap().unwrap().id, p.id);
    assert_eq!(store.project(&me, p.id).await.unwrap().unwrap().name, "Project OPS");
    assert_eq!(store.projects(&me).await.unwrap().len(), 1);
    // Another tenant may use the same key, and cannot see this project.
    let other = tenant("other");
    project(&store, &other, "OPS").await;
    assert_eq!(store.projects(&other).await.unwrap().len(), 1);
    assert!(store.project(&other, p.id).await.unwrap().is_none());
}

#[tokio::test]
async fn task_numbers_are_never_reused_even_after_a_task_is_removed() {
    let store = SqliteProjectStore::open_in_memory().unwrap();
    let me = tenant("local");
    let p = project(&store, &me, "OPS").await;
    let one = task(&store, &me, &p, "First").await;
    let two = task(&store, &me, &p, "Second").await;
    assert_eq!((one.key.number, two.key.number), (1, 2));
    assert!(store.remove_task(&me, &two.key).await.unwrap());
    let three = task(&store, &me, &p, "Third").await;
    assert_eq!(three.key.number, 3, "2 is not given again");
    // A number reserved and never used is also gone.
    let skipped = store.next_task_number(&me, p.id).await.unwrap();
    assert_eq!(skipped, 4);
    assert_eq!(task(&store, &me, &p, "Fifth").await.key.number, 5);
}

#[tokio::test]
async fn a_task_is_written_only_when_its_revision_is_current() {
    let store = SqliteProjectStore::open_in_memory().unwrap();
    let me = tenant("local");
    let p = project(&store, &me, "OPS").await;
    let t = task(&store, &me, &p, "Triage").await;
    let mut stored = store.task(&me, &t.key).await.unwrap().unwrap();
    assert_eq!(stored.revision, 0);
    stored.state = TaskState::InProgress;
    assert!(store.put_task(&me, &stored, Some(0), &[]).await.unwrap());
    // A second writer that read revision 0 loses.
    let mut late = stored.clone();
    late.title = "Late".into();
    assert!(!store.put_task(&me, &late, Some(0), &[]).await.unwrap());
    let now = store.task(&me, &t.key).await.unwrap().unwrap();
    assert_eq!((now.revision, now.state, now.title.as_str()), (1, TaskState::InProgress, "Triage"));
    // The same key cannot be inserted twice.
    assert!(!store.put_task(&me, &t, None, &[]).await.unwrap());
}

#[tokio::test]
async fn history_is_numbered_per_project_and_filtered_by_task() {
    let store = SqliteProjectStore::open_in_memory().unwrap();
    let me = tenant("local");
    let p = project(&store, &me, "OPS").await;
    let a = task(&store, &me, &p, "A").await;
    let b = task(&store, &me, &p, "B").await;
    store
        .add_events(
            &me,
            p.id,
            &[ProjectEventDraft {
                task: Some(a.key.clone()),
                by: Some(Who::Agent { slug: AgentSlug::new("atlas").unwrap() }),
                kind: ProjectEventKind::Comment { text: "hi".into() },
                at: Timestamp(2_000),
            }],
        )
        .await
        .unwrap();
    let all = store.events(&me, p.id, None, 50).await.unwrap();
    // project created, two task created, one comment; newest first with consecutive numbers
    assert_eq!(all.iter().map(|e| e.seq).collect::<Vec<_>>(), vec![4, 3, 2, 1]);
    assert!(matches!(all[0].kind, ProjectEventKind::Comment { .. }));
    let only_a = store.events(&me, p.id, Some(&a.key), 50).await.unwrap();
    assert_eq!(only_a.len(), 2);
    assert!(only_a.iter().all(|e| e.task.as_ref() == Some(&a.key)));
    assert_eq!(store.events(&me, p.id, Some(&b.key), 50).await.unwrap().len(), 1);
    assert_eq!(store.events(&me, p.id, None, 2).await.unwrap().len(), 2, "a limit applies");
}

#[tokio::test]
async fn removing_a_task_releases_the_tasks_that_waited_for_it() {
    let store = SqliteProjectStore::open_in_memory().unwrap();
    let me = tenant("local");
    let p = project(&store, &me, "OPS").await;
    let first = task(&store, &me, &p, "First").await;
    let mut second = task(&store, &me, &p, "Second").await;
    second.depends_on = vec![first.key.clone()];
    assert!(store.put_task(&me, &second, Some(0), &[]).await.unwrap());
    assert!(store.remove_task(&me, &first.key).await.unwrap());
    let after = store.task(&me, &second.key).await.unwrap().unwrap();
    assert!(after.depends_on.is_empty());
    assert_eq!(after.revision, 2, "the dependant was rewritten");
    assert!(!store.remove_task(&me, &first.key).await.unwrap(), "removing twice says so");
}

#[tokio::test]
async fn lists_counts_and_archived_projects() {
    let store = SqliteProjectStore::open_in_memory().unwrap();
    let me = tenant("local");
    let p = project(&store, &me, "OPS").await;
    let q = project(&store, &me, "WEB").await;
    let mut a = task(&store, &me, &p, "A").await;
    let b = task(&store, &me, &p, "B").await;
    let mut c = task(&store, &me, &q, "C").await;
    a.state = TaskState::InProgress;
    a.rank = 5;
    assert!(store.put_task(&me, &a, Some(0), &[]).await.unwrap());
    c.state = TaskState::InProgress;
    assert!(store.put_task(&me, &c, Some(0), &[]).await.unwrap());
    assert_eq!(store.tasks(&me, p.id).await.unwrap().len(), 2);
    let counts = store.task_counts(&me).await.unwrap();
    let of =
        |project: &Project, state| counts.iter().find(|c| c.project == project.id && c.state == state).map(|c| c.count);
    assert_eq!(
        (of(&p, TaskState::InProgress), of(&p, TaskState::Backlog), of(&q, TaskState::InProgress)),
        (Some(1), Some(1), Some(1))
    );
    assert_eq!(store.tasks_in_state(&me, TaskState::InProgress).await.unwrap().len(), 2);
    // Archiving a project takes its tasks out of the active list.
    let mut archived = q.clone();
    archived.archived = true;
    archived.updated_at = Timestamp(2_000);
    store.update_project(&me, &archived, &[]).await.unwrap();
    assert_eq!(store.tasks_in_state(&me, TaskState::InProgress).await.unwrap().len(), 1);
    // Ranks order the tasks of a project.
    let ordered = store.tasks(&me, p.id).await.unwrap();
    assert_eq!(ordered.iter().map(|t| t.key.number).collect::<Vec<_>>(), vec![b.key.number, a.key.number]);
    // The key and the counter survive an update that tries to change them.
    let mut tampered = p.clone();
    tampered.key = key("ZZZ");
    tampered.next_task = 1;
    tampered.name = "Renamed".into();
    let saved = store.update_project(&me, &tampered, &[]).await.unwrap();
    assert_eq!((saved.key.as_str(), saved.next_task, saved.name.as_str()), ("OPS", 3, "Renamed"));
}

#[tokio::test]
async fn removing_a_project_removes_its_tasks_and_history() {
    let store = SqliteProjectStore::open_in_memory().unwrap();
    let me = tenant("local");
    let p = project(&store, &me, "OPS").await;
    let t = task(&store, &me, &p, "A").await;
    assert!(store.remove_project(&me, p.id).await.unwrap());
    assert!(store.task(&me, &t.key).await.unwrap().is_none());
    assert!(store.events(&me, p.id, None, 10).await.unwrap().is_empty());
    assert!(!store.remove_project(&me, p.id).await.unwrap());
}
