//! Several connections to one file, writing at the same time: no update is lost, no event is numbered twice, and the only error a
//! writer ever sees is the conflict it is meant to retry.

use std::collections::HashSet;

use zorix_core::{Commit, CreateOutcome, Error, EventKind, ExecutionRepository, TenantId};
use zorix_runtime::testing::{event, sample_execution};
use zorix_store::SqliteRepository;

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn many_connections_writing_at_once_lose_nothing_and_number_every_event_once() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("zorix.sqlite");
    let first = SqliteRepository::open(&path).unwrap();
    let shared = sample_execution("local", None);
    let shared_id = shared.id;
    first.create(shared, vec![event(EventKind::Created)]).await.unwrap();

    const WRITERS: usize = 4;
    const ROUNDS: usize = 25;
    let mut tasks = Vec::new();
    for writer in 0..WRITERS {
        let path = path.clone();
        tasks.push(tokio::spawn(async move {
            let repo = SqliteRepository::open(&path).unwrap();
            let tenant = TenantId::local();
            let mut own = Vec::new();
            let mut conflicts = 0u32;
            for round in 0..ROUNDS {
                // An execution of its own, created and then advanced.
                let mut mine = sample_execution("local", Some(&format!("w{writer}-r{round}")));
                mine.request.prompt = format!("writer {writer} round {round}");
                let id = mine.id;
                let CreateOutcome::Created(_) = repo.create(mine, vec![event(EventKind::Created)]).await.unwrap()
                else {
                    panic!("a new key makes a new execution");
                };
                own.push(id);

                // One execution all of them want to change: each success adds one to the counter, a conflict is read again.
                loop {
                    let mut current = repo.get(&tenant, shared_id).await.unwrap().unwrap();
                    let version = current.version;
                    current.iteration += 1;
                    let mut commit = Commit::new(current, version);
                    commit.events = vec![event(EventKind::Progress)];
                    match repo.commit(commit).await {
                        Ok(_) => break,
                        Err(Error::Conflict(_)) => conflicts += 1,
                        Err(other) => panic!("a writer saw an error that is not a conflict: {other}"),
                    }
                }
            }
            (own, conflicts)
        }));
    }
    let mut created = Vec::new();
    let mut conflicts = 0;
    for task in tasks {
        let (own, c) = task.await.unwrap();
        created.extend(own);
        conflicts += c;
    }

    let tenant = TenantId::local();
    let shared = first.get(&tenant, shared_id).await.unwrap().unwrap();
    assert_eq!(shared.iteration as usize, WRITERS * ROUNDS, "no update was lost ({conflicts} conflicts were retried)");
    assert_eq!(shared.version as usize, 1 + WRITERS * ROUNDS);
    assert_eq!(created.len(), WRITERS * ROUNDS);
    for id in &created {
        assert!(first.get(&tenant, *id).await.unwrap().is_some(), "{id} was created");
    }
    // Every event of the shared execution exists once, and the numbers across the whole file are unique.
    let events = first.events(shared_id, 0, 10_000).await.unwrap();
    assert_eq!(events.len(), 1 + WRITERS * ROUNDS);
    assert!(events.windows(2).all(|w| w[0].seq < w[1].seq));
    let mut seen = HashSet::new();
    for id in created.iter().chain([&shared_id]) {
        for e in first.events(*id, 0, 10_000).await.unwrap() {
            assert!(seen.insert(e.seq), "event number {} was used twice", e.seq);
        }
    }
    let conn = rusqlite::Connection::open(&path).unwrap();
    let check: String = conn.query_row("PRAGMA integrity_check", [], |r| r.get(0)).unwrap();
    assert_eq!(check, "ok");
}
