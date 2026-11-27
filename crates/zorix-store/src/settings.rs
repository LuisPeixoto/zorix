//! The SQLite implementation of [`SettingsStore`]: one record per tenant, written whole. It opens its own connection like the other stores.

use std::path::Path;
use std::sync::Arc;

use crate::errors::storage;
use async_trait::async_trait;
use parking_lot::Mutex;
use rusqlite::{Connection as Db, OptionalExtension, params};
use zorix_core::{AppSettings, Error, Result, SettingsStore, TenantId};

/// Stores the settings in the same SQLite file as everything else.
#[derive(Clone)]
pub struct SqliteSettingsStore {
    conn: Arc<Mutex<Db>>,
}

impl SqliteSettingsStore {
    /// Open the database at `path` (creating and migrating it when needed) with its own connection.
    pub fn open(path: &Path) -> Result<Self> {
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent)
                .map_err(|e| Error::Storage(format!("could not create {}: {e}", parent.display())))?;
        }
        Self::prepare(Db::open(path).map_err(storage)?, true)
    }

    /// A database in memory, for tests.
    pub fn open_in_memory() -> Result<Self> {
        Self::prepare(Db::open_in_memory().map_err(storage)?, false)
    }

    fn prepare(mut conn: Db, file: bool) -> Result<Self> {
        conn.busy_timeout(std::time::Duration::from_secs(5)).map_err(storage)?;
        if file {
            conn.pragma_update(None, "journal_mode", "WAL").map_err(storage)?;
            conn.pragma_update(None, "synchronous", "NORMAL").map_err(storage)?;
        }
        crate::schema::migrate(&mut conn).map_err(Error::Storage)?;
        Ok(Self { conn: Arc::new(Mutex::new(conn)) })
    }
}

#[async_trait]
impl SettingsStore for SqliteSettingsStore {
    async fn settings(&self, tenant: &TenantId) -> Result<AppSettings> {
        let doc: Option<String> = self
            .conn
            .lock()
            .query_row("SELECT doc FROM app_settings WHERE tenant = ?1", params![tenant.as_str()], |r| r.get(0))
            .optional()
            .map_err(storage)?;
        match doc {
            None => Ok(AppSettings::default()),
            Some(doc) => serde_json::from_str(&doc)
                .map_err(|e| Error::Storage(format!("the saved settings could not be read: {e}"))),
        }
    }

    async fn put_settings(&self, tenant: &TenantId, settings: &AppSettings) -> Result<()> {
        settings.validate()?;
        let doc = serde_json::to_string(settings).map_err(storage)?;
        self.conn
            .lock()
            .execute(
                "INSERT INTO app_settings (tenant, doc) VALUES (?1, ?2)
                 ON CONFLICT (tenant) DO UPDATE SET doc = excluded.doc",
                params![tenant.as_str(), doc],
            )
            .map_err(storage)?;
        Ok(())
    }
}
