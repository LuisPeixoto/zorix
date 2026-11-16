//! Merging keyword and vector results, and deciding which of them are relevant at all (ADR-0014).

use std::collections::HashMap;

use zorix_core::{TextHit, VectorHit};

/// The constant of reciprocal rank fusion.
pub const RRF_K: f64 = 60.0;
/// How many of the top hits of one list can stand alone.
pub const TOP_STANDALONE: u32 = 5;
/// The cosine below which a vector-only hit is not trusted, when nothing better is known.
pub const DEFAULT_MIN_SIMILARITY: f32 = 0.35;

/// One hit after fusion.
#[derive(Clone, Debug, PartialEq)]
pub struct Fused {
    /// The id of the item, chunk or record.
    pub id: String,
    /// The fused score: larger is better.
    pub score: f64,
    /// Rank in the keyword list, if it is there.
    pub keyword_rank: Option<u32>,
    /// Rank in the vector list, if it is there.
    pub vector_rank: Option<u32>,
    /// How many distinct query terms it matched (keyword hits).
    pub matched_terms: u32,
    /// Its cosine similarity (vector hits).
    pub cosine: Option<f32>,
}

impl Fused {
    /// Which lists found it, for display.
    pub fn found_by(&self) -> Vec<String> {
        let mut by = Vec::new();
        if self.keyword_rank.is_some() {
            by.push("keyword".to_string());
        }
        if self.vector_rank.is_some() {
            by.push("vector".to_string());
        }
        by
    }
}

/// Reciprocal rank fusion of a keyword list and a vector list: `score = Σ 1 / (60 + rank)`, best first. Equal scores keep
/// a stable order (by id), so the same data always gives the same result.
pub fn fuse(keyword: &[TextHit], vector: &[VectorHit]) -> Vec<Fused> {
    let mut by_id: HashMap<&str, Fused> = HashMap::new();
    for hit in keyword {
        let entry = by_id.entry(&hit.id).or_insert_with(|| Fused {
            id: hit.id.clone(),
            score: 0.0,
            keyword_rank: None,
            vector_rank: None,
            matched_terms: 0,
            cosine: None,
        });
        entry.score += 1.0 / (RRF_K + f64::from(hit.rank));
        entry.keyword_rank = Some(hit.rank);
        entry.matched_terms = hit.matched_terms;
    }
    for hit in vector {
        let entry = by_id.entry(&hit.id).or_insert_with(|| Fused {
            id: hit.id.clone(),
            score: 0.0,
            keyword_rank: None,
            vector_rank: None,
            matched_terms: 0,
            cosine: None,
        });
        entry.score += 1.0 / (RRF_K + f64::from(hit.rank));
        entry.vector_rank = Some(hit.rank);
        entry.cosine = Some(hit.cosine);
    }
    let mut all: Vec<Fused> = by_id.into_values().collect();
    all.sort_by(|a, b| b.score.total_cmp(&a.score).then_with(|| a.id.cmp(&b.id)));
    all
}

/// Fusion always returns something, so this decides whether it is worth showing. A hit is kept when
/// - both lists found it; or
/// - it is in the keyword top 5 and matched enough distinct terms (a third of the query, at least two, at most all); or
/// - it is in the vector top 5 and at least `min_similarity` close.
pub fn relevant(hit: &Fused, query_terms: usize, min_similarity: f32) -> bool {
    if hit.keyword_rank.is_some() && hit.vector_rank.is_some() {
        return true;
    }
    if let Some(rank) = hit.keyword_rank {
        let needed = (query_terms.div_ceil(3)).max(2).min(query_terms.max(1)) as u32;
        if rank <= TOP_STANDALONE && hit.matched_terms >= needed {
            return true;
        }
    }
    if let (Some(rank), Some(cosine)) = (hit.vector_rank, hit.cosine) {
        return rank <= TOP_STANDALONE && cosine >= min_similarity;
    }
    false
}

