//! Helpers every adapter shares.

use std::collections::{HashMap, HashSet};
use std::sync::Arc;

use serde_json::Value;
use zorix_core::{ModelError, ModelErrorKind, Secret, ToolDefinition};

use crate::catalog::ModelInfo;
use crate::secrets::{SecretError, SecretStore, get_async};

/// Something that can ask its provider which models exist.
#[async_trait::async_trait]
pub trait Discover: Send + Sync {
    /// The models the provider offers, with what it reports about them. Fields it does not report are assumed and flagged.
    async fn discover(&self, cancel: &tokio_util::sync::CancellationToken) -> Result<Vec<ModelInfo>, ModelError>;

    /// The embeddings models it offers, when it can say. Providers that cannot (or have none) answer an empty list.
    async fn discover_embeddings(
        &self,
        _cancel: &tokio_util::sync::CancellationToken,
    ) -> Result<Vec<crate::embed::EmbeddingChoice>, ModelError> {
        Ok(Vec::new())
    }
}

/// Read an OpenAI-style `GET /models` list into catalogue entries. Models that are clearly not chat models are left out.
pub fn parse_openai_models(provider: &str, list: &Value) -> Vec<ModelInfo> {
    const NOT_CHAT: &[&str] =
        &["embedding", "whisper", "tts", "dall-e", "moderation", "transcribe", "image", "audio", "realtime", "rerank"];
    list["data"]
        .as_array()
        .into_iter()
        .flatten()
        .filter_map(|m| m["id"].as_str())
        .filter(|id| !NOT_CHAT.iter().any(|needle| id.to_lowercase().contains(needle)))
        .map(|id| ModelInfo {
            provider: provider.to_string(),
            id: id.to_string(),
            display_name: id.to_string(),
            context_window: 128_000,
            max_output: 16_384,
            tools: true,
            vision: false,
            thinking: false,
            latency: crate::catalog::Latency::Standard,
            tier: 3,
            price: None,
            assumed: true,
        })
        .collect()
}

/// How tool ids are written on the wire. ZORIX ids look like `math.evaluate`; providers allow fewer characters, and the
/// model must see names it can use. The map is rebuilt for every request from the tool definitions, so it needs no storage.
#[derive(Clone, Debug, Default)]
pub struct ToolNameMap {
    to_wire: HashMap<String, String>,
    from_wire: HashMap<String, String>,
}

impl ToolNameMap {
    /// Build the map for these tools. `allowed` says which characters the provider accepts in a name; the rest become `_`.
    /// Two ids that would collide get a numeric suffix, so the map is always one-to-one.
    pub fn new(tools: &[ToolDefinition], allowed: fn(char) -> bool) -> Self {
        let mut map = Self::default();
        let mut taken: HashSet<String> = HashSet::new();
        for tool in tools {
            let mut wire: String = tool.id.chars().map(|c| if allowed(c) { c } else { '_' }).collect();
            let mut n = 2;
            let stem = wire.clone();
            while !taken.insert(wire.clone()) {
                wire = format!("{stem}_{n}");
                n += 1;
            }
            map.to_wire.insert(tool.id.clone(), wire.clone());
            map.from_wire.insert(wire, tool.id.clone());
        }
        map
    }

    /// The name to send for a tool id. An id that is not in the map is sanitised the same way (it is in the history).
    pub fn wire(&self, id: &str, allowed: fn(char) -> bool) -> String {
        self.to_wire.get(id).cloned().unwrap_or_else(|| id.chars().map(|c| if allowed(c) { c } else { '_' }).collect())
    }

    /// The tool id for a name the model used. A name the model invented comes back unchanged, so the runtime reports
    /// "there is no tool called …" with the name the model actually wrote.
    pub fn original(&self, wire: &str) -> String {
        self.from_wire.get(wire).cloned().unwrap_or_else(|| wire.to_string())
    }
}

/// Hides `<think>…</think>` spans from text that arrives in pieces, so live text matches what `strip_think` keeps. A tag
/// split between two pieces is held back until it is known to be a tag.
#[derive(Debug, Default)]
pub struct ThinkFilter {
    inside: bool,
    held: String,
}

impl ThinkFilter {
    /// The visible part of the next piece.
    pub fn push(&mut self, piece: &str) -> String {
        self.held.push_str(piece);
        let mut out = String::new();
        loop {
            let tag = if self.inside { "</think>" } else { "<think>" };
            if let Some(at) = self.held.find(tag) {
                if !self.inside {
                    out.push_str(&self.held[..at]);
                }
                self.held.drain(..at + tag.len());
                self.inside = !self.inside;
                continue;
            }
            // Keep the longest ending that could still turn into the tag.
            let keep = (1..tag.len())
                .rev()
                .find(|&n| {
                    self.held.len() >= n
                        && self.held.is_char_boundary(self.held.len() - n)
                        && tag.starts_with(&self.held[self.held.len() - n..])
                })
                .unwrap_or(0);
            let cut = self.held.len() - keep;
            if !self.inside {
                out.push_str(&self.held[..cut]);
            }
            self.held.drain(..cut);
            return out;
        }
    }

    /// What is still held when the stream ends: a half tag outside thinking is only text.
    pub fn flush(&mut self) -> String {
        let rest = std::mem::take(&mut self.held);
        if self.inside { String::new() } else { rest }
    }
}

/// Letters, digits, `_` and `-`: the rule of Anthropic and OpenAI function names.
pub fn simple_name_char(c: char) -> bool {
    c.is_ascii_alphanumeric() || matches!(c, '_' | '-')
}

