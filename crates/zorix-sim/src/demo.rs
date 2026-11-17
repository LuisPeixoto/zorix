//! A rule-based model for demos and the command line. It reads the request in plain words, asks for the built-in
//! tools it needs one at a time, and writes a short answer from their results. It is deterministic and makes no
//! network calls.
//!
//! Directives in the prompt trigger behaviour that is hard to get otherwise: `[fail]` makes the call fail for good,
//! `[flaky]` fails the first two calls of the execution with a temporary error, `[hang]` never answers.

use std::collections::HashMap;

use async_trait::async_trait;
use parking_lot::Mutex;
use serde_json::{Value, json};
use tokio_util::sync::CancellationToken;
use zorix_core::{
    ExecutionId, FinishReason, Message, ModelError, ModelGateway, ModelRequest, ModelResponse, Role, ToolCallId,
    ToolCallRequest,
};

use crate::usage::usage_for;

/// What one finished tool call said.
#[derive(Debug, Clone)]
struct Seen {
    tool: String,
    ok: bool,
    body: String,
}

/// The rule-based demo model.
#[derive(Default)]
pub struct DemoModel {
    flaky_calls: Mutex<HashMap<ExecutionId, u32>>,
}

impl DemoModel {
    /// A new demo model.
    pub fn new() -> Self {
        Self::default()
    }
}

/// The task: the last trusted user message that is not the input data or a rejection notice. Earlier user messages are
/// memory (earlier prompts of the conversation, documents, past work) and must not be mistaken for it.
fn prompt_of(messages: &[Message]) -> String {
    messages
        .iter()
        .rev()
        .find(|m| {
            m.role == Role::User
                && m.provenance == zorix_core::Provenance::Trusted
                && !m.content.starts_with("Input data supplied")
                && !m.content.starts_with("Your answer was rejected")
        })
        .map(|m| m.content.clone())
        .unwrap_or_default()
}

/// Take a tool result out of the wrapper the context builder puts around it.
fn unwrap_result(content: &str) -> (bool, String) {
    let ok = !content.contains("status=\"error\"");
    let body = content
        .split_once('\n')
        .map(|(_, rest)| rest)
        .unwrap_or(content)
        .trim_end()
        .trim_end_matches("</tool_result>")
        .trim()
        .replace("<\\/tool_result", "</tool_result");
    (ok, body)
}

fn results_of(messages: &[Message]) -> Vec<Seen> {
    let mut names: HashMap<&ToolCallId, &str> = HashMap::new();
    let mut seen = Vec::new();
    for message in messages {
        for call in &message.tool_calls {
            names.insert(&call.id, call.tool.as_str());
        }
        if message.role == Role::Tool
            && let Some(id) = &message.tool_call_id
        {
            let (ok, body) = unwrap_result(&message.content);
            seen.push(Seen { tool: names.get(id).copied().unwrap_or("unknown").to_string(), ok, body });
        }
    }
    seen
}

/// The part of the prompt that looks like arithmetic, such as `17*23` or `(2+3)*4`.
fn find_expression(prompt: &str) -> Option<String> {
    let allowed = |c: char| c.is_ascii_digit() || "+-*/()%^. xX×÷".contains(c);
    let mut best: Option<String> = None;
    let mut run = String::new();
    for c in prompt.chars().chain(std::iter::once('\n')) {
        if allowed(c) {
            run.push(c);
            continue;
        }
        let candidate = run.trim().replace(['x', 'X', '×'], "*").replace('÷', "/");
        let has_digit = candidate.chars().any(|c| c.is_ascii_digit());
        let has_operator = candidate.chars().skip(1).any(|c| "+-*/%^".contains(c));
        if has_digit && has_operator && best.as_ref().is_none_or(|b| candidate.len() > b.len()) {
            best = Some(candidate);
        }
        run.clear();
    }
    best
}

fn quoted(prompt: &str) -> Vec<String> {
    prompt.split('"').skip(1).step_by(2).map(str::to_string).collect()
}

fn word_after(prompt: &str, marker: &str) -> Option<String> {
    let lower = prompt.to_lowercase();
    let at = lower.find(marker)? + marker.len();
    let word: String = prompt[at..]
        .trim_start()
        .trim_start_matches(['"', '\''])
        .chars()
        .take_while(|c| !c.is_whitespace() && !matches!(c, '"' | '\'' | ',' | ';' | '?'))
        .collect();
    let word = word.trim_end_matches('.').to_string();
    (!word.is_empty()).then_some(word)
}

