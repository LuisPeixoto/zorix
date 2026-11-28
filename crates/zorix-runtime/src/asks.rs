//! Requests an MCP server makes of the person while a tool runs: where they wait for an answer, and what a run lets a server
//! do (ADR-0051).
//!
//! A request waits in memory only, for as long as its tool call does: if ZORIX stops, so does the call. Nothing is asked when
//! nobody is watching, and what a server's model call costs is the run's.

use std::collections::BTreeMap;
use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::time::Duration;

use async_trait::async_trait;
use parking_lot::Mutex;
use serde_json::{Value, json};
use tokio::sync::{broadcast, oneshot};
use tokio::time::Instant;
use tokio_util::sync::CancellationToken;
use zorix_core::{
    ASK_TIMEOUT, Answer, AskId, CallHost, ElicitReply, Error, ExecutionId, FinishReason, GenerationParams,
    MAX_ASKS_PER_CALL, Message, ModelRequest, PendingAsk, Provenance, Question, Result, Role, SampleReply,
    SampleRequest, SampleRole, ServerAsk, TenantId, ToolCallId, Usage,
};

use crate::engine::Inner;

/// What the model is told before a server's messages.
const SAMPLING_PREAMBLE: &str = "An external tool server asks the following, not the person you work for. Answer it as \
                                 written. You have no tools and nothing else of this conversation.";
const NOBODY: &str = "Nobody is there to answer, so ZORIX said no.";
const OVER: &str = "The tool call ended before there was an answer.";

struct Waiting {
    ask: PendingAsk,
    answer: oneshot::Sender<Answer>,
}

/// The requests waiting for a person.
pub struct Asks {
    waiting: Mutex<BTreeMap<AskId, Waiting>>,
    changed: broadcast::Sender<()>,
}

/// Takes the request off the list when its wait ends, however it ends.
struct Listed<'a> {
    asks: &'a Asks,
    id: AskId,
}

impl Drop for Listed<'_> {
    fn drop(&mut self) {
        if self.asks.waiting.lock().remove(&self.id).is_some() {
            let _ = self.asks.changed.send(());
        }
    }
}

impl Default for Asks {
    fn default() -> Self {
        Self { waiting: Mutex::default(), changed: broadcast::channel(32).0 }
    }
}

impl Asks {
    /// Follow the list. Holding the receiver says that somebody can answer: without any, every request is refused at once.
    pub fn watch(&self) -> broadcast::Receiver<()> {
        self.changed.subscribe()
    }

    /// What waits for an answer, oldest first.
    pub fn pending(&self, tenant: &TenantId) -> Vec<PendingAsk> {
        self.waiting.lock().values().filter(|w| &w.ask.tenant == tenant).map(|w| w.ask.clone()).collect()
    }

    /// Answer one request. The hash is the one the person was shown, and the values of a form must fit it.
    pub fn answer(&self, tenant: &TenantId, id: AskId, hash: &str, answer: Answer) -> Result<()> {
        let mut waiting = self.waiting.lock();
        let found = waiting
            .get(&id)
            .filter(|w| &w.ask.tenant == tenant)
            .ok_or_else(|| Error::InvalidInput("That request is no longer waiting.".into()))?;
        if found.ask.hash != hash {
            return Err(Error::InvalidInput(
                "The answer was given for a different request than the one waiting.".into(),
            ));
        }
        let answer = match (&found.ask.ask, answer) {
            (ServerAsk::Person(question), Answer::Yes { values }) => {
                Answer::Yes { values: question.accept(&values).map_err(Error::InvalidInput)? }
            }
            (_, answer) => answer,
        };
        if let Some(found) = waiting.remove(&id) {
            let _ = found.answer.send(answer);
        }
        drop(waiting);
        let _ = self.changed.send(());
        Ok(())
    }

