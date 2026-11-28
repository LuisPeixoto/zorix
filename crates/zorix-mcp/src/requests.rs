//! What a server asks while one of its tools runs, read from the protocol and answered in it (ADR-0051).
//!
//! The request is the server's text: it is bounded and checked here, and everything that is not plain text and plain fields is
//! refused before anybody sees it.

use serde_json::{Map, Value, json};
use zorix_core::{
    CallHost, ElicitReply, FieldKind, FormField, MAX_CHOICES, MAX_FIELDS, MAX_QUESTION_BYTES, MAX_SAMPLE_BYTES,
    MAX_SAMPLE_MESSAGES, MAX_SAMPLE_TOKENS, Question, SampleMessage, SampleRequest, SampleRole,
};

use crate::error::METHOD_NOT_FOUND;

const SAMPLING: &str = "sampling/createMessage";
const ELICITATION: &str = "elicitation/create";
const INVALID_PARAMS: i64 = -32602;
/// The specification's code for a person who said no to sampling.
const REJECTED: i64 = -1;
const MAX_URL: usize = 2048;

/// The capabilities ZORIX declares when it can pass requests on.
pub(crate) fn capabilities(answers: bool) -> Value {
    if answers { json!({"sampling": {}, "elicitation": {"form": {}, "url": {}}}) } else { json!({}) }
}

/// Answer one request of `server`, through `host`. The error is a JSON-RPC code and message.
pub(crate) async fn answer(
    server: &str,
    host: &dyn CallHost,
    method: &str,
    params: &Value,
) -> Result<Value, (i64, String)> {
    match method {
        SAMPLING => {
            let request = sample(params).map_err(|why| (INVALID_PARAMS, why))?;
            let reply = host.sample(server, request).await.map_err(|why| (REJECTED, why))?;
            Ok(json!({
                "role": "assistant",
                "content": {"type": "text", "text": reply.text},
                "model": reply.model,
                "stopReason": if reply.at_limit { "maxTokens" } else { "endTurn" },
            }))
        }
        ELICITATION => {
            let question = question(params).map_err(|why| (INVALID_PARAMS, why))?;
            let link = matches!(question, Question::Link { .. });
            Ok(match host.ask(server, question).await {
                Ok(ElicitReply::Accepted(_)) if link => json!({"action": "accept"}),
                Ok(ElicitReply::Accepted(values)) => json!({"action": "accept", "content": values}),
                Ok(ElicitReply::Declined) => json!({"action": "decline"}),
                // Nobody could be asked: the specification's word for a question dismissed without a choice.
                Err(_) => json!({"action": "cancel"}),
            })
        }
        _ => Err((METHOD_NOT_FOUND, "ZORIX does not answer that request.".into())),
    }
}

fn text_of(content: &Value) -> Option<String> {
    match content {
        Value::Array(blocks) => blocks.iter().map(text_of).collect::<Option<Vec<_>>>().map(|t| t.join("\n")),
        Value::Object(block) if block.get("type")?.as_str()? == "text" => {
            block.get("text")?.as_str().map(str::to_string)
        }
        _ => None,
    }
}

fn sample(params: &Value) -> Result<SampleRequest, String> {
    if ["tools", "toolChoice"].iter().any(|key| params.get(*key).is_some_and(|v| !v.is_null())) {
        return Err("ZORIX does not give a server's model call tools.".into());
    }
    let raw = params["messages"].as_array().ok_or("messages are missing")?;
    if raw.is_empty() || raw.len() > MAX_SAMPLE_MESSAGES {
        return Err(format!("ZORIX passes on 1 to {MAX_SAMPLE_MESSAGES} messages."));
    }
    let messages = raw
        .iter()
        .map(|message| {
            let role = match message["role"].as_str() {
                Some("user") => SampleRole::User,
                Some("assistant") => SampleRole::Assistant,
                _ => return Err("A message has no known role.".to_string()),
            };
            let text =
                text_of(&message["content"]).ok_or("ZORIX passes on text only, not images, audio or tool results.")?;
            Ok(SampleMessage { role, text })
        })
        .collect::<Result<Vec<_>, String>>()?;
    let system = params["systemPrompt"].as_str().filter(|s| !s.is_empty()).map(str::to_string);
    let size: usize = messages.iter().map(|m| m.text.len()).sum::<usize>() + system.as_ref().map_or(0, String::len);
    if size > MAX_SAMPLE_BYTES {
        return Err(format!("The request is longer than the {} KiB ZORIX passes on.", MAX_SAMPLE_BYTES / 1024));
    }
    let max_tokens = params["maxTokens"].as_u64().filter(|n| *n > 0).ok_or("maxTokens is missing")?;
    let max_tokens = u32::try_from(max_tokens).unwrap_or(MAX_SAMPLE_TOKENS).min(MAX_SAMPLE_TOKENS);
    Ok(SampleRequest { system, messages, max_tokens })
}

fn bounded(value: &Value, max: usize) -> Option<String> {
    value.as_str().filter(|s| !s.is_empty()).map(|s| s.chars().take(max).collect())
}

