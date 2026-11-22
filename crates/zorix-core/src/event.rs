//! Events: the ordered record of everything that happens to an execution.

use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::ids::ExecutionId;
use crate::time::Timestamp;

/// The version of the event format. Adding fields keeps it; changing what a field means raises it.
pub const EVENT_SCHEMA_VERSION: u32 = 1;

macro_rules! event_kinds {
    ($($variant:ident => $name:literal),+ $(,)?) => {
        /// What happened.
        #[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
        pub enum EventKind {
            $(
                #[doc = concat!("`", $name, "`")]
                #[serde(rename = $name)]
                $variant,
            )+
        }

        impl EventKind {
            /// Every kind.
            pub const ALL: &'static [EventKind] = &[$(EventKind::$variant),+];

            /// The dotted name used in storage, logs and interfaces.
            pub fn name(&self) -> &'static str {
                match self { $(EventKind::$variant => $name),+ }
            }
        }
    };
}

event_kinds! {
    Created => "execution.created",
    Queued => "execution.queued",
    Started => "execution.started",
    Progress => "execution.progress",
    Paused => "execution.paused",
    Resumed => "execution.resumed",
    Recovered => "execution.recovered",
    Completed => "execution.completed",
    Failed => "execution.failed",
    Cancelled => "execution.cancelled",
    ModelRequested => "execution.model.requested",
    ModelResponded => "execution.model.responded",
    ModelRetrying => "execution.model.retrying",
    ModelFallback => "execution.model.fallback",
    ToolRequested => "execution.tool.requested",
    ToolDenied => "execution.tool.denied",
    ToolUnattended => "execution.tool.unattended",
    ToolSuggested => "execution.tool.suggested",
    ToolStarted => "execution.tool.started",
    ToolCompleted => "execution.tool.completed",
    ToolFailed => "execution.tool.failed",
    WaitingApproval => "execution.waiting_approval",
    WaitingChildren => "execution.waiting_children",
    ChildStarted => "execution.child.started",
    ChildSettled => "execution.child.settled",
    ApprovalGranted => "execution.approval.granted",
    ApprovalDenied => "execution.approval.denied",
    ResultRejected => "execution.result.rejected",
    Checkpoint => "execution.checkpoint",
}

impl std::fmt::Display for EventKind {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.name())
    }
}

/// An event before the repository numbers it.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct NewEvent {
    /// What happened.
    pub kind: EventKind,
    /// Details. Never holds secrets.
    pub payload: Value,
    /// When.
    pub at: Timestamp,
}

impl NewEvent {
    /// An event with details.
    pub fn new(kind: EventKind, at: Timestamp, payload: Value) -> Self {
        Self { kind, payload, at }
    }
}

/// A stored event.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct ExecutionEvent {
    /// Position in the global order. Strictly increasing, so a subscriber can resume after the last one it saw.
    pub seq: u64,
    /// The execution it belongs to.
    pub execution_id: ExecutionId,
    /// When it happened.
    pub at: Timestamp,
    /// The format version.
    pub schema_version: u32,
    /// What happened.
    pub kind: EventKind,
    /// Details.
    pub payload: Value,
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashSet;

    #[test]
    fn names_are_unique_dotted_and_serialize_as_themselves() {
        let mut seen = HashSet::new();
        for kind in EventKind::ALL {
            assert!(kind.name().starts_with("execution."), "{kind}");
            assert!(seen.insert(kind.name()), "duplicate {kind}");
            assert_eq!(serde_json::to_string(kind).unwrap(), format!("\"{}\"", kind.name()));
        }
    }

    #[test]
    fn the_names_from_the_brief_exist() {
        let names: Vec<&str> = EventKind::ALL.iter().map(EventKind::name).collect();
        for wanted in [
            "execution.created",
            "execution.started",
            "execution.progress",
            "execution.model.requested",
            "execution.tool.requested",
            "execution.tool.started",
            "execution.tool.completed",
            "execution.waiting_approval",
            "execution.completed",
            "execution.failed",
            "execution.cancelled",
        ] {
            assert!(names.contains(&wanted), "{wanted}");
        }
    }
}
