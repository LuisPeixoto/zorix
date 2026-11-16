//! Embeddings: what is known about the models, and the parts every adapter shares (ADR-0014).
//!
//! Vectors are always L2-normalised here, whatever the provider does, so a dot product is a cosine. The size of a vector
//! comes from the answer, never from this table alone: the table only sizes batches and carries the prefixes some models need.

use reqwest::Url;
use serde_json::{Value, json};
use zorix_core::{
    EmbedPurpose, EmbeddingProfile, EmbeddingRequest, EmbeddingResponse, ModelError, ModelErrorKind, Secret, Usage,
};

use crate::http::{Header, HttpClient};

/// The most texts sent to a provider in one call, when the model does not say.
pub const DEFAULT_BATCH: usize = 32;
/// The most characters of one text sent for embedding (longer texts are cut, never rejected).
pub const MAX_INPUT_CHARS: usize = 32_000;
/// How similar a hit must be, when nothing better is known (ADR-0014).
pub const DEFAULT_MIN_SIMILARITY: f32 = 0.35;

/// What is known about one embeddings model. Sizes and limits as documented in November 2026 *(verify against the provider)*.
#[derive(Clone, Copy, Debug)]
pub struct EmbeddingModel {
    /// `ollama`, `openai` or `gemini` (the kind of provider that serves it).
    pub kind: &'static str,
    /// The model id, without a tag.
    pub id: &'static str,
    /// The usual size of its vectors.
    pub dimensions: u32,
    /// The longest input, in tokens.
    pub max_input_tokens: u64,
    /// The most texts per call.
    pub max_batch: usize,
    /// Put before a query, for models trained with prefixes.
    pub query_prefix: &'static str,
    /// Put before a document.
    pub document_prefix: &'static str,
    /// The cosine below which a vector-only hit is not trusted.
    pub min_similarity: f32,
}

/// The embeddings models ZORIX knows. Others work too: their size is read from the first answer.
pub const EMBEDDING_MODELS: &[EmbeddingModel] = &[
    EmbeddingModel {
        kind: "openai",
        id: "text-embedding-3-small",
        dimensions: 1536,
        max_input_tokens: 8_192,
        max_batch: 64,
        query_prefix: "",
        document_prefix: "",
        min_similarity: DEFAULT_MIN_SIMILARITY,
    },
    EmbeddingModel {
        kind: "openai",
        id: "text-embedding-3-large",
        dimensions: 3072,
        max_input_tokens: 8_192,
        max_batch: 64,
        query_prefix: "",
        document_prefix: "",
        min_similarity: DEFAULT_MIN_SIMILARITY,
    },
    EmbeddingModel {
        kind: "gemini",
        id: "gemini-embedding-001",
        dimensions: 3072,
        max_input_tokens: 2_048,
        max_batch: 50,
        query_prefix: "",
        document_prefix: "",
        min_similarity: DEFAULT_MIN_SIMILARITY,
    },
    EmbeddingModel {
        kind: "ollama",
        id: "all-minilm",
        dimensions: 384,
        max_input_tokens: 256,
        max_batch: 32,
        query_prefix: "",
        document_prefix: "",
        min_similarity: DEFAULT_MIN_SIMILARITY,
    },
    EmbeddingModel {
        kind: "ollama",
        id: "nomic-embed-text",
        dimensions: 768,
        max_input_tokens: 8_192,
        max_batch: 32,
        query_prefix: "search_query: ",
        document_prefix: "search_document: ",
        min_similarity: DEFAULT_MIN_SIMILARITY,
    },
    EmbeddingModel {
        kind: "ollama",
        id: "mxbai-embed-large",
        dimensions: 1024,
        max_input_tokens: 512,
        max_batch: 32,
        query_prefix: "Represent this sentence for searching relevant passages: ",
        document_prefix: "",
        min_similarity: DEFAULT_MIN_SIMILARITY,
    },
    EmbeddingModel {
        kind: "ollama",
        id: "bge-m3",
        dimensions: 1024,
        max_input_tokens: 8_192,
        max_batch: 32,
        query_prefix: "",
        document_prefix: "",
        min_similarity: DEFAULT_MIN_SIMILARITY,
    },
    EmbeddingModel {
        kind: "ollama",
        id: "embeddinggemma",
        dimensions: 768,
        max_input_tokens: 2_048,
        max_batch: 32,
        query_prefix: "task: search result | query: ",
        document_prefix: "title: none | text: ",
        min_similarity: DEFAULT_MIN_SIMILARITY,
    },
];

/// An embeddings model a person can choose.
#[derive(Clone, Debug, PartialEq, Eq, serde::Serialize)]
pub struct EmbeddingChoice {
    /// The provider instance (`ollama`, `openai`…).
    pub provider: String,
    /// The model id, as the provider names it.
    pub id: String,
    /// The size of its vectors, when known.
    pub dimensions: Option<u32>,
    /// The longest input, in tokens, when known.
    pub max_input_tokens: Option<u64>,
    /// It runs on this computer.
    pub local: bool,
    /// It is installed or available now (a model only in the table may still need to be pulled).
    pub installed: bool,
}

