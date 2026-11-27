//! The settings of the program that do not belong to one feature's own store: what the runtime does with long conversations, how long
//! it keeps what it stored, where it sends telemetry and whether it reads scanned pages. One record per tenant, read by both programs
//! (the command line and the desktop app) and by the parts that act on it while they run.
//!
//! Every setting is off, or the conservative choice, until a person turns it on: nothing is deleted, nothing leaves the computer and no
//! extra model call is made by default except the one that keeps a long conversation inside the model's window.

use std::sync::{Arc, RwLock};

use async_trait::async_trait;
use serde::{Deserialize, Serialize};

use crate::error::{Error, Result};
use crate::ids::TenantId;

/// The longest an address may be.
const MAX_ENDPOINT: usize = 2_048;
/// The fewest days a finished run is kept: a run from yesterday is still something a person looks at.
pub const MIN_RETENTION_DAYS: u32 = 1;
/// The most days: ten years, which is "keep it" for any practical purpose and keeps the arithmetic far from overflow.
pub const MAX_RETENTION_DAYS: u32 = 3_650;

/// What the runtime does when a conversation no longer fits the model's window.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct ContextSettings {
    /// Summarise the turns that no longer fit instead of dropping them. It costs one extra model call each time a span is summarised,
    /// counted in the run's usage and limits.
    pub summaries: bool,
}

impl Default for ContextSettings {
    fn default() -> Self {
        Self { summaries: true }
    }
}

/// What retention does to a run that is old enough.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RetentionMode {
    /// Delete what is bulky (events, checkpoints, tool calls, approvals, recall snapshots) and keep the run with its request, its answer,
    /// its usage and its cost, so lists, spend figures and budgets stay the same.
    #[default]
    Compact,
    /// Delete the run altogether. Refused for a run that a task, a workflow run or another run still refers to.
    Delete,
}

/// How long finished runs are kept.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct RetentionSettings {
    /// Finished runs older than this many days are compacted or deleted. `None` keeps everything for ever.
    pub days: Option<u32>,
    /// What happens to them.
    pub mode: RetentionMode,
}

/// What retention did, or would do, to the runs that are old enough. Counts of rows, and the room they take.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct RetentionReport {
    /// Runs that were compacted or deleted.
    pub runs: u64,
    /// Of them, compacted (the run stays, without its events, steps, approvals and recall).
    pub compacted: u64,
    /// Of them, deleted altogether.
    pub deleted: u64,
    /// Runs that `Delete` would have removed but something refers to (a task, a workflow run, a report): they were compacted.
    pub kept_because_referenced: u64,
    /// Events removed.
    pub events: u64,
    /// Checkpoints removed.
    pub checkpoints: u64,
    /// Tool call records removed.
    pub tool_calls: u64,
    /// Approvals removed.
    pub approvals: u64,
    /// Recall snapshots removed.
    pub snapshots: u64,
    /// About how many bytes those rows take.
    pub bytes: u64,
}

/// Where traces go.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct TelemetrySettings {
    /// The address of an OpenTelemetry collector (OTLP over HTTP), such as `https://collector.example.com:4318`. `None` sends nothing.
    pub endpoint: Option<String>,
    /// The name of a header the collector needs, such as `authorization`. Its value is a secret and lives in the system keychain, never here.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub header: Option<String>,
}

/// What reading documents does.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct KnowledgeSettings {
    /// Read the text of scanned PDFs on this computer. Needs two small models, which are downloaded when this is turned on.
    pub ocr: bool,
}

/// All of them.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct AppSettings {
    /// Long conversations.
    pub context: ContextSettings,
    /// How long runs are kept.
    pub retention: RetentionSettings,
    /// Where traces go.
    pub telemetry: TelemetrySettings,
    /// Reading documents.
    pub knowledge: KnowledgeSettings,
}

impl AppSettings {
    /// Refuse values that make no sense, with a sentence that says what to change.
    pub fn validate(&self) -> Result<()> {
        let bad = |text: String| Err(Error::InvalidInput(text));
        if let Some(days) = self.retention.days
            && !(MIN_RETENTION_DAYS..=MAX_RETENTION_DAYS).contains(&days)
        {
            return bad(format!(
                "Keep runs for between {MIN_RETENTION_DAYS} and {MAX_RETENTION_DAYS} days, or leave the number empty to keep them for ever."
            ));
        }
        if let Some(endpoint) = &self.telemetry.endpoint {
            let endpoint = endpoint.as_str();
            if endpoint.len() > MAX_ENDPOINT || endpoint.chars().any(|c| c.is_control() || c.is_whitespace()) {
                return bad("The telemetry address must be one line of at most 2048 characters, with no spaces.".into());
            }
            if !(endpoint.starts_with("https://") || endpoint.starts_with("http://")) {
                return bad(
                    "The telemetry address must start with https:// (or http:// for a collector on this computer)."
                        .into(),
                );
            }
        }
        if let Some(name) = &self.telemetry.header {
            let token = |c: char| c.is_ascii_alphanumeric() || "!#$%&'*+-.^_`|~".contains(c);
            if name.is_empty() || name.len() > 64 || !name.chars().all(token) {
                return bad("A header name has letters, digits and - only, and at most 64 characters.".into());
            }
        }
        Ok(())
    }
}