fn question(params: &Value) -> Result<Question, String> {
    let message = params["message"].as_str().unwrap_or("");
    if message.is_empty() || message.len() > MAX_QUESTION_BYTES {
        return Err(format!("A question needs a message of at most {MAX_QUESTION_BYTES} bytes."));
    }
    let message = message.to_string();
    match params["mode"].as_str() {
        Some("url") => {
            let url = params["url"].as_str().filter(|u| u.len() <= MAX_URL).ok_or("url is missing or too long")?;
            let parsed = reqwest::Url::parse(url).map_err(|_| "url is not an address".to_string())?;
            let local = matches!(parsed.host_str(), Some("localhost" | "127.0.0.1" | "[::1]"));
            if !(parsed.scheme() == "https" || parsed.scheme() == "http" && local)
                || !parsed.username().is_empty()
                || parsed.password().is_some()
                || parsed.host_str().is_none()
            {
                return Err("ZORIX opens only https addresses, without a user name in them.".into());
            }
            Ok(Question::Link { message, url: parsed.to_string() })
        }
        None | Some("form") => Ok(Question::Form { message, fields: fields(&params["requestedSchema"])? }),
        Some(other) => Err(format!("ZORIX does not know the mode {other}.")),
    }
}

fn fields(schema: &Value) -> Result<Vec<FormField>, String> {
    let empty = Map::new();
    let properties = match &schema["properties"] {
        Value::Null => &empty,
        Value::Object(map) => map,
        _ => return Err("requestedSchema.properties is not an object".into()),
    };
    if properties.len() > MAX_FIELDS {
        return Err(format!("ZORIX asks at most {MAX_FIELDS} fields at once."));
    }
    let required: Vec<&str> =
        schema["required"].as_array().map(|r| r.iter().filter_map(Value::as_str).collect()).unwrap_or_default();
    properties
        .iter()
        .map(|(name, property)| {
            if name.is_empty() || name.len() > 64 {
                return Err("A field's name is empty or longer than 64 bytes.".to_string());
            }
            let choices = match (&property["enum"], &property["oneOf"]) {
                (Value::Array(values), _) => Some(values.iter().map(Value::as_str).collect::<Vec<_>>()),
                (_, Value::Array(options)) => Some(options.iter().map(|o| o["const"].as_str()).collect()),
                _ => None,
            };
            let kind = match (property["type"].as_str(), choices) {
                (Some("string"), Some(options)) => {
                    let options = options.into_iter().collect::<Option<Vec<_>>>().ok_or("A choice is not text.")?;
                    if options.is_empty() || options.len() > MAX_CHOICES || options.iter().any(|o| o.len() > 200) {
                        return Err(format!("A list of choices has 1 to {MAX_CHOICES} short texts."));
                    }
                    FieldKind::Choice { options: options.into_iter().map(str::to_string).collect() }
                }
                (Some("string"), None) => FieldKind::Text,
                (Some("number"), _) => FieldKind::Number,
                (Some("integer"), _) => FieldKind::Integer,
                (Some("boolean"), _) => FieldKind::Boolean,
                _ => return Err(format!("ZORIX does not ask for the kind of value of {name}.")),
            };
            Ok(FormField {
                name: name.clone(),
                title: bounded(&property["title"], 200),
                description: bounded(&property["description"], 500),
                kind,
                required: required.contains(&name.as_str()),
            })
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn message(content: Value) -> Value {
        json!({"role": "user", "content": content})
    }

    #[test]
    fn sampling_takes_bounded_text_and_no_tools() {
        let ok = sample(&json!({"messages": [message(json!({"type": "text", "text": "hi"}))], "maxTokens": 1_000_000}))
            .unwrap();
        assert_eq!(ok.max_tokens, MAX_SAMPLE_TOKENS);
        let long = "x".repeat(MAX_SAMPLE_BYTES + 1);
        for bad in [
            json!({"messages": [], "maxTokens": 1}),
            json!({"messages": [message(json!({"type": "image", "data": "AA==", "mimeType": "image/png"}))], "maxTokens": 1}),
            json!({"messages": [message(json!({"type": "text", "text": long}))], "maxTokens": 1}),
            json!({"messages": [message(json!({"type": "text", "text": "a"}))]}),
            json!({"messages": [message(json!({"type": "text", "text": "a"}))], "maxTokens": 1, "tools": []}),
            json!({"messages": [{"role": "system", "content": {"type": "text", "text": "a"}}], "maxTokens": 1}),
        ] {
            assert!(sample(&bad).is_err(), "{bad}");
        }
    }

    #[test]
    fn questions_take_flat_fields_and_https_links() {
        let form = question(&json!({"message": "Which?", "requestedSchema": {"type": "object", "properties": {
            "repo": {"type": "string", "title": "Repository"},
            "kind": {"type": "string", "oneOf": [{"const": "a", "title": "A"}]},
        }, "required": ["repo"]}}))
        .unwrap();
        let Question::Form { fields, .. } = form else { panic!() };
        assert!(fields.iter().any(|f| f.name == "repo" && f.required && f.kind == FieldKind::Text));
        assert!(fields.iter().any(|f| f.kind == FieldKind::Choice { options: vec!["a".into()] }));
        let link = |url: &str| question(&json!({"mode": "url", "message": "Go", "url": url}));
        assert!(link("https://example.com/x").is_ok() && link("http://127.0.0.1:8000/x").is_ok());
        for bad in ["http://example.com", "file:///etc/passwd", "javascript:alert(1)", "https://user@example.com", "x"]
        {
            assert!(link(bad).is_err(), "{bad}");
        }
        for bad in [
            json!({"message": "", "requestedSchema": {}}),
            json!({"message": "m", "requestedSchema": {"properties": {"a": {"type": "object"}}}}),
            json!({"message": "m", "requestedSchema": {"properties": {"a": {"type": "string", "enum": [1]}}}}),
            json!({"message": "m", "mode": "other"}),
        ] {
            assert!(question(&bad).is_err(), "{bad}");
        }
    }
}
