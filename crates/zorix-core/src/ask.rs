//! What an MCP server may ask of the person while one of its tools runs: a call to the model (*sampling*) or an answer
//! (*elicitation*). The types, their limits and the check of a person's answer (ADR-0051).
//!
//! Everything in a request is text from the server and is treated as data: it is shown, never obeyed.

use std::time::Duration;

use async_trait::async_trait;
use serde::{Deserialize, Serialize};
use serde_json::{Map, Value};

use crate::approval::canonical_hash;
use crate::ids::{AskId, ExecutionId, TenantId, ToolCallId};
use crate::time::Timestamp;

/// How many requests one tool call may make.
pub const MAX_ASKS_PER_CALL: usize = 5;
/// How long a person has to answer one.
pub const ASK_TIMEOUT: Duration = Duration::from_secs(300);
/// The most output tokens a server's model call may ask for.
pub const MAX_SAMPLE_TOKENS: u32 = 4_096;
/// The most messages a sampling request carries.
pub const MAX_SAMPLE_MESSAGES: usize = 32;
/// The most text a sampling request carries, in bytes, system prompt included.
pub const MAX_SAMPLE_BYTES: usize = 32 * 1024;
/// The most fields a form asks for.
pub const MAX_FIELDS: usize = 16;
/// The most options a choice offers.
pub const MAX_CHOICES: usize = 50;
/// The longest question, in bytes.
pub const MAX_QUESTION_BYTES: usize = 4 * 1024;
/// The longest text a person's answer may hold, in bytes.
pub const MAX_ANSWER_BYTES: usize = 8 * 1024;

/// Who said a message a server wants the model to read.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SampleRole {
    /// The side asking.
    User,
    /// The model.
    Assistant,
}

/// One message of a sampling request. Only text is accepted.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct SampleMessage {
    /// Who said it.
    pub role: SampleRole,
    /// The text.
    pub text: String,
}

/// A server asking for a model call.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct SampleRequest {
    /// Its instructions to the model, if any.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub system: Option<String>,
    /// The conversation.
    pub messages: Vec<SampleMessage>,
    /// The most output tokens, already capped at [`MAX_SAMPLE_TOKENS`].
    pub max_tokens: u32,
}

/// The model's answer to a server.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct SampleReply {
    /// The text.
    pub text: String,
    /// The model that answered.
    pub model: String,
    /// True when it stopped because it reached the token limit.
    pub at_limit: bool,
}

/// What kind of value a form field takes.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum FieldKind {
    /// Text.
    Text,
    /// A number.
    Number,
    /// A whole number.
    Integer,
    /// Yes or no.
    Boolean,
    /// One of a list.
    Choice {
        /// The values to choose from.
        options: Vec<String>,
    },
}

/// One field of a form.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct FormField {
    /// The name the server gets the value under.
    pub name: String,
    /// What to call it on screen.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub title: Option<String>,
    /// What it is for.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
    /// What it takes.
    pub kind: FieldKind,
    /// Whether an answer must give it.
    #[serde(default)]
    pub required: bool,
}

/// Something a server asks the person.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "mode", rename_all = "snake_case")]
pub enum Question {
    /// Fill in values.
    Form {
        /// What the server says.
        message: String,
        /// What it asks for.
        fields: Vec<FormField>,
    },
    /// Open an address, in the person's own browser.
    Link {
        /// What the server says.
        message: String,
        /// Where it leads.
        url: String,
    },
}

/// A request waiting for the person.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum ServerAsk {
    /// Use the model.
    Model(SampleRequest),
    /// Answer a question.
    Person(Question),
}

/// The person's answer.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "answer", rename_all = "snake_case")]
pub enum Answer {
    /// Allow the model call, open the link, or send these values.
    Yes {
        /// The values of a form, by field name.
        #[serde(default)]
        values: Map<String, Value>,
    },
    /// No.
    No,
}

/// What the server is told about a question.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ElicitReply {
    /// The person accepted, with the values of a form (empty for a link).
    Accepted(Map<String, Value>),
    /// The person (or ZORIX for them) said no.
    Declined,
}

/// A request that waits for an answer, with where it comes from.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct PendingAsk {
    /// Its identity.
    pub id: AskId,
    /// Whose run asks.
    pub tenant: TenantId,
    /// The run.
    pub execution_id: ExecutionId,
    /// The tool call during which the server asks.
    pub call_id: ToolCallId,
    /// The server.
    pub server: String,
    /// What it asks.
    pub ask: ServerAsk,
    /// [`ServerAsk::hash`]: an answer names it, so it cannot be applied to another request.
    pub hash: String,
    /// When it asked.
    pub asked_at: Timestamp,
}

