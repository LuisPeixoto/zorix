//! `zorix agent`, `zorix skill`, `zorix eval` and `zorix run --agent`, run as a real program on a throwaway data folder.

use std::path::PathBuf;
use std::process::{Command, Output, Stdio};

use serde_json::Value;

struct Home {
    dir: tempfile::TempDir,
}

impl Home {
    fn new() -> Self {
        Self { dir: tempfile::tempdir().unwrap() }
    }

    fn path(&self) -> PathBuf {
        self.dir.path().join("home")
    }

    fn zorix_env(&self, args: &[&str], env: &[(&str, &str)]) -> Output {
        Command::new(env!("CARGO_BIN_EXE_zorix"))
            .arg("--home")
            .arg(self.path())
            .args(args)
            .env("ZORIX_SECRETS", "env")
            .env_remove("ZORIX_HOME")
            .env_remove("EDITOR")
            .env_remove("VISUAL")
            .envs(env.iter().copied())
            .stdin(Stdio::null())
            .output()
            .expect("start zorix")
    }

    fn zorix(&self, args: &[&str]) -> Output {
        self.zorix_env(args, &[])
    }

    /// Run and require success; returns stdout and stderr together.
    fn ok(&self, args: &[&str]) -> (String, String) {
        let out = self.zorix(args);
        assert_eq!(out.status.code(), Some(0), "zorix {args:?}\n{}", text(&out.stderr));
        (text(&out.stdout), text(&out.stderr))
    }

    fn json(&self, args: &[&str]) -> Value {
        let mut all = vec!["--json"];
        all.extend_from_slice(args);
        let (stdout, _) = self.ok(&all);
        serde_json::from_str(stdout.lines().last().unwrap_or("null")).unwrap_or_else(|e| panic!("{e}: {stdout}"))
    }

    fn fails(&self, args: &[&str]) -> String {
        let out = self.zorix(args);
        assert_ne!(out.status.code(), Some(0), "zorix {args:?} should fail");
        text(&out.stderr) + &text(&out.stdout)
    }
}

fn text(bytes: &[u8]) -> String {
    String::from_utf8_lossy(bytes).into_owned()
}

fn write(home: &Home, name: &str, content: &str) -> String {
    let path = home.dir.path().join(name);
    std::fs::create_dir_all(path.parent().unwrap()).unwrap();
    std::fs::write(&path, content).unwrap();
    path.display().to_string()
}

const CALC: &[&str] = &[
    "agent",
    "create",
    "calc",
    "--name",
    "Calculator",
    "--description",
    "Does arithmetic.",
    "--model",
    "demo",
    "--instructions",
    "You calculate carefully.",
    "--tool",
    "math.evaluate",
];

#[test]
fn templates_are_listed_added_once_and_shown_with_their_skills_and_cases() {
    let home = Home::new();
    let listed = home.json(&["agent", "templates"]);
    let ids: Vec<&str> = listed.as_array().unwrap().iter().map(|t| t["template"].as_str().unwrap()).collect();
    assert_eq!(ids, ["researcher", "code-reviewer", "data-analyst", "writer"]);
    assert!(home.ok(&["agent", "list"]).1.contains("No agents yet"));

    let added = home.json(&["agent", "add", "--template", "researcher"]);
    assert_eq!(
        (added["agent"].as_str(), added["created"].as_bool(), added["cases"].as_u64()),
        (Some("researcher"), Some(true), Some(3))
    );
    assert_eq!(added["skills"][0], "citing-sources");
    let again = home.json(&["agent", "add", "--template", "researcher"]);
    assert_eq!(again["created"], false, "adding twice makes nothing new");
    assert_eq!(again["version"], 1);

    let (out, _) = home.ok(&["agent", "list"]);
    assert!(out.contains("researcher") && out.contains("v1"), "{out}");
    let (out, _) = home.ok(&["agent", "show", "researcher"]);
    assert!(
        out.contains("citing-sources@") && out.contains("en.wikipedia.org") && out.contains("careful researcher"),
        "{out}"
    );
    let (out, _) = home.ok(&["eval", "list", "researcher"]);
    assert!(out.contains("cites-its-sources") && out.contains("says-when-it-found-nothing"), "{out}");
    let (out, _) = home.ok(&["skill", "list"]);
    assert!(out.contains("citing-sources"), "{out}");
    assert!(home.fails(&["agent", "add", "--template", "nope"]).contains("no template called nope"));

    // A copy under another name.
    home.ok(&["agent", "add", "--template", "researcher", "--as", "my-researcher"]);
    assert!(home.ok(&["agent", "list"]).0.contains("my-researcher"));
}

