//! A model that plays back a script. Tests use it to drive the runtime through exact situations.

use std::collections::VecDeque;
use std::time::Duration;

use async_trait::async_trait;
use parking_lot::Mutex;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use tokio_util::sync::CancellationToken;
use zorix_core::{FinishReason, ModelError, ModelGateway, ModelRequest, ModelResponse, ToolCallId, ToolCallRequest};

use crate::usage::usage_for;

/// A tool call the script makes.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct ScriptedCall {
    /// The tool id.
    pub tool: String,
    /// The arguments.
    #[serde(default)]
    pub arguments: Value,
}

/// How a scripted failure looks.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Failure {
    /// A temporary error: the runtime should retry.
    Transient(String),
    /// A final error.
    Permanent(String),
}

/// One step of a script.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Turn {
    /// Answer with this text. The execution can finish.
    Say(String),
    /// Ask for tools (with optional text).
    Call {
        /// Text that goes with the request.
        #[serde(default)]
        text: String,
        /// The tools, in order.
        calls: Vec<ScriptedCall>,
    },
    /// Fail.
    Fail(Failure),
    /// Wait this many milliseconds (or until cancelled), then do the inner turn.
    Delay {
        /// How long.
        ms: u64,
        /// What to do afterwards.
        then: Box<Turn>,
    },
    /// Never answer; only cancellation ends the call.
    Hang,
}

impl Turn {
    /// A turn that asks for one tool.
    pub fn call(tool: &str, arguments: Value) -> Turn {
        Turn::Call { text: String::new(), calls: vec![ScriptedCall { tool: tool.into(), arguments }] }
    }
}

/// Plays a list of turns, one per model call, and remembers every request it received.
pub struct ScriptedModel {
    turns: Mutex<VecDeque<Turn>>,
    seen: Mutex<Vec<ModelRequest>>,
}

impl ScriptedModel {
    /// A model with this script.
    pub fn new(turns: impl IntoIterator<Item = Turn>) -> Self {
        Self { turns: Mutex::new(turns.into_iter().collect()), seen: Mutex::new(Vec::new()) }
    }

    /// Read a script from JSON: a list of turns such as `[{"say": "hi"}]`.
    pub fn from_json(text: &str) -> Result<Self, String> {
        let turns: Vec<Turn> = serde_json::from_str(text).map_err(|e| format!("The script is not valid: {e}"))?;
        Ok(Self::new(turns))
    }

    /// Every request received so far.
    pub fn requests(&self) -> Vec<ModelRequest> {
        self.seen.lock().clone()
    }

    /// How many turns are left.
    pub fn remaining(&self) -> usize {
        self.turns.lock().len()
    }
}

