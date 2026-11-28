//! ZORIX as an MCP server (ADR-0023): any set of [`Tool`]s served over a pair of byte streams (stdio for `zorix mcp serve`).
//!
//! It speaks both protocol eras: a request that carries the modern `_meta` is served statelessly, and `initialize` selects a
//! legacy revision. It never announces changes (the tool set is fixed for the life of the process) and never runs a tool that needs a
//! person's approval. The one thing it asks the client for is a person's yes, for a tool set up with a [`Confirmation`] (ADR-0052):
//! through `elicitation/create`, or an `input_required` result in the modern revision. A client that cannot ask is refused.

use std::collections::{BTreeMap, HashMap, HashSet};
use std::path::PathBuf;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::time::Duration;

use async_trait::async_trait;
use parking_lot::Mutex;
use serde_json::{Value, json};
use sha2::{Digest, Sha256};
use tokio::io::{AsyncBufReadExt, AsyncRead, AsyncWrite, AsyncWriteExt, BufReader};
use tokio::sync::{Semaphore, mpsc, oneshot};
use tokio::task::JoinSet;
use tokio_util::sync::CancellationToken;
use zorix_core::{ExecutionId, Grants, TenantId, Tool, ToolCallId, ToolContext, ToolError};

use crate::protocol::{LEGACY_VERSIONS, MODERN_VERSION};
use crate::wire::{self, Incoming};

/// The largest request line accepted.
pub const MAX_REQUEST_BYTES: usize = 4 * 1024 * 1024;
/// How many calls may run at once.
pub const MAX_IN_FLIGHT: usize = 8;

const PARSE_ERROR: i64 = -32700;
const INVALID_REQUEST: i64 = -32600;
const METHOD_NOT_FOUND: i64 = -32601;
const INVALID_PARAMS: i64 = -32602;
const UNSUPPORTED_VERSION: i64 = -32022;
const BUSY: i64 = -32000;

/// What the server is and what its calls may do.
#[derive(Clone, Debug)]
pub struct ServerConfig {
    /// The server's name, reported to clients.
    pub name: String,
    /// Its version.
    pub version: String,
    /// What a model should know about these tools.
    pub instructions: Option<String>,
    /// The tenant every call runs as.
    pub tenant: TenantId,
    /// What calls may do. A tool whose required permissions are not covered is not run.
    pub grants: Grants,
    /// The project folder calls are scoped to (memory visibility, file tools), when there is one.
    pub workspace_root: Option<PathBuf>,
}

/// How long a person has to answer a question asked through the client.
const CONFIRM_TIMEOUT: Duration = Duration::from_secs(300);

/// A person's yes, asked through the client before a tool runs.
#[async_trait]
pub trait Confirmation: Send + Sync {
    /// What to ask, naming exactly what the call would do, or why it is refused without asking.
    async fn question(&self, args: &Value) -> Result<String, String>;
}

/// A read-only resource the server offers.
#[derive(Clone, Debug)]
pub struct ResourceInfo {
    /// Its address.
    pub uri: String,
    /// Its name.
    pub name: String,
    /// What it is.
    pub description: String,
}

/// Read-only data the server offers as resources.
#[async_trait]
pub trait Resources: Send + Sync {
    /// What there is to read.
    async fn list(&self) -> Vec<ResourceInfo>;
    /// One resource as JSON text, or `None` when there is none by that address.
    async fn read(&self, uri: &str) -> Option<String>;
}

/// A server over a fixed set of tools.
pub struct McpServer {
    config: ServerConfig,
    tools: BTreeMap<String, Entry>,
    confirmations: HashMap<String, Arc<dyn Confirmation>>,
    resources: Option<Arc<dyn Resources>>,
    /// A secret of this process, so a modern client cannot make up the state that binds a person's answer to a call.
    secret: [u8; 32],
}

struct Entry {
    tool: Arc<dyn Tool>,
    validator: Option<jsonschema::Validator>,
}

