//! Files attached to a run (ADR-0049): what the form asks while a person is choosing them. Starting a run reads them again (in `start_run` and
//! `agent_ask`), so what is checked here is only to tell the person early; nothing here is trusted by the run.

use std::path::{Path, PathBuf};

use serde::Serialize;
use tauri::State;
use zorix_core::{DocStatus, MAX_ATTACHMENTS, MemoryStore};
use zorix_memory::attach::look;
use zorix_memory::index::walk::{DEFAULT_MAX_FILE_BYTES, walk};

use crate::state::{AppState, Services};
use crate::ui_error::{UiError, ui};

/// Read the files a run is started with. Blocking work (disk, PDFs, the OCR engine) is done off the async threads. The first refused file is the
/// answer, named.
pub async fn read(services: &Services, paths: &[String]) -> Result<Vec<zorix_core::Attachment>, UiError> {
    if paths.is_empty() {
        return Ok(Vec::new());
    }
    let (stack, paths): (_, Vec<PathBuf>) = (services.memory.clone(), paths.iter().map(PathBuf::from).collect());
    tokio::task::spawn_blocking(move || stack.read_attachments(&paths))
        .await
        .map_err(|e| UiError::message(e.to_string()))?
        .map_err(|e| UiError::message(e.to_string()))
}

/// What the form knows about a file it was given.
#[derive(Serialize)]
pub struct Checked {
    /// The path, as it was given.
    path: String,
    /// The name the run will know it by.
    name: String,
    /// Its size in bytes.
    bytes: u64,
    /// Why it cannot be attached, as a sentence.
    problem: Option<String>,
    /// It is not inside the workspace folder (a person chose it, so it is allowed; the form says so).
    outside_workspace: bool,
}

/// Check files as they are chosen or dropped: that each is a file that can be attached, so a refusal is shown when it happens and not when the
/// run starts. Nothing is read.
#[tauri::command]
pub async fn attachments_check(paths: Vec<String>, workspace: Option<String>) -> Result<Vec<Checked>, UiError> {
    if paths.len() > 200 {
        return Err(UiError::message(format!("At most {MAX_ATTACHMENTS} files can be attached to a run.")));
    }
    tokio::task::spawn_blocking(move || {
        let root =
            workspace.as_deref().map(str::trim).filter(|w| !w.is_empty()).and_then(|w| std::fs::canonicalize(w).ok());
        paths
            .into_iter()
            .map(|path| match look(Path::new(&path)) {
                Ok(file) => Checked {
                    outside_workspace: root.as_ref().is_some_and(|r| !file.path.starts_with(r)),
                    name: file.name,
                    bytes: file.bytes,
                    problem: None,
                    path,
                },
                Err(error) => Checked {
                    name: Path::new(&path)
                        .file_name()
                        .map(|n| n.to_string_lossy().into_owned())
                        .unwrap_or_else(|| path.clone()),
                    bytes: 0,
                    problem: Some(error.to_string()),
                    outside_workspace: false,
                    path,
                },
            })
            .collect()
    })
    .await
    .map_err(|e| UiError::message(e.to_string()))
}

/// One file the `@` menu offers.
#[derive(Serialize)]
pub struct Candidate {
    /// Its full path (what is attached when it is chosen).
    path: String,
    /// Its name.
    name: String,
    /// Where it is, relative to the workspace or to the indexed folder.
    folder: String,
}

/// What the `@` menu lists.
#[derive(Serialize)]
pub struct Candidates {
    /// Indexed documents.
    documents: Vec<Candidate>,
    /// Files of the workspace folder.
    files: Vec<Candidate>,
}

/// How many of each the menu shows.
const SHOWN: usize = 8;

fn matches(query: &str, rel: &str) -> bool {
    let wanted = query.trim().to_lowercase();
    wanted.is_empty() || wanted.split_whitespace().all(|word| rel.to_lowercase().contains(word))
}

fn candidate(root: &Path, rel: &str) -> Candidate {
    let (folder, name) = rel.rsplit_once('/').map_or(("", rel), |(f, n)| (f, n));
    Candidate {
        path: root.join(rel).to_string_lossy().into_owned(),
        name: name.to_string(),
        folder: folder.to_string(),
    }
}