impl EmbeddingChoice {
    /// How to name it in a request: `provider/model`.
    pub fn reference(&self) -> String {
        format!("{}/{}", self.provider, self.id)
    }
}

/// The entry for a model of a kind of provider. An Ollama tag (`all-minilm:latest`) is ignored.
pub fn lookup(kind: &str, id: &str) -> Option<&'static EmbeddingModel> {
    let bare = id.trim_start_matches("models/");
    let bare = if kind == "ollama" { bare.split(':').next().unwrap_or(bare) } else { bare };
    EMBEDDING_MODELS.iter().find(|m| m.kind == kind && m.id == bare)
}

impl EmbeddingModel {
    /// What a gateway tells the runtime about it.
    pub fn profile(&self) -> EmbeddingProfile {
        EmbeddingProfile {
            dimensions: self.dimensions,
            max_input_tokens: self.max_input_tokens,
            max_batch: self.max_batch,
        }
    }

    /// The prefix for a purpose.
    pub fn prefix(&self, purpose: EmbedPurpose) -> &'static str {
        match purpose {
            EmbedPurpose::Query => self.query_prefix,
            EmbedPurpose::Document => self.document_prefix,
        }
    }
}

fn invalid(message: impl Into<String>) -> ModelError {
    ModelError::provider(ModelErrorKind::Invalid, message)
}

/// Check the texts before anything is sent, and cut the ones that are too long.
pub fn prepare_inputs(provider: &str, request: &EmbeddingRequest, prefix: &str) -> Result<Vec<String>, ModelError> {
    if request.inputs.iter().any(|t| t.trim().is_empty()) {
        return Err(invalid(format!("{provider}: a text to embed is empty.")));
    }
    Ok(request
        .inputs
        .iter()
        .map(|t| format!("{prefix}{}", t.chars().take(MAX_INPUT_CHARS).collect::<String>()))
        .collect())
}

/// Scale a vector to length 1. Returns false when it cannot be (empty, zero, or not finite).
pub fn normalise(vector: &mut [f32]) -> bool {
    let norm = vector.iter().map(|x| x * x).sum::<f32>().sqrt();
    if vector.is_empty() || !norm.is_finite() || norm == 0.0 || vector.iter().any(|x| !x.is_finite()) {
        return false;
    }
    vector.iter_mut().for_each(|x| *x /= norm);
    true
}

/// Check what came back: one vector per text, all of one size, none unusable. Normalises them.
pub fn finish_vectors(provider: &str, wanted: usize, mut vectors: Vec<Vec<f32>>) -> Result<Vec<Vec<f32>>, ModelError> {
    let unavailable = |what: String| ModelError::provider(ModelErrorKind::Unavailable, format!("{provider}: {what}"));
    if vectors.len() != wanted {
        return Err(unavailable(format!("it returned {} vector(s) for {wanted} text(s).", vectors.len())));
    }
    let size = vectors.first().map(Vec::len).unwrap_or(0);
    for vector in &mut vectors {
        if vector.len() != size {
            return Err(unavailable("the vectors are not all of the same size.".into()));
        }
        if !normalise(vector) {
            return Err(unavailable("a vector was empty or had no usable values.".into()));
        }
    }
    Ok(vectors)
}

/// Split a list of texts in batches no larger than `max`, keeping order.
pub fn batches(inputs: &[String], max: usize) -> impl Iterator<Item = &[String]> {
    inputs.chunks(max.max(1))
}

/// `…/chat/completions` or `…/responses` becomes `…/embeddings`.
pub fn sibling_url(url: &Url, last_segments: &str) -> Url {
    let mut out = url.clone();
    let path = url.path().trim_end_matches('/');
    let base = path.strip_suffix(last_segments.trim_matches('/')).unwrap_or(path).trim_end_matches('/');
    out.set_path(&format!("{base}/embeddings"));
    out
}

/// `POST /v1/embeddings` of the OpenAI family (OpenAI, and servers that copy it: LM Studio, vLLM, OpenRouter…).
pub async fn openai_embed(
    http: &HttpClient,
    url: &Url,
    key: Option<&Secret>,
    provider: &str,
    request: &EmbeddingRequest,
    max_batch: usize,
    cancel: &tokio_util::sync::CancellationToken,
) -> Result<EmbeddingResponse, ModelError> {
    let inputs = prepare_inputs(provider, request, "")?;
    let headers: Vec<(&str, Header)> = key.iter().map(|k| ("authorization", Header::Bearer(k))).collect();
    let mut vectors: Vec<Vec<f32>> = Vec::with_capacity(inputs.len());
    let mut usage = Usage::default();
    let mut model = request.model.clone();
    for batch in batches(&inputs, max_batch) {
        let mut body = json!({"model": request.model, "input": batch, "encoding_format": "float"});
        if let Some(dimensions) = request.dimensions {
            body["dimensions"] = json!(dimensions);
        }
        let response = http.post_json(url, &headers, &body, false, cancel).await?;
        let answer = http.read_json(response, cancel).await?;
        let mut rows: Vec<(usize, Vec<f32>)> = answer["data"]
            .as_array()
            .into_iter()
            .flatten()
            .enumerate()
            .map(|(position, row)| (row["index"].as_u64().map_or(position, |i| i as usize), floats(&row["embedding"])))
            .collect();
        rows.sort_by_key(|(index, _)| *index);
        vectors.extend(rows.into_iter().map(|(_, v)| v));
        usage.input_tokens +=
            answer["usage"]["prompt_tokens"].as_u64().or(answer["usage"]["total_tokens"].as_u64()).unwrap_or(0);
        if let Some(m) = answer["model"].as_str().filter(|m| !m.is_empty()) {
            model = m.to_string();
        }
    }
    Ok(EmbeddingResponse {
        model,
        provider: Some(provider.to_string()),
        vectors: finish_vectors(provider, inputs.len(), vectors)?,
        usage,
    })
}