struct Session {
    /// A legacy client has sent `initialize`.
    initialized: AtomicBool,
    /// A legacy client said at `initialize` that it can ask the person.
    elicits: AtomicBool,
    /// Questions sent to the client and not yet answered, by their id.
    asked: Mutex<HashMap<String, oneshot::Sender<Value>>>,
    /// States of modern questions already answered, so one answer is not used twice.
    spent: Mutex<HashSet<String>>,
    next_question: AtomicU64,
    out: mpsc::Sender<String>,
    in_flight: Mutex<HashMap<String, CancellationToken>>,
    slots: Arc<Semaphore>,
    cancel: CancellationToken,
}

impl McpServer {
    /// A server of these tools. Tools with an invalid input schema are left out: a call could not be checked.
    pub fn new(config: ServerConfig, tools: Vec<Arc<dyn Tool>>) -> Self {
        let mut map = BTreeMap::new();
        for tool in tools {
            let spec = tool.spec();
            if let Ok(validator) = jsonschema::validator_for(&spec.input_schema) {
                map.insert(spec.id.clone(), Entry { tool: tool.clone(), validator: Some(validator) });
            }
        }
        let mut secret = [0u8; 32];
        // Without randomness no state can be trusted; a zero secret would make states guessable, so confirmations are refused instead.
        let _ = getrandom::fill(&mut secret);
        Self { config, tools: map, confirmations: HashMap::new(), resources: None, secret }
    }

    /// Ask the person through the client before `tool` runs.
    pub fn confirming(mut self, tool: &str, confirmation: Arc<dyn Confirmation>) -> Self {
        self.confirmations.insert(tool.to_string(), confirmation);
        self
    }

    /// Offer these resources.
    pub fn with_resources(mut self, resources: Arc<dyn Resources>) -> Self {
        self.resources = Some(resources);
        self
    }

    /// The ids of the tools it serves.
    pub fn tool_ids(&self) -> Vec<String> {
        self.tools.keys().cloned().collect()
    }

    /// Serve until the client closes the input or `cancel` fires. Calls still running are cancelled.
    pub async fn serve<R, W>(self: Arc<Self>, reader: R, writer: W, cancel: CancellationToken) -> std::io::Result<()>
    where
        R: AsyncRead + Unpin + Send,
        W: AsyncWrite + Unpin + Send + 'static,
    {
        let (out, mut lines) = mpsc::channel::<String>(64);
        let writer_task = tokio::spawn(async move {
            let mut writer = writer;
            while let Some(line) = lines.recv().await {
                if writer.write_all(line.as_bytes()).await.is_err()
                    || writer.write_all(b"\n").await.is_err()
                    || writer.flush().await.is_err()
                {
                    break;
                }
            }
        });
        let session = Arc::new(Session {
            initialized: AtomicBool::new(false),
            elicits: AtomicBool::new(false),
            asked: Mutex::new(HashMap::new()),
            spent: Mutex::new(HashSet::new()),
            next_question: AtomicU64::new(1),
            out: out.clone(),
            in_flight: Mutex::new(HashMap::new()),
            slots: Arc::new(Semaphore::new(MAX_IN_FLIGHT)),
            cancel: cancel.child_token(),
        });
        let mut tasks = JoinSet::new();
        let mut reader = BufReader::new(reader);
        loop {
            let line = tokio::select! {
                _ = cancel.cancelled() => break,
                line = read_line(&mut reader) => line?,
            };
            let text = match line {
                Line::Eof => break,
                Line::TooLong => {
                    let _ = out.send(wire::error_line(&Value::Null, PARSE_ERROR, "The request was too large.")).await;
                    continue;
                }
                Line::Text(text) => text,
            };
            if text.trim().is_empty() {
                continue;
            }
            match wire::parse(&text) {
                Err(e) => {
                    let _ = out.send(wire::error_line(&Value::Null, PARSE_ERROR, &e.to_string())).await;
                }
                Ok(Incoming::Notification { method, params }) => {
                    if method == "notifications/cancelled"
                        && let Some(id) = params.get("requestId")
                        && let Some(token) = session.in_flight.lock().get(&id.to_string())
                    {
                        token.cancel();
                    }
                }
                Ok(Incoming::Response { id, result }) => {
                    // The answer to a question this server asked; anything else is nothing it waits for.
                    if let Some(waiting) = id.as_str().and_then(|id| session.asked.lock().remove(id)) {
                        let _ = waiting.send(result.unwrap_or(Value::Null));
                    }
                }
                Ok(Incoming::Request { id, method, params }) => {
                    let server = self.clone();
                    let session = session.clone();
                    let out = out.clone();
                    tasks.spawn(async move {
                        if let Some(line) = server.handle(&session, &id, &method, params).await {
                            let _ = out.send(line).await;
                        }
                    });
                }
            }
        }
        session.cancel.cancel();
        drop(out);
        let drain = async { while tasks.join_next().await.is_some() {} };
        if tokio::time::timeout(Duration::from_secs(5), drain).await.is_err() {
            tasks.abort_all();
        }
        // The session holds a way to write too; the writer ends only when every one is gone.
        drop(session);
        let _ = writer_task.await;
        Ok(())
    }

