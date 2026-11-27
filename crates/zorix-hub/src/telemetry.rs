//! Sending what happens in a run to an OpenTelemetry collector (ADR-0046).
//!
//! Off until a person gives an address. What leaves the computer is only the spans the runtime makes (`invoke_agent`, `chat`, `execute_tool`),
//! with the attributes it puts on them: the model, token counts, the cost, the tool's name, how it ended, and ids. Never a prompt, an answer, a
//! tool's arguments or its result, and no log line or event: the layer lets through spans of the runtime only, so a span another part of the
//! program opens, or a message written inside one, never reaches the exporter. The export is HTTP with protobuf, through a bounded queue and a
//! batch every few seconds, so a slow or dead collector costs a run nothing.

use std::collections::HashMap;
use std::net::IpAddr;
use std::sync::{Arc, OnceLock};
use std::time::Duration;

use opentelemetry::KeyValue;
use opentelemetry::trace::TracerProvider as _;
use opentelemetry_otlp::{Protocol, SpanExporter, WithExportConfig, WithHttpConfig};
use opentelemetry_sdk::Resource;
use opentelemetry_sdk::trace::{BatchConfigBuilder, BatchSpanProcessor, SdkTracerProvider};
use parking_lot::Mutex;
use tracing_subscriber::filter::{FilterFn, Filtered, filter_fn};
use tracing_subscriber::layer::SubscriberExt;
use tracing_subscriber::util::SubscriberInitExt;
use tracing_subscriber::{EnvFilter, Layer, Registry, reload};
use url::Url;
use zorix_core::{Secret, TelemetrySettings};
use zorix_providers::SecretStore;

use crate::HubError;

