//! Answering a server's requests on the terminal while following a run (ADR-0051). Everything the server says is shown with its
//! control characters written out, so it cannot move the cursor, hide a line or imitate the prompt.

use std::io::Write;

use serde_json::{Map, Value, json};
use zorix_core::{Answer, FieldKind, FormField, PendingAsk, Question, ServerAsk, TenantId};
use zorix_runtime::Engine;

use crate::render::visible;

/// The most of one message shown.
const SHOWN: usize = 2_000;

fn block(text: &str) -> String {
    let mut shown = visible(text);
    if let Some((at, _)) = shown.char_indices().nth(SHOWN) {
        shown.truncate(at);
        shown.push('…');
    }
    shown.lines().map(|line| format!("    {line}")).collect::<Vec<_>>().join("\n")
}

/// What the person is shown about a request.
pub fn describe(ask: &PendingAsk) -> String {
    let server = visible(&ask.server);
    match &ask.ask {
        ServerAsk::Model(request) => {
            let mut text = format!(
                "\n  The server {server} asks to use your model ({} tokens at most, charged to this run).\n",
                request.max_tokens
            );
            if let Some(system) = &request.system {
                text.push_str(&format!("  Its instructions:\n{}\n", block(system)));
            }
            for message in &request.messages {
                text.push_str(&format!(
                    "  {}:\n{}\n",
                    json!(message.role).as_str().unwrap_or("?"),
                    block(&message.text)
                ));
            }
            text
        }
        ServerAsk::Person(Question::Form { message, .. }) => format!(
            "\n  The server {server} asks:\n{}\n  Never type a password, a key or a token here.\n",
            block(message)
        ),
        ServerAsk::Person(Question::Link { message, url }) => format!(
            "\n  The server {server} asks you to open an address in your browser:\n{}\n  {}\n",
            block(message),
            visible(url)
        ),
    }
}

/// Read one field. `None` ends the form (the input is over); `Some(None)` leaves an optional field out.
fn field(field: &FormField, read: &mut impl FnMut(&str) -> Option<String>) -> Option<Option<Value>> {
    let label = visible(field.title.as_deref().unwrap_or(&field.name)).replace('\n', " ");
    let hint = match &field.kind {
        FieldKind::Text => String::new(),
        FieldKind::Number => " (a number)".into(),
        FieldKind::Integer => " (a whole number)".into(),
        FieldKind::Boolean => " (y or n)".into(),
        FieldKind::Choice { options } => format!(" (one of: {})", visible(&options.join(", ")).replace('\n', " ")),
    };
    let skip = if field.required { "" } else { " [Enter to skip]" };
    if let Some(description) = &field.description {
        eprintln!("{}", block(description));
    }
    for _ in 0..3 {
        let line = read(&format!("  {label}{hint}{skip}: "))?;
        let line = line.trim_end_matches(['\r', '\n']);
        if line.is_empty() && !field.required {
            return Some(None);
        }
        let value = match &field.kind {
            _ if line.is_empty() => None,
            FieldKind::Text => Some(json!(line)),
            FieldKind::Integer => line.trim().parse::<i64>().ok().map(|n| json!(n)),
            FieldKind::Number => line.trim().parse::<f64>().ok().filter(|n| n.is_finite()).map(|n| json!(n)),
            FieldKind::Boolean => match line.trim().to_lowercase().as_str() {
                "y" | "yes" | "true" => Some(json!(true)),
                "n" | "no" | "false" => Some(json!(false)),
                _ => None,
            },
            FieldKind::Choice { options } => options.iter().find(|o| *o == line.trim()).map(|o| json!(o)),
        };
        if value.is_some() {
            return Some(value);
        }
        eprintln!("  That does not fit.");
    }
    None
}

fn yes(line: Option<String>) -> bool {
    line.is_some_and(|line| matches!(line.trim().to_lowercase().as_str(), "y" | "yes" | "s" | "sim"))
}

