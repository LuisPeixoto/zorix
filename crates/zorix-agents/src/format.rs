//! An agent as a Markdown file: YAML frontmatter for the definition, the instructions as the body (ADR-0024).
//!
//! ```text
//! ---
//! zorix: agent/1
//! slug: researcher
//! name: "Researcher"
//! ...
//! ---
//!
//! Instructions in Markdown.
//! ```
//!
//! Writing is deterministic (fixed key order, every text quoted), and `parse(render(spec)) == spec`. Reading is strict: unknown
//! keys are refused, so a typo is an error and not a silently ignored setting.

use std::collections::BTreeMap;

use serde_json::{Map, Value, json};
use zorix_core::{AgentModels, AgentSlug, AgentSpec, Avatar, Limits, MemoryOptions, SkillPin};

use crate::yaml;

/// The version of the file format written and understood.
pub const FORMAT: &str = "agent/1";

const KEYS: &[&str] = &[
    "zorix",
    "slug",
    "name",
    "description",
    "avatar",
    "models",
    "tools",
    "access",
    "skills",
    "limits",
    "memory",
    "agent_memory",
    "params",
    "expected_output",
    "metadata",
    "version",
];

/// A skill an agent file names: by name alone (the current version is used) or pinned by hash.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SkillRef {
    /// The skill's name.
    pub name: String,
    /// The pinned content hash, when the file has one.
    pub hash: Option<String>,
}

/// What was read from a file: everything but the skill pins, which need the library to resolve.
#[derive(Clone, Debug, PartialEq)]
pub struct ParsedAgent {
    /// The definition, with `skills` empty until [`ParsedAgent::into_spec`].
    pub spec: AgentSpec,
    /// The skills the file names.
    pub skills: Vec<SkillRef>,
    /// The version the file says it was exported from. Informational only.
    pub version: Option<u32>,
}

impl ParsedAgent {
    /// Pin the skills: those with a hash keep it, the others take what `current` says (the library's current hash).
    pub fn into_spec(self, current: impl Fn(&str) -> Option<String>) -> Result<AgentSpec, String> {
        let mut spec = self.spec;
        for skill in self.skills {
            let hash = match skill.hash {
                Some(hash) => hash,
                None => current(&skill.name).ok_or_else(|| {
                    format!("The skill {} is not in the library. Import it first (`zorix skill import`).", skill.name)
                })?,
            };
            spec.skills.push(SkillPin { name: skill.name, hash });
        }
        Ok(spec)
    }
}

fn quote(text: &str) -> String {
    serde_json::to_string(text).unwrap_or_else(|_| "\"\"".into())
}

fn key(text: &str) -> String {
    let plain = !text.is_empty()
        && text.chars().next().is_some_and(|c| c.is_ascii_alphabetic() || c == '_')
        && text.chars().all(|c| c.is_ascii_alphanumeric() || matches!(c, '_' | '-' | '.'));
    if plain { text.to_string() } else { quote(text) }
}

fn scalar(value: &Value) -> String {
    match value {
        Value::String(text) => quote(text),
        other => other.to_string(),
    }
}

fn list(out: &mut String, name: &str, items: &[String]) {
    if items.is_empty() {
        out.push_str(&format!("{name}: []\n"));
        return;
    }
    out.push_str(&format!("{name}:\n"));
    for item in items {
        out.push_str(&format!("  - {}\n", quote(item)));
    }
}

fn map(out: &mut String, name: &str, entries: &Map<String, Value>) {
    let shown: Vec<(&String, &Value)> = entries.iter().filter(|(_, v)| !v.is_null()).collect();
    if shown.is_empty() {
        out.push_str(&format!("{name}: {{}}\n"));
        return;
    }
    out.push_str(&format!("{name}:\n"));
    for (k, v) in shown {
        out.push_str(&format!("  {}: {}\n", key(k), scalar(v)));
    }
}

