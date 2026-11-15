//! The HTTP client every adapter uses, and how failures are classified.
//!
//! Rules it enforces (ADR-0009): HTTPS except for loopback, no credentials inside URLs, timeouts, a cap on body sizes,
//! secrets only in headers marked sensitive, and error text that has been redacted and shortened.

use std::net::IpAddr;
use std::time::Duration;

use reqwest::header::{CONTENT_TYPE, HeaderName, HeaderValue, RETRY_AFTER};
use reqwest::{StatusCode, Url};
use serde_json::{Value, json};
use tokio_util::sync::CancellationToken;
use zorix_core::{ModelError, ModelErrorKind, Secret, redact_text};

use crate::sse::SseReader;

/// Limits and timeouts for one provider's connection.
#[derive(Clone, Copy, Debug)]
pub struct HttpConfig {
    /// How long connecting may take.
    pub connect_timeout: Duration,
    /// How long a whole non-streaming request may take.
    pub request_timeout: Duration,
    /// How long a stream may be silent before it counts as stalled.
    pub stream_idle_timeout: Duration,
    /// The largest non-streaming response body, in bytes.
    pub max_body_bytes: usize,
    /// The most a stream may deliver in total, in bytes.
    pub max_stream_bytes: usize,
}

impl Default for HttpConfig {
    fn default() -> Self {
        Self {
            connect_timeout: Duration::from_secs(10),
            request_timeout: Duration::from_secs(600),
            stream_idle_timeout: Duration::from_secs(120),
            max_body_bytes: 16 * 1024 * 1024,
            max_stream_bytes: 64 * 1024 * 1024,
        }
    }
}