#[test]
fn an_agent_can_be_exported_imported_versioned_compared_rolled_back_archived_and_removed() {
    let home = Home::new();
    home.ok(CALC);
    assert!(home.fails(CALC).contains("already an agent"));

    let file = home.dir.path().join("calc.md");
    home.ok(&["agent", "export", "calc", "--output", &file.display().to_string()]);
    let exported = std::fs::read_to_string(&file).unwrap();
    assert!(
        exported.starts_with("---\nzorix: agent/1\nslug: calc\n") && exported.contains("You calculate carefully."),
        "{exported}"
    );

    // The same file imports as no change; an edited one makes v2; --as makes a copy.
    let same = home.json(&["agent", "import", &file.display().to_string()]);
    assert_eq!((same["created"].as_bool(), same["changed"].as_bool()), (Some(false), Some(false)));
    let edited = write(&home, "calc-edited.md", &exported.replace("carefully.", "carefully and show your working."));
    let second = home.json(&["agent", "import", &edited, "--note", "show the working"]);
    assert_eq!((second["version"].as_u64(), second["changed"].as_bool()), (Some(2), Some(true)));
    let copy = home.json(&["agent", "import", &file.display().to_string(), "--as", "calc-two"]);
    assert_eq!(copy["created"], true);

    let (versions, _) = home.ok(&["agent", "versions", "calc"]);
    assert!(
        versions.contains("v2") && versions.contains("← current") && versions.contains("show the working"),
        "{versions}"
    );
    let (diff, _) = home.ok(&["agent", "diff", "calc", "1", "2"]);
    assert!(diff.contains("+ ") && diff.contains("show your working") && diff.contains("- "), "{diff}");
    assert!(home.ok(&["agent", "diff", "calc", "1", "1"]).1.contains("are the same"));

    let rolled = home.json(&["agent", "rollback", "calc", "1"]);
    assert_eq!(rolled["version"], 3);
    assert!(!home.ok(&["agent", "export", "calc"]).0.contains("show your working"));
    assert!(home.fails(&["agent", "rollback", "calc", "9"]).contains("no version 9"));

    // Archived agents leave the list and come back; removing needs --yes.
    home.ok(&["agent", "archive", "calc-two"]);
    assert!(!home.ok(&["agent", "list"]).0.contains("calc-two"));
    assert!(home.ok(&["agent", "list", "--all"]).0.contains("calc-two  "), "archived agents show with --all");
    home.ok(&["agent", "archive", "calc-two", "--undo"]);
    assert!(home.ok(&["agent", "list"]).0.contains("calc-two"));
    assert!(home.fails(&["agent", "remove", "calc-two"]).contains("--yes"));
    home.ok(&["agent", "remove", "calc-two", "--yes"]);
    assert!(home.fails(&["agent", "show", "calc-two"]).contains("no agent called calc-two"));

    // Bad input is refused with a reason.
    assert!(!home.zorix(&["agent", "show", "Not A Slug"]).status.success());
    assert!(home.fails(&["agent", "create", "bare"]).contains("--instructions"));
    let bad = write(&home, "bad.md", "no header at all");
    assert!(home.fails(&["agent", "import", &bad]).contains("three dashes"));
}

#[cfg(unix)]
#[test]
fn edit_opens_the_editor_and_a_saved_change_is_a_new_version() {
    let home = Home::new();
    home.ok(CALC);
    assert!(home.fails(&["agent", "edit", "calc"]).contains("$EDITOR"));
    let script = write(&home, "append.sh", "printf '\\nAlways say the units.\\n' >> \"$1\"\n");
    let editor = format!("sh {script}");
    let out = home.zorix_env(&["agent", "edit", "calc"], &[("EDITOR", &editor)]);
    assert_eq!(out.status.code(), Some(0), "{}", text(&out.stderr));
    assert!(text(&out.stderr).contains("now at v2"), "{}", text(&out.stderr));
    assert!(home.ok(&["agent", "export", "calc"]).0.contains("Always say the units."));

    // An editor that changes nothing, or fails, leaves things as they are.
    let out = home.zorix_env(&["agent", "edit", "calc"], &[("EDITOR", "true")]);
    assert!(text(&out.stderr).contains("unchanged"), "{}", text(&out.stderr));
    let out = home.zorix_env(&["agent", "edit", "calc"], &[("EDITOR", "false")]);
    assert_ne!(out.status.code(), Some(0));
    assert!(home.ok(&["agent", "versions", "calc"]).0.contains("v2"));
    assert!(!home.ok(&["agent", "versions", "calc"]).0.contains("v3"));
}

