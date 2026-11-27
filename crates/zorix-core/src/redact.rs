//! Keeping secrets out of events, logs and traces.
//!
//! Two layers, both best effort and both documented as such in `SECURITY.md`:
//! 1. values stored under a key that names a secret (`api_key`, `token`, `password`, …) are replaced;
//! 2. words in text that look like well-known credentials (`sk-…`, `ghp_…`, `AIza…`, `Bearer …`, JWTs), the value of a
//!    `password=…` or `secret: …` pair, and PEM private key blocks are replaced.
//!
//! The arguments of a tool call are kept as they are in the tool call record, because the tool needs them. They are
//! redacted wherever they are *published* (events) or *written to logs*.

use crate::secret::REDACTED;
use serde_json::Value;

/// Key names that always hold a secret when they appear anywhere in the key.
const SECRET_WORDS: &[&str] = &[
    "api_key",
    "apikey",
    "api-key",
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
    if SECRET_WORDS.iter().any(|word| key.contains(word)) {
        return true;
    }
    // A token is a secret, but a *count* of tokens is not: `input_tokens` and `max_total_tokens` must stay readable.
    key == "token" || key.ends_with("_token") || key.ends_with("-token") || key.ends_with(".token")
}

fn looks_like_credential(word: &str) -> bool {
    let word = word.trim_matches(|c: char| !c.is_ascii_alphanumeric() && !matches!(c, '-' | '_' | '.'));
    let long_enough = |min: usize| word.len() >= min;
    let plain =
        |min: usize| word.len() >= min && word.chars().all(|c| c.is_ascii_alphanumeric() || matches!(c, '-' | '_'));
    (word.starts_with("sk-") && long_enough(20))
        || (word.starts_with("ghp_") && long_enough(30))
        || (word.starts_with("github_pat_") && long_enough(30))
        || (["gho_", "ghu_", "ghs_", "ghr_"].iter().any(|p| word.starts_with(p)) && plain(30))
        || (word.starts_with("xox") && word.contains('-') && long_enough(20))
        || (word.starts_with("AKIA") && word.len() == 20 && word.chars().all(|c| c.is_ascii_alphanumeric()))
        // A Google API key is `AIza` and 35 more characters.
        || (word.starts_with("AIza") && word.len() == 39 && plain(39))
        // Stripe secret and restricted keys, live or test.
        || (["sk_live_", "sk_test_", "rk_live_", "rk_test_"].iter().any(|p| word.starts_with(p)) && plain(20))
        || (word.starts_with("glpat-") && plain(20))
        || (word.starts_with("eyJ") && word.matches('.').count() == 2 && long_enough(30))
}

/// A key whose value is a secret when it appears in text: the names `redact` hides in JSON, and `pwd`.
fn is_secret_text_key(key: &str) -> bool {
    is_secret_key(key) || key.eq_ignore_ascii_case("pwd")
}

fn is_key_char(c: char) -> bool {
    c.is_ascii_alphanumeric() || matches!(c, '_' | '-' | '.')
}

/// Where the value of a `key=value` pair ends inside one word.
fn ends_value(c: char) -> bool {
    matches!(c, '&' | ',' | ';' | '"' | '\'' | ')' | '}' | ']' | '>' | '`')
}

