//! Where API keys live (ADR-0011): the operating system's keychain, or environment variables.
//!
//! A key is read at the moment of use into a [`Secret`], which never shows itself. Nothing here writes a key to a file or a
//! database.

use std::collections::HashMap;
use std::sync::Arc;

use parking_lot::Mutex;
use zorix_core::Secret;

/// Why a key could not be read or written.
#[derive(Clone, Debug, PartialEq, Eq, thiserror::Error)]
pub enum SecretError {
    /// The store cannot be used here (no keychain service, or the platform is not supported).
    #[error("{0}")]
    Unavailable(String),
    /// The store refused or failed.
    #[error("{0}")]
    Failed(String),
    /// The name or the value is not acceptable.
    #[error("{0}")]
    Invalid(String),
}

/// The longest key accepted, in bytes.
pub const MAX_SECRET_BYTES: usize = 4096;

/// Check the name of a key: 1 to 64 lowercase letters, digits, `-`, `_` or `.`.
pub fn validate_name(name: &str) -> Result<(), SecretError> {
    let ok = !name.is_empty()
        && name.len() <= 64
        && name.chars().all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || matches!(c, '-' | '_' | '.'));
    if ok {
        Ok(())
    } else {
        Err(SecretError::Invalid(format!(
            "“{name}” is not a valid key name: use 1 to 64 lowercase letters, digits, '-', '_' or '.'."
        )))
    }
}

/// Check a key's value: not empty, no whitespace or control characters, not enormous.
pub fn validate_value(value: &str) -> Result<(), SecretError> {
    if value.is_empty() {
        return Err(SecretError::Invalid("The key is empty.".into()));
    }
    if value.len() > MAX_SECRET_BYTES {
        return Err(SecretError::Invalid("The key is too long.".into()));
    }
    if value.chars().any(|c| c.is_whitespace() || c.is_control()) {
        return Err(SecretError::Invalid(
            "The key has spaces or control characters; paste only the key itself.".into(),
        ));
    }
    Ok(())
}

/// A place that keeps keys by name.
pub trait SecretStore: Send + Sync {
    /// The key called `name`, if there is one.
    fn get(&self, name: &str) -> Result<Option<Secret>, SecretError>;

    /// Store a key, replacing an earlier one.
    fn set(&self, name: &str, value: &Secret) -> Result<(), SecretError>;

    /// Delete a key. Returns whether there was one.
    fn remove(&self, name: &str) -> Result<bool, SecretError>;

    /// Where the keys are, in words, for messages ("the macOS keychain").
    fn describe(&self) -> String;
}

/// Read a key without blocking the async threads (keychain services can be slow).
pub async fn get_async(store: Arc<dyn SecretStore>, name: String) -> Result<Option<Secret>, SecretError> {
    tokio::task::spawn_blocking(move || store.get(&name))
        .await
        .map_err(|e| SecretError::Failed(format!("The key lookup failed: {e}")))?
}

/// Keys held in memory. For tests.
#[derive(Default)]
pub struct MemoryStore {
    keys: Mutex<HashMap<String, String>>,
}

impl MemoryStore {
    /// An empty store.
    pub fn new() -> Self {
        Self::default()
    }
}

impl SecretStore for MemoryStore {
    fn get(&self, name: &str) -> Result<Option<Secret>, SecretError> {
        Ok(self.keys.lock().get(name).map(|v| Secret::new(v.clone())))
    }

    fn set(&self, name: &str, value: &Secret) -> Result<(), SecretError> {
        validate_name(name)?;
        validate_value(value.expose())?;
        self.keys.lock().insert(name.to_string(), value.expose().to_string());
        Ok(())
    }

    fn remove(&self, name: &str) -> Result<bool, SecretError> {
        Ok(self.keys.lock().remove(name).is_some())
    }

    fn describe(&self) -> String {
        "memory".into()
    }
}

type Reader = Arc<dyn Fn(&str) -> Option<String> + Send + Sync>;

/// Keys from environment variables. Read-only.
///
/// A key called `name` is looked up in `ZORIX_KEY_<NAME>` (uppercase, `-` and `.` become `_`), then in the variables
/// registered for it (such as `ANTHROPIC_API_KEY` for `anthropic`).
#[derive(Clone)]
pub struct EnvStore {
    reader: Reader,
    variables: HashMap<String, Vec<String>>,
}

