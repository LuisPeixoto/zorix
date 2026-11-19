//! The hub's agent operations: a file in, a version out; the same file again changes nothing; a run is built from a version.

use std::sync::Arc;

use zorix_agents::{AgentAsk, ModelResolver};
use zorix_core::{AgentStore, SkillFile, SkillRecord, TenantId, Timestamp};
use zorix_hub::Hub;
use zorix_providers::MemoryStore;
use zorix_runtime::InMemoryToolRegistry;

struct Anything;

impl ModelResolver for Anything {
    fn candidates(&self, model: &str) -> Result<Vec<(Option<String>, String)>, String> {
        Ok(vec![(None, model.to_string())])
    }
}

fn hub() -> (Hub, tempfile::TempDir) {
    let dir = tempfile::tempdir().unwrap();
    let hub =
        Hub::open(dir.path(), TenantId::local(), Arc::new(MemoryStore::new()), Arc::new(InMemoryToolRegistry::new()))
            .unwrap();
    (hub, dir)
}

const FILE: &str = "---\nzorix: agent/1\nslug: writer\nname: Writer\ndescription: Edits text.\nmodels:\n  default: demo\ntools:\n  - clock.now\n---\n\nYou edit text kindly.\n";

#[tokio::test]
async fn a_file_makes_an_agent_and_the_same_file_again_changes_nothing() {
    let (hub, _dir) = hub();
    let first = hub.import_agent(FILE, None, Some("from a file".into())).await.unwrap();
    assert!(first.created && first.changed);
    assert_eq!((first.agent.slug.as_str(), first.version.version), ("writer", 1));

    let again = hub.import_agent(FILE, None, None).await.unwrap();
    assert!(!again.created && !again.changed, "the same content is not a new version");
    assert_eq!(again.agent.current_version, 1);

    let edited = FILE.replace("kindly", "kindly and briefly");
    let second = hub.import_agent(&edited, None, Some("shorter".into())).await.unwrap();
    assert!(!second.created && second.changed);
    assert_eq!(second.version.version, 2);

    // Exporting gives back what was imported, and exporting an old version gives the old text.
    let newest = hub.export_agent("writer", None).await.unwrap();
    assert!(newest.contains("kindly and briefly") && newest.contains("version: 2"), "{newest}");
    let oldest = hub.export_agent("writer", Some(1)).await.unwrap();
    assert!(!oldest.contains("briefly") && oldest.contains("version: 1"));
    let again = hub.import_agent(&newest, None, None).await.unwrap();
    assert!(!again.changed, "an exported file imports as no change");
    assert!(hub.export_agent("writer", Some(9)).await.unwrap_err().to_string().contains("no version 9"));
    assert!(hub.agent("nobody").await.unwrap_err().to_string().contains("zorix agent list"));
}

#[tokio::test]
async fn rename_imports_under_another_slug_and_rollback_and_diff_work() {
    let (hub, _dir) = hub();
    hub.import_agent(FILE, None, None).await.unwrap();
    let copy = hub.import_agent(FILE, Some("writer-two"), None).await.unwrap();
    assert!(copy.created);
    assert_eq!(copy.agent.slug.as_str(), "writer-two");
    assert!(hub.import_agent(FILE, Some("Not A Slug"), None).await.is_err());

    hub.import_agent(&FILE.replace("kindly", "sternly"), None, None).await.unwrap();
    let lines = hub.diff_agent("writer", 1, 2).await.unwrap();
    assert!(lines.iter().any(|l| l.text.contains("sternly")), "{lines:?}");
    let back = hub.rollback_agent("writer", 1).await.unwrap();
    assert_eq!((back.version, back.note.as_deref()), (3, Some("rollback to v1")));
    assert!(hub.export_agent("writer", None).await.unwrap().contains("kindly"));
}

#[tokio::test]
async fn a_skill_named_in_a_file_is_pinned_to_the_librarys_current_version_or_the_import_says_what_is_missing() {
    let (hub, _dir) = hub();
    let with_skill = FILE.replace("tools:", "skills:\n  - checklist\ntools:");
    let error = hub.import_agent(&with_skill, None, None).await.unwrap_err().to_string();
    assert!(error.contains("checklist") && error.contains("not in the library"), "{error}");

    let hash = "a".repeat(64);
    hub.agents
        .record_skill(
            hub.tenant(),
            SkillRecord {
                name: "checklist".into(),
                hash: hash.clone(),
                description: "A checklist.".into(),
                files: vec![SkillFile { path: "SKILL.md".into(), size: 1, sha256: hash.clone() }],
                license: None,
                compatibility: None,
                metadata: Default::default(),
                allowed_tools: vec![],
                source: None,
                imported_at: Timestamp(1),
            },
        )
        .await
        .unwrap_or_else(|_| panic!("record"));
    let done = hub.import_agent(&with_skill, None, None).await.unwrap();
    assert_eq!(done.version.spec.skills[0].hash, hash);
    let file = hub.export_agent("writer", None).await.unwrap();
    assert!(file.contains(&format!("checklist@sha256:{hash}")), "{file}");
}

#[tokio::test]
async fn a_run_is_built_from_a_version_and_the_version_is_chosen_by_number() {
    let (hub, _dir) = hub();
    hub.import_agent(FILE, None, None).await.unwrap();
    hub.import_agent(&FILE.replace("kindly", "sternly"), None, None).await.unwrap();
    let ask = |p: &str| AgentAsk { prompt: p.into(), ..AgentAsk::default() };
    let newest = hub.prepare_agent_run("writer", None, ask("edit this"), &Anything).await.unwrap();
    assert!(newest.request.instructions.unwrap().contains("sternly"));
    assert_eq!(newest.agent.as_ref().unwrap().version, 2);
    let old = hub.prepare_agent_run("writer", Some(1), ask("edit this"), &Anything).await.unwrap();
    assert!(old.request.instructions.unwrap().contains("kindly"));
    assert_eq!(old.agent.unwrap().version, 1);
    assert!(newest.grants.allows(&zorix_core::Permission::use_tool("clock.now")));
    assert!(hub.prepare_agent_run("writer", Some(7), ask("x"), &Anything).await.is_err());
}