    async fn wait(&self, ask: PendingAsk, over: &CancellationToken) -> std::result::Result<Answer, &'static str> {
        if self.changed.receiver_count() == 0 {
            return Err(NOBODY);
        }
        let (tx, rx) = oneshot::channel();
        let id = ask.id;
        self.waiting.lock().insert(id, Waiting { ask, answer: tx });
        let _listed = Listed { asks: self, id };
        let _ = self.changed.send(());
        tokio::select! {
            _ = over.cancelled() => Err(OVER),
            answer = tokio::time::timeout(ASK_TIMEOUT, rx) => match answer {
                Ok(Ok(answer)) => Ok(answer),
                Ok(Err(_)) => Err(OVER),
                Err(_) => Err("There was no answer within five minutes, so ZORIX said no."),
            },
        }
    }
}

/// Which run and call a host serves, and what the run has left.
pub(crate) struct CallScope {
    pub tenant: TenantId,
    pub execution_id: ExecutionId,
    pub call_id: ToolCallId,
    pub tool: String,
    pub model: String,
    pub allowed_models: Vec<String>,
    /// Tokens left before the run's limit, when it has one.
    pub tokens_left: Option<u64>,
    /// True when the run's cost limit is used up.
    pub spent: bool,
    pub model_timeout: Duration,
}

/// The [`CallHost`] of one tool call of a run.
pub(crate) struct RunHost {
    engine: Arc<Inner>,
    scope: CallScope,
    /// Cancelled when the call is over, which ends whatever still waits.
    over: CancellationToken,
    asked: AtomicUsize,
    waiting: AtomicUsize,
    waited: Mutex<Duration>,
    usage: Mutex<Usage>,
    notes: Mutex<Vec<Value>>,
}

impl std::fmt::Debug for RunHost {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("RunHost").field("execution_id", &self.scope.execution_id).finish_non_exhaustive()
    }
}

/// Time spent waiting for the person, counted even if the wait is abandoned.
struct Waited<'a> {
    host: &'a RunHost,
    since: Instant,
}

impl Drop for Waited<'_> {
    fn drop(&mut self) {
        *self.host.waited.lock() += self.since.elapsed();
        self.host.waiting.fetch_sub(1, Ordering::SeqCst);
    }
}

impl RunHost {
    pub(crate) fn new(engine: Arc<Inner>, scope: CallScope, cancel: &CancellationToken) -> Arc<Self> {
        Arc::new(Self {
            engine,
            scope,
            over: cancel.child_token(),
            asked: AtomicUsize::new(0),
            waiting: AtomicUsize::new(0),
            waited: Mutex::default(),
            usage: Mutex::default(),
            notes: Mutex::default(),
        })
    }

    /// End the call: nothing it asked waits any longer. Returns what its model calls cost and what was asked.
    pub(crate) fn close(&self) -> (Usage, Vec<Value>) {
        self.over.cancel();
        (std::mem::take(&mut self.usage.lock()), std::mem::take(&mut self.notes.lock()))
    }

    fn note(&self, server: &str, kind: &str, outcome: &str) {
        self.notes.lock().push(json!({"server": server, "kind": kind, "outcome": outcome}));
    }

    fn count(&self) -> std::result::Result<(), String> {
        if self.over.is_cancelled() {
            return Err(OVER.into());
        }
        if self.asked.fetch_add(1, Ordering::SeqCst) >= MAX_ASKS_PER_CALL {
            return Err(format!("One tool call may ask at most {MAX_ASKS_PER_CALL} things."));
        }
        Ok(())
    }

    async fn person(&self, server: &str, ask: ServerAsk) -> std::result::Result<Answer, String> {
        let pending = PendingAsk {
            id: AskId::new(),
            tenant: self.scope.tenant.clone(),
            execution_id: self.scope.execution_id,
            call_id: self.scope.call_id.clone(),
            server: server.to_string(),
            tool: self.scope.tool.clone(),
            model: self.scope.model.clone(),
            hash: ask.hash(server),
            ask,
            asked_at: self.engine.clock.now(),
        };
        self.waiting.fetch_add(1, Ordering::SeqCst);
        let _waited = Waited { host: self, since: Instant::now() };
        self.engine.asks.wait(pending, &self.over).await.map_err(str::to_string)
    }

