//! Keeping the index of a folder in step with the folder (ADR-0017).
//!
//! Work happens in two phases. First the text: each changed file is read, cut and written in its own transaction, so keyword
//! search works as soon as the phase ends. Then the vectors: only text that has none yet is embedded, in batches. Both can
//! be cancelled; what is already written stays, and the next sync carries on.

use std::path::{Path, PathBuf};
use std::sync::Arc;

use serde::Serialize;
use serde_json::json;
use tokio_util::sync::CancellationToken;
use zorix_core::{
    DocState, DocStatus, DocumentWrite, KnowledgeRoot, MemoryStore, NewRoot, Scope, TenantId, Timestamp, bytes_hash,
};

use super::chunk::chunk;
use super::extract::{Unreadable, extract};
use super::walk::{DEFAULT_MAX_FILE_BYTES, walk};
use crate::embedder::Embedder;
use crate::{MemoryError, Result};

/// Vectors made per call while embedding.
const EMBED_BATCH: usize = 64;

/// Which part of the work is being done.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum Phase {
    /// Looking at the folder.
    Scanning,
    /// Reading and cutting files.
    Indexing,
    /// Making vectors.
    Embedding,
    /// Finished (or stopped).
    Done,
}

/// How far a sync has got.
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct Progress {
    /// The part of the work.
    pub phase: Phase,
    /// Done so far in this phase.
    pub done: u64,
    /// To do in this phase (0 when not known yet).
    pub total: u64,
    /// The file being worked on.
    pub file: Option<String>,
    /// Files that failed so far.
    pub errors: u64,
}

/// What a sync did.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize)]
pub struct SyncReport {
    /// Files looked at.
    pub scanned: u64,
    /// Files read and written.
    pub indexed: u64,
    /// Files that had not changed.
    pub unchanged: u64,
    /// Files left out on purpose, with a reason stored.
    pub skipped: u64,
    /// Files that could not be read.
    pub failed: u64,
    /// Files that are gone from the folder.
    pub removed: u64,
    /// Pieces written.
    pub chunks: u64,
    /// Vectors made.
    pub embedded: u64,
    /// The folder had more files than are looked at.
    pub too_many_files: bool,
    /// It was stopped before it finished.
    pub cancelled: bool,
    /// Why vectors were not made, when they should have been.
    pub degraded: Option<String>,
}

/// Indexes folders into the memory store.
pub struct Indexer {
    store: Arc<dyn MemoryStore>,
    embedder: Option<Arc<Embedder>>,
    /// Reads the text of a scanned PDF, when reading scans is on and its models are there.
    ocr: Option<Arc<dyn super::PageReader>>,
}

/// The kind of a document that was read from pictures.
pub use zorix_core::OCR_KIND;

/// Read a file's text; for a PDF that is a scan, read its pictures when there is something to read them with. The second answer says it was read that way.
pub(crate) fn extract_or_scan(
    kind: super::walk::Kind,
    bytes: &[u8],
    #[allow(unused_variables)] ocr: Option<Arc<dyn super::PageReader>>,
) -> (Result<super::extract::Extracted, Unreadable>, bool) {
    let first = extract(kind, bytes);
    #[cfg(feature = "ocr")]
    if kind == super::walk::Kind::Pdf
        && let Err(Unreadable::Skip(why)) = &first
        && why.starts_with(super::pdf::SCAN)
        && let Some(reader) = ocr
    {
        return (super::pdf::read_scan(bytes, reader, super::pdf::OCR_BUDGET), true);
    }
    (first, false)
}

fn now() -> Timestamp {
    use zorix_core::Clock;
    zorix_core::SystemClock.now()
}

impl Indexer {
    /// An indexer over `store`. Without an embedder only keyword search is prepared.
    pub fn new(store: Arc<dyn MemoryStore>, embedder: Option<Arc<Embedder>>) -> Self {
        Self { store, embedder, ocr: None }
    }

    /// Read scanned PDFs with `reader` (or, with `None`, leave them out with the reason).
    pub fn with_ocr(mut self, reader: Option<Arc<dyn super::PageReader>>) -> Self {
        self.ocr = reader;
        self
    }

    /// Add a folder. The path is made absolute and must be a folder that exists. `scope` is `Global` or `Project`.
    pub async fn add_root(
        &self,
        tenant: &TenantId,
        scope: Scope,
        path: &Path,
        include: Vec<String>,
        exclude: Vec<String>,
        auto_sync: bool,
    ) -> Result<KnowledgeRoot> {
        let canonical: PathBuf = std::fs::canonicalize(path).map_err(|e| {
            zorix_core::Error::InvalidInput(format!("The folder {} cannot be used: {e}", path.display()))
        })?;
        if !canonical.is_dir() {
            return Err(zorix_core::Error::InvalidInput(format!("{} is not a folder.", canonical.display())).into());
        }
        // Check the patterns now, so a mistake is reported when the folder is added and not at the first sync.
        walk(&canonical, &include, &exclude, 0).map_err(zorix_core::Error::InvalidInput)?;
        let root = NewRoot {
            scope,
            path: canonical.to_string_lossy().into_owned(),
            include,
            exclude,
            max_file_bytes: DEFAULT_MAX_FILE_BYTES,
            auto_sync,
        };
        Ok(self.store.add_root(tenant, root, now()).await?)
    }