impl EnvStore {
    /// An environment store with the well-known variables registered.
    pub fn new() -> Self {
        Self::with_reader(Arc::new(|name| std::env::var(name).ok()))
    }

    /// An environment store that reads variables through `reader` (so tests need not touch the real environment).
    pub fn with_reader(reader: Reader) -> Self {
        let mut variables: HashMap<String, Vec<String>> = HashMap::new();
        for (name, var) in [
            ("anthropic", "ANTHROPIC_API_KEY"),
            ("openai", "OPENAI_API_KEY"),
            ("gemini", "GEMINI_API_KEY"),
            ("gemini", "GOOGLE_API_KEY"),
        ] {
            variables.entry(name.to_string()).or_default().push(var.to_string());
        }
        Self { reader, variables }
    }

    /// Also look in `variable` for the key called `name`.
    pub fn with_variable(mut self, name: &str, variable: &str) -> Self {
        self.variables.entry(name.to_string()).or_default().push(variable.to_string());
        self
    }

    fn own_variable(name: &str) -> String {
        format!("ZORIX_KEY_{}", name.to_uppercase().replace(['-', '.'], "_"))
    }
}

impl Default for EnvStore {
    fn default() -> Self {
        Self::new()
    }
}

impl SecretStore for EnvStore {
    fn get(&self, name: &str) -> Result<Option<Secret>, SecretError> {
        let mut candidates = vec![Self::own_variable(name)];
        candidates.extend(self.variables.get(name).cloned().unwrap_or_default());
        for variable in candidates {
            if let Some(value) = (self.reader)(&variable) {
                let value = value.trim().to_string();
                if !value.is_empty() {
                    return Ok(Some(Secret::new(value)));
                }
            }
        }
        Ok(None)
    }

    fn set(&self, _name: &str, _value: &Secret) -> Result<(), SecretError> {
        Err(SecretError::Failed(
            "Environment variables cannot be changed from here. Use the keychain, or set the variable yourself.".into(),
        ))
    }

    fn remove(&self, _name: &str) -> Result<bool, SecretError> {
        Ok(false)
    }

    fn describe(&self) -> String {
        "environment variables".into()
    }
}

/// Keys in the platform's credential store: the macOS Keychain, the Windows Credential Manager or the Linux Secret Service.
pub struct KeychainStore {
    service: String,
}

impl KeychainStore {
    /// The store ZORIX uses: service name `zorix`.
    pub fn new() -> Self {
        Self { service: "zorix".into() }
    }

    /// A store under another service name (tests use their own, so they never touch real keys).
    pub fn with_service(service: &str) -> Self {
        Self { service: service.into() }
    }

    fn entry(&self, name: &str) -> Result<keyring::Entry, SecretError> {
        validate_name(name)?;
        keyring::Entry::new(&self.service, name).map_err(map_error)
    }
}

impl Default for KeychainStore {
    fn default() -> Self {
        Self::new()
    }
}

fn map_error(error: keyring::Error) -> SecretError {
    match error {
        keyring::Error::NoDefaultStore | keyring::Error::NotSupportedByStore(_) => SecretError::Unavailable(
            "There is no keychain service here. Set the key with an environment variable instead (for example ANTHROPIC_API_KEY).".into(),
        ),
        keyring::Error::NoStorageAccess(_) => SecretError::Unavailable(
            "The keychain could not be opened (it may be locked). Unlock it, or use an environment variable.".into(),
        ),
        other => SecretError::Failed(format!("The keychain failed: {other}")),
    }
}

impl SecretStore for KeychainStore {
    fn get(&self, name: &str) -> Result<Option<Secret>, SecretError> {
        match self.entry(name)?.get_password() {
            Ok(value) => Ok(Some(Secret::new(value))),
            Err(keyring::Error::NoEntry) => Ok(None),
            Err(error) => Err(map_error(error)),
        }
    }

    fn set(&self, name: &str, value: &Secret) -> Result<(), SecretError> {
        validate_value(value.expose())?;
        self.entry(name)?.set_password(value.expose()).map_err(map_error)
    }

