//! Why something failed, as a stable code and its values (ADR-0040).
//!
//! An error's sentence is English and is what the command line prints. A person who reads another language needs the same error as
//! something the interface can write in its own words: a `code` that never changes (`not_found.execution`) and the `args` that fill its
//! sentence (`id`). The interface has a message for each code; a code it does not know, from a newer backend, falls back to the sentence.

use std::collections::BTreeMap;

use crate::error::Error;
use crate::model::{ModelError, ModelErrorKind};
use crate::tool::ToolError;

/// A failure as the interface reads it.
#[derive(Clone, Debug, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct Reason {
    /// A stable name for the kind of failure.
    pub code: &'static str,
    /// The values that fill its sentence.
    pub args: BTreeMap<&'static str, String>,
}

impl Reason {
    /// A reason with these values.
    pub fn new<const N: usize>(code: &'static str, args: [(&'static str, String); N]) -> Self {
        Self { code, args: args.into_iter().collect() }
    }
}

/// Something that can say what it is in a code and values.
pub trait Coded {
    /// The failure as a code and its values.
    fn reason(&self) -> Reason;
}

/// Every code a failure of the core can have, with the English sentence the interface starts from. The interface's catalogue has a message for
/// each (`reason.<code>`), and a test checks that none is missing.
pub const CODES: &[(&str, &str)] = &[
    ("lifecycle.invalid_step", "An execution that is {from} cannot take the step “{trigger}”."),
    ("invalid", "Invalid request: {detail}"),
    ("missing", "{detail}"),
    ("not_found.execution", "Execution {id} was not found."),
    ("conflict.execution", "Execution {id} was changed by something else at the same time. Load it again and retry."),
    ("storage", "The storage failed: {detail}"),
    (
        "storage.full",
        "The disk is full, so ZORIX could not save. Free some space and carry on: nothing that was saved is lost, and work that was interrupted continues from where it stopped.",
    ),
    ("internal", "Internal error: {detail}"),
    ("model.failed", "{detail}"),
    ("model.temporary", "{detail}"),
    ("model.cancelled", "The model call was cancelled."),
    ("model.auth", "{detail}"),
    ("model.not_found", "{detail}"),
    ("model.context_too_long", "{detail}"),
    ("model.rate_limited", "{detail}"),
    ("model.unavailable", "{detail}"),
    ("model.refused", "{detail}"),
    ("model.invalid", "{detail}"),
    ("model.unsupported", "{detail}"),
    ("tool.failed", "{detail}"),
    ("tool.temporary", "{detail}"),
    ("id.invalid", "{detail}"),
    ("message", "{detail}"),
];

