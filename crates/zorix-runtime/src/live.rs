//! Live text: the pieces of an answer while a model is still writing it.
//!
//! Nothing here is stored and nothing waits for a reader. A reader that falls behind loses pieces (the stored
//! `model.responded` event is the record), and with nobody listening, a piece costs one failed send.

use std::sync::Arc;

use futures_util::StreamExt;
use futures_util::stream::{self, BoxStream};
use tokio::sync::broadcast;
use zorix_core::{ExecutionId, LiveEvent, TextSink};

/// How many pieces a reader may fall behind before the oldest are dropped.
const CAPACITY: usize = 4096;

/// Fans live events out to readers in this process.
#[derive(Clone)]
pub struct LiveBus {
    sender: broadcast::Sender<LiveEvent>,
}

impl Default for LiveBus {
    fn default() -> Self {
        Self::new()
    }
}

impl LiveBus {
    /// A bus with no readers.
    pub fn new() -> Self {
        Self { sender: broadcast::channel(CAPACITY).0 }
    }

    /// The events of one execution from now on.
    pub fn watch(&self, id: ExecutionId) -> BoxStream<'static, LiveEvent> {
        let receiver = self.sender.subscribe();
        stream::unfold(receiver, move |mut receiver| async move {
            loop {
                match receiver.recv().await {
                    Ok(event) => {
                        let mine = match &event {
                            LiveEvent::Text { execution, .. } | LiveEvent::Reset { execution } => *execution == id,
                        };
                        if mine {
                            return Some((event, receiver));
                        }
                    }
                    // Pieces were dropped: the next ones and the stored answer still tell the story.
                    Err(broadcast::error::RecvError::Lagged(_)) => continue,
                    Err(broadcast::error::RecvError::Closed) => return None,
                }
            }
        })
        .boxed()
    }

    /// A sink that publishes what one execution writes.
    pub fn sink(&self, id: ExecutionId) -> Arc<dyn TextSink> {
        Arc::new(Sink { sender: self.sender.clone(), id })
    }

    /// True when somebody is reading, so a caller can skip work that only feeds readers.
    pub fn has_readers(&self) -> bool {
        self.sender.receiver_count() > 0
    }
}

struct Sink {
    sender: broadcast::Sender<LiveEvent>,
    id: ExecutionId,
}

impl TextSink for Sink {
    fn text(&self, delta: &str) {
        if !delta.is_empty() {
            let _ = self.sender.send(LiveEvent::Text { execution: self.id, delta: delta.to_string() });
        }
    }

    fn reset(&self) {
        let _ = self.sender.send(LiveEvent::Reset { execution: self.id });
    }
}
