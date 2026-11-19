//! The strict subset of YAML that agent files and skills use, read into `serde_json::Value`.
//!
//! Scalars, lists and maps, quoted and plain strings, folded and literal blocks. **Refused**: anchors, aliases, tags, more than
//! one document, nesting deeper than [`MAX_DEPTH`], and text larger than [`MAX_BYTES`]. These are the features that make YAML
//! dangerous to read from a file somebody else wrote (expansion attacks, type confusion) and that no agent or skill needs.

use serde_json::{Map, Number, Value};
use yaml_rust2::parser::{Event, EventReceiver, Parser};
use yaml_rust2::scanner::TScalarStyle;

/// The most bytes of YAML read.
pub const MAX_BYTES: usize = 16 * 1024;
/// The deepest nesting of lists and maps (the top-level map is depth 1).
pub const MAX_DEPTH: usize = 4;

enum Frame {
    List(Vec<Value>),
    Map { entries: Map<String, Value>, key: Option<String> },
}

struct Builder {
    stack: Vec<Frame>,
    done: Option<Value>,
    documents: usize,
    error: Option<String>,
}

impl Builder {
    fn fail(&mut self, message: impl Into<String>) {
        if self.error.is_none() {
            self.error = Some(message.into());
        }
    }

    fn put(&mut self, value: Value) {
        let mut problem: Option<String> = None;
        match self.stack.last_mut() {
            None => self.done = Some(value),
            Some(Frame::List(items)) => items.push(value),
            Some(Frame::Map { entries, key }) => match key.take() {
                Some(k) => {
                    if entries.contains_key(&k) {
                        problem = Some(format!("The key “{k}” appears twice."));
                    }
                    entries.insert(k, value);
                }
                None => match value {
                    Value::String(k) => *key = Some(k),
                    _ => problem = Some("A key must be plain text.".into()),
                },
            },
        }
        if let Some(problem) = problem {
            self.fail(problem);
        }
    }

    /// Whether the next scalar is a key (its map is waiting for one).
    fn expects_key(&self) -> bool {
        matches!(self.stack.last(), Some(Frame::Map { key: None, .. }))
    }
}

fn plain(text: &str) -> Value {
    match text {
        "" | "~" | "null" | "Null" | "NULL" => Value::Null,
        "true" | "True" | "TRUE" => Value::Bool(true),
        "false" | "False" | "FALSE" => Value::Bool(false),
        other => {
            if let Ok(n) = other.parse::<i64>() {
                return Value::Number(n.into());
            }
            if other.contains(['.', 'e', 'E'])
                && other.chars().all(|c| c.is_ascii_digit() || matches!(c, '.' | 'e' | 'E' | '+' | '-'))
                && let Ok(f) = other.parse::<f64>()
                && let Some(n) = Number::from_f64(f)
            {
                return Value::Number(n);
            }
            Value::String(other.to_string())
        }
    }
}

impl EventReceiver for Builder {
    fn on_event(&mut self, event: Event) {
        if self.error.is_some() {
            return;
        }
        match event {
            Event::Nothing | Event::StreamStart | Event::StreamEnd | Event::DocumentEnd => {}
            Event::DocumentStart => {
                self.documents += 1;
                if self.documents > 1 {
                    self.fail("Only one YAML document is allowed.");
                }
            }
            Event::Alias(_) => self.fail("YAML aliases are not allowed."),
            Event::Scalar(text, style, anchor, tag) => {
                if anchor != 0 {
                    return self.fail("YAML anchors are not allowed.");
                }
                if tag.is_some() {
                    return self.fail("YAML tags are not allowed.");
                }
                // A key is always text, whatever it looks like (`true: x`, `1: x`).
                let value = if style == TScalarStyle::Plain && !self.expects_key() {
                    plain(&text)
                } else {
                    Value::String(text)
                };
                self.put(value);
            }
            Event::SequenceStart(anchor, tag) | Event::MappingStart(anchor, tag) if anchor != 0 || tag.is_some() => {
                self.fail("YAML anchors and tags are not allowed.");
            }
            Event::SequenceStart(..) => {
                if self.stack.len() >= MAX_DEPTH {
                    return self.fail(format!("The YAML nests deeper than {MAX_DEPTH} levels."));
                }
                self.stack.push(Frame::List(Vec::new()));
            }
            Event::MappingStart(..) => {
                if self.stack.len() >= MAX_DEPTH {
                    return self.fail(format!("The YAML nests deeper than {MAX_DEPTH} levels."));
                }
                self.stack.push(Frame::Map { entries: Map::new(), key: None });
            }
            Event::SequenceEnd => {
                if let Some(Frame::List(items)) = self.stack.pop() {
                    self.put(Value::Array(items));
                }
            }
            Event::MappingEnd => {
                if let Some(Frame::Map { entries, .. }) = self.stack.pop() {
                    self.put(Value::Object(entries));
                }
            }
        }
    }
}

