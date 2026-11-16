# ADR-0017: Knowledge is a local index of folders people choose

Status: accepted (Phase 3)

## Context

People want to ask about their own documents. Reading files means touching things that may hold secrets, being fooled by
hostile content and spending time on large trees.

## Decision

- A **root** is a folder (global or tied to a project). The indexer walks it with `.gitignore`, `.ignore`, the global ignore
  and `.zorixignore`, never follows links, skips hidden files, and has a fixed **deny list** (`.git`, `.env*`, keys,
  certificates, `.npmrc`, `.netrc`, `credentials*`, `secrets.*`).
- Files are chosen by an extension allow-list plus include/exclude globs; text must be UTF-8 and not binary. HTML goes
  through a text converter; **PDF** text through `pdf-extract`, one file at a time, with panic isolation, a size and page
  limit and a time budget. No OCR.
- Text is split into about 800-token chunks: Markdown by headings, code and data by line windows with overlap, PDF by page.
  Each chunk keeps its citation (path, lines or page) and passes through secret redaction.
- Sync compares size and modification time, then a content hash; only changed files are re-chunked and only new text is
  embedded. Files that disappeared are removed. Work happens in two phases: text and keywords first (usable at once), then
  embeddings. Sync can be cancelled and resumed. There is no file watcher in 0.3.
- Limits: 50 roots per tenant, 20k files per root, 2 MB per text file, 2k chunks per file, 200k chunks per tenant.
- Retrieved chunks are shown to the model as untrusted `<document>` blocks with a citation, and the instructions ask it to
  cite them.
- Removing a root deletes only ZORIX's index, never the files.

## Consequences

- Hostile text in a document cannot become an instruction or a memory without approval.
- Scanned PDFs and unusual encodings are reported, not guessed.
- Indexes go stale until the next sync (or the start-up stat pass when `auto_sync` is on).