/// The numbers of a JSON array (anything that is not a number is dropped, which `finish_vectors` then reports).
pub fn floats(value: &Value) -> Vec<f32> {
    value.as_array().into_iter().flatten().filter_map(Value::as_f64).map(|f| f as f32).collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_table_is_found_by_kind_and_id_ignoring_tags_and_prefixes() {
        assert_eq!(lookup("ollama", "all-minilm:latest").unwrap().dimensions, 384);
        assert_eq!(lookup("ollama", "nomic-embed-text:v1.5").unwrap().document_prefix, "search_document: ");
        assert_eq!(lookup("gemini", "models/gemini-embedding-001").unwrap().dimensions, 3072);
        assert!(lookup("openai", "all-minilm").is_none(), "a model belongs to its kind");
        assert!(lookup("ollama", "llama3").is_none());
        let model = lookup("openai", "text-embedding-3-small").unwrap();
        assert_eq!(model.profile().max_batch, 64);
        assert_eq!(lookup("ollama", "nomic-embed-text").unwrap().prefix(EmbedPurpose::Query), "search_query: ");
        assert_eq!(model.prefix(EmbedPurpose::Query), "");
    }

    #[test]
    fn vectors_are_scaled_to_length_one_and_bad_ones_are_refused() {
        let mut v = vec![3.0, 4.0];
        assert!(normalise(&mut v));
        assert!((v[0] - 0.6).abs() < 1e-6 && (v[1] - 0.8).abs() < 1e-6);
        for bad in [vec![], vec![0.0, 0.0], vec![f32::NAN, 1.0], vec![f32::INFINITY, 1.0]] {
            assert!(!normalise(&mut bad.clone()), "{bad:?}");
        }
        let ok = finish_vectors("p", 2, vec![vec![1.0, 0.0], vec![0.0, 2.0]]).unwrap();
        assert_eq!(ok, vec![vec![1.0, 0.0], vec![0.0, 1.0]]);
        assert!(finish_vectors("p", 3, vec![vec![1.0]]).is_err(), "a vector is missing");
        assert!(finish_vectors("p", 2, vec![vec![1.0, 0.0], vec![1.0]]).is_err(), "sizes differ");
        assert!(finish_vectors("p", 1, vec![vec![0.0, 0.0]]).is_err());
        assert_eq!(finish_vectors("p", 0, vec![]).unwrap(), Vec::<Vec<f32>>::new());
    }

    #[test]
    fn texts_are_checked_prefixed_and_cut() {
        let request = |inputs: Vec<&str>| EmbeddingRequest {
            model: "m".into(),
            inputs: inputs.into_iter().map(String::from).collect(),
            purpose: EmbedPurpose::Query,
            dimensions: None,
        };
        assert!(prepare_inputs("p", &request(vec!["ok", "  "]), "").is_err());
        assert_eq!(prepare_inputs("p", &request(vec!["a", "b"]), "q: ").unwrap(), vec!["q: a", "q: b"]);
        let long = "x".repeat(MAX_INPUT_CHARS + 10);
        assert_eq!(prepare_inputs("p", &request(vec![long.as_str()]), "").unwrap()[0].chars().count(), MAX_INPUT_CHARS);
    }

    #[test]
    fn batches_keep_order_and_never_have_size_zero() {
        let inputs: Vec<String> = (0..5).map(|i| i.to_string()).collect();
        let sizes: Vec<usize> = batches(&inputs, 2).map(<[String]>::len).collect();
        assert_eq!(sizes, vec![2, 2, 1]);
        assert_eq!(batches(&inputs, 0).count(), 5);
    }

    #[test]
    fn the_embeddings_address_sits_next_to_the_chat_one() {
        let chat = Url::parse("https://openrouter.ai/api/v1/chat/completions").unwrap();
        assert_eq!(sibling_url(&chat, "/chat/completions").as_str(), "https://openrouter.ai/api/v1/embeddings");
        let responses = Url::parse("https://api.openai.com/v1/responses").unwrap();
        assert_eq!(sibling_url(&responses, "responses").as_str(), "https://api.openai.com/v1/embeddings");
    }
}
