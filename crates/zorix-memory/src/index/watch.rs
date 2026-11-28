//! Watching a folder of documents for changes (ADR-0048).
//!
//! The operating system tells a watcher when something under a folder changes. That says nothing about *what* changed that an index needs (the
//! sync already skips a file whose size and time are the same, and one whose hash is), so a change only wakes a sync: after things have been quiet for a
//! moment, or after a burst has gone on for a while, `on_change` is called once. The watcher never reads a file, so a link inside the folder that
//! points elsewhere cannot make it read anything; links are not followed.
//!
//! A folder some systems cannot watch (a network drive, a limit on watches) is an error with its reason, and the caller falls back to checking on a timer.

use std::path::{Component, Path};
use std::sync::mpsc;
use std::time::{Duration, Instant};

use notify::{Config, EventKind, RecommendedWatcher, RecursiveMode, Watcher};

/// How long a folder must be quiet before a sync is asked for.
pub const QUIET: Duration = Duration::from_secs(2);
/// The longest a burst of changes is allowed to keep a sync waiting.
pub const AT_MOST: Duration = Duration::from_secs(30);

/// A folder being watched. Dropping it stops the watch.
pub struct FolderWatch {
    _watcher: RecommendedWatcher,
}

/// True when an event is something an index cares about: a file or folder made, changed, renamed or removed, below the folder, not in a hidden place.
fn matters(root: &Path, event: &notify::Event) -> bool {
    if matches!(event.kind, EventKind::Access(_) | EventKind::Other) {
        return false;
    }
    event.paths.iter().any(|path| {
        // Some systems say the folder itself was modified whenever something inside it was made or removed (even a hidden file): that says
        // nothing. The folder going away does.
        if path == root {
            return matches!(event.kind, EventKind::Remove(_));
        }
        let below = path.strip_prefix(root).unwrap_or(path);
        // Hidden files and folders (`.git`, `.DS_Store`) are not indexed, so a change in one asks for nothing.
        !below.components().any(|c| matches!(c, Component::Normal(name) if name.to_string_lossy().starts_with('.')))
    })
}

