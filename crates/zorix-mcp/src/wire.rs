//! JSON-RPC 2.0 messages as MCP uses them: one JSON object per line on stdio, one per POST on HTTP.

use serde_json::{Value, json};

use crate::error::McpError;

/// A JSON-RPC error object.
#[derive(Clone, Debug, PartialEq)]
pub struct RpcError {
    /// The code.
    pub code: i64,
    /// The message.
    pub message: String,
    /// Extra data.
    pub data: Option<Value>,
}

impl From<RpcError> for McpError {
    fn from(e: RpcError) -> Self {
        McpError::Rpc { code: e.code, message: e.message, data: e.data }
    }
}

/// A message from the server.
#[derive(Clone, Debug, PartialEq)]
pub enum Incoming {
    /// The answer to one of our requests.
    Response {
        /// The id we sent.
        id: Value,
        /// The result or the error.
        result: Result<Value, RpcError>,
    },
    /// A request the server makes of us (legacy revisions only).
    Request {
        /// Its id, to answer with.
        id: Value,
        /// The method.
        method: String,
        /// The parameters.
        params: Value,
    },
    /// A notification.
    Notification {
        /// The method.
        method: String,
        /// The parameters.
        params: Value,
    },
}

/// Read one message.
pub fn parse(text: &str) -> Result<Incoming, McpError> {
    let value: Value =
        serde_json::from_str(text).map_err(|e| McpError::Protocol(format!("a line was not JSON ({e})")))?;
    let object = value.as_object().ok_or_else(|| McpError::Protocol("a message was not a JSON object".into()))?;
    if object.get("jsonrpc").and_then(Value::as_str) != Some("2.0") {
        return Err(McpError::Protocol("a message was not JSON-RPC 2.0".into()));
    }
    let method = object.get("method").and_then(Value::as_str);
    let id = object.get("id").cloned().filter(|id| !id.is_null());
    match (method, id) {
        (Some(method), Some(id)) => Ok(Incoming::Request {
            id,
            method: method.to_string(),
            params: object.get("params").cloned().unwrap_or(Value::Null),
        }),
        (Some(method), None) => Ok(Incoming::Notification {
            method: method.to_string(),
            params: object.get("params").cloned().unwrap_or(Value::Null),
        }),
        (None, Some(id)) => {
            if let Some(error) = object.get("error") {
                let code = error.get("code").and_then(Value::as_i64).unwrap_or(-32603);
                let message = error.get("message").and_then(Value::as_str).unwrap_or("no message").to_string();
                Ok(Incoming::Response { id, result: Err(RpcError { code, message, data: error.get("data").cloned() }) })
            } else if let Some(result) = object.get("result") {
                Ok(Incoming::Response { id, result: Ok(result.clone()) })
            } else {
                Err(McpError::Protocol("a response had neither a result nor an error".into()))
            }
        }
        (None, None) => Err(McpError::Protocol("a message had neither a method nor an id".into())),
    }
}

/// A request line.
pub fn request(id: i64, method: &str, params: &Value) -> String {
    json!({"jsonrpc": "2.0", "id": id, "method": method, "params": params}).to_string()
}

/// A notification line.
pub fn notification(method: &str, params: &Value) -> String {
    if params.is_null() {
        json!({"jsonrpc": "2.0", "method": method}).to_string()
    } else {
        json!({"jsonrpc": "2.0", "method": method, "params": params}).to_string()
    }
}

/// A success answer to a request of the server's.
pub fn result_line(id: &Value, result: &Value) -> String {
    json!({"jsonrpc": "2.0", "id": id, "result": result}).to_string()
}

/// An error answer to a request of the server's.
pub fn error_line(id: &Value, code: i64, message: &str) -> String {
    json!({"jsonrpc": "2.0", "id": id, "error": {"code": code, "message": message}}).to_string()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_four_kinds_of_message_are_told_apart() {
        assert!(matches!(
            parse(r#"{"jsonrpc":"2.0","id":3,"result":{"a":1}}"#).unwrap(),
            Incoming::Response { result: Ok(_), .. }
        ));
        let err = parse(r#"{"jsonrpc":"2.0","id":"x","error":{"code":-32022,"message":"no","data":{"supported":[]}}}"#)
            .unwrap();
        assert!(matches!(err, Incoming::Response { result: Err(RpcError { code: -32022, .. }), .. }));
        assert!(matches!(parse(r#"{"jsonrpc":"2.0","id":9,"method":"ping"}"#).unwrap(), Incoming::Request { .. }));
        assert!(matches!(
            parse(r#"{"jsonrpc":"2.0","method":"notifications/tools/list_changed"}"#).unwrap(),
            Incoming::Notification { .. }
        ));
    }

    #[test]
    fn rubbish_is_a_protocol_error_not_a_crash() {
        for bad in [
            "",
            "not json",
            "[]",
            "3",
            r#"{"id":1,"result":{}}"#,
            r#"{"jsonrpc":"2.0"}"#,
            r#"{"jsonrpc":"2.0","id":1}"#,
        ] {
            assert!(matches!(parse(bad), Err(McpError::Protocol(_))), "{bad}");
        }
    }

    #[test]
    fn lines_never_contain_a_newline() {
        let line = request(1, "tools/call", &json!({"arguments": {"text": "a\nb"}}));
        assert!(!line.contains('\n'), "stdio frames messages with newlines");
        assert_eq!(
            notification("notifications/initialized", &Value::Null),
            r#"{"jsonrpc":"2.0","method":"notifications/initialized"}"#
        );
    }
}
