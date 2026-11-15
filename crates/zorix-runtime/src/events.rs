//! The in-process event bus and the "replay, then follow" stream (ADR-0005).
//!
//! The bus only *wakes subscribers up*: what they emit always comes from the repository, so the order is the stored
//! order even when two commits publish at the same time, and a slow subscriber that lags simply reads again.

use std::collections::VecDeque;
use std::sync::Arc;

use futures_util::StreamExt;
use futures_util::stream::{self, BoxStream};
use tokio::sync::broadcast;
use zorix_core::{EventKind, EventPublisher, ExecutionEvent, ExecutionId, ExecutionRepository};

/// How many events a subscriber may fall behind before it has to read again from storage.
const BUS_CAPACITY: usize = 1024;

/// How many stored events one read fetches.
const PAGE: usize = 500;

/// Fans committed events out to subscribers in this process.
#[derive(Clone)]
pub struct EventBus {
    sender: broadcast::Sender<ExecutionEvent>,
}

impl Default for EventBus {
    fn default() -> Self {
        Self::new()
    }
}

impl EventBus {
    /// A bus with no subscribers.
    pub fn new() -> Self {
        Self { sender: broadcast::channel(BUS_CAPACITY).0 }
    }

    /// Receive every event published from now on, for every execution.
    pub fn subscribe(&self) -> broadcast::Receiver<ExecutionEvent> {
        self.sender.subscribe()
    }
}

impl EventPublisher for EventBus {
    fn publish(&self, events: &[ExecutionEvent]) {
        for event in events {
            // No subscribers is normal, not an error.
            let _ = self.sender.send(event.clone());
        }
    }
}

/// True for the event that ends an execution's stream.
pub fn is_final(kind: EventKind) -> bool {
    matches!(kind, EventKind::Completed | EventKind::Failed | EventKind::Cancelled)
}

struct Follow {
    repository: Arc<dyn ExecutionRepository>,
    wakeups: broadcast::Receiver<ExecutionEvent>,
    id: ExecutionId,
    last_seq: u64,
    buffer: VecDeque<ExecutionEvent>,
    caught_up: bool,
    finished: bool,
}

/// Events of one execution after `after_seq`, then live ones, in stored order, each exactly once.
/// The stream ends after the event that completes, fails or cancels the execution.
pub fn follow(
    repository: Arc<dyn ExecutionRepository>,
    bus: &EventBus,
    id: ExecutionId,
    after_seq: u64,
) -> BoxStream<'static, ExecutionEvent> {
    // Subscribe *before* the first read, so nothing committed in between is missed.
    let state = Follow {
        repository,
        wakeups: bus.subscribe(),
        id,
        last_seq: after_seq,
        buffer: VecDeque::new(),
        caught_up: false,
        finished: false,
    };
    stream::unfold(state, |mut s| async move {
        loop {
            if let Some(event) = s.buffer.pop_front() {
                s.last_seq = event.seq;
                if is_final(event.kind) {
                    s.finished = true;
                    s.buffer.clear();
                }
                return Some((event, s));
            }
            if s.finished {
                return None;
            }
            if !s.caught_up {
                match s.repository.events(s.id, s.last_seq, PAGE).await {
                    Ok(page) if !page.is_empty() => s.buffer.extend(page),
                    Ok(_) => s.caught_up = true,
                    Err(error) => {
                        tracing::warn!(%error, "could not read events; ending the stream");
                        return None;
                    }
                }
                continue;
            }
            match s.wakeups.recv().await {
                Ok(event) if event.execution_id == s.id => s.caught_up = false,
                Ok(_) => {}
                Err(broadcast::error::RecvError::Lagged(_)) => s.caught_up = false,
                Err(broadcast::error::RecvError::Closed) => return None,
            }
        }
    })
    .boxed()
}

#[cfg(test)]
mod tests {
    use std::time::Duration;

    use serde_json::json;
    use zorix_core::{Commit, CreateOutcome, NewEvent, Timestamp, Trigger};

    use super::*;
    use crate::MemoryRepository;
    use crate::testing::{event, sample_execution};

    async fn publish(repo: &MemoryRepository, bus: &EventBus, commit: Commit) {
        let done = repo.commit(commit).await.unwrap();
        bus.publish(&done.events);
    }

