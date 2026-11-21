//! `{{path}}` templates in prompts, messages and addresses (ADR-0033).
//!
//! A template reads values from the run: `{{trigger.payload.title}}`, `{{steps.triage.output.text}}`, `{{item}}`. A path that
//! points at nothing is an error, never an empty string, so a typo cannot send an empty message.

use serde_json::Value;

use crate::expr::{ExprError, Segment, lookup, parse_path};

/// Why a template could not be filled.
#[derive(Clone, Debug, PartialEq, Eq, thiserror::Error)]
#[error("{0}")]
pub struct TemplateError(pub String);

impl From<ExprError> for TemplateError {
    fn from(e: ExprError) -> Self {
        TemplateError(e.0)
    }
}

enum Piece<'a> {
    Text(&'a str),
    Path(&'a str),
}

fn pieces(template: &str) -> Result<Vec<Piece<'_>>, TemplateError> {
    let mut out = Vec::new();
    let mut rest = template;
    while let Some(open) = rest.find("{{") {
        if open > 0 {
            out.push(Piece::Text(&rest[..open]));
        }
        let after = &rest[open + 2..];
        let Some(close) = after.find("}}") else {
            return Err(TemplateError("A {{ is never closed with }}.".into()));
        };
        out.push(Piece::Path(after[..close].trim()));
        rest = &after[close + 2..];
    }
    if !rest.is_empty() {
        out.push(Piece::Text(rest));
    }
    Ok(out)
}

/// The paths a template reads, checked for form.
pub fn paths(template: &str) -> Result<Vec<Vec<Segment>>, TemplateError> {
    pieces(template)?
        .into_iter()
        .filter_map(|p| match p {
            Piece::Path(path) => Some(parse_path(path).map_err(TemplateError::from)),
            Piece::Text(_) => None,
        })
        .collect()
}

fn show(value: &Value) -> String {
    match value {
        Value::Null => String::new(),
        Value::String(s) => s.clone(),
        other => other.to_string(),
    }
}

fn read<'a>(root: &'a Value, path: &str) -> Result<&'a Value, TemplateError> {
    let parsed = parse_path(path)?;
    lookup(root, &parsed).ok_or_else(|| {
        TemplateError(format!("{{{{{path}}}}} points at nothing: that value does not exist in this run (yet)."))
    })
}

/// Fill a template with text.
pub fn render(template: &str, root: &Value) -> Result<String, TemplateError> {
    let mut out = String::new();
    for piece in pieces(template)? {
        match piece {
            Piece::Text(t) => out.push_str(t),
            Piece::Path(p) => out.push_str(&show(read(root, p)?)),
        }
    }
    Ok(out)
}

/// A template that is exactly one `{{path}}` gives that value as it is (a list stays a list); anything else gives text.
pub fn render_value(template: &str, root: &Value) -> Result<Value, TemplateError> {
    let parts = pieces(template)?;
    if let [Piece::Path(p)] = parts.as_slice() {
        return Ok(read(root, p)?.clone());
    }
    Ok(Value::String(render(template, root)?))
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::*;

    fn root() -> Value {
        json!({"trigger": {"payload": {"title": "Checkout fails", "n": 3, "labels": ["a", "b"]}}, "steps": {"triage": {"output": {"text": "High"}}}})
    }

    #[test]
    fn values_are_put_into_text() {
        assert_eq!(
            render("Issue: {{trigger.payload.title}} ({{ trigger.payload.n }})", &root()).unwrap(),
            "Issue: Checkout fails (3)"
        );
        assert_eq!(render("{{steps.triage.output.text}}", &root()).unwrap(), "High");
        assert_eq!(render("no templates", &root()).unwrap(), "no templates");
        assert_eq!(render("{{trigger.payload.labels}}", &root()).unwrap(), "[\"a\",\"b\"]");
    }

    #[test]
    fn a_missing_value_is_an_error_not_an_empty_string() {
        let error = render("Hello {{trigger.payload.nope}}", &root()).unwrap_err().to_string();
        assert!(error.contains("trigger.payload.nope") && error.contains("points at nothing"), "{error}");
        assert!(render("{{steps.other.output}}", &root()).is_err());
    }

    #[test]
    fn a_broken_template_is_refused() {
        assert!(render("{{trigger.payload.title", &root()).is_err());
        assert!(render("{{}}", &root()).is_err());
        assert!(render("{{a..b}}", &root()).is_err());
        assert!(paths("{{ok}} and {{a..b}}").is_err());
        assert_eq!(paths("{{a.b}} {{c}}").unwrap().len(), 2);
    }

    #[test]
    fn one_path_alone_keeps_its_type() {
        assert_eq!(render_value("{{trigger.payload.labels}}", &root()).unwrap(), json!(["a", "b"]));
        assert_eq!(render_value("{{trigger.payload.n}}", &root()).unwrap(), json!(3));
        assert_eq!(render_value("n={{trigger.payload.n}}", &root()).unwrap(), json!("n=3"));
    }
}
