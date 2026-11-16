//! A deterministic embedding for tests and demos: a bag of words hashed into a small vector. Texts that share words are
//! close; texts that share none are far. No network, no model.

use zorix_core::{EmbeddingProfile, EmbeddingRequest, EmbeddingResponse, ModelError, ModelErrorKind, Usage};

/// The size of the vectors.
pub const DIMENSIONS: usize = 64;

/// The vector of a text: each word adds to one of [`DIMENSIONS`] places (with a sign), and the result has length 1.
pub fn hash_embedding(text: &str) -> Vec<f32> {
    let mut vector = vec![0.0f32; DIMENSIONS];
    for word in text.split(|c: char| !c.is_alphanumeric()).filter(|w| !w.is_empty()) {
        let word = word.to_lowercase();
        // FNV-1a: stable across platforms and releases.
        let mut hash: u64 = 0xcbf2_9ce4_8422_2325;
        for byte in word.bytes() {
            hash ^= u64::from(byte);
            hash = hash.wrapping_mul(0x0000_0100_0000_01b3);
        }
        let place = (hash % DIMENSIONS as u64) as usize;
        vector[place] += if (hash >> 32) & 1 == 0 { 1.0 } else { -1.0 };
    }
    let norm = vector.iter().map(|x| x * x).sum::<f32>().sqrt();
    if norm == 0.0 {
        // No words at all: a fixed unit vector, so the result is still usable.
        vector[0] = 1.0;
        return vector;
    }
    vector.iter().map(|x| x / norm).collect()
}

/// Answer an embeddings request with [`hash_embedding`].
pub fn embed(request: &EmbeddingRequest) -> Result<EmbeddingResponse, ModelError> {
    if request.inputs.iter().any(|t| t.trim().is_empty()) {
        return Err(ModelError::provider(ModelErrorKind::Invalid, "sim: a text to embed is empty."));
    }
    let words: u64 = request.inputs.iter().map(|t| t.split_whitespace().count() as u64).sum();
    Ok(EmbeddingResponse {
        model: request.model.clone(),
        provider: Some("sim".into()),
        vectors: request.inputs.iter().map(|t| hash_embedding(t)).collect(),
        usage: Usage { input_tokens: words, ..Usage::default() },
    })
}

/// What the simulated embeddings model reports about itself.
pub fn profile() -> EmbeddingProfile {
    EmbeddingProfile { dimensions: DIMENSIONS as u32, max_input_tokens: 8_192, max_batch: 64 }
}

#[cfg(test)]
mod tests {
    use zorix_core::EmbedPurpose;

    use super::*;

    fn dot(a: &[f32], b: &[f32]) -> f32 {
        a.iter().zip(b).map(|(x, y)| x * y).sum()
    }

    #[test]
    fn vectors_have_length_one_and_are_the_same_every_time() {
        let v = hash_embedding("Deploys go through staging.");
        assert_eq!(v.len(), DIMENSIONS);
        assert!((dot(&v, &v) - 1.0).abs() < 1e-5);
        assert_eq!(v, hash_embedding("deploys GO through staging!"), "case and punctuation do not matter");
        let empty = hash_embedding("   ");
        assert!((dot(&empty, &empty) - 1.0).abs() < 1e-5);
    }

    #[test]
    fn texts_that_share_words_are_closer_than_texts_that_share_none() {
        let query = hash_embedding("how do deploys reach staging");
        let related = hash_embedding("deploys reach staging first and then production");
        let unrelated = hash_embedding("lunch is served at noon in the cafeteria");
        assert!(
            dot(&query, &related) > dot(&query, &unrelated) + 0.2,
            "{} vs {}",
            dot(&query, &related),
            dot(&query, &unrelated)
        );
    }

    #[test]
    fn a_request_gets_one_vector_per_text() {
        let request = EmbeddingRequest {
            model: "hash-embed".into(),
            inputs: vec!["a b".into(), "c d e".into()],
            purpose: EmbedPurpose::Document,
            dimensions: None,
        };
        let response = embed(&request).unwrap();
        assert_eq!((response.vectors.len(), response.usage.input_tokens), (2, 5));
        let bad = EmbeddingRequest { inputs: vec![" ".into()], ..request };
        assert!(embed(&bad).is_err());
    }
}