/// Start watching `folder`. `on_change` is called from a thread of its own, once after changes have been quiet for `quiet` (or `at_most` after the first of
/// a burst that never goes quiet), and must not block for long. Fails, with a sentence that says why, when the system cannot watch the folder.
pub fn watch(
    folder: &Path,
    quiet: Duration,
    at_most: Duration,
    on_change: impl Fn() + Send + 'static,
) -> Result<FolderWatch, String> {
    let (sender, receiver) = mpsc::channel::<()>();
    let root = folder.to_path_buf();
    let seen = root.clone();
    let mut watcher = RecommendedWatcher::new(
        move |event: notify::Result<notify::Event>| {
            // An error from the system (a dropped event, a folder that went away) is also a reason to look again.
            if event.as_ref().map_or(true, |event| matters(&seen, event)) {
                let _ = sender.send(());
            }
        },
        Config::default().with_follow_symlinks(false),
    )
    .map_err(|e| format!("The system could not start watching: {e}"))?;
    watcher
        .watch(folder, RecursiveMode::Recursive)
        .map_err(|e| format!("The system cannot watch {} ({e}).", folder.display()))?;
    std::thread::Builder::new()
        .name("zorix-folder-watch".into())
        .spawn(move || {
            // Wait for the first change, then for quiet; stop when the watcher (the sender) is gone.
            while receiver.recv().is_ok() {
                let began = Instant::now();
                loop {
                    let left = at_most.saturating_sub(began.elapsed()).min(quiet);
                    match receiver.recv_timeout(left) {
                        Ok(()) if began.elapsed() < at_most => continue,
                        Ok(()) | Err(mpsc::RecvTimeoutError::Timeout) => break,
                        Err(mpsc::RecvTimeoutError::Disconnected) => return,
                    }
                }
                on_change();
            }
        })
        .map_err(|e| format!("Could not start the watch: {e}"))?;
    Ok(FolderWatch { _watcher: watcher })
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;
    use std::sync::atomic::{AtomicUsize, Ordering};

    use super::*;

    fn event(kind: EventKind, paths: &[&str]) -> notify::Event {
        let mut event = notify::Event::new(kind);
        for path in paths {
            event = event.add_path(path.into());
        }
        event
    }

    #[test]
    fn a_change_to_a_visible_file_matters_and_a_read_or_a_hidden_place_does_not() {
        let root = Path::new("/docs");
        let created = EventKind::Create(notify::event::CreateKind::File);
        assert!(matters(root, &event(created, &["/docs/a.md"])));
        assert!(matters(root, &event(EventKind::Remove(notify::event::RemoveKind::Folder), &["/docs/sub"])));
        assert!(matters(root, &event(EventKind::Modify(notify::event::ModifyKind::Any), &["/docs/sub/deep/b.txt"])));
        assert!(
            !matters(root, &event(EventKind::Access(notify::event::AccessKind::Any), &["/docs/a.md"])),
            "a read changes nothing"
        );
        assert!(!matters(root, &event(EventKind::Modify(notify::event::ModifyKind::Any), &["/docs/.git/index"])));
        assert!(!matters(root, &event(created, &["/docs/sub/.DS_Store"])));
        // A rename from a hidden name to a visible one is a change to a visible file.
        assert!(matters(
            root,
            &event(EventKind::Modify(notify::event::ModifyKind::Any), &["/docs/.draft", "/docs/final.md"])
        ));
        // The folder itself being touched is nothing; its removal is something.
        assert!(!matters(root, &event(EventKind::Modify(notify::event::ModifyKind::Any), &["/docs"])));
        assert!(matters(root, &event(EventKind::Remove(notify::event::RemoveKind::Folder), &["/docs"])));
        // The folder itself having a dot in its path does not hide what is below it.
        assert!(matters(Path::new("/home/.me/docs"), &event(created, &["/home/.me/docs/a.md"])));
    }

    /// Wait until `done` holds, or fail after `seconds`.
    fn wait_for(seconds: u64, done: impl Fn() -> bool) -> bool {
        let until = Instant::now() + Duration::from_secs(seconds);
        while Instant::now() < until {
            if done() {
                return true;
            }
            std::thread::sleep(Duration::from_millis(50));
        }
        done()
    }

    #[test]
    fn a_burst_of_changes_is_one_call_and_a_later_change_is_another() {
        let dir = tempfile::tempdir().unwrap();
        let folder = std::fs::canonicalize(dir.path()).unwrap();
        let calls = Arc::new(AtomicUsize::new(0));
        let counter = calls.clone();
        let _watch = watch(&folder, Duration::from_millis(400), Duration::from_secs(20), move || {
            counter.fetch_add(1, Ordering::SeqCst);
        })
        .expect("the system can watch a temporary folder");
        // Let the operating system settle the watch, then write a burst.
        std::thread::sleep(Duration::from_millis(500));
        for n in 0..8 {
            std::fs::write(folder.join(format!("note-{n}.md")), format!("text {n}")).unwrap();
            std::thread::sleep(Duration::from_millis(20));
        }
        assert!(wait_for(20, || calls.load(Ordering::SeqCst) >= 1), "no change was noticed");
        // Nothing more happens by itself.
        std::thread::sleep(Duration::from_millis(1_500));
        let after_burst = calls.load(Ordering::SeqCst);
        assert!(after_burst <= 3, "eight quick writes made {after_burst} calls, not about one");
        // A later change is noticed again, below a subfolder too.
        std::fs::create_dir(folder.join("sub")).unwrap();
        std::fs::write(folder.join("sub").join("later.md"), "later").unwrap();
        assert!(wait_for(20, || calls.load(Ordering::SeqCst) > after_burst), "a later change was not noticed");
    }

    #[test]
    fn a_folder_that_is_not_there_is_refused_with_a_reason_and_hidden_changes_wake_nothing() {
        let dir = tempfile::tempdir().unwrap();
        let missing = dir.path().join("not-here");
        let error = watch(&missing, QUIET, AT_MOST, || {}).err().expect("a missing folder cannot be watched");
        assert!(error.contains("cannot watch"), "{error}");

        let folder = std::fs::canonicalize(dir.path()).unwrap();
        let calls = Arc::new(AtomicUsize::new(0));
        let counter = calls.clone();
        let _watch = watch(&folder, Duration::from_millis(300), Duration::from_secs(20), move || {
            counter.fetch_add(1, Ordering::SeqCst);
        })
        .unwrap();
        std::thread::sleep(Duration::from_millis(500));
        std::fs::create_dir(folder.join(".git")).unwrap();
        std::fs::write(folder.join(".git").join("HEAD"), "ref").unwrap();
        std::thread::sleep(Duration::from_millis(1_500));
        // The folder `.git` itself appears in the event for the folder, which is hidden too: nothing is asked for.
        assert_eq!(calls.load(Ordering::SeqCst), 0, "a change inside a hidden folder asked for a sync");
    }
}
