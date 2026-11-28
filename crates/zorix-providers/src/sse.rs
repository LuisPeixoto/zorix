//! A small, strict parser for Server-Sent Events, and a reader that applies timeouts and size limits to a stream.

use std::collections::VecDeque;
use std::pin::Pin;
use std::time::Duration;

use bytes::Bytes;
use futures_util::{Stream, StreamExt};
use tokio_util::sync::CancellationToken;
use zorix_core::{ModelError, ModelErrorKind};

/// The longest a single line may be, in bytes.
pub const MAX_LINE: usize = 1024 * 1024;
/// The most data one event may hold, in bytes, whatever the number of lines it is spread over. Every line may be under [`MAX_LINE`]
/// and an event can still grow for ever if its `data:` lines never stop; this is what ends it.
pub const MAX_EVENT: usize = 4 * 1024 * 1024;

/// One event of the stream.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct SseEvent {
    /// The `event:` name, when the server gave one.
    pub event: Option<String>,
    /// The `data:` lines joined with newlines.
    pub data: String,
}

/// Why the stream cannot be parsed.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum TooLong {
    /// A line is longer than [`MAX_LINE`].
    Line,
    /// The data of one event is more than [`MAX_EVENT`].
    Event,
}

/// Turns chunks of bytes into events. It buffers partial lines (and partial UTF-8 characters) between chunks.
#[derive(Default)]
pub struct SseParser {
    line: Vec<u8>,
    pending_cr: bool,
    first: bool,
    event: Option<String>,
    data: Vec<String>,
    has_data: bool,
    /// The bytes of data held for the event being read.
    event_bytes: usize,
}

impl SseParser {
    /// A parser at the start of a stream.
    pub fn new() -> Self {
        Self { first: true, ..Self::default() }
    }

    /// Feed a chunk. Returns the events that completed, in order.
    pub fn feed(&mut self, chunk: &[u8]) -> Result<Vec<SseEvent>, TooLong> {
        let mut events = Vec::new();
        for &byte in chunk {
            if self.pending_cr {
                self.pending_cr = false;
                if byte == b'\n' {
                    continue; // the second half of CRLF
                }
            }
            match byte {
                b'\r' => {
                    self.pending_cr = true;
                    self.finish_line(&mut events)?;
                }
                b'\n' => self.finish_line(&mut events)?,
                other => {
                    if self.line.len() >= MAX_LINE {
                        return Err(TooLong::Line);
                    }
                    self.line.push(other);
                }
            }
        }
        Ok(events)
    }

    fn finish_line(&mut self, events: &mut Vec<SseEvent>) -> Result<(), TooLong> {
        let mut bytes = std::mem::take(&mut self.line);
        if self.first {
            self.first = false;
            if bytes.starts_with(&[0xEF, 0xBB, 0xBF]) {
                bytes.drain(..3);
            }
        }
        let line = String::from_utf8_lossy(&bytes);
        if line.is_empty() {
            if self.has_data || self.event.is_some() {
                events.push(SseEvent { event: self.event.take(), data: std::mem::take(&mut self.data).join("\n") });
            }
            self.has_data = false;
            self.data.clear();
            self.event_bytes = 0;
            return Ok(());
        }
        if line.starts_with(':') {
            return Ok(()); // a comment, often a keep-alive
        }
        let (field, value) = match line.split_once(':') {
            Some((field, value)) => (field, value.strip_prefix(' ').unwrap_or(value)),
            None => (&*line, ""),
        };
        match field {
            "event" => self.event = Some(value.to_string()),
            "data" => {
                self.event_bytes += value.len() + 1;
                if self.event_bytes > MAX_EVENT {
                    return Err(TooLong::Event);
                }
                self.data.push(value.to_string());
                self.has_data = true;
            }
            _ => {} // id, retry and unknown fields do not matter here
        }
        Ok(())
    }
}

type ByteStream = Pin<Box<dyn Stream<Item = reqwest::Result<Bytes>> + Send>>;

/// Reads events from a response body, with an idle timeout, a size cap and cancellation.
pub struct SseReader {
    stream: ByteStream,
    parser: SseParser,
    ready: VecDeque<SseEvent>,
    idle: Duration,
    received: usize,
    max_bytes: usize,
    provider: String,
}

impl SseReader {
    /// Read the body of a response as an event stream.
    pub fn new(response: reqwest::Response, provider: &str, idle: Duration, max_bytes: usize) -> Self {
        Self {
            stream: Box::pin(response.bytes_stream()),
            parser: SseParser::new(),
            ready: VecDeque::new(),
            idle,
            received: 0,
            max_bytes,
            provider: provider.to_string(),
        }
    }

