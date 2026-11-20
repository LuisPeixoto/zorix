//! The starter agents are valid: their files parse, their skills import and are the ones the agents name, their access
//! compiles, and their evaluation cases are well formed. Nothing here needs a model.

use std::collections::HashSet;

use zorix_agents::skills::frontmatter::parse_skill_md;
use zorix_agents::starters::{all, get};
use zorix_agents::{SkillLibrary, format};
use zorix_core::access::compile_access;
use zorix_core::{EvalCase, Timestamp};

#[test]
fn there_are_five_starters_with_distinct_ids() {
    let ids: Vec<&str> = all().iter().map(|s| s.id).collect();
    assert_eq!(ids, ["researcher", "code-reviewer", "data-analyst", "research-lead", "writer"]);
    assert_eq!(ids.iter().collect::<HashSet<_>>().len(), 5);
    assert!(get("writer").is_some() && get("nope").is_none());
}

#[test]
fn every_agent_file_parses_validates_and_names_only_its_own_skills() {
    for starter in all() {
        let parsed = format::parse(starter.agent).unwrap_or_else(|e| panic!("{}: {e}", starter.id));
        assert_eq!(parsed.spec.slug.as_str(), starter.id, "the slug is the id");
        let named: Vec<&str> = parsed.skills.iter().map(|s| s.name.as_str()).collect();
        let shipped: Vec<&str> = starter.skills.iter().map(|s| s.name).collect();
        assert_eq!(named, shipped, "{} names the skills it ships", starter.id);
        assert!(
            parsed.skills.iter().all(|s| s.hash.is_none()),
            "starters pin nothing: the install pins what it imports"
        );
        let spec = parsed.into_spec(|name| Some(format!("{:0>64}", name.len()))).unwrap();
        spec.validate().unwrap_or_else(|e| panic!("{}: {e}", starter.id));
        compile_access(&spec.access).unwrap_or_else(|e| panic!("{}: {e}", starter.id));
        assert!(spec.agent_memory, "{} uses its own memory", starter.id);
        assert!(!spec.instructions.contains("TODO"), "{}", starter.id);
        // Nothing wide: no write access and no network beyond named hosts.
        assert!(spec.access.iter().all(|a| !a.starts_with("write:") && !a.ends_with(":write")), "{:?}", spec.access);
        assert!(spec.access.iter().filter(|a| a.starts_with("http:")).all(|a| !a.contains('*')), "{:?}", spec.access);
    }
}

#[test]
fn every_skill_imports_under_its_own_name_and_the_script_is_listed() {
    let home = tempfile::tempdir().unwrap();
    let library = SkillLibrary::new(home.path());
    for starter in all() {
        for skill in starter.skills {
            let text = std::str::from_utf8(skill.files[0].bytes).unwrap();
            assert_eq!(skill.files[0].path, "SKILL.md");
            let (meta, body) = parse_skill_md(text, Some(skill.name)).unwrap();
            assert_eq!(meta.name, skill.name);
            assert!(body.trim().len() > 100, "{} has real instructions", skill.name);
            let record = library
                .import_files(skill.owned_files(), Some(skill.name), Some("template".into()), Timestamp(1))
                .unwrap();
            assert_eq!(record.name, skill.name);
            assert!(library.has(&record.hash));
            assert_eq!(record.files.len(), skill.files.len());
            // The same content gives the same hash, wherever and whenever it is imported.
            let again = library.import_files(skill.owned_files(), Some(skill.name), None, Timestamp(9)).unwrap();
            assert_eq!(again.hash, record.hash);
        }
    }
}

#[test]
fn every_case_is_valid_named_once_and_checks_something() {
    for starter in all() {
        let mut names = HashSet::new();
        assert!(starter.cases.len() >= 3, "{} has a suite", starter.id);
        for text in starter.cases {
            let case: EvalCase = serde_json::from_str(text).unwrap_or_else(|e| panic!("{}: {e}", starter.id));
            case.validate().unwrap_or_else(|e| panic!("{} {}: {e}", starter.id, case.name));
            assert!(names.insert(case.name.clone()), "{} repeats {}", starter.id, case.name);
            assert!(case.judge.is_none(), "starters need no judge model");
            assert!(!case.allow_writes && case.approve.is_empty(), "cases never approve or write");
            for check in &case.checks {
                if let zorix_core::EvalCheck::Regex { pattern } = check {
                    regex::Regex::new(pattern).unwrap_or_else(|e| panic!("{} {}: {e}", starter.id, case.name));
                }
            }
        }
    }
}

#[test]
fn the_profiling_script_is_the_one_the_skill_describes() {
    let skill = get("data-analyst").unwrap().skills[0].files;
    let script = std::str::from_utf8(skill.iter().find(|f| f.path == "scripts/profile.py").unwrap().bytes).unwrap();
    let doc = std::str::from_utf8(skill[0].bytes).unwrap();
    for word in ["rows", "columns", "median", "common"] {
        assert!(script.contains(word) && doc.contains(word), "{word}");
    }
    assert!(!script.contains("import pandas") && !script.contains("import numpy"), "standard library only");
}

#[test]
fn the_starter_team_names_starters_and_its_lead_may_ask_exactly_its_members() {
    use zorix_agents::starters::{team, teams};
    assert_eq!(teams().iter().map(|t| t.id).collect::<Vec<_>>(), ["research-desk"]);
    let desk = team("research-desk").unwrap();
    assert!(team("nope").is_none());
    // Every agent of the team is a starter, and the lead's own access names the members, so it works alone (for evaluations) too.
    for id in std::iter::once(&desk.lead).chain(desk.members) {
        assert!(get(id).is_some(), "{id} is a starter");
    }
    let lead = format::parse(get(desk.lead).unwrap().agent).unwrap();
    let access: Vec<String> = desk.members.iter().map(|m| format!("agent:{m}")).collect();
    for entry in &access {
        assert!(lead.spec.access.contains(entry), "{entry} in the lead's access: {:?}", lead.spec.access);
    }
    assert!(!lead.spec.access.iter().any(|a| a == "agent:*"), "a starter never asks everyone");
    assert!(!desk.members.contains(&desk.lead));
}