/// Write an agent as a file. `version` says which version this is, for the people reading it.
pub fn render(spec: &AgentSpec, version: Option<u32>) -> String {
    let mut out = String::from("---\n");
    out.push_str(&format!("zorix: {FORMAT}\n"));
    out.push_str(&format!("slug: {}\n", spec.slug));
    out.push_str(&format!("name: {}\n", quote(&spec.name)));
    out.push_str(&format!("description: {}\n", quote(&spec.description)));
    out.push_str(&format!("avatar:\n  emoji: {}\n  color: {}\n", quote(&spec.avatar.emoji), quote(&spec.avatar.color)));
    out.push_str(&format!("models:\n  default: {}\n", quote(&spec.models.default)));
    if spec.models.allowed.is_empty() {
        out.push_str("  allowed: []\n");
    } else {
        out.push_str("  allowed:\n");
        for pattern in &spec.models.allowed {
            out.push_str(&format!("    - {}\n", quote(pattern)));
        }
    }
    list(&mut out, "tools", &spec.tools);
    list(&mut out, "access", &spec.access);
    let skills: Vec<String> = spec.skills.iter().map(|pin| format!("{}@sha256:{}", pin.name, pin.hash)).collect();
    list(&mut out, "skills", &skills);
    map(
        &mut out,
        "limits",
        serde_json::to_value(spec.limits).ok().and_then(|v| v.as_object().cloned()).as_ref().unwrap_or(&Map::new()),
    );
    map(
        &mut out,
        "memory",
        serde_json::to_value(spec.memory).ok().and_then(|v| v.as_object().cloned()).as_ref().unwrap_or(&Map::new()),
    );
    out.push_str(&format!("agent_memory: {}\n", spec.agent_memory));
    map(
        &mut out,
        "params",
        serde_json::to_value(spec.params).ok().and_then(|v| v.as_object().cloned()).as_ref().unwrap_or(&Map::new()),
    );
    if let Some(schema) = &spec.expected_output {
        out.push_str(&format!("expected_output: {}\n", quote(&schema.to_string())));
    }
    if !spec.metadata.is_empty() {
        let entries: Map<String, Value> = spec.metadata.iter().map(|(k, v)| (k.clone(), json!(v))).collect();
        map(&mut out, "metadata", &entries);
    }
    if let Some(version) = version {
        out.push_str(&format!("version: {version}\n"));
    }
    out.push_str("---\n\n");
    out.push_str(&spec.instructions);
    out.push('\n');
    out
}

/// Split a file into its frontmatter and its body.
pub(crate) fn split(text: &str) -> Result<(String, String), String> {
    let text = text.replace("\r\n", "\n");
    let text = text.strip_prefix('\u{feff}').unwrap_or(&text);
    let rest = text
        .strip_prefix("---\n")
        .ok_or("An agent file starts with a line of three dashes, then the definition, then three dashes again.")?;
    let end = rest
        .match_indices("\n---")
        .find(|(at, _)| rest[at + 4..].is_empty() || rest[at + 4..].starts_with('\n'))
        .map(|(at, _)| at)
        .ok_or("The definition is not closed by a line of three dashes.")?;
    let yaml = &rest[..end + 1];
    let after = rest[end + 4..].strip_prefix('\n').unwrap_or("");
    let body = after.strip_prefix('\n').unwrap_or(after);
    let body = body.strip_suffix('\n').unwrap_or(body);
    Ok((yaml.to_string(), body.to_string()))
}

fn object<'a>(value: &'a Value, what: &str) -> Result<&'a Map<String, Value>, String> {
    value.as_object().ok_or_else(|| format!("{what} must be a group of settings."))
}

fn text(value: &Value, what: &str) -> Result<String, String> {
    match value {
        Value::String(s) => Ok(s.clone()),
        Value::Number(n) => Ok(n.to_string()),
        Value::Bool(b) => Ok(b.to_string()),
        _ => Err(format!("{what} must be text.")),
    }
}

fn strings(value: Option<&Value>, what: &str) -> Result<Vec<String>, String> {
    match value {
        None | Some(Value::Null) => Ok(Vec::new()),
        Some(Value::Array(items)) => items.iter().map(|i| text(i, what)).collect(),
        Some(_) => Err(format!("{what} must be a list.")),
    }
}

fn only(map: &Map<String, Value>, allowed: &[&str], what: &str) -> Result<(), String> {
    match map.keys().find(|k| !allowed.contains(&k.as_str())) {
        Some(unknown) => {
            Err(format!("{what} has a setting that is not known: {unknown}. Known: {}.", allowed.join(", ")))
        }
        None => Ok(()),
    }
}