    fn remove(&self, name: &str) -> Result<bool, SecretError> {
        match self.entry(name)?.delete_credential() {
            Ok(()) => Ok(true),
            Err(keyring::Error::NoEntry) => Ok(false),
            Err(error) => Err(map_error(error)),
        }
    }

    fn describe(&self) -> String {
        if cfg!(target_os = "macos") {
            "the macOS keychain".into()
        } else if cfg!(target_os = "windows") {
            "the Windows Credential Manager".into()
        } else {
            "the system keyring (Secret Service)".into()
        }
    }
}

/// Several stores tried in order for reading; writes go to the first store that accepts them.
pub struct ChainStore {
    stores: Vec<Arc<dyn SecretStore>>,
}

impl ChainStore {
    /// A chain of stores. The first one that has a key wins.
    pub fn new(stores: Vec<Arc<dyn SecretStore>>) -> Self {
        Self { stores }
    }

    /// The default for the apps: environment variables first (so servers and CI need no keychain), then the keychain.
    pub fn standard() -> Self {
        Self::new(vec![Arc::new(EnvStore::new()), Arc::new(KeychainStore::new())])
    }
}

impl SecretStore for ChainStore {
    fn get(&self, name: &str) -> Result<Option<Secret>, SecretError> {
        let mut first_error = None;
        for store in &self.stores {
            match store.get(name) {
                Ok(Some(secret)) => return Ok(Some(secret)),
                Ok(None) => {}
                // A store that cannot be used must not hide a key that another one has.
                Err(error) => first_error = first_error.or(Some(error)),
            }
        }
        match first_error {
            Some(error) => Err(error),
            None => Ok(None),
        }
    }

    fn set(&self, name: &str, value: &Secret) -> Result<(), SecretError> {
        let mut last = SecretError::Unavailable("There is nowhere to keep the key.".into());
        for store in &self.stores {
            match store.set(name, value) {
                Ok(()) => return Ok(()),
                Err(error @ SecretError::Invalid(_)) => return Err(error),
                Err(error) => last = error,
            }
        }
        Err(last)
    }

    fn remove(&self, name: &str) -> Result<bool, SecretError> {
        let mut removed = false;
        for store in &self.stores {
            removed |= store.remove(name)?;
        }
        Ok(removed)
    }

