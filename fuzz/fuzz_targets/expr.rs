//! The condition language of workflows: any text parses or is refused, and what parses can be tested against any JSON without a panic.
#![no_main]
use libfuzzer_sys::fuzz_target;
use serde_json::json;
use zorix_workflows::expr::{Expr, parse_path};

fuzz_target!(|text: &str| {
    let _ = parse_path(text);
    if let Ok(expr) = Expr::parse(text) {
        for value in [json!({}), json!({"a": 1, "b": {"c": [1, 2, 3], "d": "text"}, "n": null}), json!([1, "x", null])] {
            let _ = expr.test(&value);
        }
    }
});
