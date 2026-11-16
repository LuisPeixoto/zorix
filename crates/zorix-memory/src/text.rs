//! Small text helpers: the words of a query, token estimates and safe wrapping.

use std::collections::HashSet;

use zorix_core::redact_text;

/// Most terms in a keyword query.
pub const MAX_TERMS: usize = 32;
/// Most characters of a query that are looked at.
pub const MAX_QUERY_CHARS: usize = 2_000;
/// A prompt shorter than this is a follow-up that needs the previous one to be understood.
pub const SHORT_PROMPT_CHARS: usize = 60;

const STOPWORDS: &[&str] = &[
    // English
    "a", "an", "and", "are", "as", "at", "be", "but", "by", "can", "did", "do", "does", "for", "from", "had", "has",
    "have", "how", "i", "if", "in", "is", "it", "its", "me", "my", "no", "not", "of", "on", "or", "our", "please",
    "so", "that", "the", "their", "them", "then", "there", "these", "they", "this", "to", "us", "was", "we", "what",
    "when", "where", "which", "who", "why", "will", "with", "would", "you", "your", // Portuguese
    "ao", "aos", "as", "com", "como", "da", "das", "de", "do", "dos", "e", "ela", "ele", "em", "essa", "esse", "esta",
    "este", "foi", "isso", "isto", "mais", "mas", "me", "na", "nas", "no", "nos", "não", "nao", "o", "os", "ou",
    "para", "por", "qual", "quais", "que", "se", "sem", "ser", "são", "sao", "sua", "seu", "também", "um", "uma",
    "uns", "umas", "você", "voce",
];

/// The words worth searching for: lowercase, at least two characters, not a stopword, each once, in order of appearance.
pub fn terms(text: &str) -> Vec<String> {
    let stop: HashSet<&str> = STOPWORDS.iter().copied().collect();
    let mut seen = HashSet::new();
    let mut out = Vec::new();
    for word in text.split(|c: char| !(c.is_alphanumeric() || c == '_')) {
        let word = word.to_lowercase();
        if word.chars().count() < 2 || stop.contains(word.as_str()) || !seen.insert(word.clone()) {
            continue;
        }
        out.push(word);
        if out.len() == MAX_TERMS {
            break;
        }
    }
    out
}

/// The text a query is made of: the prompt, with the previous prompt of the conversation when this one is short, with
/// secrets removed and cut to [`MAX_QUERY_CHARS`].
pub fn query_text(prompt: &str, previous: Option<&str>) -> String {
    let mut text = String::new();
    if prompt.chars().count() < SHORT_PROMPT_CHARS
        && let Some(previous) = previous
    {
        text.push_str(previous);
        text.push('\n');
    }
    text.push_str(prompt);
    redact_text(&text).chars().take(MAX_QUERY_CHARS).collect()
}

/// A rough size in tokens, the same rule as `Message::estimated_tokens`, times the model's calibration factor.
pub fn tokens(text: &str, factor: f64) -> u64 {
    let raw = (text.chars().count() as u64).div_ceil(4) + 4;
    (raw as f64 * factor).ceil() as u64
}

/// Make text safe to put inside `<tag>…</tag>`: a closing tag in it is defused, so it cannot end its own wrapper.
pub fn defuse(text: &str, tag: &str) -> String {
    text.replace(&format!("</{tag}"), &format!("<\\/{tag}"))
}

/// Escape a value for a double-quoted attribute.
pub fn attribute(value: &str) -> String {
    value
        .replace('&', "&amp;")
        .replace('"', "&quot;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
        .replace(['\n', '\r'], " ")
}

/// At most `max` characters, ending with "…" when cut.
pub fn cut(text: &str, max: usize) -> String {
    if text.chars().count() <= max {
        return text.to_string();
    }
    let mut out: String = text.chars().take(max.saturating_sub(1)).collect();
    out.push('…');
    out
}

/// The `YYYY-MM-DD` of a timestamp.
pub fn date(at: zorix_core::Timestamp) -> String {
    at.to_rfc3339().chars().take(10).collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn terms_drop_stopwords_short_words_and_repeats_in_two_languages() {
        assert_eq!(
            terms("How do the deploys go through staging, and staging again?"),
            ["deploys", "go", "through", "staging", "again"]
        );
        assert_eq!(terms("Como funciona o deploy para a produção?"), ["funciona", "deploy", "produção"]);
        assert_eq!(
            terms("snake_case X 42 id"),
            ["snake_case", "42", "id"],
            "identifiers stay whole; one-letter words go"
        );
        assert!(terms("the and of").is_empty());
        let long: String = (0..100).map(|i| format!("word{i} ")).collect();
        assert_eq!(terms(&long).len(), MAX_TERMS);
    }

    #[test]
    fn a_short_prompt_borrows_the_one_before_it_and_secrets_never_get_in() {
        assert_eq!(
            query_text("and the second one?", Some("List the deploy steps")),
            "List the deploy steps\nand the second one?"
        );
        assert_eq!(
            query_text(
                "a prompt that is long enough not to need any help from the one before it at all",
                Some("earlier")
            ),
            "a prompt that is long enough not to need any help from the one before it at all"
        );
        assert!(!query_text("use key sk-abcdefghijklmnopqrstuvwxyz0123456789 now", None).contains("sk-abcdef"));
        assert_eq!(query_text(&"x".repeat(5_000), None).chars().count(), MAX_QUERY_CHARS);
    }

    #[test]
    fn tokens_follow_the_estimate_and_the_factor() {
        assert_eq!(tokens(&"x".repeat(400), 1.0), 104);
        assert_eq!(tokens(&"x".repeat(400), 2.0), 208);
        assert_eq!(tokens("", 1.0), 4);
    }

    #[test]
    fn a_closing_tag_cannot_end_its_wrapper_and_attributes_cannot_break_out() {
        assert_eq!(defuse("a </document> b </document", "document"), "a <\\/document> b <\\/document");
        assert_eq!(defuse("</past_work>", "document"), "</past_work>", "only the named tag");
        assert_eq!(attribute("a\"b<c>\nd&e"), "a&quot;b&lt;c&gt; d&amp;e");
        assert_eq!(cut("abcdef", 4), "abc…");
        assert_eq!(cut("abc", 4), "abc");
        assert_eq!(date(zorix_core::Timestamp(0)), "1970-01-01");
    }
}
