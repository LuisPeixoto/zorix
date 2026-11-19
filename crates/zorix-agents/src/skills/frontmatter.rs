//! Reading `SKILL.md`: YAML frontmatter, then Markdown. The rules are those of the Agent Skills specification; where ZORIX is
//! stricter (ASCII names) or looser (unknown keys are noted, not refused, because published skills carry product-specific
//! ones) it says so.

use std::collections::BTreeMap;

use serde_json::Value;
use zorix_core::valid_skill_name;

use crate::{format, yaml};

/// Longest `SKILL.md`, in bytes.
pub const MAX_SKILL_MD: usize = 100 * 1024;

/// What a skill says about itself.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct SkillMeta {
    /// Its name.
    pub name: String,
    /// What it does and when to use it.
    pub description: String,
    /// Its licence.
    pub license: Option<String>,
    /// What it needs.
    pub compatibility: Option<String>,
    /// Free metadata.
    pub metadata: BTreeMap<String, String>,
    /// Tools it asks to be pre-approved (never granted).
    pub allowed_tools: Vec<String>,
    /// Keys the specification does not define, which were ignored.
    pub ignored: Vec<String>,
}

const KNOWN: &[&str] = &["name", "description", "license", "compatibility", "metadata", "allowed-tools"];

fn text(value: &Value, what: &str) -> Result<String, String> {
    match value {
        Value::String(s) => Ok(s.trim().to_string()),
        Value::Number(n) => Ok(n.to_string()),
        Value::Bool(b) => Ok(b.to_string()),
        _ => Err(format!("{what} must be text.")),
    }
}