    /// The next event, or `None` when the server ended the stream.
    pub async fn next(&mut self, cancel: &CancellationToken) -> Result<Option<SseEvent>, ModelError> {
        loop {
            if let Some(event) = self.ready.pop_front() {
                return Ok(Some(event));
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
                Ok(None) => return Ok(None),
                Ok(Some(Err(error))) => {
                    let text = zorix_core::redact_text(&error.without_url().to_string());
                    return Err(unavailable(format!("{}: the response broke off ({text}).", self.provider)));
                }
                Ok(Some(Ok(bytes))) => {
                    self.received += bytes.len();
                    if self.received > self.max_bytes {
                        return Err(unavailable(format!("{}: the response is larger than the limit.", self.provider)));
                    }
                    match self.parser.feed(&bytes) {
                        Ok(events) => self.ready.extend(events),
                        Err(too_long) => {
                            return Err(unavailable(format!(
                                "{}: the response has {} that is too long.",
                                self.provider,
                                if too_long == TooLong::Line { "a line" } else { "an event" }
                            )));
                        }
                    }
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn parse(chunks: &[&[u8]]) -> Vec<SseEvent> {
        let mut parser = SseParser::new();
        chunks.iter().flat_map(|chunk| parser.feed(chunk).unwrap()).collect()
    }

    fn event(name: Option<&str>, data: &str) -> SseEvent {
        SseEvent { event: name.map(str::to_string), data: data.to_string() }
    }

    #[test]
    fn reads_named_and_unnamed_events() {
        let events = parse(&[b"event: message_start\ndata: {\"a\":1}\n\ndata: [DONE]\n\n"]);
        assert_eq!(events, vec![event(Some("message_start"), "{\"a\":1}"), event(None, "[DONE]")]);
    }

    #[test]
    fn handles_every_line_ending_and_a_split_between_chunks() {
        let want = vec![event(Some("x"), "one"), event(None, "two")];
        assert_eq!(parse(&[b"event: x\r\ndata: one\r\n\r\ndata: two\r\n\r\n"]), want);
        assert_eq!(parse(&[b"event: x\rdata: one\r\rdata: two\r\r"]), want);
        assert_eq!(parse(&[b"event: x\n", b"data: o", b"ne\n", b"\n", b"data: two\n\n"]), want);
        assert_eq!(
            parse(&[b"event: x\r", b"\ndata: one\r", b"\n\r", b"\ndata: two\n\n"]),
            want,
            "CRLF split across chunks"
        );
    }

    #[test]
    fn joins_several_data_lines_with_newlines() {
        assert_eq!(parse(&[b"data: a\ndata: b\ndata:\ndata: c\n\n"]), vec![event(None, "a\nb\n\nc")]);
    }

    #[test]
    fn ignores_comments_unknown_fields_and_empty_blocks() {
        let events = parse(&[b": keep-alive\n\nid: 7\nretry: 100\n\ndata: real\n\n"]);
        assert_eq!(events, vec![event(None, "real")]);
    }

    #[test]
    fn a_field_without_a_space_or_a_colon_still_works() {
        assert_eq!(parse(&[b"data:tight\n\n"]), vec![event(None, "tight")]);
        assert_eq!(parse(&[b"data\n\n"]), vec![event(None, "")]);
    }

    #[test]
    fn a_byte_order_mark_at_the_start_is_skipped() {
        assert_eq!(parse(&[b"\xEF\xBB\xBFdata: hi\n\n"]), vec![event(None, "hi")]);
    }

    #[test]
    fn a_character_split_between_chunks_is_not_damaged() {
        let text = "data: héllo → ✓\n\n".as_bytes();
        for split in 0..text.len() {
            let events = parse(&[&text[..split], &text[split..]]);
            assert_eq!(events, vec![event(None, "héllo → ✓")], "split at {split}");
        }
    }

    #[test]
    fn an_event_that_never_ends_is_not_delivered() {
        assert!(parse(&[b"data: cut off\n"]).is_empty());
    }

    #[test]
    fn a_line_that_is_too_long_is_refused_instead_of_buffered_forever() {
        let mut parser = SseParser::new();
        let big = vec![b'a'; MAX_LINE + 1];
        assert_eq!(parser.feed(&big), Err(TooLong::Line));
    }

    #[test]
    fn an_event_made_of_many_short_lines_is_bounded_too() {
        // Every line is far under the line limit, and the event still never ends: the lines add up.
        let mut parser = SseParser::new();
        let line = format!("data: {}\n", "a".repeat(60 * 1024));
        let mut fed = 0;
        let outcome = loop {
            match parser.feed(line.as_bytes()) {
                Ok(events) => assert!(events.is_empty()),
                Err(e) => break e,
            }
            fed += line.len();
            assert!(fed <= MAX_EVENT + line.len(), "the event grew past its limit without being refused");
        };
        assert_eq!(outcome, TooLong::Event);
        // Events that end are not held against each other: the same amount in separate events is fine.
        let mut parser = SseParser::new();
        for _ in 0..(3 * MAX_EVENT / line.len()) {
            let mut bytes = line.clone().into_bytes();
            bytes.push(b'\n');
            assert_eq!(parser.feed(&bytes).unwrap().len(), 1);
        }
    }

    #[test]
    fn hostile_input_never_panics() {
        let mut parser = SseParser::new();
        for chunk in [&b"\xff\xfe\x00\n\n"[..], b"data: \xff\n\n", b"\r\r\r\n\n:\n:\n", b"event\n\ndata\n\n"] {
            let _ = parser.feed(chunk);
        }
    }
}