    /// Bring the index of `root` up to date. `progress` is called as it goes; `cancel` stops it between files.
    pub async fn sync(
        &self,
        root: &KnowledgeRoot,
        progress: &(dyn Fn(Progress) + Send + Sync),
        cancel: &CancellationToken,
    ) -> Result<SyncReport> {
        let mut report = SyncReport::default();
        let say = |phase: Phase, done: u64, total: u64, file: Option<&str>, errors: u64| {
            progress(Progress { phase, done, total, file: file.map(str::to_string), errors });
        };
        say(Phase::Scanning, 0, 0, None, 0);
        let folder = PathBuf::from(&root.path);
        if !folder.is_dir() {
            return Err(zorix_core::Error::Missing(format!("The folder {} is not there any more.", root.path)).into());
        }
        let (include, exclude, max) = (root.include.clone(), root.exclude.clone(), root.max_file_bytes);
        let walking = folder.clone();
        let walked = tokio::task::spawn_blocking(move || walk(&walking, &include, &exclude, max))
            .await
            .map_err(|e| zorix_core::Error::Internal(format!("the scan failed: {e}")))?
            .map_err(zorix_core::Error::InvalidInput)?;
        report.too_many_files = walked.truncated;
        report.scanned = walked.files.len() as u64;
        let known = self.store.document_states(root.id).await?;
        let total = walked.files.len() as u64;
        // Scans that were left out because nothing could read them are tried again once something can. One that was tried and failed is not,
        // until its file changes, so a sync does not spend minutes on the same page again.
        let waiting: std::collections::HashSet<String> = if self.ocr.is_some() {
            self.store
                .problem_documents(root.id)
                .await?
                .into_iter()
                .filter(|(_, status, why)| *status == DocStatus::Skipped && why.starts_with(zorix_core::SCAN_NOTE))
                .map(|(file, _, _)| file)
                .collect()
        } else {
            std::collections::HashSet::new()
        };

        // ---- phase 1: text ----
        for (index, file) in walked.files.iter().enumerate() {
            if cancel.is_cancelled() {
                report.cancelled = true;
                break;
            }
            say(Phase::Indexing, index as u64, total, Some(&file.rel_path), report.failed);
            let unchanged = known.get(&file.rel_path).is_some_and(|k| {
                k.size == file.size && k.mtime_ns == file.mtime_ns && !waiting.contains(&file.rel_path)
            });
            if unchanged {
                report.unchanged += 1;
                continue;
            }
            let Some(kind) = file.kind else { continue };
            let write = if let Some(reason) = &file.skip {
                report.skipped += 1;
                self.write_of(
                    root,
                    file,
                    kind.as_str(),
                    String::new(),
                    DocStatus::Skipped,
                    Some(reason.clone()),
                    Vec::new(),
                )
            } else {
                let path = folder.join(&file.rel_path);
                let read = tokio::task::spawn_blocking(move || std::fs::read(&path))
                    .await
                    .map_err(|e| zorix_core::Error::Internal(e.to_string()))?;
                match read {
                    Err(error) => {
                        report.failed += 1;
                        self.write_of(
                            root,
                            file,
                            kind.as_str(),
                            String::new(),
                            DocStatus::Failed,
                            Some(format!("it could not be read: {error}")),
                            Vec::new(),
                        )
                    }
                    Ok(bytes) => {
                        let hash = bytes_hash(&bytes);
                        // The time or the size changed but the bytes did not (a copy, a checkout, a touch): nothing to cut again.
                        // Only a file that was indexed counts; one that failed or was skipped is tried again, in case that is fixed.
                        let same = known
                            .get(&file.rel_path)
                            .is_some_and(|k| k.status == DocStatus::Indexed && k.content_hash == hash);
                        if same {
                            self.store.touch_document(root.id, &file.rel_path, file.size, file.mtime_ns).await?;
                            report.unchanged += 1;
                            continue;
                        }
                        let ocr = self.ocr.clone();
                        let (extracted, via_ocr) =
                            tokio::task::spawn_blocking(move || extract_or_scan(kind, &bytes, ocr))
                                .await
                                .map_err(|e| zorix_core::Error::Internal(e.to_string()))?;
                        match extracted {
                            Err(Unreadable::Skip(why)) => {
                                report.skipped += 1;
                                self.write_of(
                                    root,
                                    file,
                                    kind.as_str(),
                                    hash,
                                    DocStatus::Skipped,
                                    Some(why),
                                    Vec::new(),
                                )
                            }
                            Err(Unreadable::Fail(why)) => {
                                report.failed += 1;
                                self.write_of(root, file, kind.as_str(), hash, DocStatus::Failed, Some(why), Vec::new())
                            }
                            Ok(extracted) => {
                                let cut = chunk(kind, &extracted.pages);
                                let note = cut
                                    .truncated
                                    .then(|| format!("only the first {} pieces were indexed", cut.chunks.len()));
                                report.indexed += 1;
                                report.chunks += cut.chunks.len() as u64;
                                // What was read from pictures is marked, so the screens can say how many files came from a scan.
                                let label = if via_ocr { OCR_KIND } else { kind.as_str() };
                                self.write_of(root, file, label, hash, DocStatus::Indexed, note, cut.chunks)
                            }
                        }
                    }
                }
            };
            if let Err(error) = self.store.write_document(write).await {
                // One file that cannot be stored (for example, the limit of pieces) does not stop the rest.
                report.failed += 1;
                let marker = self.write_of(
                    root,
                    file,
                    kind.as_str(),
                    String::new(),
                    DocStatus::Failed,
                    Some(error.to_string()),
                    Vec::new(),
                );
                let _ = self.store.write_document(marker).await;
            }
        }

        // Files that are no longer in the folder (or are now ignored).
        if !report.cancelled {
            let present: std::collections::HashSet<&str> = walked.files.iter().map(|f| f.rel_path.as_str()).collect();
            let gone: Vec<String> = known.keys().filter(|k| !present.contains(k.as_str())).cloned().collect();
            report.removed = gone.len() as u64;
            if !gone.is_empty() {
                self.store.remove_documents(root.id, &gone).await?;
            }
        }
        say(Phase::Indexing, total, total, None, report.failed);

        // ---- phase 2: vectors ----
        if !report.cancelled {
            self.embed(root, &mut report, progress, cancel).await?;
        }
        let summary = json!({
            "scanned": report.scanned, "indexed": report.indexed, "unchanged": report.unchanged, "skipped": report.skipped,
            "failed": report.failed, "removed": report.removed, "chunks": report.chunks, "embedded": report.embedded,
            "too_many_files": report.too_many_files, "cancelled": report.cancelled, "degraded": report.degraded,
        });
        self.store.finish_sync(&root.tenant, root.id, now(), summary).await?;
        say(Phase::Done, total, total, None, report.failed);
        Ok(report)
    }