#[test]
fn a_skill_folder_is_imported_listed_shown_exported_with_an_agent_and_cleaned_up() {
    let home = Home::new();
    write(
        &home,
        "skills/greeting/SKILL.md",
        "---\nname: greeting\ndescription: \"How to greet people. Use at the start of a chat.\"\nlicense: MIT\n---\n\n# Greeting\n\nSay hello first.\n",
    );
    write(&home, "skills/greeting/references/tone.md", "Be warm.\n");
    let folder = home.dir.path().join("skills/greeting").display().to_string();
    let (_, err) = home.ok(&["skill", "import", &folder]);
    assert!(err.contains("greeting imported") && err.contains("2 files"), "{err}");
    let (out, _) = home.ok(&["skill", "show", "greeting"]);
    assert!(
        out.contains("How to greet people") && out.contains("references/tone.md") && out.contains("License"),
        "{out}"
    );
    assert!(home.ok(&["skill", "list"]).0.contains("greeting"));

    // A folder whose name does not match, or without SKILL.md, is refused with a reason.
    write(&home, "skills/other/SKILL.md", "---\nname: not-other\ndescription: x\n---\nbody\n");
    assert!(
        home.fails(&["skill", "import", &home.dir.path().join("skills/other").display().to_string()]).contains("other")
    );
    write(&home, "skills/empty/README.md", "nothing");
    assert!(
        home.fails(&["skill", "import", &home.dir.path().join("skills/empty").display().to_string()])
            .contains("SKILL.md")
    );

    // An agent that names it pins it; its folder travels with the export; importing elsewhere restores it.
    home.ok(&["agent", "create", "greeter", "--model", "demo", "--instructions", "Greet.", "--skill", "greeting"]);
    assert!(home.ok(&["agent", "show", "greeter"]).0.contains("greeting@"));
    let out_dir = home.dir.path().join("travel");
    let file = out_dir.join("greeter.md");
    std::fs::create_dir_all(&out_dir).unwrap();
    home.ok(&[
        "agent",
        "export",
        "greeter",
        "-o",
        &file.display().to_string(),
        "--with-skills",
        &out_dir.display().to_string(),
    ]);
    assert!(out_dir.join("greeting/SKILL.md").is_file() && out_dir.join("greeting/references/tone.md").is_file());

    let other = Home::new();
    assert!(other.fails(&["agent", "import", &file.display().to_string()]).contains("not in the library"));
    let (_, err) =
        other.ok(&["agent", "import", &file.display().to_string(), "--skills", &out_dir.display().to_string()]);
    assert!(err.contains("Skill greeting imported") && err.contains("Imported greeter"), "{err}");

    // Removing a skill keeps the agent working; gc only deletes what nothing names.
    home.ok(&["skill", "remove", "greeting"]);
    assert!(home.ok(&["skill", "gc"]).1.contains("0 skill folder(s) deleted"), "still pinned by greeter");
    home.ok(&["agent", "remove", "greeter", "--yes"]);
    assert!(home.ok(&["skill", "gc"]).1.contains("1 skill folder(s) deleted"));
    assert!(home.fails(&["skill", "show", "greeting"]).contains("no skill called greeting"));
}

#[test]
fn run_agent_uses_the_agent_records_its_version_and_flags_only_narrow_it() {
    let home = Home::new();
    home.ok(CALC);
    let out = home.zorix(&["run", "--agent", "calc", "What is 17*23?"]);
    assert_eq!(out.status.code(), Some(0), "{}", text(&out.stderr));
    assert!(text(&out.stdout).contains("391"), "{}", text(&out.stdout));
    assert!(text(&out.stderr).contains("calc v1"), "{}", text(&out.stderr));

    let listed = home.json(&["list", "--agent", "calc"]);
    assert_eq!(listed.as_array().map(Vec::len), Some(1), "{listed}");
    let id = listed[0]["id"].as_str().unwrap().to_string();
    assert_eq!(listed[0]["agent"]["slug"], "calc");
    let (shown, _) = home.ok(&["show", &id]);
    assert!(shown.contains("Agent      calc v1"), "{shown}");
    assert_eq!(home.json(&["list", "--agent", "calc", "--limit", "5"]).as_array().unwrap().len(), 1);
    assert!(home.fails(&["list", "--agent", "nobody"]).contains("no agent called nobody"));

    // A flag that allows something else narrows the agent: it keeps only what both allow, so math is gone.
    let out = home.zorix(&["run", "--agent", "calc", "--allow", "clock.now", "What is 17*23?"]);
    assert!(!text(&out.stdout).contains("391"), "{}", text(&out.stdout));

    // A new version is used from then on, and an older one can be asked for.
    let file =
        write(&home, "v2.md", &home.ok(&["agent", "export", "calc"]).0.replace("carefully.", "carefully, always."));
    home.ok(&["agent", "import", &file]);
    let out = home.zorix(&["run", "--agent", "calc", "--agent-version", "1", "What is 17*23?"]);
    assert!(text(&out.stderr).contains("calc v1"), "{}", text(&out.stderr));
    let out = home.zorix(&["run", "--agent", "calc", "What is 17*23?"]);
    assert!(text(&out.stderr).contains("calc v2"), "{}", text(&out.stderr));

    // Mistakes are explained.
    assert!(home.fails(&["run", "--agent", "nobody", "hi"]).contains("no agent called nobody"));
    assert!(home.fails(&["run", "--agent", "calc", "--instructions", "x", "hi"]).contains("own instructions"));
    assert!(!home.zorix(&["run", "--agent-version", "1", "hi"]).status.success(), "--agent-version needs --agent");
    // A model the agent may use is chosen with --model; one that cannot be reached is a clear failure, not a hang.
    assert!(!home.zorix(&["run", "--agent", "calc", "--model", "anthropic/claude-opus-5-5", "hi"]).status.success());
}