fn detail(text: &str) -> [(&'static str, String); 1] {
    [("detail", text.to_string())]
}

impl Coded for Error {
    fn reason(&self) -> Reason {
        match self {
            Error::InvalidTransition(t) => Reason::new(
                "lifecycle.invalid_step",
                [("from", t.from.to_string()), ("trigger", t.trigger.to_string())],
            ),
            Error::InvalidInput(text) => Reason::new("invalid", detail(text)),
            Error::NotFound(id) => Reason::new("not_found.execution", [("id", id.to_string())]),
            Error::Conflict(id) => Reason::new("conflict.execution", [("id", id.to_string())]),
            Error::Missing(text) => Reason::new("missing", detail(text)),
            Error::Storage(text) if text.starts_with("The disk is full") => Reason::new("storage.full", []),
            Error::Storage(text) => {
                Reason::new("storage", detail(text.strip_prefix("The storage failed: ").unwrap_or(text)))
            }
            Error::Internal(text) => Reason::new("internal", detail(text)),
        }
    }
}

impl Coded for ModelError {
    fn reason(&self) -> Reason {
        match self {
            ModelError::Transient { message, .. } => Reason::new("model.temporary", detail(message)),
            ModelError::Permanent(message) => Reason::new("model.failed", detail(message)),
            ModelError::Cancelled => Reason::new("model.cancelled", []),
            ModelError::Provider { kind, message, .. } => Reason::new(
                match kind {
                    ModelErrorKind::Auth => "model.auth",
                    ModelErrorKind::NotFound => "model.not_found",
                    ModelErrorKind::ContextTooLong => "model.context_too_long",
                    ModelErrorKind::RateLimited => "model.rate_limited",
                    ModelErrorKind::Unavailable => "model.unavailable",
                    ModelErrorKind::Refused => "model.refused",
                    ModelErrorKind::Invalid => "model.invalid",
                    ModelErrorKind::Cancelled => "model.cancelled",
                    ModelErrorKind::Unsupported => "model.unsupported",
                },
                detail(message),
            ),
        }
    }
}

impl Coded for ToolError {
    fn reason(&self) -> Reason {
        match self {
            ToolError::Retryable(text) => Reason::new("tool.temporary", detail(text)),
            ToolError::Failed(text) => Reason::new("tool.failed", detail(text)),
        }
    }
}

impl Coded for crate::ids::IdError {
    fn reason(&self) -> Reason {
        Reason::new("id.invalid", detail(&self.to_string()))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ids::ExecutionId;

    fn english(reason: &Reason) -> String {
        let template = CODES
            .iter()
            .find(|(code, _)| *code == reason.code)
            .unwrap_or_else(|| panic!("{} has no entry in CODES", reason.code))
            .1;
        reason.args.iter().fold(template.to_string(), |text, (name, value)| text.replace(&format!("{{{name}}}"), value))
    }

    #[test]
    fn a_coded_error_says_in_english_what_its_own_sentence_says() {
        let id = ExecutionId::new();
        let samples: Vec<(Error, Reason)> = vec![
            (Error::NotFound(id), Error::NotFound(id).reason()),
            (Error::Conflict(id), Error::Conflict(id).reason()),
            (
                Error::InvalidInput("Say who is deciding.".into()),
                Error::InvalidInput("Say who is deciding.".into()).reason(),
            ),
            (
                Error::Missing("This execution has no such approval.".into()),
                Error::Missing("This execution has no such approval.".into()).reason(),
            ),
            (Error::Storage("no such table".into()), Error::Storage("no such table".into()).reason()),
            (Error::Internal("a bug".into()), Error::Internal("a bug".into()).reason()),
        ];
        for (error, reason) in samples {
            assert_eq!(english(&reason), error.to_string(), "{}", reason.code);
        }
    }

    #[test]
    fn a_full_disk_has_its_own_code_and_every_code_is_listed_once() {
        let full = Error::Storage("The disk is full, so ZORIX could not save. Free some space and carry on: nothing that was saved is lost, and work that was interrupted continues from where it stopped.".into());
        assert_eq!(full.reason().code, "storage.full");
        assert_eq!(
            english(&full.reason()),
            full.to_string().strip_prefix("The storage failed: ").unwrap_or(&full.to_string())
        );
        let mut seen = std::collections::HashSet::new();
        for (code, _) in CODES {
            assert!(seen.insert(*code), "{code} is listed twice");
        }
    }

    /// `reasons.json` lists every code with its English sentence, for the interface's catalogue and its test. Run the tests with
    /// `UPDATE_REASONS=1` to write it; without it, this fails when the file is not what `CODES` says.
    #[test]
    fn the_list_the_interface_reads_is_current() {
        let list: Vec<[&str; 2]> = CODES.iter().map(|(code, template)| [*code, *template]).collect();
        let text = format!("{}\n", serde_json::to_string_pretty(&list).unwrap());
        let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("reasons.json");
        if std::env::var_os("UPDATE_REASONS").is_some() {
            std::fs::write(&path, &text).unwrap();
        }
        // A Windows checkout may turn the file's line ends into CRLF; the content is what must match.
        let on_disk = std::fs::read_to_string(&path).unwrap_or_default().replace("\r\n", "\n");
        assert_eq!(on_disk, text, "reasons.json is stale: run the tests with UPDATE_REASONS=1");
    }

    #[test]
    fn model_and_tool_failures_have_codes_by_kind() {
        let rate = ModelError::Provider {
            kind: ModelErrorKind::RateLimited,
            message: "Slow down.".into(),
            retry_after_ms: Some(5),
        };
        assert_eq!((rate.reason().code, rate.reason().args["detail"].as_str()), ("model.rate_limited", "Slow down."));
        assert_eq!(ModelError::Cancelled.reason().code, "model.cancelled");
        assert_eq!(
            ModelError::Transient { message: "busy".into(), retry_after_ms: None }.reason().code,
            "model.temporary"
        );
        assert_eq!(ToolError::Failed("the disk is full".into()).reason().code, "tool.failed");
        assert_eq!(ToolError::Retryable("try again".into()).reason().code, "tool.temporary");
    }
}
