//! Tool parameters that a server wants mirrored into HTTP headers (`x-mcp-header`, revision 2026-07-28), and the encoding of
//! values that cannot travel as plain header text.

use std::collections::HashSet;

use base64::Engine;
use base64::engine::general_purpose::STANDARD;
use serde_json::Value;

/// A parameter to mirror: where it is in the arguments, and the name part of its header.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Mirror {
    /// The chain of property names from the root of the arguments.
    pub path: Vec<String>,
    /// The `x-mcp-header` value: the header is `Mcp-Param-<name>`.
    pub name: String,
}

fn token_char(c: char) -> bool {
    c.is_ascii_alphanumeric() || "!#$%&'*+-.^_`|~".contains(c)
}

/// Find the `x-mcp-header` annotations of an input schema, or say why the tool must be rejected.
pub fn mirrors(schema: &Value) -> Result<Vec<Mirror>, String> {
    let mut found = Vec::new();
    walk(schema, Some(Vec::new()), &mut found)?;
    let mut seen = HashSet::new();
    for mirror in &found {
        if !seen.insert(mirror.name.to_ascii_lowercase()) {
            return Err(format!("two parameters use the header name “{}”", mirror.name));
        }
    }
    Ok(found)
}

fn walk(node: &Value, path: Option<Vec<String>>, found: &mut Vec<Mirror>) -> Result<(), String> {
    let Some(object) = node.as_object() else {
        if let Some(items) = node.as_array() {
            for item in items {
                walk(item, None, found)?;
            }
        }
        return Ok(());
    };
    if let Some(annotation) = object.get("x-mcp-header") {
        let name = annotation.as_str().unwrap_or("");
        if name.is_empty() || !name.chars().all(token_char) {
            return Err(format!("“{annotation}” is not a valid x-mcp-header name"));
        }
        let Some(path) = &path else {
            return Err(format!(
                "x-mcp-header “{name}” is somewhere a parameter cannot be mirrored from (inside an array or a combination of schemas)"
            ));
        };
        match object.get("type").and_then(Value::as_str) {
            Some("string" | "integer" | "boolean") => {}
            _ => {
                return Err(format!(
                    "x-mcp-header “{name}” is on a parameter that is not a string, integer or boolean"
                ));
            }
        }
        found.push(Mirror { path: path.clone(), name: name.to_string() });
    }
    for (key, value) in object {
        if key == "properties" {
            if let Some(properties) = value.as_object() {
                for (property, schema) in properties {
                    let next =
                        path.as_ref().map(|p| p.iter().cloned().chain(std::iter::once(property.clone())).collect());
                    walk(schema, next, found)?;
                }
            }
        } else if key != "x-mcp-header" && (value.is_object() || value.is_array()) {
            walk(value, None, found)?;
        }
    }
    Ok(())
}

/// A value as it travels in a header: plain when it can, otherwise `=?base64?…?=`.
pub fn encode_value(value: &str) -> String {
    let plain = value.chars().all(|c| c == ' ' || c == '\t' || ('\u{21}'..='\u{7e}').contains(&c))
        && value == value.trim_matches([' ', '\t'])
        && !(value.starts_with("=?base64?") && value.ends_with("?="));
    if plain { value.to_string() } else { format!("=?base64?{}?=", STANDARD.encode(value.as_bytes())) }
}

/// The `Mcp-Param-*` headers for a call.
pub fn param_headers(mirrors: &[Mirror], arguments: &Value) -> Vec<(String, String)> {
    mirrors
        .iter()
        .filter_map(|mirror| {
            let mut at = arguments;
            for step in &mirror.path {
                at = at.get(step)?;
            }
            let text = match at {
                Value::String(s) => s.clone(),
                Value::Bool(b) => b.to_string(),
                Value::Number(n) if n.is_i64() || n.is_u64() => n.to_string(),
                _ => return None,
            };
            Some((format!("Mcp-Param-{}", mirror.name), encode_value(&text)))
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn annotated_parameters_are_found_by_their_property_path() {
        let schema = json!({"type": "object", "properties": {
            "region": {"type": "string", "x-mcp-header": "Region"},
            "query": {"type": "string"},
            "opts": {"type": "object", "properties": {"tier": {"type": "integer", "x-mcp-header": "Tier"}}}
        }});
        let mut found = mirrors(&schema).unwrap();
        found.sort_by(|a, b| a.name.cmp(&b.name));
        assert_eq!(
            found,
            [
                Mirror { path: vec!["region".into()], name: "Region".into() },
                Mirror { path: vec!["opts".into(), "tier".into()], name: "Tier".into() }
            ]
        );
        assert!(mirrors(&json!({"type": "object"})).unwrap().is_empty());
    }

    #[test]
    fn a_tool_with_a_bad_annotation_is_rejected_for_the_reason() {
        let bad = |schema: Value| mirrors(&schema).unwrap_err();
        assert!(bad(json!({"properties": {"a": {"type": "string", "x-mcp-header": ""}}})).contains("not a valid"));
        assert!(
            bad(json!({"properties": {"a": {"type": "string", "x-mcp-header": "has space"}}})).contains("not a valid")
        );
        assert!(
            bad(json!({"properties": {"a": {"type": "number", "x-mcp-header": "N"}}}))
                .contains("not a string, integer or boolean")
        );
        assert!(
            bad(json!({"properties": {"a": {"type": "array", "items": {"type": "string", "x-mcp-header": "I"}}}}))
                .contains("cannot be mirrored")
        );
        assert!(
            bad(json!({"anyOf": [{"properties": {"a": {"type": "string", "x-mcp-header": "A"}}}]}))
                .contains("cannot be mirrored")
        );
        assert!(bad(json!({"properties": {"a": {"type": "string", "x-mcp-header": "Same"}, "b": {"type": "string", "x-mcp-header": "same"}}})).contains("two parameters"));
    }

    #[test]
    fn values_that_cannot_be_plain_header_text_are_base64_with_the_sentinel() {
        assert_eq!(encode_value("us-west1"), "us-west1");
        assert_eq!(encode_value("Hello, 世界"), "=?base64?SGVsbG8sIOS4lueVjA==?=");
        assert_eq!(encode_value(" padded "), "=?base64?IHBhZGRlZCA=?=");
        assert_eq!(encode_value("line1\nline2"), "=?base64?bGluZTEKbGluZTI=?=");
        assert_eq!(encode_value("=?base64?literal?="), "=?base64?PT9iYXNlNjQ/bGl0ZXJhbD89?=");
    }

    #[test]
    fn headers_are_built_from_the_call_and_missing_or_null_values_are_left_out() {
        let found = vec![
            Mirror { path: vec!["region".into()], name: "Region".into() },
            Mirror { path: vec!["n".into()], name: "N".into() },
            Mirror { path: vec!["on".into()], name: "On".into() },
        ];
        let headers = param_headers(&found, &json!({"region": "us-west1", "n": 42, "on": true, "x": 1}));
        assert_eq!(
            headers,
            [
                ("Mcp-Param-Region".into(), "us-west1".into()),
                ("Mcp-Param-N".into(), "42".into()),
                ("Mcp-Param-On".into(), "true".into())
            ]
        );
        assert!(param_headers(&found, &json!({"region": null})).is_empty());
    }
}