/// What to tell the model when the request expects a structured answer. Providers differ in how they enforce a schema, so
/// the same plain instruction is given everywhere, and the runtime validates the answer itself.
pub fn schema_instruction(schema: &Value) -> String {
    format!(
        "Reply with a single JSON value that matches this JSON Schema, and nothing else (no prose, no code fence):\n{}",
        serde_json::to_string(schema).unwrap_or_default()
    )
}

/// The error for a provider that has no key.
pub fn missing_key(provider: &str, key_name: &str) -> ModelError {
    ModelError::provider(
        ModelErrorKind::Auth,
        format!(
            "{provider}: there is no API key. Run `zorix provider set-key {key_name}` or set the environment variable for it."
        ),
    )
}

/// Read a provider's key for one call.
pub async fn resolve_key(store: &Arc<dyn SecretStore>, provider: &str, key_name: &str) -> Result<Secret, ModelError> {
    match get_async(store.clone(), key_name.to_string()).await {
        Ok(Some(secret)) => Ok(secret),
        Ok(None) => Err(missing_key(provider, key_name)),
        Err(SecretError::Unavailable(message)) => {
            Err(ModelError::provider(ModelErrorKind::Auth, format!("{provider}: {message}")))
        }
        Err(other) => Err(ModelError::provider(ModelErrorKind::Auth, format!("{provider}: {other}"))),
    }
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::*;

    fn tool(id: &str) -> ToolDefinition {
        ToolDefinition { id: id.into(), description: String::new(), input_schema: json!({"type": "object"}) }
    }

    #[test]
    fn the_think_filter_matches_strip_think_however_the_text_is_cut() {
        let text = "Hi <think>let me think, <b>really</think>The answer <thin is 4.<think>never closed";
        let expected = crate::openai_chat::strip_think(text);
        for size in 1..=text.len() {
            let mut filter = ThinkFilter::default();
            let mut seen = String::new();
            let mut at = 0;
            while at < text.len() {
                let mut end = (at + size).min(text.len());
                while !text.is_char_boundary(end) {
                    end += 1;
                }
                seen.push_str(&filter.push(&text[at..end]));
                at = end;
            }
            seen.push_str(&filter.flush());
            assert_eq!(seen.trim(), expected, "pieces of {size}");
        }
    }

    #[test]
    fn a_half_tag_at_the_end_is_text() {
        let mut filter = ThinkFilter::default();
        assert_eq!(filter.push("1 <thi"), "1 ");
        assert_eq!(filter.flush(), "<thi");
    }

    #[test]
    fn an_openai_model_list_is_read_and_non_chat_models_are_left_out() {
        let list = json!({"data": [{"id": "gpt-test"}, {"id": "text-embedding-3-small"}, {"id": "whisper-1"}, {"id": "meta/llama-test"}]});
        let ids: Vec<String> = parse_openai_models("openai", &list).into_iter().map(|m| m.id).collect();
        assert_eq!(ids, vec!["gpt-test", "meta/llama-test"]);
        assert!(parse_openai_models("x", &list).iter().all(|m| m.assumed));
    }

    #[test]
    fn dots_become_underscores_and_come_back() {
        let tools = [tool("math.evaluate"), tool("notes.put")];
        let map = ToolNameMap::new(&tools, simple_name_char);
        assert_eq!(map.wire("math.evaluate", simple_name_char), "math_evaluate");
        assert_eq!(map.original("math_evaluate"), "math.evaluate");
        assert_eq!(map.original("notes_put"), "notes.put");
    }

    #[test]
    fn names_that_would_collide_stay_apart() {
        let tools = [tool("a.b"), tool("a_b"), tool("a:b")];
        let map = ToolNameMap::new(&tools, simple_name_char);
        let wires: HashSet<String> = tools.iter().map(|t| map.wire(&t.id, simple_name_char)).collect();
        assert_eq!(wires.len(), 3, "{wires:?}");
        for t in &tools {
            assert_eq!(map.original(&map.wire(&t.id, simple_name_char)), t.id);
        }
    }

    #[test]
    fn a_name_the_model_invented_is_reported_as_written() {
        let map = ToolNameMap::new(&[tool("clock.now")], simple_name_char);
        assert_eq!(map.original("delete_everything"), "delete_everything");
    }

    #[test]
    fn a_tool_in_the_history_but_not_offered_is_still_sanitised() {
        let map = ToolNameMap::new(&[], simple_name_char);
        assert_eq!(map.wire("old.tool", simple_name_char), "old_tool");
    }

    #[test]
    fn the_schema_instruction_carries_the_schema() {
        let text = schema_instruction(&json!({"type": "object", "required": ["total"]}));
        assert!(text.contains("single JSON value") && text.contains("\"required\":[\"total\"]"));
    }

    #[tokio::test]
    async fn a_missing_key_says_how_to_set_one_and_a_present_key_is_returned() {
        let store: Arc<dyn SecretStore> = Arc::new(crate::secrets::MemoryStore::new());
        let error = resolve_key(&store, "anthropic", "anthropic").await.unwrap_err();
        assert_eq!(error.kind(), ModelErrorKind::Auth);
        assert!(error.to_string().contains("zorix provider set-key anthropic"), "{error}");
        store.set("anthropic", &Secret::new("k")).unwrap();
        assert_eq!(resolve_key(&store, "anthropic", "anthropic").await.unwrap().expose(), "k");
    }
}