#[async_trait]
impl ModelGateway for ScriptedModel {
    async fn generate(&self, request: ModelRequest, cancel: CancellationToken) -> Result<ModelResponse, ModelError> {
        self.seen.lock().push(request.clone());
        let mut turn = self
            .turns
            .lock()
            .pop_front()
            .ok_or_else(|| ModelError::Permanent("The scripted model has no more turns.".into()))?;
        loop {
            match turn {
                Turn::Delay { ms, then } => {
                    tokio::select! {
                        _ = tokio::time::sleep(Duration::from_millis(ms)) => {}
                        _ = cancel.cancelled() => return Err(ModelError::Cancelled),
                    }
                    turn = *then;
                }
                Turn::Hang => {
                    cancel.cancelled().await;
                    return Err(ModelError::Cancelled);
                }
                Turn::Fail(Failure::Transient(message)) => {
                    return Err(ModelError::Transient { message, retry_after_ms: None });
                }
                Turn::Fail(Failure::Permanent(message)) => return Err(ModelError::Permanent(message)),
                Turn::Say(text) => {
                    let usage = usage_for(&request, text.chars().count());
                    return Ok(ModelResponse::new(request.model, text, Vec::new(), usage, FinishReason::Stop));
                }
                Turn::Call { text, calls } => {
                    let tool_calls: Vec<ToolCallRequest> = calls
                        .into_iter()
                        .enumerate()
                        .map(|(index, call)| ToolCallRequest {
                            id: ToolCallId(String::new()),
                            provider_id: Some(format!("scripted_{index}")),
                            tool: call.tool,
                            arguments: call.arguments,
                        })
                        .collect();
                    let chars =
                        text.chars().count() + tool_calls.iter().map(|c| c.arguments.to_string().len()).sum::<usize>();
                    let usage = usage_for(&request, chars);
                    return Ok(ModelResponse::new(request.model, text, tool_calls, usage, FinishReason::ToolCalls));
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use serde_json::json;
    use zorix_core::ExecutionId;

    use super::*;

    fn request() -> ModelRequest {
        ModelRequest {
            model: "sim".into(),
            execution_id: ExecutionId::new(),
            messages: vec![zorix_core::Message::user("hello")],
            tools: vec![],
            response_schema: None,
            ..ModelRequest::default()
        }
    }

    #[tokio::test]
    async fn plays_the_turns_in_order_and_records_requests() {
        let model = ScriptedModel::new([Turn::call("clock.now", json!({})), Turn::Say("done".into())]);
        let first = model.generate(request(), CancellationToken::new()).await.unwrap();
        assert_eq!(first.finish, FinishReason::ToolCalls);
        assert_eq!(first.tool_calls[0].tool, "clock.now");
        let second = model.generate(request(), CancellationToken::new()).await.unwrap();
        assert_eq!((second.text.as_str(), second.finish), ("done", FinishReason::Stop));
        assert!(second.usage.total_tokens() > 0 && second.usage.model_calls == 1);
        assert_eq!(model.requests().len(), 2);
        assert_eq!(model.remaining(), 0);
    }

    #[tokio::test]
    async fn an_empty_script_is_a_permanent_error() {
        let model = ScriptedModel::new([]);
        let error = model.generate(request(), CancellationToken::new()).await.unwrap_err();
        assert!(matches!(error, ModelError::Permanent(_)));
    }

    #[tokio::test]
    async fn failures_map_to_model_errors() {
        let model = ScriptedModel::new([
            Turn::Fail(Failure::Transient("busy".into())),
            Turn::Fail(Failure::Permanent("bad key".into())),
        ]);
        assert!(matches!(model.generate(request(), CancellationToken::new()).await, Err(ModelError::Transient { .. })));
        assert_eq!(
            model.generate(request(), CancellationToken::new()).await,
            Err(ModelError::Permanent("bad key".into()))
        );
    }

    #[tokio::test]
    async fn a_hanging_call_ends_only_when_cancelled() {
        let model = ScriptedModel::new([Turn::Hang]);
        let cancel = CancellationToken::new();
        let stop = cancel.clone();
        tokio::spawn(async move {
            tokio::time::sleep(Duration::from_millis(20)).await;
            stop.cancel();
        });
        assert_eq!(model.generate(request(), cancel).await, Err(ModelError::Cancelled));
    }

    #[tokio::test]
    async fn a_delayed_turn_waits_then_answers() {
        let model = ScriptedModel::new([Turn::Delay { ms: 30, then: Box::new(Turn::Say("late".into())) }]);
        let started = std::time::Instant::now();
        let response = model.generate(request(), CancellationToken::new()).await.unwrap();
        assert_eq!(response.text, "late");
        assert!(started.elapsed() >= Duration::from_millis(25));
    }

    #[test]
    fn scripts_load_from_json() {
        let model = ScriptedModel::from_json(
            r#"[{"call": {"calls": [{"tool": "math.evaluate", "arguments": {"expression": "1+1"}}]}},
                {"fail": {"transient": "busy"}},
                {"delay": {"ms": 5, "then": {"say": "ok"}}},
                {"say": "bye"}]"#,
        )
        .unwrap();
        assert_eq!(model.remaining(), 4);
        assert!(ScriptedModel::from_json("[{\"nope\": 1}]").is_err());
    }
}
