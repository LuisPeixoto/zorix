//! Protocol constants and the two eras (ADR-0018): the modern revision that has no handshake, and the legacy ones that do.

use serde_json::{Value, json};

/// The modern revision ZORIX speaks.
pub const MODERN_VERSION: &str = "2026-07-28";
/// The legacy revisions ZORIX speaks, newest first.
pub const LEGACY_VERSIONS: &[&str] = &["2025-11-25", "2025-06-18", "2025-03-26", "2024-11-05"];

/// Which kind of server this is. Cached per process (stdio) or origin (HTTP).
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Era {
    /// Every request carries its own metadata; no handshake, no server requests.
    Modern,
    /// `initialize`, then requests; the server may send requests of its own.
    Legacy {
        /// The revision agreed in `initialize`.
        version: String,
    },
}

impl Era {
    /// `modern` or `legacy`, as stored.
    pub fn label(&self) -> &'static str {
        match self {
            Era::Modern => "modern",
            Era::Legacy { .. } => "legacy",
        }
    }

    /// The revision in use.
    pub fn version(&self) -> &str {
        match self {
            Era::Modern => MODERN_VERSION,
            Era::Legacy { version } => version,
        }
    }
}

/// Who we say we are.
pub fn client_info() -> Value {
    json!({"name": "zorix", "version": env!("CARGO_PKG_VERSION")})
}

/// The metadata every modern request carries. ZORIX declares no client capabilities: it answers no sampling, elicitation or
/// roots requests, so it does not claim to.
pub fn modern_meta() -> Value {
    json!({
        "io.modelcontextprotocol/protocolVersion": MODERN_VERSION,
        "io.modelcontextprotocol/clientInfo": client_info(),
        "io.modelcontextprotocol/clientCapabilities": {},
    })
}

/// `params` with the modern metadata added under `_meta`.
pub fn with_modern_meta(params: Value) -> Value {
    let mut object = match params {
        Value::Object(map) => map,
        Value::Null => serde_json::Map::new(),
        other => {
            let mut map = serde_json::Map::new();
            map.insert("value".into(), other);
            map
        }
    };
    object.insert("_meta".into(), modern_meta());
    Value::Object(object)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn modern_requests_carry_version_identity_and_no_capabilities() {
        let params = with_modern_meta(json!({"name": "echo"}));
        assert_eq!(params["name"], "echo");
        assert_eq!(params["_meta"]["io.modelcontextprotocol/protocolVersion"], MODERN_VERSION);
        assert_eq!(params["_meta"]["io.modelcontextprotocol/clientInfo"]["name"], "zorix");
        assert_eq!(
            params["_meta"]["io.modelcontextprotocol/clientCapabilities"],
            json!({}),
            "we promise nothing we do not do"
        );
        assert!(with_modern_meta(Value::Null)["_meta"].is_object());
    }
}
