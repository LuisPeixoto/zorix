//! How a failure of SQLite becomes the error a person reads.

use zorix_core::Error;

/// What SQLite says when the file cannot grow (`SQLITE_FULL`): the disk has no room, or the size limit set for the file is reached.
const FULL: &str = "database or disk is full";

/// A storage failure. A full disk is the one a person can do something about, so it is said in words that tell them what to do, and
/// the work that was stopped can continue from where it was once there is room.
pub(crate) fn storage(error: impl std::fmt::Display) -> Error {
    let text = error.to_string();
    if text.to_lowercase().contains(FULL) {
        return Error::Storage(
            "The disk is full, so ZORIX could not save. Free some space and carry on: nothing that was saved is lost, and work that was interrupted continues from where it stopped.".into(),
        );
    }
    Error::Storage(text)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_full_disk_is_told_in_words_and_other_failures_are_kept_as_they_are() {
        let full = storage("database or disk is full");
        assert!(matches!(&full, Error::Storage(text) if text.starts_with("The disk is full")), "{full:?}");
        assert!(
            matches!(storage("Database or disk is FULL (code 13)"), Error::Storage(t) if t.starts_with("The disk is full"))
        );
        assert_eq!(storage("no such table: x"), Error::Storage("no such table: x".into()));
    }
}