/// A layer of the subscriber that can be filled or emptied while the program runs.
pub type BoxedLayer = Box<dyn Layer<Registry> + Send + Sync>;
/// The place in the subscriber where the export goes. Add it directly over the registry. The filter that keeps everything but the spans of the
/// runtime out is part of it, because a filter has to be there when the subscriber is made, not when a layer is swapped in.
pub type Slot =
    Filtered<reload::Layer<Option<BoxedLayer>, Registry>, FilterFn<fn(&tracing::Metadata<'_>) -> bool>, Registry>;

/// Only spans, and only the ones the runtime opens: no event (so no message with an error in it), and nothing another part of the program opens.
fn runtime_spans(meta: &tracing::Metadata<'_>) -> bool {
    meta.is_span() && meta.target().starts_with("zorix_runtime")
}

/// Where the value of the collector's header is kept in the keychain (its name is in the settings).
pub const HEADER_SECRET: &str = "telemetry.header";

static SWITCH: OnceLock<Arc<Telemetry>> = OnceLock::new();

/// Install the program's tracing subscriber: log lines as `ZORIX_LOG` says (to standard error when asked, or standard output), and the slot the
/// export goes in. Returns the switch for the export, which is also found later with [`switch`].
pub fn init_tracing(default_filter: &str, stderr: bool) -> Arc<Telemetry> {
    let (slot, telemetry) = channel();
    let filter = EnvFilter::try_from_env("ZORIX_LOG").unwrap_or_else(|_| default_filter.into());
    let lines = tracing_subscriber::fmt::layer();
    let lines = if stderr { lines.with_writer(std::io::stderr).boxed() } else { lines.boxed() };
    let _ = tracing_subscriber::registry().with(slot).with(lines.with_filter(filter)).try_init();
    let telemetry = Arc::new(telemetry);
    let _ = SWITCH.set(telemetry.clone());
    SWITCH.get().cloned().unwrap_or(telemetry)
}

/// The switch of the program's export, once [`init_tracing`] ran.
pub fn switch() -> Option<Arc<Telemetry>> {
    SWITCH.get().cloned()
}

/// The header the settings name, with its value from the keychain. `None` when no header is set or its value is missing.
pub fn header_of(settings: &TelemetrySettings, secrets: &dyn SecretStore) -> Option<(String, String)> {
    let name = settings.header.as_ref()?;
    let value: Secret = secrets.get(HEADER_SECRET).ok()??;
    Some((name.clone(), value.expose().to_string()))
}

/// How long a collector may take to answer one batch.
const EXPORT_TIMEOUT: Duration = Duration::from_secs(10);
/// How often a batch is sent.
const BATCH_EVERY: Duration = Duration::from_secs(5);
/// The most spans waiting to be sent; more are dropped, never waited for.
const QUEUE: usize = 2_048;
/// The most spans in one batch.
const BATCH: usize = 512;
/// How long shutting down waits for what is still queued.
const FLUSH: Duration = Duration::from_secs(5);
/// The longest header value.
const MAX_HEADER_VALUE: usize = 4_096;

/// Turns the export on and off while the program runs.
pub struct Telemetry {
    handle: reload::Handle<Option<BoxedLayer>, Registry>,
    provider: Mutex<Option<SdkTracerProvider>>,
}

/// The slot to put in the subscriber, and the handle that fills it.
pub fn channel() -> (Slot, Telemetry) {
    let (reloadable, handle) = reload::Layer::new(None);
    let slot = reloadable.with_filter(filter_fn(runtime_spans as fn(&tracing::Metadata<'_>) -> bool));
    (slot, Telemetry { handle, provider: Mutex::new(None) })
}

/// The address a collector listens on: `https`, or `http` only for a collector on this computer; no credentials in it; the path defaults to
/// `/v1/traces`.
pub fn parse_endpoint(text: &str) -> Result<Url, HubError> {
    let bad = |why: &str| Err(HubError(why.to_string()));
    let url =
        Url::parse(text.trim()).map_err(|_| HubError("The telemetry address is not a valid web address.".into()))?;
    let loopback = match url.host() {
        Some(url::Host::Domain(name)) => name.eq_ignore_ascii_case("localhost"),
        Some(url::Host::Ipv4(ip)) => IpAddr::V4(ip).is_loopback(),
        Some(url::Host::Ipv6(ip)) => IpAddr::V6(ip).is_loopback(),
        None => return bad("The telemetry address has no host."),
    };
    match url.scheme() {
        "https" => {}
        "http" if loopback => {}
        "http" => return bad("A collector that is not on this computer must be reached over https."),
        _ => {
            return bad(
                "The telemetry address must start with https:// (or http:// for a collector on this computer).",
            );
        }
    }
    if !url.username().is_empty() || url.password().is_some() {
        return bad("The telemetry address must not hold a name or a password: put the header in its own setting.");
    }
    if url.query().is_some() || url.fragment().is_some() {
        return bad("The telemetry address must not have a query or a fragment.");
    }
    let mut url = url;
    if url.path() == "/" || url.path().is_empty() {
        url.set_path("/v1/traces");
    }
    Ok(url)
}

/// A header a collector may need, such as `authorization`. The name is a token and the value is one line of visible characters.
pub fn check_header(name: &str, value: &str) -> Result<(), HubError> {
    let token = |c: char| c.is_ascii_alphanumeric() || "!#$%&'*+-.^_`|~".contains(c);
    if name.is_empty() || name.len() > 64 || !name.chars().all(token) {
        return Err(HubError("A header name has letters, digits and - only, and at most 64 characters.".into()));
    }
    if value.is_empty() || value.len() > MAX_HEADER_VALUE || !value.chars().all(|c| (' '..='~').contains(&c)) {
        return Err(HubError("A header value is one line of printable characters.".into()));
    }
    Ok(())
}

fn client_for(url: &Url) -> Result<reqwest::blocking::Client, HubError> {
    let _ = rustls::crypto::ring::default_provider().install_default();
    let mut builder =
        reqwest::blocking::Client::builder().timeout(EXPORT_TIMEOUT).connect_timeout(Duration::from_secs(5));
    // A collector on this computer is not behind the system's proxy.
    if matches!(url.host_str(), Some("localhost" | "127.0.0.1" | "[::1]")) {
        builder = builder.no_proxy();
    }
    builder.build().map_err(|e| HubError(format!("Could not prepare the connection to the collector: {e}")))
}

fn build_provider(url: &Url, header: Option<(String, String)>) -> Result<SdkTracerProvider, HubError> {
    let mut headers = HashMap::new();
    if let Some((name, value)) = header {
        check_header(&name, &value)?;
        headers.insert(name, value);
    }
    let exporter = SpanExporter::builder()
        .with_http()
        .with_http_client(client_for(url)?)
        .with_endpoint(url.as_str())
        .with_protocol(Protocol::HttpBinary)
        .with_headers(headers)
        .with_timeout(EXPORT_TIMEOUT)
        .build()
        .map_err(|e| HubError(format!("Could not prepare the export: {e}")))?;
    let batch = BatchConfigBuilder::default()
        .with_max_queue_size(QUEUE)
        .with_max_export_batch_size(BATCH)
        .with_scheduled_delay(BATCH_EVERY)
        .build();
    // Nothing of the machine or the environment: a name and a version.
    let resource = Resource::builder_empty()
        .with_service_name("zorix")
        .with_attribute(KeyValue::new("service.version", env!("CARGO_PKG_VERSION")))
        .build();
    Ok(SdkTracerProvider::builder()
        .with_resource(resource)
        .with_span_processor(BatchSpanProcessor::builder(exporter).with_batch_config(batch).build())
        .build())
}

impl Telemetry {
    /// Start sending to `endpoint` (with `header`, kept in the keychain), or stop when it is `None`. What is queued for the old collector is sent
    /// first. A bad address or header is refused with a sentence, and the export stays as it was.
    pub async fn apply(&self, endpoint: Option<&str>, header: Option<(String, String)>) -> Result<(), HubError> {
        let Some(endpoint) = endpoint else {
            self.stop().await;
            return Ok(());
        };
        let url = parse_endpoint(endpoint)?;
        // The blocking HTTP client makes its own runtime, which must not be made or dropped on an async thread.
        let built = tokio::task::spawn_blocking(move || build_provider(&url, header))
            .await
            .map_err(|e| HubError(format!("Could not prepare the export: {e}")))??;
        let tracer = built.tracer("zorix");
        let boxed: BoxedLayer = Box::new(tracing_opentelemetry::layer().with_tracer(tracer));
        self.handle
            .modify(|slot| *slot = Some(boxed))
            .map_err(|e| HubError(format!("Could not start the export: {e}")))?;
        let old = self.provider.lock().replace(built);
        if let Some(old) = old {
            let _ = tokio::task::spawn_blocking(move || old.shutdown_with_timeout(FLUSH)).await;
        }
        Ok(())
    }

    /// Start sending at the beginning of a program, when there is no async context to wait in. A bad address is reported and the export stays off.
    pub fn apply_at_start(&self, settings: &TelemetrySettings, secrets: &dyn SecretStore) -> Result<(), HubError> {
        let Some(endpoint) = settings.endpoint.as_deref() else { return Ok(()) };
        let url = parse_endpoint(endpoint)?;
        let header = header_of(settings, secrets);
        // The blocking HTTP client makes its own runtime: build it on a thread that is in none.
        let built = std::thread::scope(|scope| scope.spawn(|| build_provider(&url, header)).join())
            .map_err(|_| HubError("Could not prepare the export.".into()))??;
        let boxed: BoxedLayer = Box::new(tracing_opentelemetry::layer().with_tracer(built.tracer("zorix")));
        self.handle
            .modify(|slot| *slot = Some(boxed))
            .map_err(|e| HubError(format!("Could not start the export: {e}")))?;
        *self.provider.lock() = Some(built);
        Ok(())
    }

    /// Stop sending, after sending what is queued.
    pub async fn stop(&self) {
        let _ = self.handle.modify(|slot| *slot = None);
        let old = self.provider.lock().take();
        if let Some(old) = old {
            let _ = tokio::task::spawn_blocking(move || old.shutdown_with_timeout(FLUSH)).await;
        }
    }

    /// Whether spans are being sent.
    pub fn is_on(&self) -> bool {
        self.provider.lock().is_some()
    }

    /// Send what is queued now and stop. For the end of a program.
    pub fn shutdown_blocking(&self) {
        let _ = self.handle.modify(|slot| *slot = None);
        if let Some(old) = self.provider.lock().take() {
            let _ = old.shutdown_with_timeout(FLUSH);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn only_https_or_a_collector_on_this_computer_is_accepted_and_the_path_defaults() {
        for good in [
            "https://otel.example.com",
            "https://otel.example.com:4318/",
            "http://127.0.0.1:4318",
            "http://localhost:4318",
            "http://[::1]:4318",
        ] {
            let url = parse_endpoint(good).unwrap_or_else(|e| panic!("{good}: {e}"));
            assert!(url.path().starts_with("/v1/traces") || good.contains("/v1"), "{good} -> {url}");
        }
        assert_eq!(parse_endpoint("https://otel.example.com/custom/path").unwrap().path(), "/custom/path");
        assert_eq!(parse_endpoint("https://otel.example.com").unwrap().as_str(), "https://otel.example.com/v1/traces");
        for bad in [
            "http://otel.example.com",
            "http://10.0.0.5:4318",
            "ftp://otel.example.com",
            "otel.example.com",
            "https://user:pass@otel.example.com",
            "https://user@otel.example.com",
            "https://otel.example.com/?token=abc",
            "https://otel.example.com/#x",
            "",
            "https://",
        ] {
            assert!(parse_endpoint(bad).is_err(), "{bad:?}");
        }
    }

    #[test]
    fn a_header_is_a_token_and_one_line() {
        assert!(check_header("authorization", "Bearer abc.def").is_ok());
        assert!(check_header("x-honeycomb-team", "k3y").is_ok());
        for (name, value) in [
            ("", "v"),
            ("bad name", "v"),
            ("a:b", "v"),
            ("authorization", ""),
            ("authorization", "line\r\nx-injected: 1"),
            ("authorization", "tab\there"),
            ("authorization", "é"),
        ] {
            assert!(check_header(name, value).is_err(), "{name:?} {value:?}");
        }
        assert!(check_header("authorization", &"a".repeat(MAX_HEADER_VALUE + 1)).is_err());
    }
}
