//! The local scheduler: an in-process queue that hands each execution to one worker.

use std::collections::HashSet;

use async_trait::async_trait;
use parking_lot::Mutex;
use tokio::sync::mpsc;
use tokio_util::sync::CancellationToken;
use zorix_core::{Error, ExecutionId, ExecutionScheduler, Result};

/// A FIFO queue. Enqueuing an id that is already waiting does nothing, so an execution is never handed out twice for
/// one request to run.
pub struct LocalScheduler {
    sender: mpsc::UnboundedSender<ExecutionId>,
    receiver: tokio::sync::Mutex<mpsc::UnboundedReceiver<ExecutionId>>,
    waiting: Mutex<HashSet<ExecutionId>>,
    closed: CancellationToken,
}

impl LocalScheduler {
    /// An empty, open queue.
    pub fn new() -> Self {
        let (sender, receiver) = mpsc::unbounded_channel();
        Self {
            sender,
            receiver: tokio::sync::Mutex::new(receiver),
            waiting: Mutex::new(HashSet::new()),
            closed: CancellationToken::new(),
        }
    }
}

impl Default for LocalScheduler {
    fn default() -> Self {
        Self::new()
    }
}

#[async_trait]
impl ExecutionScheduler for LocalScheduler {
    async fn enqueue(&self, id: ExecutionId) -> Result<()> {
        if self.closed.is_cancelled() {
            return Err(Error::Internal("The scheduler is closed.".into()));
        }
        if self.waiting.lock().insert(id) {
            self.sender.send(id).map_err(|_| Error::Internal("The scheduler is closed.".into()))?;
        }
        Ok(())
    }

    async fn next(&self) -> Option<ExecutionId> {
        let received = tokio::select! {
            _ = self.closed.cancelled() => None,
            id = async { self.receiver.lock().await.recv().await } => id,
        };
        if let Some(id) = received {
            self.waiting.lock().remove(&id);
        }
        received
    }

    fn close(&self) {
        self.closed.cancel();
    }
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;
    use std::time::Duration;

    use super::*;

    #[tokio::test]
    async fn hands_out_ids_in_order() {
        let scheduler = LocalScheduler::new();
        let (a, b) = (ExecutionId::new(), ExecutionId::new());
        scheduler.enqueue(a).await.unwrap();
        scheduler.enqueue(b).await.unwrap();
        assert_eq!(scheduler.next().await, Some(a));
        assert_eq!(scheduler.next().await, Some(b));
    }

    #[tokio::test]
    async fn an_id_that_is_already_waiting_is_not_queued_twice_but_can_be_queued_again_once_taken() {
        let scheduler = LocalScheduler::new();
        let id = ExecutionId::new();
        scheduler.enqueue(id).await.unwrap();
        scheduler.enqueue(id).await.unwrap();
        assert_eq!(scheduler.next().await, Some(id));
        assert!(tokio::time::timeout(Duration::from_millis(30), scheduler.next()).await.is_err(), "no duplicate");
        scheduler.enqueue(id).await.unwrap();
        assert_eq!(scheduler.next().await, Some(id));
    }

    #[tokio::test]
    async fn closing_wakes_waiting_workers_and_refuses_new_work() {
        let scheduler = Arc::new(LocalScheduler::new());
        let waiter = {
            let scheduler = scheduler.clone();
            tokio::spawn(async move { scheduler.next().await })
        };
        tokio::time::sleep(Duration::from_millis(20)).await;
        scheduler.close();
        assert_eq!(waiter.await.unwrap(), None);
        assert!(scheduler.enqueue(ExecutionId::new()).await.is_err());
    }

    #[tokio::test]
    async fn many_workers_each_get_a_different_id() {
        let scheduler = Arc::new(LocalScheduler::new());
        let ids: Vec<ExecutionId> = (0..20).map(|_| ExecutionId::new()).collect();
        let workers: Vec<_> = (0..4)
            .map(|_| {
                let scheduler = scheduler.clone();
                tokio::spawn(async move {
                    let mut got = Vec::new();
                    while let Ok(Some(id)) = tokio::time::timeout(Duration::from_millis(100), scheduler.next()).await {
                        got.push(id);
                    }
                    got
                })
            })
            .collect();
        for id in &ids {
            scheduler.enqueue(*id).await.unwrap();
        }
        let mut all: Vec<ExecutionId> = Vec::new();
        for worker in workers {
            all.extend(worker.await.unwrap());
        }
        all.sort();
        let mut expected = ids;
        expected.sort();
        assert_eq!(all, expected);
    }
}