    fn describe(&self) -> String {
        self.stores.iter().map(|s| s.describe()).collect::<Vec<_>>().join(", then ")
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn env(pairs: &[(&str, &str)]) -> EnvStore {
        let map: HashMap<String, String> = pairs.iter().map(|(k, v)| (k.to_string(), v.to_string())).collect();
        EnvStore::with_reader(Arc::new(move |name| map.get(name).cloned()))
    }

    #[test]
    fn names_and_values_are_checked() {
        for good in ["anthropic", "openai-work", "my.key_2"] {
            assert!(validate_name(good).is_ok(), "{good}");
        }
        for bad in ["", "Upper", "a b", "a/b", &"x".repeat(65)] {
            assert!(validate_name(bad).is_err(), "{bad}");
        }
        assert!(validate_value("sk-abc123").is_ok());
        for bad in ["", "a b", "a\nb", "tab\t", &"k".repeat(MAX_SECRET_BYTES + 1)] {
            assert!(validate_value(bad).is_err());
        }
    }

    #[test]
    fn the_memory_store_keeps_replaces_and_removes() {
        let store = MemoryStore::new();
        assert!(store.get("a").unwrap().is_none());
        store.set("a", &Secret::new("one")).unwrap();
        store.set("a", &Secret::new("two")).unwrap();
        assert_eq!(store.get("a").unwrap().unwrap().expose(), "two");
        assert!(store.remove("a").unwrap());
        assert!(!store.remove("a").unwrap());
        assert!(store.set("Bad Name", &Secret::new("x")).is_err());
        assert!(store.set("ok", &Secret::new("has space")).is_err());
    }

    #[test]
    fn environment_variables_are_found_by_name_or_by_the_well_known_variable() {
        let store =
            env(&[("ANTHROPIC_API_KEY", "  sk-ant  "), ("ZORIX_KEY_MY_PROXY", "p-key"), ("OPENAI_API_KEY", "")]);
        assert_eq!(store.get("anthropic").unwrap().unwrap().expose(), "sk-ant", "trimmed");
        assert_eq!(store.get("my-proxy").unwrap().unwrap().expose(), "p-key");
        assert!(store.get("openai").unwrap().is_none(), "an empty variable is no key");
        assert!(store.get("gemini").unwrap().is_none());
        let custom = env(&[("MY_VAR", "v")]).with_variable("work", "MY_VAR");
        assert_eq!(custom.get("work").unwrap().unwrap().expose(), "v");
    }

    #[test]
    fn the_own_variable_wins_over_the_well_known_one() {
        let store = env(&[("ANTHROPIC_API_KEY", "general"), ("ZORIX_KEY_ANTHROPIC", "specific")]);
        assert_eq!(store.get("anthropic").unwrap().unwrap().expose(), "specific");
    }

    #[test]
    fn the_environment_cannot_be_written() {
        let store = env(&[]);
        assert!(store.set("a", &Secret::new("x")).is_err());
        assert_eq!(store.remove("a"), Ok(false));
    }

    #[test]
    fn a_chain_reads_the_first_store_that_has_the_key_and_writes_to_the_first_that_accepts() {
        let memory = Arc::new(MemoryStore::new());
        let chain = ChainStore::new(vec![Arc::new(env(&[("ANTHROPIC_API_KEY", "from-env")])), memory.clone()]);
        assert_eq!(chain.get("anthropic").unwrap().unwrap().expose(), "from-env", "the environment wins");
        chain.set("openai", &Secret::new("saved")).unwrap();
        assert_eq!(
            memory.get("openai").unwrap().unwrap().expose(),
            "saved",
            "the environment refused, the next accepted"
        );
        assert_eq!(chain.get("openai").unwrap().unwrap().expose(), "saved");
        assert!(chain.remove("openai").unwrap());
        assert!(chain.get("openai").unwrap().is_none());
        assert!(chain.describe().contains("environment variables, then memory"));
    }

    struct Broken;

    impl SecretStore for Broken {
        fn get(&self, _: &str) -> Result<Option<Secret>, SecretError> {
            Err(SecretError::Unavailable("no keychain".into()))
        }

        fn set(&self, _: &str, _: &Secret) -> Result<(), SecretError> {
            Err(SecretError::Unavailable("no keychain".into()))
        }

        fn remove(&self, _: &str) -> Result<bool, SecretError> {
            Err(SecretError::Unavailable("no keychain".into()))
        }

        fn describe(&self) -> String {
            "broken".into()
        }
    }

    #[test]
    fn a_store_that_cannot_be_used_does_not_hide_a_key_held_by_another() {
        let chain = ChainStore::new(vec![Arc::new(Broken), Arc::new(env(&[("ANTHROPIC_API_KEY", "k")]))]);
        assert_eq!(chain.get("anthropic").unwrap().unwrap().expose(), "k");
        // With nothing found anywhere, the failure is reported rather than a misleading "no key".
        assert!(matches!(chain.get("openai"), Err(SecretError::Unavailable(_))));
    }

    #[test]
    fn an_invalid_value_is_reported_at_once_not_passed_down_the_chain() {
        let chain = ChainStore::new(vec![Arc::new(Broken), Arc::new(MemoryStore::new())]);
        assert!(matches!(chain.set("a", &Secret::new("has space")), Err(SecretError::Invalid(_))));
    }

    #[tokio::test]
    async fn a_key_can_be_read_without_blocking_the_async_threads() {
        let store: Arc<dyn SecretStore> = Arc::new(env(&[("ANTHROPIC_API_KEY", "k")]));
        assert_eq!(get_async(store, "anthropic".into()).await.unwrap().unwrap().expose(), "k");
    }

    /// Talks to the real keychain of this computer. Run it by hand: `cargo test -p zorix-providers -- --ignored keychain`.
    #[test]
    #[ignore = "uses the real operating system keychain"]
    fn the_real_keychain_stores_reads_and_deletes_a_key() {
        let store = KeychainStore::with_service("zorix-test");
        let name = format!("test-{}", std::process::id());
        store.set(&name, &Secret::new("not-a-real-key")).unwrap();
        assert_eq!(store.get(&name).unwrap().unwrap().expose(), "not-a-real-key");
        assert!(store.remove(&name).unwrap());
        assert!(store.get(&name).unwrap().is_none());
        assert!(!store.remove(&name).unwrap());
    }
}
