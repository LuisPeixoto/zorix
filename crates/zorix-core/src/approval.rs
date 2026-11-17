//! Human approval of a tool call.
//!
//! An approval is bound to one call: the id the runtime issued, and a hash of the tool and its exact arguments.

use serde::{Deserialize, Serialize};
use serde_json::Value;
use sha2::{Digest, Sha256};

use crate::ids::{ApprovalId, ExecutionId, ToolCallId};
use crate::time::Timestamp;

/// Where an approval stands.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ApprovalStatus {
    /// Nobody decided yet.
    Pending,
    /// A person said yes.
    Approved,
    /// A person said no.
    Denied,
}

/// A person's answer.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct ApprovalDecision {
    /// True to allow the call, false to refuse it.
    pub approve: bool,
    /// Who decided (a user name, a client id). Recorded in the audit trail.
    pub decided_by: String,
    /// An optional reason, shown to the model when the call is refused.
    #[serde(default)]
    pub note: Option<String>,
    /// The hash the person was shown. When given, it must match the pending call, so an answer cannot be applied to a
    /// different call than the one on screen.
    #[serde(default)]
    pub call_hash: Option<String>,
    /// True when no person looked at this: an automatic mode answered. A tool that must be approved by a person
    /// (`ToolSpec::approval_by_person_only`) refuses an automatic approval.
    #[serde(default)]
    pub automatic: bool,
}

/// A request for a person's decision.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Approval {
    /// Issued by the runtime.
    pub id: ApprovalId,
    /// The execution waiting.
    pub execution_id: ExecutionId,
    /// The call waiting.
    pub call_id: ToolCallId,
    /// The tool.
    pub tool: String,
    /// The exact arguments, shown to the person.
    pub arguments: Value,
    /// `call_hash(tool, arguments)`.
    pub call_hash: String,
    /// Why approval is needed.
    pub reason: String,
    /// Where it stands.
    pub status: ApprovalStatus,
    /// When it was asked.
    pub requested_at: Timestamp,
    /// When it was answered.
    pub decided_at: Option<Timestamp>,
    /// Who answered.
    pub decided_by: Option<String>,
    /// The answer's note.
    pub note: Option<String>,
}

/// The hash that identifies a call: SHA-256 of the tool id and the arguments in canonical JSON (sorted keys, no spaces).
pub fn call_hash(tool: &str, arguments: &Value) -> String {
    let mut text = String::new();
    text.push_str(tool);
    text.push('\n');
    canonical_json(arguments, &mut text);
    let digest = Sha256::digest(text.as_bytes());
    digest.iter().map(|byte| format!("{byte:02x}")).collect()
}

fn canonical_json(value: &Value, out: &mut String) {
    match value {
        Value::Object(map) => {
            let mut keys: Vec<&String> = map.keys().collect();
            keys.sort();
            out.push('{');
            for (index, key) in keys.into_iter().enumerate() {
                if index > 0 {
                    out.push(',');
                }
                out.push_str(&Value::String(key.clone()).to_string());
                out.push(':');
                canonical_json(&map[key], out);
            }
            out.push('}');
        }
        Value::Array(items) => {
            out.push('[');
            for (index, item) in items.iter().enumerate() {
                if index > 0 {
                    out.push(',');
                }
                canonical_json(item, out);
            }
            out.push(']');
        }
        other => out.push_str(&other.to_string()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn the_hash_ignores_key_order_and_spacing() {
        let a = call_hash("t", &json!({"a": 1, "b": {"x": [1, 2], "y": "z"}}));
        let b = call_hash("t", &serde_json::from_str(r#"{ "b": {"y":"z", "x":[1,2]}, "a":1 }"#).unwrap());
        assert_eq!(a, b);
        assert_eq!(a.len(), 64);
    }

    #[test]
    fn the_hash_changes_with_the_tool_or_any_argument() {
        let base = call_hash("t", &json!({"path": "a.txt"}));
        assert_ne!(base, call_hash("u", &json!({"path": "a.txt"})));
        assert_ne!(base, call_hash("t", &json!({"path": "b.txt"})));
        assert_ne!(base, call_hash("t", &json!({"path": "a.txt", "extra": null})));
    }

    #[test]
    fn the_tool_name_cannot_be_confused_with_the_arguments() {
        assert_ne!(call_hash("ab", &json!(1)), call_hash("a", &json!("b1")));
    }
}
