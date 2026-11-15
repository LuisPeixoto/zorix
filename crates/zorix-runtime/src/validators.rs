//! The validators every execution gets, and the parsing of structured answers.

use async_trait::async_trait;
use serde_json::Value;
use zorix_core::{Execution, ExecutionResult, ResultValidator, Validation};

/// Parse a model's answer as JSON. Accepts the answer wrapped in a Markdown code fence, which models often add.
pub fn parse_structured(text: &str) -> Option<Value> {
    let trimmed = text.trim();
    let inner = trimmed
        .strip_prefix("```json")
        .or_else(|| trimmed.strip_prefix("```"))
        .and_then(|rest| rest.trim().strip_suffix("```"))
        .unwrap_or(trimmed);
    serde_json::from_str(inner.trim()).ok()
}

/// Rejects an empty answer.
pub struct NonEmptyValidator;

#[async_trait]
impl ResultValidator for NonEmptyValidator {
    fn name(&self) -> &str {
        "non_empty"
    }

    async fn validate(&self, _execution: &Execution, result: &ExecutionResult) -> Validation {
        if result.text.trim().is_empty() && result.structured.is_none() {
            Validation::Reject { reason: "The answer is empty.".into() }
        } else {
            Validation::Accept
        }
    }
}

/// When the request expects structured output, the answer must be JSON that matches the schema.
pub struct OutputSchemaValidator;

#[async_trait]
impl ResultValidator for OutputSchemaValidator {
    fn name(&self) -> &str {
        "output_schema"
    }

    async fn validate(&self, execution: &Execution, result: &ExecutionResult) -> Validation {
        let Some(schema) = &execution.request.expected_output else { return Validation::Accept };
        let Some(value) = &result.structured else {
            return Validation::Reject {
                reason: "The answer must be a single JSON value, and it could not be read as JSON.".into(),
            };
        };
        let validator = match jsonschema::validator_for(schema) {
            Ok(validator) => validator,
            Err(error) => {
                return Validation::Reject { reason: format!("The requested output schema is not valid: {error}") };
            }
        };
        let problems: Vec<String> = validator.iter_errors(value).map(|e| e.to_string()).take(3).collect();
        if problems.is_empty() {
            Validation::Accept
        } else {
            Validation::Reject {
                reason: format!("The answer does not match the requested schema: {}", problems.join("; ")),
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use serde_json::json;
    use zorix_core::{ExecutionId, NewExecution, TaskRequest, Timestamp};

    use super::*;

    fn execution(schema: Option<Value>) -> Execution {
        let new = NewExecution {
            request: TaskRequest {
                prompt: "p".into(),
                model: "m".into(),
                expected_output: schema,
                ..TaskRequest::default()
            },
            ..NewExecution::default()
        };
        Execution::new(ExecutionId::new(), new, Timestamp(0))
    }

    fn result(text: &str) -> ExecutionResult {
        ExecutionResult { text: text.into(), structured: parse_structured(text) }
    }

    #[test]
    fn json_is_read_with_or_without_a_code_fence() {
        assert_eq!(parse_structured("{\"a\":1}"), Some(json!({"a": 1})));
        assert_eq!(parse_structured("```json\n{\"a\":1}\n```"), Some(json!({"a": 1})));
        assert_eq!(parse_structured("```\n[1,2]\n```"), Some(json!([1, 2])));
        assert_eq!(parse_structured("Sure! {\"a\":1}"), None);
    }

    #[tokio::test]
    async fn empty_answers_are_rejected() {
        let run = execution(None);
        assert_eq!(
            NonEmptyValidator.validate(&run, &result("  ")).await,
            Validation::Reject { reason: "The answer is empty.".into() }
        );
        assert_eq!(NonEmptyValidator.validate(&run, &result("hi")).await, Validation::Accept);
    }

    #[tokio::test]
    async fn without_a_schema_anything_passes_the_schema_validator() {
        assert_eq!(OutputSchemaValidator.validate(&execution(None), &result("free text")).await, Validation::Accept);
    }

    #[tokio::test]
    async fn with_a_schema_the_answer_must_be_json_that_matches() {
        let run = execution(Some(
            json!({"type": "object", "required": ["total"], "properties": {"total": {"type": "number"}}}),
        ));
        assert_eq!(OutputSchemaValidator.validate(&run, &result("{\"total\": 391}")).await, Validation::Accept);
        let Validation::Reject { reason } = OutputSchemaValidator.validate(&run, &result("it is 391")).await else {
            panic!("rejected")
        };
        assert!(reason.contains("JSON"));
        let Validation::Reject { reason } = OutputSchemaValidator.validate(&run, &result("{\"total\": \"x\"}")).await
        else {
            panic!("rejected")
        };
        assert!(reason.contains("does not match"), "{reason}");
    }
}