#[cfg(test)]
mod tests {
    use zorix_core::Corpus;

    use super::*;

    fn kw(id: &str, rank: u32, matched: u32) -> TextHit {
        TextHit { corpus: Corpus::Knowledge, id: id.into(), rank, score: 1.0, matched_terms: matched }
    }

    fn vec_hit(id: &str, rank: u32, cosine: f32) -> VectorHit {
        VectorHit { corpus: Corpus::Knowledge, id: id.into(), rank, cosine }
    }

    #[test]
    fn a_hit_in_both_lists_beats_one_that_is_first_in_a_single_list() {
        let fused = fuse(&[kw("a", 1, 3), kw("both", 2, 3)], &[vec_hit("both", 1, 0.6), vec_hit("b", 2, 0.5)]);
        assert_eq!(fused.iter().map(|f| f.id.as_str()).collect::<Vec<_>>(), ["both", "a", "b"]);
        let both = &fused[0];
        assert!((both.score - (1.0 / 62.0 + 1.0 / 61.0)).abs() < 1e-12);
        assert_eq!(
            (both.keyword_rank, both.vector_rank, both.found_by()),
            (Some(2), Some(1), vec!["keyword".to_string(), "vector".to_string()])
        );
    }

    #[test]
    fn equal_scores_have_a_stable_order() {
        let first = fuse(&[kw("b", 1, 2), kw("a", 1, 2)], &[]);
        assert_eq!(first.iter().map(|f| f.id.as_str()).collect::<Vec<_>>(), ["a", "b"]);
        assert_eq!(fuse(&[], &[]), vec![]);
    }

    #[test]
    fn the_gate_keeps_what_is_found_twice_or_strongly_once() {
        let both = fuse(&[kw("x", 40, 1)], &[vec_hit("x", 30, 0.05)]);
        assert!(relevant(&both[0], 6, 0.35), "found by both: kept even at low ranks");
        // Keyword only: needs rank <= 5 and a third of the terms (at least two).
        let strong = &fuse(&[kw("x", 2, 3)], &[])[0];
        assert!(relevant(strong, 9, 0.35));
        assert!(!relevant(&fuse(&[kw("x", 2, 2)], &[])[0], 9, 0.35), "9 terms need 3 matches");
        assert!(!relevant(&fuse(&[kw("x", 6, 5)], &[])[0], 9, 0.35), "rank 6 cannot stand alone");
        assert!(!relevant(&fuse(&[kw("x", 1, 1)], &[])[0], 6, 0.35), "one matching word is not enough");
        // Short queries: one term must be enough, or nothing could ever match.
        assert!(relevant(&fuse(&[kw("x", 1, 1)], &[])[0], 1, 0.35));
        assert!(relevant(&fuse(&[kw("x", 1, 2)], &[])[0], 2, 0.35));
        // Vector only: needs the top 5 and enough similarity.
        assert!(relevant(&fuse(&[], &[vec_hit("v", 3, 0.5)])[0], 4, 0.35));
        assert!(!relevant(&fuse(&[], &[vec_hit("v", 3, 0.2)])[0], 4, 0.35), "close enough only above the minimum");
        assert!(!relevant(&fuse(&[], &[vec_hit("v", 9, 0.9)])[0], 4, 0.35), "rank 9 cannot stand alone");
    }

    #[test]
    fn an_unrelated_corpus_yields_nothing() {
        // Everything is far and only weakly matched: the nearest neighbours are returned but none is relevant.
        let keyword: Vec<TextHit> = (1..=5).map(|r| kw(&format!("k{r}"), r, 1)).collect();
        let vector: Vec<VectorHit> = (1..=5).map(|r| vec_hit(&format!("v{r}"), r, 0.1)).collect();
        let kept: Vec<_> =
            fuse(&keyword, &vector).into_iter().filter(|h| relevant(h, 8, DEFAULT_MIN_SIMILARITY)).collect();
        assert!(kept.is_empty(), "{kept:?}");
    }
}