    /// The answer line to one request, or `None` when the request was cancelled and gets no answer.
    async fn handle(&self, session: &Session, id: &Value, method: &str, params: Value) -> Option<String> {
        let modern = modern_meta_version(&params);
        if let Some(version) = &modern
            && version != MODERN_VERSION
        {
            let mut supported = vec![MODERN_VERSION];
            supported.extend(LEGACY_VERSIONS.iter().copied());
            return Some(
                json!({"jsonrpc": "2.0", "id": id, "error": {
                    "code": UNSUPPORTED_VERSION,
                    "message": format!("The protocol version {version} is not supported."),
                    "data": {"supported": supported},
                }})
                .to_string(),
            );
        }
        let is_modern = modern.is_some();
        match method {
            "ping" => Some(wire::result_line(id, &json!({}))),
            "server/discover" => Some(wire::result_line(id, &self.discover())),
            "initialize" => {
                let wanted = params.get("protocolVersion").and_then(Value::as_str).unwrap_or("");
                let version = LEGACY_VERSIONS.iter().find(|v| **v == wanted).copied().unwrap_or(LEGACY_VERSIONS[0]);
                session.initialized.store(true, Ordering::SeqCst);
                session.elicits.store(params["capabilities"].get("elicitation").is_some(), Ordering::SeqCst);
                let mut result = json!({
                    "protocolVersion": version,
                    "capabilities": self.capabilities(),
                    "serverInfo": {"name": self.config.name, "version": self.config.version},
                });
                if let Some(instructions) = &self.config.instructions {
                    result["instructions"] = json!(instructions);
                }
                Some(wire::result_line(id, &result))
            }
            "resources/list" | "resources/read" if self.resources.is_some() => {
                if !is_modern && !session.initialized.load(Ordering::SeqCst) {
                    return Some(wire::error_line(id, INVALID_REQUEST, "Initialize the session first."));
                }
                let resources = self.resources.as_ref()?;
                if method == "resources/list" {
                    let list: Vec<Value> = resources
                        .list()
                        .await
                        .into_iter()
                        .map(|r| json!({"uri": r.uri, "name": r.name, "description": r.description, "mimeType": "application/json"}))
                        .collect();
                    return Some(wire::result_line(id, &complete(json!({"resources": list}), is_modern)));
                }
                let uri = params.get("uri").and_then(Value::as_str).unwrap_or("");
                Some(match resources.read(uri).await {
                    Some(text) => wire::result_line(
                        id,
                        &complete(
                            json!({"contents": [{"uri": uri, "mimeType": "application/json", "text": text}]}),
                            is_modern,
                        ),
                    ),
                    None => wire::error_line(id, INVALID_PARAMS, &format!("There is no resource at {uri}.")),
                })
            }
            "tools/list" | "tools/call" => {
                if !is_modern && !session.initialized.load(Ordering::SeqCst) {
                    return Some(wire::error_line(
                        id,
                        INVALID_REQUEST,
                        "Send the protocol metadata with the request, or initialize the session first.",
                    ));
                }
                if method == "tools/list" {
                    Some(wire::result_line(id, &complete(self.list(), is_modern)))
                } else {
                    self.call(session, id, params, is_modern).await
                }
            }
            _ => Some(wire::error_line(id, METHOD_NOT_FOUND, "Method not found")),
        }
    }