/// Indexed documents and workspace files that match what was typed after the `@`. Only names that could be attached are offered: the walk leaves
/// out hidden files, ignored files and private names, as the indexer does.
#[tauri::command]
pub async fn attachment_candidates(
    state: State<'_, AppState>,
    query: String,
    workspace: Option<String>,
) -> Result<Candidates, UiError> {
    let services = state.services()?;
    let mut documents = Vec::new();
    for root in services.memory_store.all_roots(&services.tenant).await.map_err(ui)?.into_iter().filter(|r| r.enabled) {
        let base = PathBuf::from(&root.path);
        let mut rels: Vec<String> = services
            .memory_store
            .document_states(root.id)
            .await
            .map_err(ui)?
            .into_iter()
            .filter(|(rel, s)| s.status == DocStatus::Indexed && matches(&query, rel))
            .map(|(rel, _)| rel)
            .collect();
        rels.sort();
        documents.extend(rels.iter().take(SHOWN).map(|rel| candidate(&base, rel)));
        if documents.len() >= SHOWN {
            break;
        }
    }
    documents.truncate(SHOWN);

    let files = match workspace.as_deref().map(str::trim).filter(|w| !w.is_empty()) {
        Some(folder) => {
            let (folder, query) = (folder.to_string(), query.clone());
            tokio::task::spawn_blocking(move || {
                let Ok(base) = std::fs::canonicalize(&folder) else { return Vec::new() };
                let Ok(walked) = walk(&base, &[], &[], DEFAULT_MAX_FILE_BYTES) else { return Vec::new() };
                let mut found: Vec<_> = walked
                    .files
                    .into_iter()
                    .filter(|f| f.kind.is_some() && f.skip.is_none() && matches(&query, &f.rel_path))
                    .collect();
                // The files touched most recently come first: that is usually what is meant.
                found.sort_by(|a, b| b.mtime_ns.cmp(&a.mtime_ns).then_with(|| a.rel_path.cmp(&b.rel_path)));
                found.iter().take(SHOWN).map(|f| candidate(&base, &f.rel_path)).collect()
            })
            .await
            .map_err(|e| UiError::message(e.to_string()))?
        }
        None => Vec::new(),
    };
    Ok(Candidates { documents, files })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn what_was_typed_after_the_at_sign_matches_every_word_anywhere_in_the_path() {
        assert!(matches("", "docs/plan.md"));
        assert!(matches("  ", "docs/plan.md"));
        assert!(matches("plan", "docs/Plan.md"));
        assert!(matches("docs plan", "docs/plan.md"));
        assert!(matches("PLAN docs", "docs/plan.md"));
        assert!(!matches("plan budget", "docs/plan.md"));
        assert!(!matches("x", "docs/plan.md"));
    }

    #[test]
    fn a_candidate_has_its_full_path_its_name_and_its_folder() {
        let one = candidate(Path::new("/work/acme"), "docs/guides/plan.md");
        assert_eq!((one.name.as_str(), one.folder.as_str()), ("plan.md", "docs/guides"));
        assert!(one.path.ends_with("plan.md") && one.path.starts_with("/work") || cfg!(windows), "{}", one.path);
        let top = candidate(Path::new("/work/acme"), "README.md");
        assert_eq!((top.name.as_str(), top.folder.as_str()), ("README.md", ""));
    }

    #[tokio::test]
    async fn files_are_checked_as_they_are_chosen_and_nothing_is_read() {
        let dir = tempfile::tempdir().unwrap();
        let workspace = dir.path().join("work");
        std::fs::create_dir_all(&workspace).unwrap();
        let inside = workspace.join("notes.md");
        std::fs::write(&inside, "hello").unwrap();
        let outside = dir.path().join("elsewhere.txt");
        std::fs::write(&outside, "hi").unwrap();
        let secret = dir.path().join(".env");
        std::fs::write(&secret, "TOKEN=1").unwrap();
        let paths: Vec<String> = [&inside, &outside, &secret, &dir.path().join("missing.md")]
            .iter()
            .map(|p| p.to_string_lossy().into_owned())
            .collect();
        let checked = attachments_check(paths, Some(workspace.to_string_lossy().into_owned())).await.unwrap();
        assert_eq!(checked.len(), 4);
        assert!(checked[0].problem.is_none() && !checked[0].outside_workspace && checked[0].bytes == 5);
        assert!(
            checked[1].problem.is_none() && checked[1].outside_workspace,
            "a person chose it, so it is allowed, and the form says it is outside"
        );
        assert!(checked[2].problem.as_deref().is_some_and(|p| p.contains("holds secrets")), "{:?}", checked[2].problem);
        assert!(
            checked[3].problem.as_deref().is_some_and(|p| p.contains("cannot be found")),
            "{:?}",
            checked[3].problem
        );
        assert!(
            attachments_check(vec!["x".into(); 201], None).await.is_err(),
            "a list this long is not a list of attachments"
        );
    }
}