/// Replace the value of every `secret=value` and `secret:value` pair inside one word. The second result says the word ends in a
/// bare `secret:` or `secret=`, so the value is the next word.
fn redact_pairs(word: &str) -> (String, bool) {
    let mut out = String::with_capacity(word.len());
    let mut copied = 0; // what has been written to `out`
    let mut search = 0;
    let mut pending = false;
    while let Some(offset) = word[search..].find(['=', ':']) {
        let sep = search + offset;
        search = sep + 1;
        // The key is the run of key characters before the separator, with one closing quote allowed in between (`"password":`).
        let key_end = if word[..sep].ends_with(['"', '\'']) { sep - 1 } else { sep };
        let key_start = word[..key_end]
            .char_indices()
            .rev()
            .take_while(|(_, c)| is_key_char(*c))
            .last()
            .map_or(key_end, |(at, _)| at);
        let key = &word[key_start..key_end];
        if key.is_empty() || key_start < copied || !is_secret_text_key(key) {
            continue;
        }
        let after = &word[sep + 1..];
        let value_start = sep + 1 + (after.len() - after.trim_start_matches(['"', '\'']).len());
        let rest = &word[value_start..];
        if rest.starts_with(REDACTED) {
            continue;
        }
        let value_len = rest.find(ends_value).unwrap_or(rest.len());
        if value_len == 0 {
            // `password:` at the end of the word: the value is the next word.
            pending |= rest.is_empty();
            continue;
        }
        out.push_str(&word[copied..value_start]);
        out.push_str(REDACTED);
        copied = value_start + value_len;
        search = copied;
    }
    out.push_str(&word[copied..]);
    (out, pending)
}

/// Replace every PEM private key block, from its `BEGIN` line to its `END` line (or to the end of the text when it has none, so a
/// cut-off key is not left behind). A certificate or a public key is public and stays.
fn redact_private_keys(text: &str) -> String {
    const BEGIN: &str = "-----BEGIN ";
    const DASHES: &str = "-----";
    let mut out = String::with_capacity(text.len());
    let mut rest = text;
    while let Some(start) = rest.find(BEGIN) {
        let after = &rest[start + BEGIN.len()..];
        let Some(label_end) = after.find(DASHES) else { break };
        let label = &after[..label_end];
        let body_start = start + BEGIN.len() + label_end + DASHES.len();
        if !label.contains("PRIVATE KEY") {
            out.push_str(&rest[..body_start]);
            rest = &rest[body_start..];
            continue;
        }
        out.push_str(&rest[..start]);
        out.push_str(REDACTED);
        let end = format!("-----END {label}{DASHES}");
        let body = &rest[body_start..];
        rest = match body.find(&end) {
            Some(at) => &body[at + end.len()..],
            None => "",
        };
    }
    out.push_str(rest);
    out
}

/// Words that name how a credential is presented: the secret is the word after them.
fn is_scheme(word: &str) -> bool {
    ["basic", "bearer", "token", "digest"].iter().any(|s| word.eq_ignore_ascii_case(s))
}

/// A word that is the value of a secret: its quotes and closing punctuation stay, the value goes.
fn redact_value(word: &str) -> String {
    let lead = word.len() - word.trim_start_matches(['"', '\'']).len();
    let rest = &word[lead..];
    let len = rest.find(ends_value).unwrap_or(rest.len());
    if len == 0 || rest.starts_with(REDACTED) {
        return word.to_string();
    }
    format!("{}{REDACTED}{}", &word[..lead], &rest[len..])
}

