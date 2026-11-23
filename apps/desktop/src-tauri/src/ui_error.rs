//! An error as the interface receives it (ADR-0040): a stable `code`, the `args` that fill its sentence, and the English `message` the
//! command line would print. The interface writes the sentence from its own catalogue (`reason.<code>`) and uses `message` only for a code it does
//! not know, so a newer backend never shows a blank.

use std::collections::BTreeMap;

use serde::Serialize;
use zorix_core::{Coded, Error, ModelError, ToolError};

/// What a command rejects with.
#[derive(Clone, Debug, Serialize)]
pub struct UiError {
    /// A stable name for the kind of failure.
    pub code: String,
    /// The values that fill its sentence.
    pub args: BTreeMap<String, String>,
    /// The English sentence, as the command line says it.
    pub message: String,
}

impl UiError {
    /// An error that has only its sentence: a failure of a library, or of the operating system, whose own words are shown as they are
    /// (the code is `message`, and the sentence is its one value).
    pub fn message(text: impl Into<String>) -> Self {
        let text = text.into();
        Self { code: "message".into(), args: BTreeMap::from([("detail".to_string(), text.clone())]), message: text }
    }

    fn coded(error: &(impl Coded + std::fmt::Display)) -> Self {
        let reason = error.reason();
        Self {
            code: reason.code.to_string(),
            args: reason.args.into_iter().map(|(name, value)| (name.to_string(), value)).collect(),
            message: error.to_string(),
        }
    }
}

impl std::fmt::Display for UiError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.message)
    }
}

/// What can become a [`UiError`].
pub trait IntoUi {
    /// The error as the interface reads it.
    fn into_ui(self) -> UiError;
}

macro_rules! coded {
    ($($type:ty),* $(,)?) => {
        $(impl IntoUi for $type {
            fn into_ui(self) -> UiError {
                UiError::coded(&self)
            }
        })*
    };
}

coded!(Error, ModelError, ToolError, zorix_core::IdError);

/// Errors of libraries and of the other crates whose own sentence is what a person reads: the code is `message` and the sentence is its value.
macro_rules! sentence {
    ($($type:ty),* $(,)?) => {
        $(impl IntoUi for $type {
            fn into_ui(self) -> UiError {
                UiError::message(self.to_string())
            }
        })*
    };
}

sentence!(
    zorix_hub::HubError,
    zorix_providers::SecretError,
    zorix_mcp::McpError,
    zorix_agents::SkillError,
    zorix_sandbox::SandboxError,
    zorix_memory::MemoryError,
    zorix_store::LockError,
    tokio::task::JoinError,
    serde_json::Error,
);

impl From<String> for UiError {
    fn from(text: String) -> Self {
        UiError::message(text)
    }
}

impl From<&str> for UiError {
    fn from(text: &str) -> Self {
        UiError::message(text)
    }
}

impl From<Error> for UiError {
    fn from(error: Error) -> Self {
        error.into_ui()
    }
}

impl IntoUi for UiError {
    fn into_ui(self) -> UiError {
        self
    }
}

impl IntoUi for String {
    fn into_ui(self) -> UiError {
        UiError::message(self)
    }
}

impl IntoUi for &str {
    fn into_ui(self) -> UiError {
        UiError::message(self)
    }
}

/// `.map_err(ui)`: the error of any command step, as the interface reads it.
pub fn ui<E: IntoUi>(error: E) -> UiError {
    error.into_ui()
}

#[cfg(test)]
mod tests {
    use zorix_core::ExecutionId;

    use super::*;

    #[test]
    fn a_core_error_keeps_its_code_and_its_values_and_its_english_sentence() {
        let id = ExecutionId::new();
        let error = ui(Error::NotFound(id));
        assert_eq!((error.code.as_str(), error.args["id"].as_str()), ("not_found.execution", id.to_string().as_str()));
        assert_eq!(error.message, format!("Execution {id} was not found."));
        let json = serde_json::to_value(&error).unwrap();
        assert_eq!(json["code"], "not_found.execution");
        assert_eq!(json["args"]["id"], id.to_string());
    }

    #[test]
    fn a_plain_sentence_is_the_message_code_with_the_sentence_as_its_value() {
        let error = ui("The folder cannot be used.");
        assert_eq!(
            (error.code.as_str(), error.args["detail"].as_str(), error.message.as_str()),
            ("message", "The folder cannot be used.", "The folder cannot be used.")
        );
        assert_eq!(ui(String::from("x")).code, "message");
    }
}