    fn capabilities(&self) -> Value {
        let mut capabilities = json!({"tools": {"listChanged": false}});
        if self.resources.is_some() {
            capabilities["resources"] = json!({});
        }
        capabilities
    }

    fn discover(&self) -> Value {
        let mut supported = vec![MODERN_VERSION];
        supported.extend(LEGACY_VERSIONS.iter().copied());
        let mut result = json!({
            "resultType": "complete",
            "supportedVersions": supported,
            "capabilities": self.capabilities(),
            "_meta": {"io.modelcontextprotocol/serverInfo": {"name": self.config.name, "version": self.config.version}},
        });
        if let Some(instructions) = &self.config.instructions {
            result["instructions"] = json!(instructions);
        }
        result
    }

    fn list(&self) -> Value {
        let tools: Vec<Value> = self
            .tools
            .values()
            .map(|entry| {
                let spec = entry.tool.spec();
                let mut tool = json!({
                    "name": spec.id,
                    "title": spec.name,
                    "description": spec.description,
                    "inputSchema": spec.input_schema,
                    "annotations": {
                        "readOnlyHint": spec.annotations.read_only,
                        "destructiveHint": spec.annotations.destructive,
                        "idempotentHint": spec.annotations.idempotent,
                        "openWorldHint": spec.annotations.open_world,
                    },
                });
                if let Some(output) = &spec.output_schema {
                    tool["outputSchema"] = output.clone();
                }
                tool
            })
            .collect();
        json!({ "tools": tools })
    }

    async fn call(&self, session: &Session, id: &Value, params: Value, modern: bool) -> Option<String> {
        let name = params.get("name").and_then(Value::as_str).unwrap_or("");
        let Some(entry) = self.tools.get(name) else {
            return Some(wire::error_line(id, INVALID_PARAMS, &format!("Unknown tool: {name}")));
        };
        let args = match params.get("arguments") {
            None | Some(Value::Null) => json!({}),
            Some(args) => args.clone(),
        };
        if let Some(validator) = &entry.validator
            && let Err(e) = validator.validate(&args)
        {
            return Some(wire::error_line(id, INVALID_PARAMS, &format!("The arguments do not fit the tool: {e}")));
        }
        let spec = entry.tool.spec();
        let required = spec.all_required_permissions();
        let missing = self.config.grants.missing(&required);
        if !missing.is_empty() {
            let list: Vec<String> = missing.iter().map(|p| p.to_string()).collect();
            return Some(tool_failure(
                id,
                modern,
                &format!("ZORIX did not allow this server to use {name}. Missing: {}.", list.join(", ")),
            ));
        }
        if spec.requires_approval || spec.approval_by_person_only {
            return Some(tool_failure(
                id,
                modern,
                &format!(
                    "{name} needs a person's approval, which can only be given in ZORIX, not through an MCP client. Nothing was done."
                ),
            ));
        }
        if let Some(confirmation) = self.confirmations.get(name) {
            let question = match confirmation.question(&args).await {
                Ok(question) => question,
                Err(why) => return Some(tool_failure(id, modern, &why)),
            };
            match self.confirmed(session, name, &args, &question, &params, modern).await {
                Confirmed::Yes => {}
                Confirmed::Ask(result) => return Some(wire::result_line(id, &result)),
                Confirmed::No(why) => return Some(tool_failure(id, modern, why)),
            }
        }
        let Ok(permit) = session.slots.clone().try_acquire_owned() else {
            return Some(wire::error_line(id, BUSY, "Too many calls are running; try again in a moment."));
        };

        let key = id.to_string();
        let token = session.cancel.child_token();
        session.in_flight.lock().insert(key.clone(), token.clone());
        let ctx = ToolContext {
            execution_id: ExecutionId::new(),
            tenant: self.config.tenant.clone(),
            call_id: ToolCallId(format!("mcp-{key}")),
            grants: self.config.grants.clone(),
            cancel: token.clone(),
            workspace_root: self.config.workspace_root.clone(),
            conversation: None,
            agent: None,
            host: None,
        };
        let timeout = Duration::from_millis(spec.timeout_ms);
        let outcome = tokio::select! {
            _ = token.cancelled() => None,
            result = tokio::time::timeout(timeout, entry.tool.call(&ctx, args)) => Some(result),
        };
        session.in_flight.lock().remove(&key);
        drop(permit);
        // A cancelled call gets no answer, even when the tool noticed the cancellation first and returned.
        if token.is_cancelled() {
            return None;
        }
        let outcome = outcome?;
        Some(match outcome {
            Err(_) => tool_failure(
                id,
                modern,
                &format!("{name} took longer than {} seconds and was stopped.", timeout.as_secs()),
            ),
            Ok(Err(ToolError::Failed(message) | ToolError::Retryable(message))) => tool_failure(id, modern, &message),
            Ok(Ok(value)) => tool_success(id, modern, &value, spec.max_output_bytes),
        })
    }
}