/// Read a YAML document into a value, or say why it is not acceptable.
pub fn parse(text: &str) -> Result<Value, String> {
    if text.len() > MAX_BYTES {
        return Err(format!("The YAML is larger than {} KB.", MAX_BYTES / 1024));
    }
    let mut builder = Builder { stack: Vec::new(), done: None, documents: 0, error: None };
    let mut parser = Parser::new_from_str(text);
    parser.load(&mut builder, true).map_err(|e| format!("The YAML could not be read: {e}"))?;
    if let Some(error) = builder.error {
        return Err(error);
    }
    Ok(builder.done.unwrap_or(Value::Null))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn maps_lists_and_scalars_read_into_values() {
        let v =
            parse("name: demo\ncount: 3\nratio: 0.5\non: true\nnothing:\nlist:\n  - a\n  - \"b c\"\nnested:\n  k: v\n")
                .unwrap();
        assert_eq!(v["name"], "demo");
        assert_eq!(v["count"], 3);
        assert_eq!(v["ratio"], 0.5);
        assert_eq!(v["on"], true);
        assert!(v["nothing"].is_null());
        assert_eq!(v["list"], serde_json::json!(["a", "b c"]));
        assert_eq!(v["nested"]["k"], "v");
    }

    #[test]
    fn quoted_values_stay_text_and_keys_are_always_text() {
        let v = parse("a: \"123\"\nb: 'true'\n1: one\ntrue: yes-text\n").unwrap();
        assert_eq!(v["a"], "123");
        assert_eq!(v["b"], "true");
        assert_eq!(v["1"], "one");
        assert_eq!(v["true"], "yes-text");
    }

    #[test]
    fn folded_and_literal_blocks_are_read() {
        let v = parse("folded: >\n  one\n  two\nliteral: |\n  one\n  two\n").unwrap();
        assert_eq!(v["folded"], "one two\n");
        assert_eq!(v["literal"], "one\ntwo\n");
    }

    #[test]
    fn what_makes_yaml_dangerous_is_refused() {
        for (text, wants) in [
            ("a: &x 1\nb: *x\n", "anchors"),
            ("a: !!str 1\n", "tags"),
            ("---\na: 1\n---\nb: 2\n", "one YAML document"),
            ("a:\n  b:\n    c:\n      d:\n        e: 1\n", "deeper"),
            ("a: 1\na: 2\n", "twice"),
        ] {
            let error = parse(text).unwrap_err();
            assert!(error.contains(wants), "{text:?} -> {error}");
        }
        assert!(parse("a: *nowhere\n").is_err(), "an alias to nothing is not read either");
        assert!(parse(&format!("a: {}\n", "x".repeat(MAX_BYTES))).unwrap_err().contains("larger"));
        assert!(parse("a: [unclosed\n").is_err());
    }

    #[test]
    fn a_billion_laughs_document_does_not_expand() {
        let bomb = "a: &a [x, x, x, x]\nb: &b [*a, *a, *a, *a]\nc: &c [*b, *b, *b, *b]\n";
        assert!(parse(bomb).is_err());
    }
}
