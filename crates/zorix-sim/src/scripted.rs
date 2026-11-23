//! A model that plays back a script. Tests use it to drive the runtime through exact situations.

use std::collections::VecDeque;
use std::time::Duration;

use async_trait::async_trait;
use parking_lot::Mutex;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use tokio_util::sync::CancellationToken;
use zorix_core::{
    FinishReason, ModelError, ModelErrorKind, ModelGateway, ModelRequest, ModelResponse, TextSink, ToolCallId,
    ToolCallRequest,
};

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
    /// A failure of a given kind, as a real provider reports it (a rate limit, a context that is too long, bad credentials, a refusal),
    /// with the pause the provider asked for.
    Kind {
        /// What kind of failure.
        kind: ModelErrorKind,
        /// The sentence a person sees.
        message: String,
        /// How long the provider asked to wait, in milliseconds.
        #[serde(default)]
        retry_after_ms: Option<u64>,
    },
}

impl Failure {
    /// A provider failure of this kind.
    pub fn kind(kind: ModelErrorKind, message: &str) -> Failure {
        Failure::Kind { kind, message: message.into(), retry_after_ms: None }
    }

    fn into_error(self) -> ModelError {
        match self {
            Failure::Transient(message) => ModelError::Transient { message, retry_after_ms: None },
            Failure::Permanent(message) => ModelError::Permanent(message),
            Failure::Kind { kind, message, retry_after_ms } => ModelError::Provider { kind, message, retry_after_ms },
        }
    }
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
    /// Write these pieces to the live text first (as a streaming provider does), then do the inner turn. An inner `Fail` is a stream
    /// that was cut; an inner `Say` is one that finished (and its text is not written a second time).
    Stream {
        /// The pieces of the answer, in order.
        chunks: Vec<String>,
        /// How the stream ends.
        then: Box<Turn>,
    },
    /// A response exactly as given, including a finish reason the content does not agree with (text and tool calls with `Stop`, no
    /// text and no calls, a cut-off `Length`), to see how the loop copes with a model that does not behave.
    Raw {
        /// The text.
        #[serde(default)]
        text: String,
        /// The tool calls.
        #[serde(default)]
        calls: Vec<ScriptedCall>,
        /// Why it says it stopped.
        finish: FinishReason,
    },
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
    native_provider: Option<String>,
}

impl ScriptedModel {
    /// A model with this script.
    pub fn new(turns: impl IntoIterator<Item = Turn>) -> Self {
        Self { turns: Mutex::new(turns.into_iter().collect()), seen: Mutex::new(Vec::new()), native_provider: None }
    }

    /// Read a script from JSON: a list of turns such as `[{"say": "hi"}]`.
    pub fn from_json(text: &str) -> Result<Self, String> {
        let turns: Vec<Turn> = serde_json::from_str(text).map_err(|e| format!("The script is not valid: {e}"))?;
        Ok(Self::new(turns))
    }

    /// Make every answer carry a native turn from this provider, as a real provider's thinking blocks would.
    pub fn with_native(mut self, provider: &str) -> Self {
        self.native_provider = Some(provider.to_string());
        self
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

impl ScriptedModel {
    fn answer(
        &self,
        model: String,
        text: String,
        tool_calls: Vec<ToolCallRequest>,
        usage: zorix_core::Usage,
        finish: FinishReason,
    ) -> ModelResponse {
        let mut response = ModelResponse::new(model.clone(), text.clone(), tool_calls, usage, finish);
        if let Some(provider) = &self.native_provider {
            response.provider = Some(provider.clone());
            response.native = Some(zorix_core::NativeTurn {
                provider: provider.clone(),
                model,
                content: serde_json::json!({ "text": text }),
            });
        }
        response
    }
}

#[async_trait]
impl ModelGateway for ScriptedModel {
    async fn embed(
        &self,
        request: zorix_core::EmbeddingRequest,
        _cancel: CancellationToken,
    ) -> Result<zorix_core::EmbeddingResponse, ModelError> {
        crate::embed::embed(&request)
    }

    fn embedding_profile(&self, _model: &str) -> Option<zorix_core::EmbeddingProfile> {
        Some(crate::embed::profile())
    }

    async fn generate(&self, request: ModelRequest, cancel: CancellationToken) -> Result<ModelResponse, ModelError> {
        self.play(request, cancel, None).await
    }

    async fn generate_live(
        &self,
        request: ModelRequest,
        cancel: CancellationToken,
        sink: std::sync::Arc<dyn TextSink>,
    ) -> Result<ModelResponse, ModelError> {
        let response = self.play(request, cancel, Some(sink.clone())).await?;
        Ok(response)
    }
}

impl ScriptedModel {
    /// Play the next turn. With a sink, the text is written to it as a provider would: a `Stream` piece by piece, and an answer that
    /// was not streamed in one piece once it is complete.
    async fn play(
        &self,
        request: ModelRequest,
        cancel: CancellationToken,
        sink: Option<std::sync::Arc<dyn TextSink>>,
    ) -> Result<ModelResponse, ModelError> {
        self.seen.lock().push(request.clone());
        let mut turn = self
            .turns
            .lock()
            .pop_front()
            .ok_or_else(|| ModelError::Permanent("The scripted model has no more turns.".into()))?;
        let mut streamed = false;
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
                Turn::Stream { chunks, then } => {
                    if let Some(sink) = &sink {
                        for chunk in &chunks {
                            if cancel.is_cancelled() {
                                return Err(ModelError::Cancelled);
                            }
                            sink.text(chunk);
                            tokio::task::yield_now().await;
                        }
                        streamed = true;
                    }
                    turn = *then;
                }
                Turn::Fail(failure) => return Err(failure.into_error()),
                Turn::Say(text) => {
                    if let (Some(sink), false, false) = (&sink, streamed, text.is_empty()) {
                        sink.text(&text);
                    }
                    let usage = usage_for(&request, text.chars().count());
                    return Ok(self.answer(request.model, text, Vec::new(), usage, FinishReason::Stop));
                }
                Turn::Call { text, calls } => {
                    return Ok(self.respond(request, text, calls, FinishReason::ToolCalls, sink.as_deref(), streamed));
                }
                Turn::Raw { text, calls, finish } => {
                    return Ok(self.respond(request, text, calls, finish, sink.as_deref(), streamed));
                }
            }
        }
    }

