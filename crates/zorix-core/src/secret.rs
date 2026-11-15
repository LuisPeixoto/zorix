//! A value that never shows itself.

use std::fmt;

use serde::{Serialize, Serializer};

/// What replaces a secret wherever one would otherwise be shown.
pub const REDACTED: &str = "[redacted]";

/// A secret (an API key, a token): not in `Debug`, not in `Display`, not when serialised.
#[derive(Clone, PartialEq, Eq)]
pub struct Secret(String);

impl Secret {
    /// Wrap a secret.
    pub fn new(value: impl Into<String>) -> Self {
        Self(value.into())
    }

    /// The secret itself. Call this only at the moment of use.
    pub fn expose(&self) -> &str {
        &self.0
    }

    /// True when there is nothing in it.
    pub fn is_empty(&self) -> bool {
        self.0.is_empty()
    }
}

impl fmt::Debug for Secret {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("Secret(***)")
    }
}

impl fmt::Display for Secret {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("***")
    }
}

impl Serialize for Secret {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        serializer.serialize_str(REDACTED)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_secret_never_shows_itself() {
        let secret = Secret::new("hunter2");
        assert_eq!(format!("{secret:?}"), "Secret(***)");
        assert_eq!(format!("{secret}"), "***");
        assert_eq!(serde_json::to_string(&secret).unwrap(), "\"[redacted]\"");
        assert_eq!(secret.expose(), "hunter2");
        assert!(!secret.is_empty());
    }
}