/// Merge the settings a file gives over the defaults of a struct, refusing settings the struct does not have.
fn overlay<T: serde::de::DeserializeOwned + serde::Serialize>(
    defaults: &T,
    given: Option<&Value>,
    what: &str,
) -> Result<T, String> {
    let Some(given) = given.filter(|v| !v.is_null()) else {
        return serde_json::from_value(serde_json::to_value(defaults).unwrap_or_default()).map_err(|e| e.to_string());
    };
    let given = object(given, what)?;
    let mut base = serde_json::to_value(defaults).map_err(|e| e.to_string())?;
    let base_map = base.as_object_mut().ok_or("internal: not a map")?;
    // Optional fields serialize as null, so the known keys come from the struct's own definition: ask a fully populated one.
    for (k, v) in given {
        if !base_map.contains_key(k) && !optional_keys(what).contains(&k.as_str()) {
            let mut known: Vec<&str> = base_map.keys().map(String::as_str).collect();
            known.extend(optional_keys(what));
            known.sort_unstable();
            return Err(format!("{what} has a setting that is not known: {k}. Known: {}.", known.join(", ")));
        }
        base_map.insert(k.clone(), v.clone());
    }
    serde_json::from_value(base).map_err(|e| format!("{what} is not valid: {e}"))
}

fn optional_keys(what: &str) -> &'static [&'static str] {
    match what {
        "limits" => &["max_total_tokens", "max_cost_micros"],
        "params" => &["max_output_tokens", "effort"],
        _ => &[],
    }
}

/// Read an agent file. Skill names come back as references; use [`ParsedAgent::into_spec`] to pin them.
pub fn parse(file: &str) -> Result<ParsedAgent, String> {
    let (front, body) = split(file)?;
    let value = yaml::parse(&front)?;
    let top = object(&value, "The definition")?;
    only(top, KEYS, "The definition")?;
    match top.get("zorix").and_then(Value::as_str) {
        Some(FORMAT) => {}
        Some(other) => return Err(format!("This file is “{other}”; this version of ZORIX reads {FORMAT}.")),
        None => return Err(format!("The file does not say which format it is. Add `zorix: {FORMAT}`.")),
    }
    let slug = AgentSlug::new(text_of(top, "slug")?)?;
    let name = text_of(top, "name")?;
    let description = top.get("description").map(|v| text(v, "description")).transpose()?.unwrap_or_default();

    let avatar = match top.get("avatar") {
        None | Some(Value::Null) => Avatar::default(),
        Some(v) => {
            let m = object(v, "avatar")?;
            only(m, &["emoji", "color"], "avatar")?;
            let d = Avatar::default();
            Avatar {
                emoji: m.get("emoji").map(|v| text(v, "avatar.emoji")).transpose()?.unwrap_or(d.emoji),
                color: m.get("color").map(|v| text(v, "avatar.color")).transpose()?.unwrap_or(d.color),
            }
        }
    };
    let models = {
        let m = object(top.get("models").ok_or("The definition needs `models` with a `default`.")?, "models")?;
        only(m, &["default", "allowed"], "models")?;
        AgentModels {
            default: text(m.get("default").ok_or("models needs a `default`.")?, "models.default")?,
            allowed: strings(m.get("allowed"), "models.allowed")?,
        }
    };
    let skills = strings(top.get("skills"), "skills")?
        .into_iter()
        .map(|entry| match entry.split_once("@sha256:") {
            Some((name, hash)) => SkillRef { name: name.to_string(), hash: Some(hash.to_string()) },
            None => SkillRef { name: entry, hash: None },
        })
        .collect();
    let limits: Limits = overlay(&Limits::default(), top.get("limits"), "limits")?;
    let memory: MemoryOptions = overlay(&MemoryOptions::default(), top.get("memory"), "memory")?;
    let params = overlay(&zorix_core::GenerationParams::default(), top.get("params"), "params")?;
    let agent_memory = match top.get("agent_memory") {
        None | Some(Value::Null) => true,
        Some(Value::Bool(b)) => *b,
        Some(_) => return Err("agent_memory must be true or false.".into()),
    };
    let expected_output = match top.get("expected_output") {
        None | Some(Value::Null) => None,
        Some(Value::String(s)) => {
            Some(serde_json::from_str(s).map_err(|e| format!("expected_output is not JSON: {e}"))?)
        }
        Some(_) => return Err("expected_output is a JSON Schema written as text.".into()),
    };
    let metadata: BTreeMap<String, String> = match top.get("metadata") {
        None | Some(Value::Null) => BTreeMap::new(),
        Some(v) => object(v, "metadata")?
            .iter()
            .map(|(k, v)| Ok((k.clone(), text(v, "a metadata value")?)))
            .collect::<Result<_, String>>()?,
    };
    let version = match top.get("version") {
        None | Some(Value::Null) => None,
        Some(v) => Some(v.as_u64().and_then(|n| u32::try_from(n).ok()).ok_or("version must be a whole number.")?),
    };
    let spec = AgentSpec {
        slug,
        name,
        description,
        avatar,
        instructions: body,
        skills: Vec::new(),
        tools: strings(top.get("tools"), "tools")?,
        access: strings(top.get("access"), "access")?,
        models,
        limits,
        memory,
        agent_memory,
        params,
        expected_output,
        metadata,
    };
    Ok(ParsedAgent { spec, skills, version })
}