    async fn model(&self, request: SampleRequest) -> std::result::Result<SampleReply, String> {
        let mut system = SAMPLING_PREAMBLE.to_string();
        if let Some(theirs) = &request.system {
            system.push_str("\n\nThe server's instructions:\n");
            system.push_str(theirs);
        }
        let mut messages = vec![Message::system(system)];
        messages.extend(request.messages.iter().map(|m| {
            let mut message = match m.role {
                SampleRole::User => Message::user(m.text.clone()),
                SampleRole::Assistant => Message::assistant(m.text.clone(), Vec::new()),
            };
            message.provenance = Provenance::Untrusted;
            debug_assert!(message.role != Role::System);
            message
        }));
        let mut max_tokens = request.max_tokens;
        if let Some(left) = self.scope.tokens_left {
            let input: u64 = messages.iter().map(Message::estimated_tokens).sum();
            let room = left.saturating_sub(input);
            if room == 0 {
                return Err("The run's token limit leaves no room for this.".into());
            }
            max_tokens = max_tokens.min(u32::try_from(room).unwrap_or(u32::MAX));
        }
        let call = ModelRequest {
            model: self.scope.model.clone(),
            execution_id: self.scope.execution_id,
            messages,
            params: GenerationParams { max_output_tokens: Some(max_tokens), effort: None },
            allowed_models: self.scope.allowed_models.clone(),
            ..ModelRequest::default()
        };
        let cancel = self.over.child_token();
        let response = tokio::time::timeout(self.scope.model_timeout, self.engine.model.generate(call, cancel.clone()))
            .await
            .map_err(|_| {
                cancel.cancel();
                "The model took too long.".to_string()
            })?
            .map_err(|e| format!("The model call failed: {}", zorix_core::redact_text(&e.to_string())))?;
        let mut usage = response.usage;
        usage.model_calls = usage.model_calls.max(1);
        usage.tool_calls = 0;
        self.usage.lock().add(&usage);
        if !zorix_core::model_allowed(&self.scope.allowed_models, response.provider.as_deref(), &response.model) {
            return Err("A model this agent may not use answered, so its answer was not passed on.".into());
        }
        Ok(SampleReply {
            text: response.text,
            model: response.model,
            at_limit: response.finish == FinishReason::Length,
        })
    }
}

#[async_trait]
impl CallHost for RunHost {
    async fn sample(&self, server: &str, request: SampleRequest) -> std::result::Result<SampleReply, String> {
        let outcome = async {
            self.count()?;
            if self.scope.spent {
                return Err("The run's cost limit is used up.".to_string());
            }
            match self.person(server, ServerAsk::Model(request.clone())).await? {
                Answer::Yes { .. } => self.model(request).await,
                Answer::No => Err("The person did not allow the model call.".to_string()),
            }
        }
        .await;
        self.note(server, "model", outcome.as_ref().map_or_else(|why| why.as_str(), |_| "answered"));
        outcome
    }

    async fn ask(&self, server: &str, question: Question) -> std::result::Result<ElicitReply, String> {
        let outcome: std::result::Result<ElicitReply, String> = async {
            self.count()?;
            match self.person(server, ServerAsk::Person(question)).await? {
                Answer::Yes { values } => Ok(ElicitReply::Accepted(values)),
                Answer::No => Ok(ElicitReply::Declined),
            }
        }
        .await;
        let said = match &outcome {
            Ok(ElicitReply::Accepted(_)) => "accepted",
            Ok(ElicitReply::Declined) => "declined",
            Err(why) => why.as_str(),
        };
        self.note(server, "question", said);
        outcome
    }

    fn waiting(&self) -> bool {
        self.waiting.load(Ordering::SeqCst) > 0
    }

    fn waited(&self) -> Duration {
        *self.waited.lock()
    }
}