/// Ask the person about one request; `read` shows a prompt and reads a line, `None` when there is no more input.
pub fn answer_of(ask: &PendingAsk, read: &mut impl FnMut(&str) -> Option<String>) -> Answer {
    eprintln!("{}", describe(ask));
    match &ask.ask {
        ServerAsk::Model(_) => {
            if yes(read("  Allow this model call? [y/N] ")) {
                Answer::Yes { values: Map::new() }
            } else {
                Answer::No
            }
        }
        ServerAsk::Person(Question::Link { .. }) => {
            if yes(read("  Will you open it? [y/N] ")) {
                Answer::Yes { values: Map::new() }
            } else {
                Answer::No
            }
        }
        ServerAsk::Person(Question::Form { fields, .. }) => {
            if !yes(read("  Answer it? [y/N] ")) {
                return Answer::No;
            }
            let mut values = Map::new();
            for wanted in fields {
                match field(wanted, read) {
                    Some(Some(value)) => {
                        values.insert(wanted.name.clone(), value);
                    }
                    Some(None) => {}
                    None => return Answer::No,
                }
            }
            Answer::Yes { values }
        }
    }
}

fn read_line(prompt: &str) -> Option<String> {
    eprint!("{prompt}");
    let _ = std::io::stderr().flush();
    let mut line = String::new();
    match std::io::stdin().read_line(&mut line) {
        Ok(0) | Err(_) => {
            eprintln!("(no answer: treating it as no)");
            None
        }
        Ok(_) => Some(line),
    }
}

/// Answer every waiting request of these runs.
pub async fn answer_waiting(engine: &Engine, tenant: &TenantId, of: impl Fn(&PendingAsk) -> bool) {
    for ask in engine.asks().pending(tenant).into_iter().filter(|ask| of(ask)) {
        let (id, hash) = (ask.id, ask.hash.clone());
        let answer = tokio::task::spawn_blocking(move || answer_of(&ask, &mut read_line)).await.unwrap_or(Answer::No);
        if let Err(error) = engine.asks().answer(tenant, id, &hash, answer) {
            eprintln!("  {error}");
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use zorix_core::{AskId, ExecutionId, SampleMessage, SampleRequest, SampleRole, Timestamp, ToolCallId};

    fn pending(ask: ServerAsk) -> PendingAsk {
        PendingAsk {
            id: AskId::new(),
            tenant: TenantId::local(),
            execution_id: ExecutionId::new(),
            call_id: ToolCallId("c".into()),
            server: "files".into(),
            hash: ask.hash("files"),
            ask,
            asked_at: Timestamp(1),
        }
    }

    fn form() -> PendingAsk {
        let field = |name: &str, kind, required| FormField {
            name: name.into(),
            title: None,
            description: None,
            kind,
            required,
        };
        pending(ServerAsk::Person(Question::Form {
            message: "Which?".into(),
            fields: vec![
                field("count", FieldKind::Integer, true),
                field("sure", FieldKind::Boolean, true),
                field("note", FieldKind::Text, false),
            ],
        }))
    }

    fn answering(ask: &PendingAsk, lines: &[&str]) -> Answer {
        let mut lines = lines.iter();
        answer_of(ask, &mut |_| lines.next().map(|l| l.to_string()))
    }

    #[test]
    fn a_form_is_filled_field_by_field_and_bad_values_are_asked_again() {
        let answer = answering(&form(), &["y", "many", "3", "maybe", "n", ""]);
        let Answer::Yes { values } = answer else { panic!("{answer:?}") };
        assert_eq!(Value::Object(values), json!({"count": 3, "sure": false}));
        assert_eq!(answering(&form(), &["n"]), Answer::No);
        assert_eq!(answering(&form(), &["y", "3"]), Answer::No, "input that ends is a no");
        assert_eq!(answering(&form(), &[]), Answer::No);
    }

    #[test]
    fn what_a_server_says_cannot_draw_on_the_terminal() {
        let text = describe(&pending(ServerAsk::Model(SampleRequest {
            system: Some("\u{1b}[2Jgone".into()),
            messages: vec![SampleMessage { role: SampleRole::User, text: "a\rAllow this? [y/N] y\u{202e}".into() }],
            max_tokens: 5,
        })));
        assert!(!text.contains('\u{1b}') && !text.contains('\r') && !text.contains('\u{202e}'), "{text:?}");
        assert!(text.contains("\\u{1b}[2Jgone") && text.contains("    a\\u{d}Allow"), "{text}");
    }
}
