//! The wrappers around text from outside: whatever arrives, the wrapped text has one opening, one closing, and nothing of ours live inside.
#![no_main]
use libfuzzer_sys::fuzz_target;
use zorix_core::untrusted::{DATA_TAGS, wrap_data};

fuzz_target!(|input: (&str, &str)| {
    let (source, text) = input;
    let wrapped = wrap_data(source, text);
    assert!(wrapped.starts_with("<data source=\"") && wrapped.ends_with("</data>"));
    let body_start = wrapped.find("untrusted=\"true\">").expect("the opening tag is whole") + "untrusted=\"true\">".len();
    let body = wrapped[body_start..wrapped.len() - "</data>".len()].to_lowercase();
    for (at, _) in body.match_indices('<') {
        let rest = body[at + 1..].trim_start_matches('/').trim_start();
        if body[at + 1..].starts_with('\\') {
            continue;
        }
        for tag in DATA_TAGS {
            if let Some(next) = rest.strip_prefix(tag) {
                assert!(next.starts_with(|c: char| c.is_ascii_alphanumeric() || c == '_'), "a live {tag} tag in {body:?}");
            }
        }
    }
});
