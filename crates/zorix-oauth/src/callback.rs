use std::collections::HashMap;
use std::time::Duration;

use reqwest::Url;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::TcpListener;
use tokio_util::sync::CancellationToken;

use crate::error::OAuthError;

/// How long a person has to finish signing in.
pub const LOGIN_TIMEOUT: Duration = Duration::from_secs(300);

const PAGE: &str = "<!doctype html><meta charset=utf-8><title>ZORIX</title><body style=\"font-family:system-ui;margin:3rem\"><h2>You are signed in</h2><p>You can close this window and go back to ZORIX.</p>";
const FAILED_PAGE: &str = "<!doctype html><meta charset=utf-8><title>ZORIX</title><body style=\"font-family:system-ui;margin:3rem\"><h2>Sign-in did not finish</h2><p>Go back to ZORIX to see why.</p>";

async fn respond(stream: &mut tokio::net::TcpStream, status: &str, body: &str) {
    let text = format!(
        "HTTP/1.1 {status}\r\nContent-Type: text/html; charset=utf-8\r\nContent-Length: {}\r\nCache-Control: no-store\r\nReferrer-Policy: no-referrer\r\nConnection: close\r\n\r\n{body}",
        body.len()
    );
    let _ = stream.write_all(text.as_bytes()).await;
    let _ = stream.shutdown().await;
}

/// Wait for the browser to come back to `http://127.0.0.1:<port>/callback`. Anything else that knocks is turned away.
pub async fn wait_for_callback(
    listener: TcpListener,
    port: u16,
    state: &str,
    cancel: &CancellationToken,
) -> Result<HashMap<String, String>, OAuthError> {
    let deadline = tokio::time::sleep(LOGIN_TIMEOUT);
    tokio::pin!(deadline);
    loop {
        let (mut stream, _) = tokio::select! {
            accepted = listener.accept() => accepted.map_err(|e| OAuthError::Transport(e.to_string()))?,
            _ = cancel.cancelled() => return Err(OAuthError::Cancelled),
            _ = &mut deadline => return Err(OAuthError::Timeout("nobody finished signing in within five minutes".into())),
        };
        let mut head = Vec::new();
        let mut buf = [0u8; 2048];
        let read = tokio::time::timeout(Duration::from_secs(5), async {
            loop {
                let n = stream.read(&mut buf).await.unwrap_or(0);
                if n == 0 {
                    break;
                }
                head.extend_from_slice(&buf[..n]);
                if head.windows(4).any(|w| w == b"\r\n\r\n") || head.len() > 16 * 1024 {
                    break;
                }
            }
        })
        .await;
        if read.is_err() {
            continue;
        }
        let text = String::from_utf8_lossy(&head).into_owned();
        let mut lines = text.lines();
        let request_line = lines.next().unwrap_or("");
        let host = lines
            .find_map(|l| l.strip_prefix("Host:").or_else(|| l.strip_prefix("host:")))
            .map(str::trim)
            .unwrap_or("");
        let target = request_line.strip_prefix("GET ").and_then(|r| r.split(' ').next()).unwrap_or("");
        let host_ok = host == format!("127.0.0.1:{port}") || host == format!("localhost:{port}");
        if !host_ok || !(target == "/callback" || target.starts_with("/callback?")) {
            respond(&mut stream, "404 Not Found", "").await;
            continue;
        }
        let params: HashMap<String, String> = Url::parse(&format!("http://127.0.0.1{target}"))
            .map(|u| u.query_pairs().map(|(k, v)| (k.into_owned(), v.into_owned())).collect())
            .unwrap_or_default();
        // Only the answer to *this* sign-in ends the wait: its `state` is a number nobody else has. Another program on this computer
        // that knocks on the port cannot cut a sign-in short, and a person is not told they are signed in by a stranger's request. An
        // error that carries no state is shown the failure page, and the wait goes on until the person cancels or the time is up.
        if params.get("state").map(String::as_str) != Some(state) {
            let page = if params.contains_key("error") { FAILED_PAGE } else { "" };
            respond(&mut stream, if page.is_empty() { "400 Bad Request" } else { "200 OK" }, page).await;
            continue;
        }
        respond(&mut stream, "200 OK", if params.contains_key("code") { PAGE } else { FAILED_PAGE }).await;
        return Ok(params);
    }
}