/// A header value, which may be a secret.
#[derive(Clone, Copy)]
pub enum Header<'a> {
    /// Safe to show in logs.
    Plain(&'a str),
    /// Never shown: sent marked sensitive.
    Secret(&'a Secret),
}

/// True for `localhost` and loopback addresses, where plain HTTP is allowed.
pub fn is_loopback(host: &str) -> bool {
    let host = host.trim_start_matches('[').trim_end_matches(']');
    host.eq_ignore_ascii_case("localhost") || host.parse::<IpAddr>().is_ok_and(|ip| ip.is_loopback())
}

/// Check a base URL given by a person: HTTPS (or HTTP to loopback), a host, and no credentials inside.
pub fn validate_base_url(text: &str) -> Result<Url, String> {
    let url = Url::parse(text.trim()).map_err(|e| format!("“{text}” is not a valid address: {e}."))?;
    let host = url.host_str().ok_or_else(|| format!("“{text}” has no host."))?;
    match url.scheme() {
        "https" => {}
        "http" if is_loopback(host) => {}
        "http" => {
            return Err(format!(
                "“{text}” uses plain HTTP. Use HTTPS, or a loopback address such as http://127.0.0.1 for a local server."
            ));
        }
        other => return Err(format!("“{text}” uses “{other}”; only https (or http to a local server) is allowed.")),
    }
    if !url.username().is_empty() || url.password().is_some() {
        return Err("Put credentials in the keychain, not inside the address.".into());
    }
    Ok(url)
}

/// A connection to one provider.
#[derive(Clone)]
pub struct HttpClient {
    client: reqwest::Client,
    config: HttpConfig,
    provider: String,
}

/// Install the TLS crypto provider the HTTP stack needs. Safe to call many times.
fn install_crypto() {
    let _ = rustls::crypto::ring::default_provider().install_default();
}

impl HttpClient {
    /// A client for the provider called `provider` (used in messages).
    pub fn new(provider: &str, config: HttpConfig) -> Result<Self, String> {
        install_crypto();
        let client = reqwest::Client::builder()
            .connect_timeout(config.connect_timeout)
            .user_agent(concat!("zorix/", env!("CARGO_PKG_VERSION")))
            .redirect(reqwest::redirect::Policy::none())
            .build()
            .map_err(|e| format!("Could not start the HTTP client: {e}"))?;
        Ok(Self { client, config, provider: provider.to_string() })
    }

    /// The limits in use.
    pub fn config(&self) -> &HttpConfig {
        &self.config
    }

    /// The provider name used in messages.
    pub fn provider(&self) -> &str {
        &self.provider
    }

    /// The provider's name in error messages.
    fn unavailable(&self, message: String) -> ModelError {
        ModelError::provider(ModelErrorKind::Unavailable, message)
    }

    fn transport_error(&self, error: reqwest::Error, url: &Url) -> ModelError {
        let host = url.host_str().unwrap_or("the server");
        if error.is_timeout() {
            return self.unavailable(format!("{}: {host} did not answer in time.", self.provider));
        }
        if error.is_connect() {
            return self.unavailable(format!("{}: could not connect to {host}.", self.provider));
        }
        let detail = redact_text(&error.without_url().to_string());
        self.unavailable(format!("{}: network error talking to {host} ({detail}).", self.provider))
    }

    /// POST a JSON body. A response that is not a success becomes an error, already classified.
    pub async fn post_json(
        &self,
        url: &Url,
        headers: &[(&str, Header<'_>)],
        body: &Value,
        streaming: bool,
        cancel: &CancellationToken,
    ) -> Result<reqwest::Response, ModelError> {
        let mut request = self.client.post(url.clone()).header(CONTENT_TYPE, "application/json").json(body);
        if !streaming {
            request = request.timeout(self.config.request_timeout);
        }
        for (name, value) in headers {
            let (text, sensitive) = match value {
                Header::Plain(text) => ((*text).to_string(), false),
                Header::Secret(secret) => (secret.expose().to_string(), true),
            };
            let name = HeaderName::from_bytes(name.as_bytes()).map_err(|_| {
                ModelError::provider(ModelErrorKind::Invalid, format!("{}: a header name is not valid.", self.provider))
            })?;
            let mut value = HeaderValue::from_str(&text).map_err(|_| {
                ModelError::provider(
                    ModelErrorKind::Auth,
                    format!("{}: the API key has characters a header cannot hold.", self.provider),
                )
            })?;
            value.set_sensitive(sensitive);
            request = request.header(name, value);
        }
        let response = tokio::select! {
            _ = cancel.cancelled() => return Err(ModelError::Cancelled),
            response = request.send() => response.map_err(|e| self.transport_error(e, url))?,
        };
        if response.status().is_success() {
            return Ok(response);
        }
        let status = response.status();
        let retry_after = retry_after_ms(response.headers());
        let body = self.read_text(response, 64 * 1024, cancel).await.unwrap_or_default();
        Err(classify(&self.provider, status, &body, retry_after))
    }

    /// GET a JSON document (used for discovery). Errors are classified as for POST.
    pub async fn get_json(
        &self,
        url: &Url,
        headers: &[(&str, Header<'_>)],
        cancel: &CancellationToken,
    ) -> Result<Value, ModelError> {
        let mut request = self.client.get(url.clone()).timeout(Duration::from_secs(30));
        for (name, value) in headers {
            let (text, sensitive) = match value {
                Header::Plain(text) => ((*text).to_string(), false),
                Header::Secret(secret) => (secret.expose().to_string(), true),
            };
            if let (Ok(name), Ok(mut value)) = (HeaderName::from_bytes(name.as_bytes()), HeaderValue::from_str(&text)) {
                value.set_sensitive(sensitive);
                request = request.header(name, value);
            }
        }
        let response = tokio::select! {
            _ = cancel.cancelled() => return Err(ModelError::Cancelled),
            response = request.send() => response.map_err(|e| self.transport_error(e, url))?,
        };
        if !response.status().is_success() {
            let status = response.status();
            let retry_after = retry_after_ms(response.headers());
            let body = self.read_text(response, 64 * 1024, cancel).await.unwrap_or_default();
            return Err(classify(&self.provider, status, &body, retry_after));
        }
        self.read_json(response, cancel).await
    }

    /// Read a success body as JSON, within the size limit.
    pub async fn read_json(
        &self,
        response: reqwest::Response,
        cancel: &CancellationToken,
    ) -> Result<Value, ModelError> {
        let text = self.read_text(response, self.config.max_body_bytes, cancel).await?;
        serde_json::from_str(&text).map_err(|_| {
            self.unavailable(format!(
                "{}: the answer was not JSON, which usually means something other than the API replied.",
                self.provider
            ))
        })
    }

    /// Read a body as text, stopping at `limit` bytes.
    async fn read_text(
        &self,
        response: reqwest::Response,
        limit: usize,
        cancel: &CancellationToken,
    ) -> Result<String, ModelError> {
        use futures_util::StreamExt;
        let mut stream = response.bytes_stream();
        let mut body = Vec::new();
        loop {
            let next = tokio::select! {
                _ = cancel.cancelled() => return Err(ModelError::Cancelled),
                next = tokio::time::timeout(self.config.stream_idle_timeout, stream.next()) => next,
            };
            match next {
                Err(_) => return Err(self.unavailable(format!("{}: the answer stopped coming.", self.provider))),
                Ok(None) => break,
                Ok(Some(Err(error))) => {
                    let detail = redact_text(&error.without_url().to_string());
                    return Err(self.unavailable(format!("{}: the answer broke off ({detail}).", self.provider)));
                }
                Ok(Some(Ok(chunk))) => {
                    if body.len() + chunk.len() > limit {
                        return Err(
                            self.unavailable(format!("{}: the answer is larger than the limit.", self.provider))
                        );
                    }
                    body.extend_from_slice(&chunk);
                }
            }
        }
        Ok(String::from_utf8_lossy(&body).into_owned())
    }

    /// Read a streaming response as events.
    pub fn sse(&self, response: reqwest::Response) -> SseReader {
        SseReader::new(response, &self.provider, self.config.stream_idle_timeout, self.config.max_stream_bytes)
    }
}

/// How long the provider asked us to wait, in milliseconds, from `retry-after-ms` or `retry-after` (in seconds).
pub fn retry_after_ms(headers: &reqwest::header::HeaderMap) -> Option<u64> {
    const MAX: u64 = 10 * 60 * 1000;
    if let Some(ms) =
        headers.get("retry-after-ms").and_then(|v| v.to_str().ok()).and_then(|v| v.trim().parse::<f64>().ok())
    {
        return Some((ms.max(0.0) as u64).min(MAX));
    }
    headers
        .get(RETRY_AFTER)
        .and_then(|v| v.to_str().ok())
        .and_then(|v| v.trim().parse::<f64>().ok())
        .map(|seconds| ((seconds.max(0.0) * 1000.0) as u64).min(MAX))
}

/// What a provider said was wrong, in a few words, taken from the usual places in an error body.
fn detail_of(body: &str) -> String {
    let from_json = serde_json::from_str::<Value>(body).ok().and_then(|json| {
        [&json["error"]["message"], &json["error"], &json["message"], &json["detail"], &json["error_description"]]
            .into_iter()
            .find_map(|value| value.as_str().map(str::to_string))
    });
    let text = from_json.unwrap_or_else(|| body.trim().to_string());
    let flat: String = text.chars().map(|c| if c.is_control() { ' ' } else { c }).collect();
    let redacted = redact_text(&flat);
    if redacted.chars().count() > 240 {
        format!("{}…", redacted.chars().take(239).collect::<String>())
    } else {
        redacted
    }
}

/// Turn an `error` object carried inside a stream (HTTP 200, then a failure) into a [`ModelError`].
pub fn classify_stream_error(provider: &str, error: &Value) -> ModelError {
    let kind_name = error["type"].as_str().unwrap_or("");
    let detail = detail_of(&error.to_string());
    let lower = detail.to_lowercase();
    let kind = match kind_name {
        "authentication_error" | "permission_error" => ModelErrorKind::Auth,
        "not_found_error" => ModelErrorKind::NotFound,
        "rate_limit_error" => ModelErrorKind::RateLimited,
        "overloaded_error" | "api_error" | "timeout_error" => ModelErrorKind::Unavailable,
        "request_too_large" => ModelErrorKind::ContextTooLong,
        "invalid_request_error" if TOO_LONG.iter().any(|needle| lower.contains(needle)) => {
            ModelErrorKind::ContextTooLong
        }
        "invalid_request_error" => ModelErrorKind::Invalid,
        _ => ModelErrorKind::Unavailable,
    };
    let text = error["message"].as_str().map(|m| detail_of(&json!({"message": m}).to_string())).unwrap_or(detail);
    ModelError::provider(kind, format!("{provider}: the stream reported an error ({kind_name}): {text}"))
}

const TOO_LONG: &[&str] = &[
    "context length",
    "context window",
    "context_length",
    "maximum context",
    "too long",
    "too many tokens",
    "exceeds the maximum",
    "request_too_large",
    "reduce the length",
];

/// Turn an HTTP failure into a [`ModelError`] with a kind the router can act on.
pub fn classify(provider: &str, status: StatusCode, body: &str, retry_after_ms: Option<u64>) -> ModelError {
    let detail = detail_of(body);
    let lower = format!("{} {}", body.to_lowercase(), detail.to_lowercase());
    let kind = match status.as_u16() {
        401 | 403 => ModelErrorKind::Auth,
        404 => ModelErrorKind::NotFound,
        413 => ModelErrorKind::ContextTooLong,
        400 | 422 if TOO_LONG.iter().any(|needle| lower.contains(needle)) => ModelErrorKind::ContextTooLong,
        408 | 409 | 425 | 429 => {
            if status.as_u16() == 429 {
                ModelErrorKind::RateLimited
            } else {
                ModelErrorKind::Unavailable
            }
        }
        500..=599 => ModelErrorKind::Unavailable,
        _ => ModelErrorKind::Invalid,
    };
    let summary = match kind {
        ModelErrorKind::Auth => "the API key is missing, wrong or not allowed",
        ModelErrorKind::NotFound => "the model or the endpoint was not found",
        ModelErrorKind::ContextTooLong => "the conversation is too long for the model",
        ModelErrorKind::RateLimited => "too many requests",
        ModelErrorKind::Unavailable => "the provider failed or is overloaded",
        _ => "the request was rejected",
    };
    let message = if detail.is_empty() {
        format!("{provider}: {summary} (HTTP {}).", status.as_u16())
    } else {
        format!("{provider}: {summary} (HTTP {}): {detail}", status.as_u16())
    };
    ModelError::Provider { kind, message, retry_after_ms }
}

#[cfg(test)]
mod tests {
    use serde_json::json;
    use wiremock::matchers::{header, method, path};
    use wiremock::{Mock, MockServer, ResponseTemplate};

    use super::*;

    fn client(config: HttpConfig) -> HttpClient {
        HttpClient::new("testprov", config).unwrap()
    }

    fn url(server: &MockServer, route: &str) -> Url {
        validate_base_url(&format!("{}{route}", server.uri())).unwrap()
    }

    async fn post(c: &HttpClient, url: &Url, key: &Secret) -> Result<Value, ModelError> {
        let cancel = CancellationToken::new();
        let response =
            c.post_json(url, &[("x-api-key", Header::Secret(key))], &json!({"hello": 1}), false, &cancel).await?;
        c.read_json(response, &cancel).await
    }

    fn kind(result: Result<Value, ModelError>) -> (ModelErrorKind, String, Option<u64>) {
        match result.unwrap_err() {
            ModelError::Provider { kind, message, retry_after_ms } => (kind, message, retry_after_ms),
            other => panic!("expected a provider error, got {other:?}"),
        }
    }

    #[test]
    fn base_urls_must_be_https_or_loopback_without_credentials() {
        for good in
            ["https://api.example.com/v1", "http://127.0.0.1:11434", "http://localhost:1234/v1", "http://[::1]:8080"]
        {
            assert!(validate_base_url(good).is_ok(), "{good}");
        }
        for bad in [
            "http://api.example.com",
            "ftp://x.com",
            "https://user:pass@api.example.com",
            "not a url",
            "https://",
            "file:///etc/passwd",
        ] {
            assert!(validate_base_url(bad).is_err(), "{bad}");
        }
    }

    #[tokio::test]
    async fn a_json_answer_is_returned_and_the_key_goes_in_the_header() {
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .and(path("/v1/x"))
            .and(header("x-api-key", "k-secret-value"))
            .and(header("content-type", "application/json"))
            .respond_with(ResponseTemplate::new(200).set_body_json(json!({"ok": true})))
            .mount(&server)
            .await;
        let value =
            post(&client(HttpConfig::default()), &url(&server, "/v1/x"), &Secret::new("k-secret-value")).await.unwrap();
        assert_eq!(value, json!({"ok": true}));
    }

    #[tokio::test]
    async fn statuses_are_classified_for_the_router() {
        let cases: [(u16, &str, ModelErrorKind); 9] = [
            (401, "{}", ModelErrorKind::Auth),
            (403, "{}", ModelErrorKind::Auth),
            (404, "{}", ModelErrorKind::NotFound),
            (413, "{}", ModelErrorKind::ContextTooLong),
            (400, r#"{"error":{"message":"prompt is too long: 250000 tokens"}}"#, ModelErrorKind::ContextTooLong),
            (400, r#"{"error":{"message":"messages: roles must alternate"}}"#, ModelErrorKind::Invalid),
            (429, "{}", ModelErrorKind::RateLimited),
            (500, "{}", ModelErrorKind::Unavailable),
            (529, "{}", ModelErrorKind::Unavailable),
        ];
        for (status, body, wanted) in cases {
            let server = MockServer::start().await;
            Mock::given(method("POST"))
                .respond_with(ResponseTemplate::new(status).set_body_raw(body, "application/json"))
                .mount(&server)
                .await;
            let (got, message, _) =
                kind(post(&client(HttpConfig::default()), &url(&server, "/"), &Secret::new("k")).await);
            assert_eq!(got, wanted, "HTTP {status}: {message}");
            assert!(message.starts_with("testprov: "), "{message}");
        }
    }

    #[tokio::test]
    async fn retry_after_is_read_from_either_header() {
        for (name, value, wanted) in [
            ("retry-after", "2", 2_000),
            ("retry-after", "0.5", 500),
            ("retry-after-ms", "1500", 1_500),
            ("retry-after", "99999999", 600_000),
        ] {
            let server = MockServer::start().await;
            Mock::given(method("POST"))
                .respond_with(ResponseTemplate::new(429).insert_header(name, value))
                .mount(&server)
                .await;
            let (got, _, retry) =
                kind(post(&client(HttpConfig::default()), &url(&server, "/"), &Secret::new("k")).await);
            assert_eq!((got, retry), (ModelErrorKind::RateLimited, Some(wanted)), "{name}: {value}");
        }
    }

    #[tokio::test]
    async fn a_key_echoed_back_in_an_error_is_redacted() {
        let key = "sk-abcdefghijklmnopqrstuvwxyz0123456789";
        let server = MockServer::start().await;
        let body = json!({"error": {"message": format!("Incorrect API key provided: {key}. Check it.")}});
        Mock::given(method("POST")).respond_with(ResponseTemplate::new(401).set_body_json(body)).mount(&server).await;
        let (kind, message, _) =
            kind(post(&client(HttpConfig::default()), &url(&server, "/"), &Secret::new(key)).await);
        assert_eq!(kind, ModelErrorKind::Auth);
        assert!(!message.contains(key) && message.contains("[redacted]"), "{message}");
    }

    #[tokio::test]
    async fn long_and_multiline_error_text_is_flattened_and_shortened() {
        let server = MockServer::start().await;
        let body = json!({"error": {"message": format!("line one\nline two {}", "x".repeat(1_000))}});
        Mock::given(method("POST")).respond_with(ResponseTemplate::new(500).set_body_json(body)).mount(&server).await;
        let (_, message, _) = kind(post(&client(HttpConfig::default()), &url(&server, "/"), &Secret::new("k")).await);
        assert!(!message.contains('\n') && message.chars().count() < 400, "{} chars", message.chars().count());
    }

    #[tokio::test]
    async fn an_answer_that_is_not_json_or_is_too_big_is_a_failure_of_the_provider() {
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .and(path("/html"))
            .respond_with(ResponseTemplate::new(200).set_body_raw("<html>login</html>", "text/html"))
            .mount(&server)
            .await;
        Mock::given(method("POST"))
            .and(path("/big"))
            .respond_with(ResponseTemplate::new(200).set_body_raw("a".repeat(5_000), "application/json"))
            .mount(&server)
            .await;
        let small = HttpConfig { max_body_bytes: 1_000, ..HttpConfig::default() };
        let (k1, m1, _) = kind(post(&client(small), &url(&server, "/html"), &Secret::new("k")).await);
        assert_eq!(k1, ModelErrorKind::Unavailable);
        assert!(m1.contains("not JSON"), "{m1}");
        let (k2, m2, _) = kind(post(&client(small), &url(&server, "/big"), &Secret::new("k")).await);
        assert_eq!(k2, ModelErrorKind::Unavailable);
        assert!(m2.contains("larger than the limit"), "{m2}");
    }

    #[tokio::test]
    async fn a_server_that_is_not_there_is_unavailable() {
        let dead = validate_base_url("http://127.0.0.1:9/").unwrap();
        let (k, m, _) = kind(post(&client(HttpConfig::default()), &dead, &Secret::new("k")).await);
        assert_eq!(k, ModelErrorKind::Unavailable);
        assert!(m.contains("could not connect to 127.0.0.1"), "{m}");
    }

    #[tokio::test]
    async fn a_slow_server_times_out_and_cancelling_stops_the_wait_at_once() {
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .respond_with(ResponseTemplate::new(200).set_delay(Duration::from_secs(5)).set_body_json(json!({})))
            .mount(&server)
            .await;
        let config = HttpConfig { request_timeout: Duration::from_millis(150), ..HttpConfig::default() };
        let (k, m, _) = kind(post(&client(config), &url(&server, "/"), &Secret::new("k")).await);
        assert_eq!(k, ModelErrorKind::Unavailable);
        assert!(m.contains("did not answer in time"), "{m}");

        let c = client(HttpConfig::default());
        let cancel = CancellationToken::new();
        let stop = cancel.clone();
        tokio::spawn(async move {
            tokio::time::sleep(Duration::from_millis(50)).await;
            stop.cancel();
        });
        let started = std::time::Instant::now();
        let result = c.post_json(&url(&server, "/"), &[], &json!({}), false, &cancel).await;
        assert!(matches!(result, Err(ModelError::Cancelled)));
        assert!(started.elapsed() < Duration::from_secs(2));
    }

    #[tokio::test]
    async fn redirects_are_not_followed_so_a_key_never_travels_to_another_host() {
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .respond_with(ResponseTemplate::new(307).insert_header("location", "http://127.0.0.1:1/steal"))
            .mount(&server)
            .await;
        let result = post(&client(HttpConfig::default()), &url(&server, "/"), &Secret::new("k")).await;
        assert!(result.is_err(), "a redirect is not a success");
    }

    #[test]
    fn classification_is_pure_and_total() {
        for status in 100..600u16 {
            let status = StatusCode::from_u16(status).unwrap();
            let error = classify("p", status, "", None);
            assert!(matches!(error, ModelError::Provider { .. }));
        }
    }
}