#[test]
fn a_template_runs_through_its_skills_and_an_evaluation_run_is_kept_and_compared() {
    let home = Home::new();
    home.ok(&["agent", "add", "--template", "writer"]);
    let out = home.zorix(&["run", "--agent", "writer", "--model", "demo", "Say hello."]);
    assert_eq!(out.status.code(), Some(0), "{}", text(&out.stderr));
    assert!(text(&out.stderr).contains("writer v1"));

    // An agent with a case that holds, and one that cannot.
    home.ok(CALC);
    let cases = write(
        &home,
        "cases.json",
        r#"[
          {"name": "multiplies", "prompt": "What is 17*23?", "tags": ["smoke"],
           "checks": [{"kind": "contains", "text": "391"}, {"kind": "tool_used", "pattern": "math.*"}]},
          {"name": "impossible", "prompt": "What is 17*23?", "checks": [{"kind": "contains", "text": "banana"}]}
        ]"#,
    );
    let (_, err) = home.ok(&["eval", "add", "calc", &cases]);
    assert!(err.contains("2 case(s) saved"), "{err}");
    assert!(home.ok(&["eval", "list", "calc"]).0.contains("multiplies"));
    let invalid = write(&home, "invalid.json", r#"{"name": "no-checks", "prompt": "x"}"#);
    assert!(home.fails(&["eval", "add", "calc", &invalid]).contains("at least one check"));

    let out = home.zorix(&["eval", "run", "calc", "--tag", "smoke"]);
    assert_eq!(out.status.code(), Some(0), "{}{}", text(&out.stdout), text(&out.stderr));
    assert!(
        text(&out.stdout).contains("PASS  multiplies") && text(&out.stdout).contains("1/1 passed"),
        "{}",
        text(&out.stdout)
    );

    let out = home.zorix(&["eval", "run", "calc"]);
    assert_eq!(out.status.code(), Some(1), "a failing case fails the command");
    let shown = text(&out.stdout);
    assert!(
        shown.contains("PASS  multiplies") && shown.contains("FAIL  impossible") && shown.contains("banana"),
        "{shown}"
    );
    assert!(shown.contains("1/2 passed"), "{shown}");

    let runs = home.json(&["eval", "runs", "calc"]);
    let runs = runs.as_array().unwrap();
    assert_eq!(runs.len(), 2);
    let (newest, older) = (runs[0]["id"].as_str().unwrap(), runs[1]["id"].as_str().unwrap());
    let (out, _) = home.ok(&["eval", "show", newest]);
    assert!(out.contains("calc v1") && out.contains("impossible"), "{out}");
    let (out, _) = home.ok(&["eval", "compare", older, newest]);
    assert!(out.contains("multiplies") && out.contains("impossible"), "{out}");
    assert!(home.fails(&["eval", "show", "not-an-id"]).contains("not an evaluation run id"));
    home.ok(&["eval", "remove", "calc", "impossible"]);
    assert!(home.fails(&["eval", "remove", "calc", "impossible"]).contains("no case called"));
    assert_eq!(home.zorix(&["eval", "run", "calc", "--offline"]).status.code(), Some(0));
}

#[test]
fn memory_can_be_scoped_to_an_agent_by_name() {
    let home = Home::new();
    home.ok(CALC);
    home.ok(&["memory", "add", "Always give units.", "--scope", "agent:calc"]);
    let (listed, _) = home.ok(&["memory", "list", "--scope", "agent:calc"]);
    assert!(listed.contains("Always give units."), "{listed}");
    assert!(!home.ok(&["memory", "list", "--scope", "global"]).0.contains("Always give units."));
    assert!(home.fails(&["memory", "add", "x", "--scope", "agent:nobody"]).contains("nobody"));
}
