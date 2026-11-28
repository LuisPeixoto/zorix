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

## Decision: reading scans

- **Local, off by default.** *Reading scans* (`knowledge.ocr`) is one switch. Turning it on fetches two models once (about 12 MB: a detection model that finds the lines of text and a
  recognition model that reads them) into `<home>/models/ocr/`, checks them and loads the engine ([`ocrs`](https://github.com/robertknight/ocrs) on `rten`, both pure Rust: nothing to
  install, nothing leaves the computer). The setting is saved only after the models are ready, so it is never on without them. Turning it off unloads the engine and keeps the files.
- **Only a PDF with no text layer is read this way.** The text path runs first; a file it reports as having no text is a scan. The picture of each page is taken out of the file
  (a JPEG through `zune-jpeg`, or raw samples packed with Flate: grey, RGB or one bit, honouring a `Decode` array that turns black and white around), handed to the engine as pixels
  and the text comes back page by page, so a citation still says `scan.pdf#page=3`. Anything else (JBIG2, CCITT, JPEG 2000, CMYK, a mask) is left out with a sentence that names it.
- **Bounded before anything is allocated.** At most 100 pages, 40 million pixels in a picture (the size a PDF declares is checked before a byte of it is decoded; a JPEG decoder is given the same
  maximum), 60,000 characters kept from a page and 300 seconds for a file, on a thread of its own with a larger stack, where a crash is reported as that file being left out. A Flate
  stream is cut to the size the picture says it has, so a bomb costs what a real picture costs.
- **A scan is tried once per version of its file.** A scan that was left out because nothing could read it is read when something can (turning the switch on reads the folders that had scans left out),
  without the file changing. A scan that was tried and failed is not tried again until the file changes, so a watched folder does not spend minutes on the same page after every sync. A document
  read this way has the kind `pdf-ocr`; the screens count them (*6 via OCR*) and the scans still left out (*2 scans not read*), and *Files* says why for each.
- **The models are the dependency.** A model file is data that a parser runs over, so nothing is loaded that does not match the size and the SHA-256 fixed in the program
  (`zorix_hub::ocr`): when it is downloaded, and again every time it is loaded (a file changed on disk afterwards is not used, and a file put there by hand is used when it is the right one). The
  download is `https` (or this computer, for tests), follows only redirects that are allowed too, stops at the size the file should have, and keeps nothing when anything is off. The
  address is the one the engine's authors publish; if it ever changes, a download is refused and the models can be placed by hand. Nothing about the person or the folder is sent: it is a plain GET.

### Security review (reading scans)

| Threat | What stops it |
|---|---|
| A swapped or tampered model file runs through the model parser | Size and SHA-256 are checked before download is kept and before every load; a mismatch means "not installed" and nothing is loaded. |
| A hostile server, redirect or downgrade serves something else | `https` only (or loopback), no credentials in the address, redirects re-checked and limited to five, body bounded by the expected size; tests cover a wrong hash, a longer and a shorter body, a wrong declared size, a server error, an unreachable server and a redirect to plain `http`. |
| A PDF that declares a huge picture, or a stream that unpacks to gigabytes | Pixels checked from the declared size first; the unpacked stream is cut to what the size allows; a short stream is refused; tests cover each. |
| A picture that crashes the decoder or the engine | A thread of its own with the panic caught: the file is left out with a reason and the sync continues. |
| A scan that never ends | Pages, pixels, characters and time are all capped; the time is checked between pages and the waiting thread gives up a minute after the budget. |
| Text read from a scan carries an injection | It enters the index like any document text: redacted for secrets, wrapped as untrusted data when a run retrieves it, never placed in the system prompt. |
| The error message leaks where the file was fetched from | A failed request says why in words (too long, unreachable, sent somewhere not allowed) and never repeats the address. |

## Consequences

- A change to a document is searchable within seconds of the last change, while the app is open. While it is closed, nothing watches: *check at start* still catches up.
- Editors that save by writing a temporary file and renaming it cause two events; they are one sync.
- Systems differ in how many events they report and how fast. Tests wait up to twenty seconds and never count events exactly.
- A scan is read about as fast as a page is large: about 0.2 s for a line or two and about 1.2 s for a full A4 page at 150 dpi on the development machine (a 300 dpi scan has four times the pixels), so a
  100-page scan takes minutes and is read in the background, cancellable like any sync. Recognition is as good as the engine: clean printed Latin text is read well, handwriting, small print and
  skewed pages are not. A real scan's quality was not measured beyond the fixtures; the *Files* view and the citations let a person check.
- The program grows by about 3 MB for the engine; the models are not in it.
- Languages: the recognition model is trained on the Latin script. Other scripts come out wrong rather than empty, which is why the card says so.