/// What came of asking the person.
enum Confirmed {
    Yes,
    /// A modern result that asks the client to ask, and to call again with the answer.
    Ask(Value),
    No(&'static str),
}

const CANNOT_ASK: &str = "This client cannot ask you (it does not offer elicitation), so ZORIX does not do this through it. Nothing was done.";
const REFUSED: &str = "You did not say yes, so nothing was done.";

/// The question as an elicitation: one box to tick.
fn elicitation(question: &str) -> Value {
    json!({
        "message": question,
        "requestedSchema": {
            "type": "object",
            "properties": {"confirm": {"type": "boolean", "title": "Yes, do it"}},
            "required": ["confirm"]
        }
    })
}

fn said_yes(answer: &Value) -> bool {
    answer["action"] == "accept" && answer["content"]["confirm"] == true
}

impl McpServer {
    /// A state that binds a modern answer to this very call and question. The number makes each asking its own, so a call that
    /// was refused can be asked about again while no answer counts twice.
    fn state(&self, number: &str, name: &str, args: &Value, question: &str) -> String {
        let mut hash = Sha256::new();
        for part in [number, name, &args.to_string(), question] {
            hash.update(self.secret);
            hash.update((part.len() as u64).to_le_bytes());
            hash.update(part.as_bytes());
        }
        let hex: String = hash.finalize().iter().map(|b| format!("{b:02x}")).collect();
        format!("{number}.{hex}")
    }

