//! Reading a response made of one JSON document per line (Ollama's streaming format).

use std::collections::VecDeque;
use std::pin::Pin;
use std::time::Duration;

use bytes::Bytes;
use futures_util::{Stream, StreamExt};
use serde_json::Value;
use tokio_util::sync::CancellationToken;
use zorix_core::{ModelError, ModelErrorKind};

/// The longest line accepted, in bytes.
pub const MAX_LINE: usize = 4 * 1024 * 1024;

type ByteStream = Pin<Box<dyn Stream<Item = reqwest::Result<Bytes>> + Send>>;

/// Reads JSON lines from a response body, with an idle timeout, a size cap and cancellation.
pub struct NdjsonReader {
    stream: ByteStream,
    buffer: Vec<u8>,
    ready: VecDeque<Value>,
    idle: Duration,
    received: usize,
    max_bytes: usize,
    provider: String,
}

impl NdjsonReader {
    /// Read the body of a response as JSON lines.
    pub fn new(response: reqwest::Response, provider: &str, idle: Duration, max_bytes: usize) -> Self {
        Self {
            stream: Box::pin(response.bytes_stream()),
            buffer: Vec::new(),
            ready: VecDeque::new(),
            idle,
            received: 0,
            max_bytes,
            provider: provider.to_string(),
        }
    }

    fn take_lines(&mut self) -> Result<(), ModelError> {
        while let Some(end) = self.buffer.iter().position(|b| *b == b'\n') {
            let line: Vec<u8> = self.buffer.drain(..=end).collect();
            let text = String::from_utf8_lossy(&line);
            let text = text.trim();
            if text.is_empty() {
                continue;
            }
            let value = serde_json::from_str(text).map_err(|_| {
                ModelError::provider(
                    ModelErrorKind::Unavailable,
                    format!("{}: a line of the response was not JSON.", self.provider),
                )
            })?;
            self.ready.push_back(value);
        }
        if self.buffer.len() > MAX_LINE {
            return Err(ModelError::provider(
                ModelErrorKind::Unavailable,
                format!("{}: the response has a line that is too long.", self.provider),
            ));
        }
        Ok(())
    }

    /// The next JSON value, or `None` when the server ended the response.
    pub async fn next(&mut self, cancel: &CancellationToken) -> Result<Option<Value>, ModelError> {
        loop {
            if let Some(value) = self.ready.pop_front() {
                return Ok(Some(value));
            }
            let chunk = tokio::select! {
                _ = cancel.cancelled() => return Err(ModelError::Cancelled),
                chunk = tokio::time::timeout(self.idle, self.stream.next()) => chunk,
            };
            let unavailable = |message: String| ModelError::provider(ModelErrorKind::Unavailable, message);
            match chunk {
                Err(_) => {
                    return Err(unavailable(format!(
                        "{}: the response stopped for {} seconds.",
                        self.provider,
                        self.idle.as_secs()
                    )));
                }
                Ok(None) => {
                    if !self.buffer.is_empty() {
                        self.buffer.push(b'\n');
                        self.take_lines()?;
                        continue;
                    }
                    return Ok(None);
                }
                Ok(Some(Err(error))) => {
                    let text = zorix_core::redact_text(&error.without_url().to_string());
                    return Err(unavailable(format!("{}: the response broke off ({text}).", self.provider)));
                }
                Ok(Some(Ok(bytes))) => {
                    self.received += bytes.len();
                    if self.received > self.max_bytes {
                        return Err(unavailable(format!("{}: the response is larger than the limit.", self.provider)));
                    }
                    self.buffer.extend_from_slice(&bytes);
                    self.take_lines()?;
                }
            }
        }
    }
}