/// The next tool to ask for, if any.
fn next_step(prompt: &str, tools: &[String], seen: &[Seen]) -> Option<(&'static str, Value)> {
    let has = |id: &str| tools.iter().any(|t| t == id);
    let used = |id: &str| seen.iter().any(|s| s.tool == id);
    let lower = prompt.to_lowercase();

    if has("math.evaluate")
        && !used("math.evaluate")
        && let Some(expression) = find_expression(prompt)
    {
        return Some(("math.evaluate", json!({ "expression": expression })));
    }
    if has("clock.now") && !used("clock.now") && ["time", "date", "hora", "clock"].iter().any(|w| lower.contains(w)) {
        return Some(("clock.now", json!({})));
    }
    if has("workspace.read")
        && !used("workspace.read")
        && let Some(path) = word_after(prompt, "read ")
            .map(|w| if w == "file" { word_after(prompt, "read file ").unwrap_or(w) } else { w })
    {
        return Some(("workspace.read", json!({ "path": path })));
    }
    if has("workspace.list")
        && !used("workspace.list")
        && (lower.contains("list files") || lower.contains("list the files"))
    {
        return Some(("workspace.list", json!({ "path": "." })));
    }
    if has("workspace.write")
        && !used("workspace.write")
        && lower.contains("write ")
        && let (Some(text), Some(path)) = (quoted(prompt).first().cloned(), word_after(prompt, " to "))
    {
        return Some(("workspace.write", json!({ "path": path, "content": text })));
    }
    if has("text.stats")
        && !used("text.stats")
        && lower.contains("count")
        && let Some(text) = quoted(prompt).first().cloned()
    {
        return Some(("text.stats", json!({ "text": text })));
    }
    if has("notes.put") && !used("notes.put") && (lower.contains("save") || lower.contains("note")) {
        let key = word_after(prompt, " as ").unwrap_or_else(|| "note".into());
        let value = match seen.iter().find(|s| s.tool == "math.evaluate" && s.ok) {
            Some(math) => {
                let parsed: Value = serde_json::from_str(&math.body).unwrap_or(Value::Null);
                format!("{} = {}", parsed["expression"].as_str().unwrap_or("?"), parsed["result"])
            }
            None => prompt.to_string(),
        };
        return Some(("notes.put", json!({ "key": key, "value": value })));
    }
    if has("notes.get") && !used("notes.get") && lower.contains("recall") {
        let key = word_after(prompt, "recall ").unwrap_or_else(|| "note".into());
        return Some(("notes.get", json!({ "key": key })));
    }
    None
}

fn answer(prompt: &str, seen: &[Seen]) -> String {
    if seen.is_empty() {
        return format!("I have no tool for that, so I can only restate the request: {prompt}");
    }
    let mut text = String::new();
    text.push_str(if seen.iter().all(|s| s.ok) { "Done.\n" } else { "I could not complete everything.\n" });
    for step in seen {
        text.push_str(&format!("- {}{}: {}\n", step.tool, if step.ok { "" } else { " (failed)" }, step.body));
    }
    text.trim_end().to_string()
}

#[async_trait]
impl ModelGateway for DemoModel {
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
        let prompt = prompt_of(&request.messages);
        if prompt.contains("[hang]") {
            cancel.cancelled().await;
            return Err(ModelError::Cancelled);
        }
        if prompt.contains("[fail]") {
            return Err(ModelError::Permanent("The demo model was asked to fail.".into()));
        }
        if prompt.contains("[flaky]") {
            let mut calls = self.flaky_calls.lock();
            let count = calls.entry(request.execution_id).or_insert(0);
            *count += 1;
            if *count <= 2 {
                return Err(ModelError::Transient {
                    message: "The demo model is busy.".into(),
                    retry_after_ms: Some(10),
                });
            }
        }

        let tools: Vec<String> = request.tools.iter().map(|t| t.id.clone()).collect();
        let seen = results_of(&request.messages);
        // A failed step ends the plan: report it instead of trying again.
        let blocked = seen.iter().any(|s| !s.ok);
        let (text, tool_calls, finish) = match (!blocked).then(|| next_step(&prompt, &tools, &seen)).flatten() {
            Some((tool, arguments)) => (
                format!("I will use {tool}."),
                vec![ToolCallRequest {
                    id: ToolCallId(String::new()),
                    provider_id: Some(format!("demo_{}", seen.len())),
                    tool: tool.into(),
                    arguments,
                }],
                FinishReason::ToolCalls,
            ),
            None => (answer(&prompt, &seen), Vec::new(), FinishReason::Stop),
        };
        let chars = text.chars().count() + tool_calls.iter().map(|c| c.arguments.to_string().len()).sum::<usize>();
        Ok(ModelResponse::new(request.model.clone(), text, tool_calls, usage_for(&request, chars), finish))
    }
}

#[cfg(test)]
mod tests {
    use zorix_core::ToolDefinition;

    use super::*;
    use crate::demo::DemoModel;

    fn tool(id: &str) -> ToolDefinition {
        ToolDefinition { id: id.into(), description: String::new(), input_schema: json!({"type": "object"}) }
    }

    fn request(prompt: &str, transcript: Vec<Message>) -> ModelRequest {
        let mut messages = vec![Message::system("preamble"), Message::user(prompt)];
        messages.extend(transcript);
        ModelRequest {
            model: "demo".into(),
            execution_id: ExecutionId::new(),
            messages,
            tools: [
                "math.evaluate",
                "clock.now",
                "notes.put",
                "notes.get",
                "workspace.read",
                "workspace.list",
                "workspace.write",
                "text.stats",
            ]
            .into_iter()
            .map(tool)
            .collect(),
            response_schema: None,
            ..ModelRequest::default()
        }
    }

