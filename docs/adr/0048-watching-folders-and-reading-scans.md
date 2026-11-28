# ADR-0048: Watching folders and reading scans

Status: accepted (Phase 11)

## Context

Documents were brought up to date only when a person pressed *Sync* (or asked for it at start). A handbook that changes during the day was stale until then, and
a PDF that is a scan, with no text layer, was left out with the words "a scan needs OCR, which is not included".

## Decision: watching

- **A change wakes a sync; it is not the sync.** The operating system tells the app that something under a watched folder changed (the `notify` crate: FSEvents
  on macOS, inotify on Linux, ReadDirectoryChanges on Windows). The watcher never reads a file. After the folder has been quiet for two seconds (or thirty
  seconds after the first change of a burst that never goes quiet) it asks for one sync, which already skips a file whose size and time are the same and one whose hash
  is, so a change to one file costs reading that file.
- **What does not matter is not noticed.** Reads (`Access`), changes inside hidden folders and files (`.git`, `.DS_Store`: the indexer does not read them) and the watched
  folder itself being touched wake nothing; the folder being removed does. Links are not followed.
- **A folder the system cannot watch is checked on a timer.** A network drive, or a limit on watches, makes starting the watch fail with a reason; the folder is
  then checked every ten minutes, and the Documents screen says so and why.
- **It is a per-folder switch.** New folders are watched unless the person says not; folders that were there before are not, until they are marked (schema 16). The
  desktop app watches the marked folders while it is open (and stops when one is removed, turned off or the app closes); `zorix knowledge watch` does the same in a terminal
  until Ctrl-C, with a first sync to catch up.
- **Security.** The watcher only decides *when* to sync; what is read is decided by the indexer, with the same limits as always (private names, no links, size caps, only below the folder). A
  hostile file cannot make the watcher read anything, and a flood of changes is one sync at most every two seconds.

## Consequences

- A change to a document is searchable within seconds of the last change, while the app is open. While it is closed, nothing watches: *check at start* still catches up.
- Editors that save by writing a temporary file and renaming it cause two events; they are one sync.
- Systems differ in how many events they report and how fast. Tests wait up to twenty seconds and never count events exactly.
