//! ZORIX providers: adapters that put real AI models behind the `ModelGateway` contract.
//!
//! This crate holds the HTTP client and the streaming parser that every adapter shares (ADR-0009).

pub mod catalog;
pub mod http;
pub mod secrets;
pub mod sse;

pub use catalog::{Catalog, Latency, ModelInfo, Price};
pub use http::{Header, HttpClient, HttpConfig, classify, is_loopback, retry_after_ms, validate_base_url};
pub use secrets::{ChainStore, EnvStore, KeychainStore, MemoryStore, SecretError, SecretStore, get_async};
pub use sse::{SseEvent, SseParser, SseReader};