    fn respond(
        &self,
        request: ModelRequest,
        text: String,
        calls: Vec<ScriptedCall>,
        finish: FinishReason,
        sink: Option<&dyn TextSink>,
        streamed: bool,
    ) -> ModelResponse {
        if let (Some(sink), false, false) = (sink, streamed, text.is_empty()) {
            sink.text(&text);
        }
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
        let chars = text.chars().count() + tool_calls.iter().map(|c| c.arguments.to_string().len()).sum::<usize>();
        let usage = usage_for(&request, chars);
        self.answer(request.model, text, tool_calls, usage, finish)
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

    struct Collect(Mutex<Vec<String>>);

    impl TextSink for Collect {
        fn text(&self, delta: &str) {
            self.0.lock().push(delta.to_string());
        }
    }

    #[tokio::test]
    async fn a_stream_writes_its_pieces_and_a_cut_stream_fails_after_them() {
        let sink = std::sync::Arc::new(Collect(Mutex::new(Vec::new())));
        let model = ScriptedModel::new([
            Turn::Stream { chunks: vec!["Hel".into(), "lo".into()], then: Box::new(Turn::Say("Hello".into())) },
            Turn::Stream {
                chunks: vec!["Half an ans".into()],
                then: Box::new(Turn::Fail(Failure::Transient("connection reset".into()))),
            },
            Turn::Say("plain".into()),
        ]);
        let done = model.generate_live(request(), CancellationToken::new(), sink.clone()).await.unwrap();
        assert_eq!(done.text, "Hello");
        assert_eq!(*sink.0.lock(), ["Hel", "lo"], "the text is not written a second time");
        sink.0.lock().clear();
        let cut = model.generate_live(request(), CancellationToken::new(), sink.clone()).await;
        assert!(matches!(cut, Err(ModelError::Transient { .. })));
        assert_eq!(*sink.0.lock(), ["Half an ans"], "what was written before the cut was written");
        sink.0.lock().clear();
        model.generate_live(request(), CancellationToken::new(), sink.clone()).await.unwrap();
        assert_eq!(*sink.0.lock(), ["plain"], "an answer that was not streamed arrives in one piece");
        // Without a sink the pieces are simply skipped.
        let model =
            ScriptedModel::new([Turn::Stream { chunks: vec!["x".into()], then: Box::new(Turn::Say("y".into())) }]);
        assert_eq!(model.generate(request(), CancellationToken::new()).await.unwrap().text, "y");
    }

    #[tokio::test]
    async fn a_failure_can_have_any_kind_and_a_response_can_say_anything_about_why_it_stopped() {
        let model = ScriptedModel::new([
            Turn::Fail(Failure::Kind {
                kind: ModelErrorKind::RateLimited,
                message: "Slow down.".into(),
                retry_after_ms: Some(1_500),
            }),
            Turn::Fail(Failure::kind(ModelErrorKind::ContextTooLong, "Too long.")),
            Turn::Raw { text: "I will call a tool".into(), calls: vec![], finish: FinishReason::ToolCalls },
            Turn::Raw { text: String::new(), calls: vec![], finish: FinishReason::Stop },
            Turn::Raw { text: "cut o".into(), calls: vec![], finish: FinishReason::Length },
        ]);
        let first = model.generate(request(), CancellationToken::new()).await.unwrap_err();
        assert_eq!(
            first,
            ModelError::Provider {
                kind: ModelErrorKind::RateLimited,
                message: "Slow down.".into(),
                retry_after_ms: Some(1_500)
            }
        );
        assert!(matches!(
            model.generate(request(), CancellationToken::new()).await,
            Err(ModelError::Provider { kind: ModelErrorKind::ContextTooLong, .. })
        ));
        let lying = model.generate(request(), CancellationToken::new()).await.unwrap();
        assert_eq!((lying.finish, lying.tool_calls.len()), (FinishReason::ToolCalls, 0));
        let empty = model.generate(request(), CancellationToken::new()).await.unwrap();
        assert_eq!((empty.finish, empty.text.as_str()), (FinishReason::Stop, ""));
        assert_eq!(model.generate(request(), CancellationToken::new()).await.unwrap().finish, FinishReason::Length);
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