/// Replace credential-looking words, the value of `secret=…` and `secret: …` pairs, private key blocks and the word after `Bearer`.
pub fn redact_text(text: &str) -> String {
    let text = redact_private_keys(text);
    let mut out = String::with_capacity(text.len());
    let mut next_is_secret = false;
    for piece in text.split_inclusive(char::is_whitespace) {
        let word = piece.trim_end_matches(char::is_whitespace);
        let spacing = &piece[word.len()..];
        if word.is_empty() {
            // Only spacing.
        } else if next_is_secret && !is_scheme(word) {
            out.push_str(&redact_value(word));
            next_is_secret = false;
        } else if next_is_secret {
            // `Basic dXNlcjpw`: the scheme stays, the value after it goes.
            out.push_str(word);
        } else if looks_like_credential(word) {
            out.push_str(REDACTED);
        } else {
            let (clean, pending) = redact_pairs(word);
            out.push_str(&clean);
            next_is_secret = pending || word.eq_ignore_ascii_case("bearer");
        }
        out.push_str(spacing);
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
    fn counts_of_tokens_are_not_secrets_but_tokens_are() {
        let value = json!({
            "usage": {"input_tokens": 40, "output_tokens": 2},
            "limits": {"max_total_tokens": 1000},
            "token": "abc",
            "access_token": "abc",
            "refresh-token": "abc",
            "x-auth-token": "abc",
        });
        let redacted = redact(value);
        assert_eq!(redacted["usage"], json!({"input_tokens": 40, "output_tokens": 2}));
        assert_eq!(redacted["limits"]["max_total_tokens"], 1000);
        for key in ["token", "access_token", "refresh-token", "x-auth-token"] {
            assert_eq!(redacted[key], REDACTED, "{key}");
        }
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
    fn the_value_of_a_secret_pair_in_text_is_replaced_whatever_the_punctuation() {
        for (text, want) in [
            ("password=hunter2", "password=[redacted]"),
            ("login with password: hunter2 and go", "login with password: [redacted] and go"),
            ("https://x.test/cb?token=abc123&page=2", "https://x.test/cb?token=[redacted]&page=2"),
            (r#"{"password":"hunter2","user":"me"}"#, r#"{"password":"[redacted]","user":"me"}"#),
            (r#"{"client_secret": "s3cret-value"}"#, r#"{"client_secret": "[redacted]"}"#),
            ("Authorization: Basic dXNlcjpwYXNz ok", "Authorization: Basic [redacted] ok"),
            ("API_KEY=abc;next", "API_KEY=[redacted];next"),
            ("password=[redacted]", "password=[redacted]"),
        ] {
            assert_eq!(redact_text(text), want, "{text}");
        }
    }

    #[test]
    fn text_that_only_mentions_a_secret_word_is_left_alone() {
        for text in [
            "The password policy needs 12 characters.",
            "Set a password later",
            "input_tokens: 40 max_total_tokens=1000",
            "Stopped: the cost limit was reached.",
            "Missing key: set the provider key",
            "password:",
        ] {
            assert_eq!(redact_text(text), text, "{text}");
        }
    }

    #[test]
    fn a_private_key_block_is_replaced_whole_and_a_certificate_is_not() {
        let key = "-----BEGIN RSA PRIVATE KEY-----\nMIIEowIBAAKCAQEA\nabc\n-----END RSA PRIVATE KEY-----";
        assert_eq!(redact_text(&format!("key follows\n{key}\nthen go on")), "key follows\n[redacted]\nthen go on");
        let open = "-----BEGIN PRIVATE KEY-----\nMIIEvQIBADANBg";
        assert_eq!(
            redact_text(&format!("leaked {open}")),
            "leaked [redacted]",
            "a block with no end is cut at the start"
        );
        let certificate = "-----BEGIN CERTIFICATE-----\nMIIC\n-----END CERTIFICATE-----";
        assert_eq!(redact_text(certificate), certificate, "a certificate is public");
    }

    #[test]
    fn google_stripe_gitlab_and_other_provider_keys_are_recognised() {
        let google = format!("AIza{}", "Sy".repeat(18).chars().take(35).collect::<String>());
        let stripe = format!("sk_live_{}", "4eC39HqLyjWDarjtT1zdp7dc");
        let restricted = format!("rk_live_{}", "4eC39HqLyjWDarjtT1zdp7dc");
        let gitlab = format!("glpat-{}", "xxxxxxxxxxxxxxxxxxxx");
        let github_app = format!("ghs_{}", "a".repeat(36));
        for key in [&google, &stripe, &restricted, &gitlab, &github_app] {
            assert_eq!(redact_text(&format!("use {key} now")), "use [redacted] now", "{key}");
        }
        for harmless in ["AIza", "sk_live_x", "glpat-", "ghs_short", "AIzaSyShort"] {
            assert_eq!(redact_text(harmless), harmless);
        }
    }

    #[test]
    fn spacing_and_newlines_are_preserved() {
        assert_eq!(redact_text("a  b\n\tc"), "a  b\n\tc");
    }
}
