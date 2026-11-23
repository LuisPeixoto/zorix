//! The app's own small settings, kept next to the data as `settings.json`. Nothing secret lives here.

use crate::ui_error::{UiError, ui};
use std::path::Path;

use serde::{Deserialize, Serialize};

/// What a person chose in Settings.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct Settings {
    /// Whether workflows receive webhooks. Off until a person turns it on.
    pub webhooks: bool,
    /// The address webhooks arrive on. This computer only, unless a person writes another.
    pub webhook_address: String,
}

impl Default for Settings {
    fn default() -> Self {
        Self { webhooks: false, webhook_address: zorix_workflows::DEFAULT_WEBHOOK_ADDRESS.into() }
    }
}

impl Settings {
    /// Read the settings; a missing or damaged file gives the defaults (the next save repairs it).
    pub fn load(home: &Path) -> Self {
        std::fs::read_to_string(home.join("settings.json"))
            .ok()
            .and_then(|t| serde_json::from_str(&t).ok())
            .unwrap_or_default()
    }

    /// Write the settings.
    pub fn save(&self, home: &Path) -> Result<(), UiError> {
        let text = serde_json::to_string_pretty(self).map_err(ui)?;
        std::fs::write(home.join("settings.json"), text)
            .map_err(|e| UiError::message(format!("The settings could not be saved: {e}")))
    }

    /// The address to listen on, when webhooks are on and the address is valid.
    pub fn listen(&self) -> Result<Option<std::net::SocketAddr>, UiError> {
        if !self.webhooks {
            return Ok(None);
        }
        zorix_workflows::parse_address(&self.webhook_address).map(Some).map_err(ui)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn webhooks_are_off_by_default_and_listen_on_this_computer() {
        let defaults = Settings::default();
        assert!(!defaults.webhooks);
        assert_eq!(defaults.listen().unwrap(), None);
        let on = Settings { webhooks: true, ..Settings::default() };
        assert!(on.listen().unwrap().unwrap().ip().is_loopback());
    }

    #[test]
    fn a_missing_or_damaged_file_gives_the_defaults() {
        let dir = tempfile::tempdir().unwrap();
        assert_eq!(Settings::load(dir.path()), Settings::default());
        std::fs::write(dir.path().join("settings.json"), "not json").unwrap();
        assert_eq!(Settings::load(dir.path()), Settings::default());
        let chosen = Settings { webhooks: true, webhook_address: "127.0.0.1:9000".into() };
        chosen.save(dir.path()).unwrap();
        assert_eq!(Settings::load(dir.path()), chosen);
    }

    #[test]
    fn a_bad_address_is_explained() {
        let bad = Settings { webhooks: true, webhook_address: "localhost".into() };
        assert!(bad.listen().unwrap_err().message.contains("not an address"));
    }
}
