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

#[tokio::test]
async fn a_skill_is_imported_listed_removed_and_its_files_are_only_cleaned_when_nothing_names_them() {
    let (hub, dir) = hub();
    let folder = dir.path().join("src").join("checklist");
    std::fs::create_dir_all(&folder).unwrap();
    std::fs::write(
        folder.join("SKILL.md"),
        "---\nname: checklist\ndescription: A checklist. Use before shipping.\n---\n\nTick the boxes.\n",
    )
    .unwrap();
    let record = hub.import_skill(&folder).await.unwrap();
    assert_eq!(record.name, "checklist");
    assert_eq!(hub.skills_list().await.unwrap().len(), 1);
    assert_eq!(hub.skill("checklist").await.unwrap().hash, record.hash);
    assert!(hub.skill("nothing").await.unwrap_err().to_string().contains("zorix skill list"));
    assert!(hub.import_skill(&dir.path().join("src").join("absent")).await.is_err());

    // An agent pins it; removing the skill keeps its files for that agent.
    let file = FILE.replace("tools:", "skills:\n  - checklist\ntools:");
    hub.import_agent(&file, None, None).await.unwrap();
    hub.remove_skill("checklist").await.unwrap();
    assert!(hub.skill("checklist").await.is_err());
    let report = hub.clean_skills().await.unwrap();
    assert_eq!(report.removed, 0, "the agent still names it");
    assert!(hub.skills.has(&record.hash));

    // With the agent gone, nothing names it and cleaning removes the files.
    let (agent, _) = hub.agent("writer").await.unwrap();
    hub.agents.remove_agent(hub.tenant(), agent.id).await.unwrap();
    let report = hub.clean_skills().await.unwrap();
    assert_eq!(report.removed, 1);
    assert!(!hub.skills.has(&record.hash));
    assert!(!hub.registry_has_skill_tools(), "nothing is registered until the app asks");
    hub.register_native_tools().unwrap();
    assert!(hub.registry_has_skill_tools());
}

#[tokio::test]
async fn the_templates_are_listed_and_installing_one_brings_its_skills_cases_and_a_pinned_agent() {
    let (hub, _dir) = hub();
    let before = hub.templates().await.unwrap();
    assert_eq!(
        before.iter().map(|t| t.id.as_str()).collect::<Vec<_>>(),
        ["researcher", "code-reviewer", "data-analyst", "writer"]
    );
    assert!(before.iter().all(|t| !t.installed && t.cases >= 3 && !t.skills.is_empty() && !t.description.is_empty()));

    let done = hub.add_template("data-analyst", None).await.unwrap();
    assert!(done.imported.created);
    assert_eq!((done.skills.len(), done.cases), (1, 3));
    let (agent, version) = hub.agent("data-analyst").await.unwrap();
    assert_eq!(version.spec.skills.len(), 1);
    assert_eq!(version.spec.skills[0].name, "csv-profiling");
    assert_eq!(version.spec.skills[0].hash, done.skills[0].hash, "the agent pins the exact skill that was imported");
    assert!(hub.skills.has(&done.skills[0].hash), "and its files are in the library");
    let cases = hub.agents.eval_cases(&TenantId::local(), agent.id).await.unwrap();
    assert_eq!(cases.len(), 3);
    assert!(hub.templates().await.unwrap().iter().any(|t| t.id == "data-analyst" && t.installed));

    // Installing again is safe: nothing new is written.
    let again = hub.add_template("data-analyst", None).await.unwrap();
    assert!(!again.imported.created && !again.imported.changed);
    assert_eq!(again.skills[0].hash, done.skills[0].hash);
    assert_eq!(
        hub.agents.eval_cases(&TenantId::local(), agent.id).await.unwrap().len(),
        3,
        "cases are replaced by name"
    );
    assert_eq!(hub.agents.versions(&TenantId::local(), agent.id).await.unwrap().len(), 1);

    // A copy under another name has its own cases.
    let copy = hub.add_template("data-analyst", Some("my-analyst")).await.unwrap();
    assert!(copy.imported.created);
    assert_eq!(hub.agents.eval_cases(&TenantId::local(), copy.imported.agent.id).await.unwrap().len(), 3);

    let error = hub.add_template("nope", None).await.unwrap_err().to_string();
    assert!(error.contains("no template called nope") && error.contains("researcher"), "{error}");
}

#[tokio::test]
async fn every_template_installs_and_makes_a_run_that_carries_its_skill_tools() {
    let (hub, _dir) = hub();
    for info in hub.templates().await.unwrap() {
        hub.add_template(&info.id, None).await.unwrap();
        let new = hub
            .prepare_agent_run(&info.id, None, AgentAsk { prompt: "hello".into(), ..AgentAsk::default() }, &Anything)
            .await
            .unwrap_or_else(|e| panic!("{}: {e}", info.id));
        let binding = new.agent.as_ref().unwrap();
        assert_eq!((binding.slug.as_str(), binding.version), (info.id.as_str(), 1));
        assert_eq!(binding.skills.len(), 1);
        assert!(new.request.instructions.as_deref().unwrap_or("").contains("<available_skills>"), "{}", info.id);
        assert!(new.grants.allows(&zorix_core::Permission::use_tool("skill.load")), "{}", info.id);
    }
}
