//! Where ZORIX keeps its data, and the lock that keeps two programs from using it at once.

use std::fs::{File, OpenOptions};
use std::io;
use std::path::{Path, PathBuf};

/// The environment variable that moves the data folder. Tests and demos set it so they never touch real data.
pub const HOME_VARIABLE: &str = "ZORIX_HOME";

/// The data folder: `$ZORIX_HOME` when set, otherwise the platform's application data folder plus `zorix`.
pub fn data_dir() -> PathBuf {
    match std::env::var_os(HOME_VARIABLE) {
        Some(path) if !path.is_empty() => PathBuf::from(path),
        _ => dirs::data_dir().unwrap_or_else(std::env::temp_dir).join("zorix"),
    }
}

/// The database file inside a data folder.
pub fn database_path(dir: &Path) -> PathBuf {
    dir.join("zorix.sqlite")
}

/// Why the data folder could not be locked.
#[derive(Debug, thiserror::Error)]
pub enum LockError {
    /// Another ZORIX program holds it.
    #[error("Another ZORIX program is using the data folder {0}. Close it and try again.")]
    InUse(PathBuf),
    /// The folder or the lock file could not be used.
    #[error("Could not use the data folder {path}: {source}")]
    Io {
        /// The folder.
        path: PathBuf,
        /// What went wrong.
        source: io::Error,
    },
}

/// An exclusive claim on a data folder, held for as long as the value lives. It also tells the engine it is the only
/// one working on the store, so leases left by a crashed program can be dropped (`EngineConfig::sole_owner`).
#[derive(Debug)]
pub struct DataDirLock {
    _file: File,
    dir: PathBuf,
}

impl DataDirLock {
    /// Create the folder if needed and lock it. Fails at once, without waiting, if someone else holds the lock.
    pub fn acquire(dir: &Path) -> Result<Self, LockError> {
        let io_error = |source| LockError::Io { path: dir.to_path_buf(), source };
        std::fs::create_dir_all(dir).map_err(io_error)?;
        let file = OpenOptions::new()
            .create(true)
            .truncate(false)
            .write(true)
            .open(dir.join("zorix.lock"))
            .map_err(|source| LockError::Io { path: dir.to_path_buf(), source })?;
        match file.try_lock() {
            Ok(()) => Ok(Self { _file: file, dir: dir.to_path_buf() }),
            Err(std::fs::TryLockError::WouldBlock) => Err(LockError::InUse(dir.to_path_buf())),
            Err(std::fs::TryLockError::Error(source)) => Err(LockError::Io { path: dir.to_path_buf(), source }),
        }
    }

    /// The locked folder.
    pub fn dir(&self) -> &Path {
        &self.dir
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_home_variable_moves_the_data_folder() {
        // Setting an environment variable affects the whole process, so this is the only test that does it.
        let here = tempfile::tempdir().unwrap();
        unsafe { std::env::set_var(HOME_VARIABLE, here.path()) };
        assert_eq!(data_dir(), here.path());
        unsafe { std::env::remove_var(HOME_VARIABLE) };
        assert!(data_dir().ends_with("zorix"));
    }

    #[test]
    fn the_database_lives_inside_the_folder() {
        assert_eq!(database_path(Path::new("/x")), Path::new("/x/zorix.sqlite"));
    }

    #[test]
    fn only_one_holder_at_a_time() {
        let dir = tempfile::tempdir().unwrap();
        let first = DataDirLock::acquire(dir.path()).unwrap();
        assert_eq!(first.dir(), dir.path());
        let second = DataDirLock::acquire(dir.path()).unwrap_err();
        assert!(matches!(second, LockError::InUse(_)));
        assert!(second.to_string().contains("Another ZORIX program"));
        drop(first);
        assert!(DataDirLock::acquire(dir.path()).is_ok(), "released when dropped");
    }

    #[test]
    fn the_folder_is_created_when_missing() {
        let dir = tempfile::tempdir().unwrap();
        let nested = dir.path().join("a/b");
        let lock = DataDirLock::acquire(&nested).unwrap();
        assert!(nested.is_dir());
        drop(lock);
    }
}
