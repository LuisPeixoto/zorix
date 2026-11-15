//! Keeping secrets out of events, logs and traces.
//!
//! Two layers, both best effort and both documented as such in `SECURITY.md`:
//! 1. values stored under a key that names a secret (`api_key`, `token`, `password`, …) are replaced;
//! 2. words in text that look like well-known credentials (`sk-…`, `ghp_…`, `Bearer …`, JWTs) are replaced.
//!
//! The arguments of a tool call are kept as they are in the tool call record, because the tool needs them. They are
//! redacted wherever they are *published* (events) or *written to logs*.

use std::fmt;

use serde::{Serialize, Serializer};
use serde_json::Value;

/// What replaces a secret.
pub const REDACTED: &str = "[redacted]";

const SECRET_KEYS: &[&str] = &[
    "api_key",
    "apikey",
    "api-key",
    "token",
    "password",
    "passwd",
    "secret",
    "authorization",
    "credential",
    "cookie",
    "private_key",
    "privatekey",
    "access_key",
];

fn is_secret_key(key: &str) -> bool {
    let key = key.to_ascii_lowercase();
    SECRET_KEYS.iter().any(|needle| key.contains(needle))
}

fn looks_like_credential(word: &str) -> bool {
    let word = word.trim_matches(|c: char| !c.is_ascii_alphanumeric() && !matches!(c, '-' | '_' | '.'));
    let long_enough = |min: usize| word.len() >= min;
    (word.starts_with("sk-") && long_enough(20))
        || (word.starts_with("ghp_") && long_enough(30))
        || (word.starts_with("github_pat_") && long_enough(30))
        || (word.starts_with("xox") && word.contains('-') && long_enough(20))
        || (word.starts_with("AKIA") && word.len() == 20 && word.chars().all(|c| c.is_ascii_alphanumeric()))
        || (word.starts_with("eyJ") && word.matches('.').count() == 2 && long_enough(30))
}

/// Replace credential-looking words and the word after `Bearer`.
pub fn redact_text(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    let mut after_bearer = false;
    for piece in text.split_inclusive(char::is_whitespace) {
        let word = piece.trim_end_matches(char::is_whitespace);
        let spacing = &piece[word.len()..];
        if !word.is_empty() && (after_bearer || looks_like_credential(word)) {
            out.push_str(REDACTED);
        } else {
            out.push_str(word);
        }
        out.push_str(spacing);
        after_bearer = word.eq_ignore_ascii_case("bearer");
    }
    out
}

/// Redact a JSON value: secret keys lose their value, and every string is scanned for credentials.
pub fn redact(value: Value) -> Value {
    match value {
        Value::Object(map) => Value::Object(
            map.into_iter()
                .map(|(key, value)| {
                    let value = if is_secret_key(&key) && !value.is_null() {
                        Value::String(REDACTED.into())
                    } else {
                        redact(value)
                    };
                    (key, value)
                })
                .collect(),
        ),
        Value::Array(items) => Value::Array(items.into_iter().map(redact).collect()),
        Value::String(text) => Value::String(redact_text(&text)),
        other => other,
    }
}

/// A value that never shows itself: not in `Debug`, not in `Display`, not when serialised.
#[derive(Clone, PartialEq, Eq)]
pub struct Secret(String);

impl Secret {
    /// Wrap a secret.
    pub fn new(value: impl Into<String>) -> Self {
        Self(value.into())
    }

    /// The secret itself. Call this only at the moment of use.
    pub fn expose(&self) -> &str {
        &self.0
    }
}

impl fmt::Debug for Secret {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("Secret(***)")
    }
}

impl fmt::Display for Secret {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("***")
    }
}

impl Serialize for Secret {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        serializer.serialize_str(REDACTED)
    }
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::*;

    #[test]
    fn values_under_secret_keys_are_replaced_at_any_depth() {
        let value = json!({
            "url": "https://example.com",
            "headers": {"Authorization": "Bearer abc", "accept": "json"},
            "list": [{"api_key": "k1"}, {"X-Auth-Token": "t"}],
            "password": null,
        });
        let redacted = redact(value);
        assert_eq!(redacted["url"], "https://example.com");
        assert_eq!(redacted["headers"]["Authorization"], REDACTED);
        assert_eq!(redacted["headers"]["accept"], "json");
        assert_eq!(redacted["list"][0]["api_key"], REDACTED);
        assert_eq!(redacted["list"][1]["X-Auth-Token"], REDACTED);
        assert_eq!(redacted["password"], Value::Null, "nothing to hide");
    }

    #[test]
    fn credential_looking_words_are_replaced_inside_text() {
        let text =
            "call with sk-abcdefghijklmnopqrstuvwxyz and ghp_abcdefghijklmnopqrstuvwxyz0123456789, then Bearer abc.def";
        let out = redact_text(text);
        assert_eq!(out, "call with [redacted] and [redacted] then Bearer [redacted]");
    }

    #[test]
    fn ordinary_text_and_numbers_survive() {
        let text = "The total is 391.\nSee notes.txt (line 2) — sk-short is not a key.";
        assert_eq!(redact_text(text), text);
        assert_eq!(redact(json!({"n": 391, "ok": true})), json!({"n": 391, "ok": true}));
    }

    #[test]
    fn jwts_and_aws_keys_are_recognised() {
        let jwt = format!("eyJ{}.{}.{}", "a".repeat(12), "b".repeat(12), "c".repeat(12));
        assert_eq!(redact_text(&format!("token {jwt}")), "token [redacted]");
        assert_eq!(redact_text("key AKIAABCDEFGHIJKLMNOP end"), "key [redacted] end");
    }

    #[test]
    fn spacing_and_newlines_are_preserved() {
        assert_eq!(redact_text("a  b\n\tc"), "a  b\n\tc");
    }

    #[test]
    fn a_secret_never_shows_itself() {
        let secret = Secret::new("hunter2");
        assert_eq!(format!("{secret:?}"), "Secret(***)");
        assert_eq!(format!("{secret}"), "***");
        assert_eq!(serde_json::to_string(&secret).unwrap(), "\"[redacted]\"");
        assert_eq!(secret.expose(), "hunter2");
        assert!(!format!("{:?}", json!({"s": secret.expose().len()})).contains("hunter2"));
    }
}
