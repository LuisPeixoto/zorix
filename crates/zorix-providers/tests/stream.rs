//! The streaming reader over a real socket: events, a stalled stream, a size cap, and cancellation.

use std::time::Duration;

use serde_json::json;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::TcpListener;
use tokio_util::sync::CancellationToken;
use zorix_core::{ModelError, ModelErrorKind};
use zorix_providers::{HttpClient, HttpConfig, validate_base_url};

/// What the raw server sends after the request: pieces with pauses, then optionally it keeps the connection open.
struct Script {
    pieces: Vec<(Duration, Vec<u8>)>,
    hang_after: bool,
}

async fn serve(script: Script) -> String {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let address = listener.local_addr().unwrap();
    tokio::spawn(async move {
        let (mut socket, _) = listener.accept().await.unwrap();
        let mut buffer = vec![0u8; 8192];
        let mut seen = Vec::new();
        while !seen.windows(4).any(|w| w == b"\r\n\r\n") {
            let n = socket.read(&mut buffer).await.unwrap_or(0);
            if n == 0 {
                return;
            }
            seen.extend_from_slice(&buffer[..n]);
        }
        let head = "HTTP/1.1 200 OK\r\ncontent-type: text/event-stream\r\ntransfer-encoding: chunked\r\n\r\n";
        let _ = socket.write_all(head.as_bytes()).await;
        for (pause, bytes) in script.pieces {
            tokio::time::sleep(pause).await;
            let chunk = format!("{:x}\r\n", bytes.len());
            if socket.write_all(chunk.as_bytes()).await.is_err()
                || socket.write_all(&bytes).await.is_err()
                || socket.write_all(b"\r\n").await.is_err()
            {
                return;
            }
        }
        if script.hang_after {
            tokio::time::sleep(Duration::from_secs(30)).await;
        } else {
            let _ = socket.write_all(b"0\r\n\r\n").await;
        }
    });
    format!("http://{address}/stream")
}

async fn open(url: &str, config: HttpConfig) -> (HttpClient, zorix_providers::SseReader) {
    let client = HttpClient::new("raw", config).unwrap();
    let url = validate_base_url(url).unwrap();
    let cancel = CancellationToken::new();
    let response = client.post_json(&url, &[], &json!({}), true, &cancel).await.unwrap();
    let reader = client.sse(response);
    (client, reader)
}

fn piece(text: &str) -> (Duration, Vec<u8>) {
    (Duration::ZERO, text.as_bytes().to_vec())
}

#[tokio::test]
async fn events_arrive_in_order_even_when_a_chunk_cuts_one_in_half() {
    let url = serve(Script {
        pieces: vec![piece("event: a\ndata: {\"n\":1}\n\nda"), piece("ta: two\n\n"), piece("data: [DONE]\n\n")],
        hang_after: false,
    })
    .await;
    let (_client, mut reader) = open(&url, HttpConfig::default()).await;
    let cancel = CancellationToken::new();
    let mut seen = Vec::new();
    while let Some(event) = reader.next(&cancel).await.unwrap() {
        seen.push((event.event, event.data));
    }
    assert_eq!(seen, vec![(Some("a".into()), "{\"n\":1}".into()), (None, "two".into()), (None, "[DONE]".into())]);
}

#[tokio::test]
async fn a_stream_that_goes_silent_is_reported_as_stalled() {
    let url = serve(Script { pieces: vec![piece("data: first\n\n")], hang_after: true }).await;
    let config = HttpConfig { stream_idle_timeout: Duration::from_millis(200), ..HttpConfig::default() };
    let (_client, mut reader) = open(&url, config).await;
    let cancel = CancellationToken::new();
    assert_eq!(reader.next(&cancel).await.unwrap().unwrap().data, "first");
    let started = std::time::Instant::now();
    let error = reader.next(&cancel).await.unwrap_err();
    assert_eq!(error.kind(), ModelErrorKind::Unavailable);
    assert!(error.to_string().contains("stopped"), "{error}");
    assert!(started.elapsed() < Duration::from_secs(3));
}

#[tokio::test]
async fn a_stream_bigger_than_the_cap_is_cut_off() {
    let big = format!("data: {}\n\n", "x".repeat(4_000));
    let url = serve(Script { pieces: vec![piece(&big), piece(&big)], hang_after: false }).await;
    let config = HttpConfig { max_stream_bytes: 5_000, ..HttpConfig::default() };
    let (_client, mut reader) = open(&url, config).await;
    let cancel = CancellationToken::new();
    let mut outcome = Ok(None);
    for _ in 0..5 {
        outcome = reader.next(&cancel).await;
        if outcome.is_err() {
            break;
        }
    }
    let error = outcome.unwrap_err();
    assert!(error.to_string().contains("larger than the limit"), "{error}");
}

#[tokio::test]
async fn cancelling_while_waiting_for_an_event_returns_at_once() {
    let url = serve(Script { pieces: vec![], hang_after: true }).await;
    let (_client, mut reader) = open(&url, HttpConfig::default()).await;
    let cancel = CancellationToken::new();
    let stop = cancel.clone();
    tokio::spawn(async move {
        tokio::time::sleep(Duration::from_millis(60)).await;
        stop.cancel();
    });
    let started = std::time::Instant::now();
    assert!(matches!(reader.next(&cancel).await, Err(ModelError::Cancelled)));
    assert!(started.elapsed() < Duration::from_secs(2));
}

#[tokio::test]
async fn a_connection_that_drops_mid_stream_is_an_error_not_a_silent_end() {
    // Advertise a chunk but close the socket before sending it all.
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let address = listener.local_addr().unwrap();
    tokio::spawn(async move {
        let (mut socket, _) = listener.accept().await.unwrap();
        let mut buffer = vec![0u8; 4096];
        let _ = socket.read(&mut buffer).await;
        let _ = socket
            .write_all(b"HTTP/1.1 200 OK\r\ncontent-type: text/event-stream\r\ntransfer-encoding: chunked\r\n\r\n100\r\ndata: half")
            .await;
    });
    let (_client, mut reader) = open(&format!("http://{address}/"), HttpConfig::default()).await;
    let cancel = CancellationToken::new();
    let error = reader.next(&cancel).await.unwrap_err();
    assert_eq!(error.kind(), ModelErrorKind::Unavailable);
}
