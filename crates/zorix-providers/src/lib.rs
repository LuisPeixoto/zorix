//! ZORIX providers: adapters that put real AI models behind the `ModelGateway` contract.
//!
//! This crate holds the HTTP client and the streaming parser that every adapter shares (ADR-0009).

pub mod adapter;
pub mod anthropic;
pub mod catalog;
pub mod config;
pub mod http;
pub mod ndjson;
pub mod ollama;
pub mod openai_chat;
pub mod openai_responses;
pub mod secrets;
pub mod sse;

pub use anthropic::{AnthropicGateway, AnthropicOptions};
pub use catalog::{Catalog, Latency, ModelInfo, Price};
pub use config::{
    Capability, Order, ProviderConfig, ProviderKind, ProvidersConfig, RouteConfig, Selector, config_path,
};
pub use http::{
    Header, HttpClient, HttpConfig, classify, classify_stream_error, is_loopback, retry_after_ms, validate_base_url,
};
pub use ollama::{OllamaGateway, OllamaOptions};
pub use openai_chat::{OpenAiChatGateway, OpenAiChatOptions};
pub use openai_responses::{OpenAiOptions, OpenAiResponsesGateway};
pub use secrets::{ChainStore, EnvStore, KeychainStore, MemoryStore, SecretError, SecretStore, get_async};
pub use sse::{SseEvent, SseParser, SseReader};