    fn turn(id: &str, tool: &str, body: &str, ok: bool) -> Vec<Message> {
        let call =
            ToolCallRequest { id: ToolCallId(id.into()), provider_id: None, tool: tool.into(), arguments: json!({}) };
        let status = if ok { "ok" } else { "error" };
        vec![
            Message::assistant("", vec![call]),
            Message::tool_result(
                ToolCallId(id.into()),
                format!(
                    "<tool_result call_id=\"{id}\" status=\"{status}\" untrusted=\"true\">\n{body}\n</tool_result>"
                ),
                !ok,
            ),
        ]
    }

    async fn ask(model: &DemoModel, request: ModelRequest) -> ModelResponse {
        model.generate(request, CancellationToken::new()).await.unwrap()
    }

    #[test]
    fn finds_arithmetic_in_a_sentence() {
        assert_eq!(find_expression("What is 17*23 and save it?").as_deref(), Some("17*23"));
        assert_eq!(find_expression("compute (2+3) x 4 please").as_deref(), Some("(2+3) * 4"));
        assert_eq!(find_expression("call me on 555 1234"), None);
        assert_eq!(find_expression("no numbers here"), None);
    }

    #[tokio::test]
    async fn the_plan_for_the_acceptance_scenario_is_math_then_note_then_answer() {
        let model = DemoModel::new();
        let prompt = "What is 17*23 and save it as result?";

        let first = ask(&model, request(prompt, vec![])).await;
        assert_eq!(first.tool_calls[0].tool, "math.evaluate");
        assert_eq!(first.tool_calls[0].arguments, json!({"expression": "17*23"}));

        let after_math = turn("c1", "math.evaluate", r#"{"expression":"17*23","result":391}"#, true);
        let second = ask(&model, request(prompt, after_math.clone())).await;
        assert_eq!(second.tool_calls[0].tool, "notes.put");
        assert_eq!(second.tool_calls[0].arguments, json!({"key": "result", "value": "17*23 = 391"}));

        let mut after_note = after_math;
        after_note.extend(turn("c2", "notes.put", r#"{"saved":"result"}"#, true));
        let last = ask(&model, request(prompt, after_note)).await;
        assert!(last.tool_calls.is_empty());
        assert_eq!(last.finish, FinishReason::Stop);
        assert!(last.text.starts_with("Done."));
        assert!(last.text.contains("391"));
    }

    #[tokio::test]
    async fn a_failed_tool_ends_the_plan_with_an_honest_answer() {
        let model = DemoModel::new();
        let transcript = turn("c1", "math.evaluate", "Division by zero.", false);
        let last = ask(&model, request("What is 1/0 and save it?", transcript)).await;
        assert!(last.tool_calls.is_empty());
        assert!(last.text.contains("could not complete") && last.text.contains("Division by zero."));
    }

    #[tokio::test]
    async fn it_picks_file_tools_from_the_words() {
        let model = DemoModel::new();
        let read = ask(&model, request("Please read notes.txt", vec![])).await;
        assert_eq!(read.tool_calls[0].tool, "workspace.read");
        assert_eq!(read.tool_calls[0].arguments, json!({"path": "notes.txt"}));
        let write = ask(&model, request("Write \"hello\" to out/a.txt", vec![])).await;
        assert_eq!(write.tool_calls[0].tool, "workspace.write");
        assert_eq!(write.tool_calls[0].arguments, json!({"path": "out/a.txt", "content": "hello"}));
        let list = ask(&model, request("list files", vec![])).await;
        assert_eq!(list.tool_calls[0].tool, "workspace.list");
    }

    #[tokio::test]
    async fn without_a_matching_tool_it_says_so() {
        let model = DemoModel::new();
        let last = ask(&model, request("Tell me a joke", vec![])).await;
        assert!(last.tool_calls.is_empty() && last.text.contains("no tool"));
    }

    #[tokio::test]
    async fn the_directives_fail_flake_and_hang() {
        let model = DemoModel::new();
        let error = model.generate(request("[fail] hi", vec![]), CancellationToken::new()).await.unwrap_err();
        assert!(matches!(error, ModelError::Permanent(_)));

        let flaky = request("[flaky] what time is it", vec![]);
        for _ in 0..2 {
            let error = model.generate(flaky.clone(), CancellationToken::new()).await.unwrap_err();
            assert!(matches!(error, ModelError::Transient { .. }));
        }
        assert_eq!(ask(&model, flaky).await.tool_calls[0].tool, "clock.now");

        let cancel = CancellationToken::new();
        let stop = cancel.clone();
        tokio::spawn(async move {
            tokio::time::sleep(std::time::Duration::from_millis(10)).await;
            stop.cancel();
        });
        assert_eq!(model.generate(request("[hang]", vec![]), cancel).await, Err(ModelError::Cancelled));
    }

    #[tokio::test]
    async fn only_offered_tools_are_used() {
        let model = DemoModel::new();
        let mut no_tools = request("What is 2+2?", vec![]);
        no_tools.tools.clear();
        assert!(ask(&model, no_tools).await.tool_calls.is_empty());
    }
}
