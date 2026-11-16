//! ZORIX providers: adapters that put real AI models behind the `ModelGateway` contract.
//!
//! One adapter per API family (Anthropic, OpenAI Responses, Chat Completions, Ollama, Gemini), built on a shared HTTP client,
//! streaming parsers, a model catalogue, secret stores and `providers.toml` (ADR-0009 to ADR-0012).

pub mod adapter;
pub mod anthropic;
pub mod catalog;
pub mod config;
pub mod gemini;
pub mod http;
pub mod ndjson;
pub mod ollama;
pub mod openai_chat;
pub mod openai_responses;
pub mod router;
pub mod secrets;
pub mod sse;

pub use anthropic::{AnthropicGateway, AnthropicOptions};
pub use catalog::{Catalog, Latency, ModelInfo, Price};
pub use config::{
    CHAT_PRESETS, Capability, Order, ProviderConfig, ProviderKind, ProvidersConfig, RouteConfig, Selector, config_path,
};
pub use gemini::{GeminiGateway, GeminiOptions};
pub use http::{
    Header, HttpClient, HttpConfig, classify, classify_stream_error, is_loopback, retry_after_ms, validate_base_url,
};
pub use ndjson::NdjsonReader;
pub use ollama::{OllamaGateway, OllamaOptions};
pub use openai_chat::{OpenAiChatGateway, OpenAiChatOptions};
pub use openai_responses::{OpenAiOptions, OpenAiResponsesGateway};
pub use router::{Candidate, ProviderEntry, Resolved, Router};
pub use secrets::{ChainStore, EnvStore, KeychainStore, MemoryStore, SecretError, SecretStore, get_async};
pub use sse::{SseEvent, SseParser, SseReader};