/// Read a `SKILL.md`. `folder` is the name of the folder it is in, when there is one: the name must match it. Returns what the
/// skill says and its instructions (everything after the frontmatter).
pub fn parse_skill_md(file: &str, folder: Option<&str>) -> Result<(SkillMeta, String), String> {
    if file.len() > MAX_SKILL_MD {
        return Err(format!("SKILL.md is larger than {} KB. Move detail into other files.", MAX_SKILL_MD / 1024));
    }
    let (front, body) = format::split(file).map_err(|e| {
        e.replace("An agent file starts", "SKILL.md starts")
            .replace("The definition is not closed", "The frontmatter is not closed")
    })?;
    let value = yaml::parse(&front)?;
    let top = value.as_object().ok_or("The frontmatter must be a group of settings: name, description…")?;

    let name = text(top.get("name").ok_or("The frontmatter needs a `name`.")?, "name")?;
    if !valid_skill_name(&name) {
        return Err(format!(
            "The name “{name}” is not valid: use 1 to 64 lowercase letters, digits and single hyphens, not starting or ending with one."
        ));
    }
    if let Some(folder) = folder
        && folder != name
    {
        return Err(format!(
            "The name “{name}” must match the folder it is in (“{folder}”). Rename the folder, or the name."
        ));
    }
    let description = text(top.get("description").ok_or("The frontmatter needs a `description`.")?, "description")?;
    if description.is_empty() || description.chars().count() > 1024 {
        return Err("The description has 1 to 1024 characters. It says what the skill does and when to use it.".into());
    }
    let license = top.get("license").filter(|v| !v.is_null()).map(|v| text(v, "license")).transpose()?;
    if license.as_ref().is_some_and(|l| l.chars().count() > 500) {
        return Err("The license is long; name it, or point at a file.".into());
    }
    let compatibility =
        top.get("compatibility").filter(|v| !v.is_null()).map(|v| text(v, "compatibility")).transpose()?;
    if let Some(c) = &compatibility
        && (c.is_empty() || c.chars().count() > 500)
    {
        return Err("compatibility has 1 to 500 characters.".into());
    }
    let metadata = match top.get("metadata") {
        None | Some(Value::Null) => BTreeMap::new(),
        Some(Value::Object(map)) => {
            map.iter().map(|(k, v)| Ok((k.clone(), text(v, "a metadata value")?))).collect::<Result<_, String>>()?
        }
        Some(_) => return Err("metadata is a group of text values.".into()),
    };
    let allowed_tools = match top.get("allowed-tools") {
        None | Some(Value::Null) => Vec::new(),
        Some(Value::String(s)) => s.split_whitespace().map(str::to_string).collect(),
        Some(Value::Array(items)) => items.iter().map(|i| text(i, "an allowed tool")).collect::<Result<_, _>>()?,
        Some(_) => return Err("allowed-tools is a space-separated list.".into()),
    };
    let ignored = top.keys().filter(|k| !KNOWN.contains(&k.as_str())).cloned().collect();
    let instructions = body.trim_start_matches('\n').to_string();
    Ok((SkillMeta { name, description, license, compatibility, metadata, allowed_tools, ignored }, instructions))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn file(front: &str, body: &str) -> String {
        format!("---\n{front}---\n\n{body}\n")
    }

    #[test]
    fn a_minimal_skill_reads_and_the_body_is_everything_after_the_frontmatter() {
        let (meta, body) = parse_skill_md(
            &file("name: pdf-tools\ndescription: Reads PDFs. Use for PDFs.\n", "# Steps\n\n1. Do it."),
            Some("pdf-tools"),
        )
        .unwrap();
        assert_eq!((meta.name.as_str(), meta.description.as_str()), ("pdf-tools", "Reads PDFs. Use for PDFs."));
        assert!(body.starts_with("# Steps"));
        assert!(meta.license.is_none() && meta.allowed_tools.is_empty() && meta.ignored.is_empty());
    }

    #[test]
    fn every_optional_field_of_the_specification_is_read() {
        let front = "name: a\ndescription: x\nlicense: Apache-2.0\ncompatibility: Requires git and network access\nmetadata:\n  author: acme\n  version: \"1.0\"\nallowed-tools: Bash(git:*) Bash(jq:*) Read\n";
        let (meta, _) = parse_skill_md(&file(front, "b"), None).unwrap();
        assert_eq!(meta.license.as_deref(), Some("Apache-2.0"));
        assert_eq!(meta.compatibility.as_deref(), Some("Requires git and network access"));
        assert_eq!(meta.metadata["author"], "acme");
        assert_eq!(meta.metadata["version"], "1.0");
        assert_eq!(meta.allowed_tools, ["Bash(git:*)", "Bash(jq:*)", "Read"]);
    }

    #[test]
    fn the_name_rules_of_the_specification_hold() {
        for bad in ["PDF-Processing", "-pdf", "pdf-", "pdf--processing", "", "has space", "é", &"a".repeat(65)] {
            let error = parse_skill_md(&file(&format!("name: \"{bad}\"\ndescription: x\n"), "b"), None).unwrap_err();
            assert!(error.contains("name"), "{bad:?} -> {error}");
        }
        for good in ["a", "pdf-processing", "data-analysis-2", &"a".repeat(64)] {
            assert!(parse_skill_md(&file(&format!("name: {good}\ndescription: x\n"), "b"), None).is_ok(), "{good}");
        }
    }

    #[test]
    fn the_name_must_match_its_folder() {
        let text = file("name: pdf-tools\ndescription: x\n", "b");
        assert!(parse_skill_md(&text, Some("pdf-tools")).is_ok());
        let error = parse_skill_md(&text, Some("pdf-tools-v2")).unwrap_err();
        assert!(error.contains("must match the folder") && error.contains("pdf-tools-v2"), "{error}");
    }

    #[test]
    fn the_description_and_compatibility_have_their_limits() {
        assert!(parse_skill_md(&file("name: a\ndescription: \"\"\n", "b"), None).is_err());
        let long = "x".repeat(1025);
        assert!(parse_skill_md(&file(&format!("name: a\ndescription: \"{long}\"\n"), "b"), None).is_err());
        let ok = "x".repeat(1024);
        assert!(parse_skill_md(&file(&format!("name: a\ndescription: \"{ok}\"\n"), "b"), None).is_ok());
        let compat = "y".repeat(501);
        assert!(
            parse_skill_md(&file(&format!("name: a\ndescription: x\ncompatibility: \"{compat}\"\n"), "b"), None)
                .is_err()
        );
        assert!(parse_skill_md(&file("name: a\n", "b"), None).unwrap_err().contains("description"));
        assert!(parse_skill_md(&file("description: x\n", "b"), None).unwrap_err().contains("name"));
    }

    #[test]
    fn a_folded_description_and_keys_the_specification_does_not_define_are_handled() {
        let front =
            "name: a\ndescription: >\n  Reads things.\n  Use when asked.\nmodel: sonnet\nargument-hint: \"[file]\"\n";
        let (meta, _) = parse_skill_md(&file(front, "b"), None).unwrap();
        assert_eq!(meta.description, "Reads things. Use when asked.");
        assert_eq!(meta.ignored, ["argument-hint", "model"], "noted, not refused");
    }

    #[test]
    fn broken_input_is_an_error_not_a_panic() {
        for bad in
            ["", "no frontmatter", "---\nname: a\n", "---\n- a\n- b\n---\n", "---\nname: &x a\n---\n", "---\n---\n"]
        {
            assert!(parse_skill_md(bad, None).is_err(), "{bad:?}");
        }
        assert!(parse_skill_md(&"x".repeat(MAX_SKILL_MD + 1), None).unwrap_err().contains("100 KB"));
    }
}