impl ServerAsk {
    /// A hash that identifies this request from this server.
    pub fn hash(&self, server: &str) -> String {
        canonical_hash(&serde_json::json!({"server": server, "ask": self}))
    }
}

impl Question {
    /// The values a server gets for what a person filled in, or why they do not fit what was asked.
    pub fn accept(&self, values: &Map<String, Value>) -> Result<Map<String, Value>, String> {
        let Self::Form { fields, .. } = self else {
            return if values.is_empty() { Ok(Map::new()) } else { Err("A link takes no values.".into()) };
        };
        if let Some(name) = values.keys().find(|name| !fields.iter().any(|f| &f.name == *name)) {
            return Err(format!("There is no field called {name}."));
        }
        let mut accepted = Map::new();
        for field in fields {
            let label = field.title.as_deref().unwrap_or(&field.name);
            let Some(value) = values.get(&field.name).filter(|v| !v.is_null()) else {
                if field.required {
                    return Err(format!("{label} is needed."));
                }
                continue;
            };
            let fits = match &field.kind {
                FieldKind::Text => value.as_str().is_some_and(|text| text.len() <= MAX_ANSWER_BYTES),
                FieldKind::Number => value.is_number(),
                FieldKind::Integer => value.is_i64() || value.is_u64(),
                FieldKind::Boolean => value.is_boolean(),
                FieldKind::Choice { options } => value.as_str().is_some_and(|v| options.iter().any(|o| o == v)),
            };
            if !fits {
                return Err(format!("{label} does not take that value."));
            }
            accepted.insert(field.name.clone(), value.clone());
        }
        Ok(accepted)
    }
}

/// What a running tool call may ask of ZORIX on a server's behalf. The runtime implements it; a call without one cannot ask.
#[async_trait]
pub trait CallHost: Send + Sync + std::fmt::Debug {
    /// Ask the person to let the server use the run's model, and make the call. `Err` says why not.
    async fn sample(&self, server: &str, request: SampleRequest) -> Result<SampleReply, String>;

    /// Put a question to the person. `Err` says why it could not be asked.
    async fn ask(&self, server: &str, question: Question) -> Result<ElicitReply, String>;

    /// Whether it is waiting for a person right now. Their time does not count against the tool's.
    fn waiting(&self) -> bool;

    /// How long it has waited for a person in all.
    fn waited(&self) -> Duration;
}

/// How long to wait before looking again at a call that has `budget` of working time and has run for `elapsed`, or `None` when
/// the budget is used up. Time spent waiting for the person ([`CallHost::waited`]) does not count.
pub fn time_left(host: Option<&dyn CallHost>, elapsed: Duration, budget: Duration) -> Option<Duration> {
    if host.is_some_and(|h| h.waiting()) {
        return Some(Duration::from_millis(100));
    }
    let used = elapsed.saturating_sub(host.map_or(Duration::ZERO, |h| h.waited()));
    budget.checked_sub(used).filter(|left| !left.is_zero())
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn form() -> Question {
        let field = |name: &str, kind, required| FormField {
            name: name.into(),
            title: None,
            description: None,
            kind,
            required,
        };
        Question::Form {
            message: "Which?".into(),
            fields: vec![
                field("repo", FieldKind::Text, true),
                field("count", FieldKind::Integer, false),
                field("kind", FieldKind::Choice { options: vec!["a".into(), "b".into()] }, false),
            ],
        }
    }

    fn values(value: Value) -> Map<String, Value> {
        value.as_object().cloned().unwrap_or_default()
    }

    #[test]
    fn an_answer_must_fit_the_form() {
        let ok = form().accept(&values(json!({"repo": "x", "count": 2, "kind": "b"}))).unwrap();
        assert_eq!(ok.len(), 3);
        for bad in [
            json!({}),
            json!({"repo": null}),
            json!({"repo": 1}),
            json!({"repo": "x", "count": 1.5}),
            json!({"repo": "x", "kind": "c"}),
            json!({"repo": "x", "extra": true}),
            json!({"repo": "x".repeat(MAX_ANSWER_BYTES + 1)}),
        ] {
            assert!(form().accept(&values(bad.clone())).is_err(), "{bad}");
        }
        let link = Question::Link { message: "Go".into(), url: "https://example.com".into() };
        assert!(link.accept(&Map::new()).is_ok());
        assert!(link.accept(&values(json!({"a": 1}))).is_err());
    }

    #[test]
    fn the_hash_names_the_request_and_its_server() {
        let ask = ServerAsk::Person(form());
        assert_eq!(ask.hash("s"), ask.clone().hash("s"));
        assert_ne!(ask.hash("s"), ask.hash("t"));
        let other = ServerAsk::Person(Question::Link { message: "Go".into(), url: "https://example.com".into() });
        assert_ne!(ask.hash("s"), other.hash("s"));
    }
}