    async fn confirmed(
        &self,
        session: &Session,
        name: &str,
        args: &Value,
        question: &str,
        params: &Value,
        modern: bool,
    ) -> Confirmed {
        if self.secret == [0u8; 32] {
            return Confirmed::No("ZORIX could not make a secure question, so nothing was done.");
        }
        if modern {
            let can = params["_meta"]["io.modelcontextprotocol/clientCapabilities"].get("elicitation").is_some();
            if !can {
                return Confirmed::No(CANNOT_ASK);
            }
            let Some(answer) = params["inputResponses"].get("confirm") else {
                let number = session.next_question.fetch_add(1, Ordering::SeqCst).to_string();
                let state = self.state(&number, name, args, question);
                return Confirmed::Ask(json!({
                    "resultType": "input_required",
                    "inputRequests": {"confirm": {"method": "elicitation/create", "params": elicitation(question)}},
                    "requestState": state
                }));
            };
            // The answer must come with the state of this call and this question, and be used once.
            let given = params["requestState"].as_str().unwrap_or("");
            let number = given.split_once('.').map_or("", |(number, _)| number);
            if given != self.state(number, name, args, question) || !session.spent.lock().insert(given.to_string()) {
                return Confirmed::No("That answer belongs to another question, so nothing was done.");
            }
            return if said_yes(answer) { Confirmed::Yes } else { Confirmed::No(REFUSED) };
        }
        if !session.elicits.load(Ordering::SeqCst) {
            return Confirmed::No(CANNOT_ASK);
        }
        let id = format!("zorix-{}", session.next_question.fetch_add(1, Ordering::SeqCst));
        let (tx, rx) = oneshot::channel();
        {
            let mut asked = session.asked.lock();
            if asked.len() >= MAX_IN_FLIGHT {
                return Confirmed::No("Too many questions are already waiting for your answer. Nothing was done.");
            }
            asked.insert(id.clone(), tx);
        }
        let request =
            json!({"jsonrpc": "2.0", "id": id, "method": "elicitation/create", "params": elicitation(question)});
        if session.out.send(request.to_string()).await.is_err() {
            return Confirmed::No(REFUSED);
        }
        let answer = tokio::select! {
            _ = session.cancel.cancelled() => None,
            answer = tokio::time::timeout(CONFIRM_TIMEOUT, rx) => answer.ok().and_then(Result::ok),
        };
        session.asked.lock().remove(&id);
        match answer {
            Some(answer) if said_yes(&answer) => Confirmed::Yes,
            _ => Confirmed::No(REFUSED),
        }
    }
}

fn complete(mut result: Value, modern: bool) -> Value {
    if modern {
        result["resultType"] = json!("complete");
    }
    result
}

fn tool_failure(id: &Value, modern: bool, message: &str) -> String {
    wire::result_line(id, &complete(json!({"content": [{"type": "text", "text": message}], "isError": true}), modern))
}

fn tool_success(id: &Value, modern: bool, value: &Value, max_bytes: usize) -> String {
    let mut text = match value {
        Value::String(text) => text.clone(),
        other => other.to_string(),
    };
    let mut cut = false;
    if text.len() > max_bytes {
        let mut end = max_bytes;
        while !text.is_char_boundary(end) {
            end -= 1;
        }
        text.truncate(end);
        text.push_str("\n[ZORIX cut the rest of this result]");
        cut = true;
    }
    let mut result = json!({"content": [{"type": "text", "text": text}], "isError": false});
    if !cut && value.is_object() {
        result["structuredContent"] = value.clone();
    }
    wire::result_line(id, &complete(result, modern))
}

/// The protocol version in a modern request's `_meta`, if it has one.
fn modern_meta_version(params: &Value) -> Option<String> {
    params.get("_meta")?.get("io.modelcontextprotocol/protocolVersion")?.as_str().map(str::to_string)
}

enum Line {
    Text(String),
    TooLong,
    Eof,
}

/// One line, never holding more than [`MAX_REQUEST_BYTES`] in memory: a longer one is skipped to its end.
async fn read_line<R: AsyncRead + Unpin>(reader: &mut BufReader<R>) -> std::io::Result<Line> {
    let mut line: Vec<u8> = Vec::new();
    let mut too_long = false;
    loop {
        let buffer = reader.fill_buf().await?;
        if buffer.is_empty() {
            return Ok(if line.is_empty() && !too_long {
                Line::Eof
            } else if too_long {
                Line::TooLong
            } else {
                Line::Text(String::from_utf8_lossy(&line).into_owned())
            });
        }
        let (used, done) = match buffer.iter().position(|b| *b == b'\n') {
            Some(at) => (at + 1, true),
            None => (buffer.len(), false),
        };
        if !too_long {
            if line.len() + used > MAX_REQUEST_BYTES {
                too_long = true;
                line.clear();
            } else {
                line.extend_from_slice(&buffer[..used]);
            }
        }
        reader.consume(used);
        if done {
            return Ok(if too_long {
                Line::TooLong
            } else {
                Line::Text(String::from_utf8_lossy(&line).trim_end_matches(['\n', '\r']).to_string())
            });
        }
    }
}