    #[allow(clippy::too_many_arguments)]
    fn write_of(
        &self,
        root: &KnowledgeRoot,
        file: &super::walk::Candidate,
        kind: &str,
        hash: String,
        status: DocStatus,
        error: Option<String>,
        chunks: Vec<zorix_core::NewChunk>,
    ) -> DocumentWrite {
        DocumentWrite {
            root: root.id,
            rel_path: file.rel_path.clone(),
            kind: kind.into(),
            state: DocState { size: file.size, mtime_ns: file.mtime_ns, content_hash: hash, status },
            error,
            chunks,
            indexed_at: now(),
        }
    }

    async fn embed(
        &self,
        root: &KnowledgeRoot,
        report: &mut SyncReport,
        progress: &(dyn Fn(Progress) + Send + Sync),
        cancel: &CancellationToken,
    ) -> Result<()> {
        let Some(embedder) = &self.embedder else { return Ok(()) };
        let settings = self.store.settings(&root.tenant).await?;
        let Some(model) = settings.embedding_model else { return Ok(()) };
        // How much there is to do, so progress can say.
        let mut remaining = self.store.count_chunks_without_vector(root.id, &model).await?;
        let total = remaining;
        let mut done = 0u64;
        while remaining > 0 {
            if cancel.is_cancelled() {
                report.cancelled = true;
                break;
            }
            let batch = self.store.chunk_hashes_without_vector(root.id, &model, EMBED_BATCH).await?;
            if batch.is_empty() {
                break;
            }
            let wanted = batch.len() as u64;
            match embedder.fill(&*self.store, &model, settings.embedding_dimensions, batch, cancel).await {
                Ok(stored) => {
                    report.embedded += stored as u64;
                    done += stored as u64;
                    if (stored as u64) < wanted {
                        // Stopped part-way: the cancel token fired.
                        report.cancelled = true;
                        break;
                    }
                }
                Err(MemoryError::Model(error)) => {
                    report.degraded = Some(format!("embeddings unavailable: {error}; keyword search still works"));
                    break;
                }
                Err(other) => return Err(other),
            }
            remaining = remaining.saturating_sub(wanted);
            progress(Progress { phase: Phase::Embedding, done, total, file: None, errors: report.failed });
        }
        Ok(())
    }
}