    #[tokio::test]
    async fn every_subscriber_gets_every_event() {
        let bus = EventBus::new();
        let (mut a, mut b) = (bus.subscribe(), bus.subscribe());
        let repo = MemoryRepository::new();
        let CreateOutcome::Created(done) =
            repo.create(sample_execution("acme", None), vec![event(EventKind::Created)]).await.unwrap()
        else {
            panic!("created")
        };
        bus.publish(&done.events);
        assert_eq!(a.recv().await.unwrap().kind, EventKind::Created);
        assert_eq!(b.recv().await.unwrap().kind, EventKind::Created);
    }

    #[tokio::test]
    async fn publishing_without_subscribers_is_fine() {
        let bus = EventBus::new();
        let repo = MemoryRepository::new();
        let CreateOutcome::Created(done) =
            repo.create(sample_execution("acme", None), vec![event(EventKind::Created)]).await.unwrap()
        else {
            panic!("created")
        };
        bus.publish(&done.events);
    }

    #[tokio::test]
    async fn follow_replays_stored_events_then_live_ones_without_repeats_and_ends_on_the_final_event() {
        let repo = Arc::new(MemoryRepository::new());
        let bus = EventBus::new();
        let mut execution = sample_execution("acme", None);
        let id = execution.id;
        repo.create(execution.clone(), vec![event(EventKind::Created)]).await.unwrap();

        let mut stream = follow(repo.clone(), &bus, id, 0);
        assert_eq!(stream.next().await.unwrap().kind, EventKind::Created, "replayed");

        let writer = {
            let (repo, bus) = (repo.clone(), bus.clone());
            tokio::spawn(async move {
                execution.version = 1;
                execution.apply(Trigger::Enqueue, Timestamp(2)).unwrap();
                let mut commit = Commit::new(execution.clone(), 1);
                commit.events = vec![event(EventKind::Queued)];
                publish(&repo, &bus, commit).await;

                tokio::time::sleep(Duration::from_millis(20)).await;
                execution.version = 2;
                execution.apply(Trigger::Cancel, Timestamp(3)).unwrap();
                let mut commit = Commit::new(execution.clone(), 2);
                commit.events =
                    vec![NewEvent::new(EventKind::Cancelled, Timestamp(3), json!({})), event(EventKind::Progress)];
                publish(&repo, &bus, commit).await;
            })
        };

        let rest: Vec<EventKind> = tokio::time::timeout(Duration::from_secs(5), stream.map(|e| e.kind).collect())
            .await
            .expect("the stream ends");
        writer.await.unwrap();
        assert_eq!(rest, vec![EventKind::Queued, EventKind::Cancelled], "nothing after the final event");
    }

    #[tokio::test]
    async fn follow_resumes_after_a_known_position() {
        let repo = Arc::new(MemoryRepository::new());
        let bus = EventBus::new();
        let execution = sample_execution("acme", None);
        let id = execution.id;
        let CreateOutcome::Created(done) = repo
            .create(execution, vec![event(EventKind::Created), event(EventKind::Queued), event(EventKind::Completed)])
            .await
            .unwrap()
        else {
            panic!("created")
        };
        let after = done.events[0].seq;
        let kinds: Vec<EventKind> = follow(repo, &bus, id, after).map(|e| e.kind).collect().await;
        assert_eq!(kinds, vec![EventKind::Queued, EventKind::Completed]);
    }

    #[tokio::test]
    async fn a_subscriber_that_lags_still_sees_everything() {
        let repo = Arc::new(MemoryRepository::new());
        let bus = EventBus::new();
        let execution = sample_execution("acme", None);
        let id = execution.id;
        repo.create(execution, vec![event(EventKind::Created)]).await.unwrap();
        let mut stream = follow(repo.clone(), &bus, id, 0);
        assert_eq!(stream.next().await.unwrap().kind, EventKind::Created);

        // Far more events than the bus holds, none consumed meanwhile.
        let mut execution = repo.get(&"acme".parse().unwrap(), id).await.unwrap().unwrap();
        let total = BUS_CAPACITY + 200;
        let mut commit = Commit::new(execution.clone(), execution.version);
        commit.events = (0..total).map(|_| event(EventKind::Progress)).collect();
        publish(&repo, &bus, commit).await;
        execution.version += 1;
        let mut last = Commit::new(execution, 2);
        last.events = vec![event(EventKind::Completed)];
        publish(&repo, &bus, last).await;

        let count = tokio::time::timeout(Duration::from_secs(5), stream.count()).await.expect("ends");
        assert_eq!(count, total + 1);
    }
}