fn text_of(top: &Map<String, Value>, name: &str) -> Result<String, String> {
    text(top.get(name).ok_or_else(|| format!("The definition needs `{name}`."))?, name)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn spec() -> AgentSpec {
        let mut spec =
            AgentSpec::new(AgentSlug::new("code-reviewer").unwrap(), "Code reviewer", "anthropic/claude-opus-5-5");
        spec.description = "Reviews a change: what it does, what could break.".into();
        spec.instructions = "You review code.\n\n## Rules\n\n- Be specific.\n- Quote the line.\n\n---\n\nA rule line above is not the end.".into();
        spec.tools = vec!["math.*".into(), "clock.now".into()];
        spec.access = vec!["repo:acme/api".into(), "read:docs/*".into(), "mcp:github".into()];
        spec.models.allowed = vec!["anthropic/*".into(), "openai/gpt-5".into()];
        spec.skills = vec![SkillPin { name: "review-checklist".into(), hash: "ab".repeat(32) }];
        spec.limits.max_total_tokens = Some(200_000);
        spec.limits.max_cost_micros = Some(1_500_000);
        spec.limits.max_iterations = 20;
        spec.memory.knowledge = zorix_core::RecallMode::Off;
        spec.agent_memory = false;
        spec.params.max_output_tokens = Some(4096);
        spec.params.effort = Some(zorix_core::Effort::High);
        spec.expected_output =
            Some(json!({"type": "object", "properties": {"verdict": {"type": "string"}}, "required": ["verdict"]}));
        spec.metadata.insert("team".into(), "platform: infra".into());
        spec.metadata.insert("with space".into(), "line\none \"quoted\" é".into());
        spec.avatar = Avatar { emoji: "🔎".into(), color: "info".into() };
        spec
    }

    fn round(spec: &AgentSpec) -> AgentSpec {
        let parsed = parse(&render(spec, Some(7))).unwrap_or_else(|e| panic!("{e}\n{}", render(spec, Some(7))));
        assert_eq!(parsed.version, Some(7));
        parsed.into_spec(|_| None).unwrap()
    }

    #[test]
    fn what_is_written_reads_back_exactly() {
        let spec = spec();
        assert_eq!(round(&spec), spec);
        // And the file is stable: writing what was read gives the same text.
        let text = render(&spec, Some(7));
        assert_eq!(render(&round(&spec), Some(7)), text);
    }

    #[test]
    fn a_bare_agent_round_trips_and_awkward_text_survives() {
        let bare = AgentSpec::new(AgentSlug::new("a").unwrap(), "A", "demo");
        assert_eq!(round(&bare), bare);
        for odd in [
            "",
            "x",
            "\n",
            "trailing\n\n",
            "---",
            "# heading\n---\nafter",
            "unicode é 中 🙂",
            "  indented",
            "tab\tsep",
            "quote \" and ' and \\",
        ] {
            let mut s = AgentSpec::new(AgentSlug::new("odd").unwrap(), "Odd", "demo");
            s.instructions = odd.into();
            s.description = odd.replace('\n', " ");
            assert_eq!(round(&s), s, "{odd:?}");
        }
    }

    #[test]
    fn the_file_reads_like_a_document_a_person_can_edit() {
        let text = render(&spec(), None);
        assert!(text.starts_with("---\nzorix: agent/1\nslug: code-reviewer\nname: \"Code reviewer\"\n"), "{text}");
        assert!(text.contains("skills:\n  - \"review-checklist@sha256:"), "{text}");
        assert!(text.contains("\n---\n\nYou review code."), "{text}");
        assert!(!text.contains("version:"), "version is only written when asked");
    }

    #[test]
    fn a_skill_without_a_hash_takes_the_current_one_or_names_what_is_missing() {
        let file = "---\nzorix: agent/1\nslug: a\nname: A\nmodels:\n  default: demo\nskills:\n  - checklist\n  - \"pinned@sha256:".to_string() + &"c".repeat(64) + "\"\n---\n\nHi\n";
        let parsed = parse(&file).unwrap();
        assert_eq!(parsed.skills[0], SkillRef { name: "checklist".into(), hash: None });
        let spec = parsed.clone().into_spec(|name| (name == "checklist").then(|| "d".repeat(64))).unwrap();
        assert_eq!(spec.skills[0].hash, "d".repeat(64));
        assert_eq!(spec.skills[1].hash, "c".repeat(64));
        let error = parsed.into_spec(|_| None).unwrap_err();
        assert!(error.contains("checklist") && error.contains("not in the library"), "{error}");
    }

    #[test]
    fn mistakes_are_errors_that_say_what_to_fix() {
        let ok = "---\nzorix: agent/1\nslug: a\nname: A\nmodels:\n  default: demo\n---\n\nHi\n";
        assert!(parse(ok).is_ok());
        for (bad, wants) in [
            ("no frontmatter\n", "starts with a line of three dashes"),
            ("---\nzorix: agent/1\nslug: a\n", "not closed"),
            ("---\nslug: a\nname: A\nmodels:\n  default: demo\n---\n", "which format"),
            ("---\nzorix: agent/2\nslug: a\nname: A\nmodels:\n  default: demo\n---\n", "agent/2"),
            ("---\nzorix: agent/1\nname: A\nmodels:\n  default: demo\n---\n", "needs `slug`"),
            ("---\nzorix: agent/1\nslug: Bad Slug\nname: A\nmodels:\n  default: demo\n---\n", "not a valid agent name"),
            ("---\nzorix: agent/1\nslug: a\nname: A\nmodels:\n  default: demo\ncolour: red\n---\n", "colour"),
            (
                "---\nzorix: agent/1\nslug: a\nname: A\nmodels:\n  default: demo\nlimits:\n  max_ittrations: 3\n---\n",
                "max_ittrations",
            ),
            (
                "---\nzorix: agent/1\nslug: a\nname: A\nmodels:\n  default: demo\nmemory:\n  everything: true\n---\n",
                "everything",
            ),
            ("---\nzorix: agent/1\nslug: a\nname: A\nmodels:\n  nope: 1\n---\n", "nope"),
            ("---\nzorix: agent/1\nslug: a\nname: A\nmodels:\n  default: demo\nx: &a 1\n---\n", "anchors"),
        ] {
            let error = parse(bad).unwrap_err();
            assert!(error.contains(wants), "{bad:?} -> {error}");
        }
    }

    #[test]
    fn windows_line_endings_and_a_byte_order_mark_are_accepted() {
        let text = "\u{feff}---\r\nzorix: agent/1\r\nslug: a\r\nname: A\r\nmodels:\r\n  default: demo\r\n---\r\n\r\nLine one\r\nLine two\r\n";
        let spec = parse(text).unwrap().spec;
        assert_eq!(spec.instructions, "Line one\nLine two");
    }

    #[test]
    fn friendly_defaults_fill_what_a_file_leaves_out() {
        let text = "---\nzorix: agent/1\nslug: a\nname: A\nmodels:\n  default: demo\nlimits:\n  max_iterations: 3\nmemory:\n  knowledge: off\n---\n\nHi\n";
        let spec = parse(text).unwrap().spec;
        assert_eq!(spec.limits.max_iterations, 3);
        assert_eq!(spec.limits.max_tool_calls, Limits::default().max_tool_calls);
        assert_eq!(spec.memory.knowledge, zorix_core::RecallMode::Off);
        assert!(spec.memory.long_term, "the rest of memory keeps its defaults");
        assert!(spec.agent_memory);
        assert!(spec.models.allowed.is_empty());
    }
}