/// Where the settings are kept.
#[async_trait]
pub trait SettingsStore: Send + Sync {
    /// The tenant's settings; the defaults when none were saved.
    async fn settings(&self, tenant: &TenantId) -> Result<AppSettings>;
    /// Save them. Refused when [`AppSettings::validate`] refuses.
    async fn put_settings(&self, tenant: &TenantId, settings: &AppSettings) -> Result<()>;
}

/// The current settings, readable without waiting, shared by everything that acts on them while the program runs. The program that
/// saves a change puts it here too, so the change takes effect at once and without a restart.
#[derive(Clone, Debug, Default)]
pub struct SettingsCell(Arc<RwLock<AppSettings>>);

impl SettingsCell {
    /// A cell holding `settings`.
    pub fn new(settings: AppSettings) -> Self {
        Self(Arc::new(RwLock::new(settings)))
    }

    /// The settings now.
    pub fn get(&self) -> AppSettings {
        self.0.read().unwrap_or_else(|poisoned| poisoned.into_inner()).clone()
    }

    /// Replace them.
    pub fn set(&self, settings: AppSettings) {
        *self.0.write().unwrap_or_else(|poisoned| poisoned.into_inner()) = settings;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_defaults_keep_everything_and_send_nothing_and_a_missing_field_takes_its_default() {
        let defaults = AppSettings::default();
        assert!(defaults.context.summaries, "the one thing that is on keeps a long conversation inside the window");
        assert_eq!(defaults.retention, RetentionSettings { days: None, mode: RetentionMode::Compact });
        assert_eq!(defaults.telemetry.endpoint, None);
        assert!(!defaults.knowledge.ocr);
        // An older record, written before a setting existed, reads with that setting's default.
        let older: AppSettings = serde_json::from_str(r#"{"retention": {"days": 90}}"#).unwrap();
        assert_eq!(older.retention.days, Some(90));
        assert!(older.context.summaries && older.telemetry.endpoint.is_none());
        assert!(AppSettings::default().validate().is_ok());
    }

    #[test]
    fn retention_days_and_the_telemetry_address_are_checked() {
        let with = |days: Option<u32>, endpoint: Option<&str>| AppSettings {
            retention: RetentionSettings { days, mode: RetentionMode::Compact },
            telemetry: TelemetrySettings { endpoint: endpoint.map(str::to_string), header: None },
            ..AppSettings::default()
        };
        assert!(with(Some(1), None).validate().is_ok());
        assert!(with(Some(3_650), None).validate().is_ok());
        assert!(with(Some(0), None).validate().is_err(), "zero days would delete everything at once");
        assert!(with(Some(3_651), None).validate().is_err());
        for good in ["https://otel.example.com:4318", "http://127.0.0.1:4318", "https://otel.example.com/v1/traces"] {
            assert!(with(None, Some(good)).validate().is_ok(), "{good}");
        }
        for bad in [
            "otel.example.com",
            "ftp://otel.example.com",
            "https://otel .example.com",
            "https://otel.example.com\n",
            "",
            &format!("https://{}", "a".repeat(2_100)),
        ] {
            assert!(with(None, Some(bad)).validate().is_err(), "{bad:?}");
        }
    }

    #[test]
    fn the_name_of_a_collector_header_is_a_token() {
        let with = |name: &str| AppSettings {
            telemetry: TelemetrySettings { endpoint: None, header: Some(name.to_string()) },
            ..AppSettings::default()
        };
        assert!(with("authorization").validate().is_ok());
        assert!(with("x-honeycomb-team").validate().is_ok());
        for bad in ["", "two words", "a:b", "line\nbreak", &"h".repeat(65)] {
            assert!(with(bad).validate().is_err(), "{bad:?}");
        }
    }

    #[test]
    fn a_cell_shows_a_change_to_everyone_who_holds_it() {
        let cell = SettingsCell::default();
        let other = cell.clone();
        assert!(!other.get().knowledge.ocr);
        cell.set(AppSettings { knowledge: KnowledgeSettings { ocr: true }, ..AppSettings::default() });
        assert!(other.get().knowledge.ocr);
    }
}
