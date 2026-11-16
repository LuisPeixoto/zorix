//! Identifiers.
//!
//! Executions and approvals use UUIDv7, so ids sort by creation time. Tenants and conversations are chosen by the
//! caller and validated, because they end up in file names, logs and queries.

use std::fmt;
use std::str::FromStr;

use serde::{Deserialize, Serialize};
use uuid::Uuid;

/// A value that is not a valid identifier.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum IdError {
    /// The text is not a UUID.
    #[error("“{value}” is not a valid {kind}.")]
    NotAUuid {
        /// Which identifier was expected.
        kind: &'static str,
        /// The text that was given.
        value: String,
    },
    /// The text is empty, too long or has characters outside `a-z A-Z 0-9 - _ . :`.
    #[error("“{value}” is not a valid {kind}: use 1 to 64 letters, digits, '-', '_', '.' or ':'.")]
    BadSlug {
        /// Which identifier was expected.
        kind: &'static str,
        /// The text that was given.
        value: String,
    },
}

macro_rules! uuid_id {
    ($(#[$meta:meta])* $name:ident) => {
        $(#[$meta])*
        #[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
        #[serde(transparent)]
        pub struct $name(Uuid);

        impl $name {
            /// A new identifier (UUIDv7, time ordered).
            pub fn new() -> Self {
                Self(Uuid::now_v7())
            }

            /// Wrap an existing UUID.
            pub fn from_uuid(uuid: Uuid) -> Self {
                Self(uuid)
            }

            /// The underlying UUID.
            pub fn as_uuid(&self) -> Uuid {
                self.0
            }
        }

        impl Default for $name {
            fn default() -> Self {
                Self::new()
            }
        }

        impl fmt::Display for $name {
            fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
                write!(f, "{}", self.0.hyphenated())
            }
        }

        impl FromStr for $name {
            type Err = IdError;

            fn from_str(text: &str) -> Result<Self, IdError> {
                Uuid::parse_str(text.trim())
                    .map(Self)
                    .map_err(|_| IdError::NotAUuid { kind: stringify!($name), value: text.to_string() })
            }
        }
    };
}

macro_rules! slug_id {
    ($(#[$meta:meta])* $name:ident) => {
        $(#[$meta])*
        #[derive(Clone, Debug, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
        #[serde(try_from = "String", into = "String")]
        pub struct $name(String);

        impl $name {
            /// Validate and wrap a value.
            pub fn new(value: impl Into<String>) -> Result<Self, IdError> {
                let value = value.into();
                if is_slug(&value) {
                    Ok(Self(value))
                } else {
                    Err(IdError::BadSlug { kind: stringify!($name), value })
                }
            }

            /// The value as text.
            pub fn as_str(&self) -> &str {
                &self.0
            }
        }

        impl fmt::Display for $name {
            fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
                f.write_str(&self.0)
            }
        }

        impl FromStr for $name {
            type Err = IdError;

            fn from_str(text: &str) -> Result<Self, IdError> {
                Self::new(text)
            }
        }

        impl TryFrom<String> for $name {
            type Error = IdError;

            fn try_from(value: String) -> Result<Self, IdError> {
                Self::new(value)
            }
        }

        impl From<$name> for String {
            fn from(id: $name) -> String {
                id.0
            }
        }
    };
}

fn is_slug(text: &str) -> bool {
    !text.is_empty()
        && text.len() <= 64
        && text.chars().all(|c| c.is_ascii_alphanumeric() || matches!(c, '-' | '_' | '.' | ':'))
}

uuid_id!(
    /// Identifies one execution: one run of one activity.
    ExecutionId
);
uuid_id!(
    /// Identifies one approval request.
    ApprovalId
);
uuid_id!(
    /// Identifies one long-term memory item.
    MemoryId
);
uuid_id!(
    /// Identifies one folder of indexed documents.
    KnowledgeRootId
);

slug_id!(
    /// The organisation (or the local user) that owns an execution. Data never crosses tenants.
    TenantId
);
slug_id!(
    /// Groups executions that came from one conversation. An execution is *not* a conversation.
    ConversationId
);

impl TenantId {
    /// The tenant used when a single person runs ZORIX on their own computer.
    pub fn local() -> Self {
        Self("local".to_string())
    }
}

/// Identifies one tool call inside an execution. Assigned by the runtime, never taken from the model.
#[derive(Clone, Debug, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(transparent)]
pub struct ToolCallId(pub String);

impl fmt::Display for ToolCallId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn execution_ids_are_unique_and_sort_by_creation() {
        let a = ExecutionId::new();
        std::thread::sleep(std::time::Duration::from_millis(2));
        let b = ExecutionId::new();
        assert_ne!(a, b);
        assert!(a < b);
    }

    #[test]
    fn ids_round_trip_through_text_and_json() {
        let id = ExecutionId::new();
        assert_eq!(id.to_string().parse::<ExecutionId>().unwrap(), id);
        let json = serde_json::to_string(&id).unwrap();
        assert_eq!(serde_json::from_str::<ExecutionId>(&json).unwrap(), id);
    }

    #[test]
    fn a_bad_uuid_is_reported_in_plain_words() {
        let error = "nope".parse::<ExecutionId>().unwrap_err();
        assert_eq!(error.to_string(), "“nope” is not a valid ExecutionId.");
    }

    #[test]
    fn tenants_accept_simple_names_only() {
        assert!(TenantId::new("acme-corp").is_ok());
        assert!(TenantId::new("").is_err());
        assert!(TenantId::new("a/b").is_err());
        assert!(TenantId::new("..").is_ok(), "dots are allowed, but never used as paths");
        assert!(TenantId::new("x".repeat(65)).is_err());
        assert!(serde_json::from_str::<TenantId>("\"bad name\"").is_err());
    }
}
