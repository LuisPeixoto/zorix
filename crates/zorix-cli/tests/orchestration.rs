//! Teams, delegation and trees through the real program: `zorix team`, `zorix run --team`, `zorix tree`, and an approval asked by a
//! sub-run that is answered from the run you started.

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

    fn zorix(&self, args: &[&str]) -> Output {
        Command::new(env!("CARGO_BIN_EXE_zorix"))
            .arg("--home")
            .arg(self.path())
            .args(args)
            .env("ZORIX_SECRETS", "env")
            .env_remove("ZORIX_HOME")
            .stdin(Stdio::null())
            .output()
            .expect("start zorix")
    }

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

    fn script(&self, name: &str, turns: &str) -> String {
        let path = self.dir.path().join(format!("{name}.json"));
        std::fs::write(&path, turns).unwrap();
        format!("script:{}", path.display())
    }

    /// A lead, a helper and a team of them, all on the scripted model.
    fn duo(&self) {
        self.ok(&[
            "agent",
            "create",
            "boss",
            "--model",
            "script",
            "--instructions",
            "You lead.",
            "--tool",
            "notes.put",
        ]);
        self.ok(&[
            "agent",
            "create",
            "helper",
            "--model",
            "script",
            "--instructions",
            "You help.",
            "--tool",
            "notes.put",
        ]);
        self.ok(&["team", "create", "duo", "--lead", "boss", "--member", "helper", "--description", "Two agents."]);
    }
}

fn text(bytes: &[u8]) -> String {
    String::from_utf8_lossy(bytes).into_owned()
}

const DELEGATE: &str = r#"{"call": {"calls": [{"tool": "agents.delegate", "arguments": {"tasks": [{"agent": "helper", "task": "Find the answer."}]}}]}}"#;

#[test]
fn teams_are_made_shown_changed_listed_and_removed() {
    let home = Home::new();
    home.duo();
    let (shown, _) = home.ok(&["team", "show", "duo"]);
    assert!(
        shown.contains("Lead         boss")
            && shown.contains("Asks         helper")
            && shown.contains("zorix run --team duo"),
        "{shown}"
    );
    assert!(home.ok(&["team", "list"]).0.contains("duo") && home.ok(&["team", "list"]).0.contains("lead boss"));
    assert!(home.fails(&["team", "create", "duo", "--lead", "boss"]).contains("already a team"));
    assert!(
        home.fails(&["team", "create", "ghosts", "--lead", "boss", "--member", "ghost"])
            .contains("no agent called ghost")
    );
    assert!(
        home.fails(&["team", "create", "loop", "--lead", "boss", "--member", "boss"]).contains("not also a member")
    );

    home.ok(&["agent", "create", "extra", "--model", "script", "--instructions", "More."]);
    let (_, err) = home.ok(&["team", "edit", "duo", "--add-member", "extra", "--description", "Three agents."]);
    assert!(err.contains("Changed the team duo") && err.contains("asks 2"), "{err}");
    assert!(home.ok(&["team", "edit", "duo", "--add-member", "extra"]).1.contains("unchanged"));
    home.ok(&["team", "edit", "duo", "--remove-member", "extra"]);
    assert_eq!(home.json(&["team", "show", "duo"])["members"], serde_json::json!(["helper"]));

    // A member that is removed later is reported, and a run refuses to start on a team with a hole in it.
    home.ok(&["agent", "remove", "helper", "--yes"]);
    assert!(home.ok(&["team", "show", "duo"]).0.contains("Missing      helper"));
    assert!(home.fails(&["run", "--team", "duo", "--model", "demo", "hi"]).contains("not an agent any more"));
    assert!(home.fails(&["team", "remove", "duo"]).contains("--yes"));
    home.ok(&["team", "remove", "duo", "--yes"]);
    assert!(home.fails(&["team", "show", "duo"]).contains("no team called duo"));
    assert!(home.ok(&["agent", "list"]).0.contains("boss"), "the agents stay");
}

#[test]
fn a_ready_made_team_is_added_with_its_agents_and_runs_as_its_lead() {
    let home = Home::new();
    let templates = home.json(&["team", "templates"]);
    assert_eq!(templates[0]["template"], "research-desk");
    let (_, err) = home.ok(&["team", "add", "--template", "research-desk"]);
    assert!(
        err.contains("Added the team research-desk") && err.contains("research-lead") && err.contains("writer"),
        "{err}"
    );
    let agents = home.ok(&["agent", "list"]).0;
    for slug in ["research-lead", "researcher", "writer"] {
        assert!(agents.contains(slug), "{agents}");
    }
    assert!(home.ok(&["team", "add", "--template", "research-desk"]).1.contains("none, they were already here"));
    assert!(home.fails(&["team", "add", "--template", "nope"]).contains("no team template called nope"));
    // Its lead is told whom it may ask.
    let out = home.zorix(&["run", "--team", "research-desk", "--model", "demo", "Say hello."]);
    assert_eq!(out.status.code(), Some(0), "{}", text(&out.stderr));
    assert!(text(&out.stderr).contains("team research-desk, led by research-lead v1"), "{}", text(&out.stderr));
    let id = home.json(&["list"])[0]["id"].as_str().unwrap().to_string();
    assert!(home.ok(&["show", &id]).0.contains("(team research-desk)"));
}

#[test]
fn a_team_run_delegates_and_the_sub_run_is_followed_listed_and_drawn_as_a_tree() {
    let home = Home::new();
    home.duo();
    let model = home.script(
        "turns",
        &format!(r#"[{DELEGATE}, {{"say": "The helper found it."}}, {{"say": "Final: the helper found it."}}]"#),
    );
    let out = home.zorix(&["run", "--team", "duo", "--model", &model, "Please find the answer."]);
    let (stdout, stderr) = (text(&out.stdout), text(&out.stderr));
    assert_eq!(out.status.code(), Some(0), "{stderr}");
    assert!(stdout.contains("Final: the helper found it."), "{stdout}");
    assert!(stderr.contains("team duo, led by boss v1"), "{stderr}");
    assert!(stderr.contains("waiting_children") && stderr.contains("1 task(s) delegated"), "{stderr}");
    assert!(stderr.contains("↳ helper"), "the sub-run's events are shown under its agent: {stderr}");
    assert!(stderr.contains("child.settled") && stderr.contains("helper completed"), "{stderr}");

    // Only the run somebody asked for is listed, unless you ask for everything.
    let roots = home.json(&["list"]);
    assert_eq!(roots.as_array().unwrap().len(), 1);
    let root = roots[0]["id"].as_str().unwrap().to_string();
    let all = home.json(&["list", "--all"]);
    assert_eq!(all.as_array().unwrap().len(), 2);
    let child = all.as_array().unwrap().iter().find(|e| e["parent"].is_string()).unwrap();
    assert_eq!(
        (child["parent"].as_str(), child["root"].as_str(), child["depth"].as_u64()),
        (Some(root.as_str()), Some(root.as_str()), Some(1))
    );
    assert!(home.ok(&["list", "--all"]).0.contains("↳ "));

    let (tree, err) = home.ok(&["tree", &root]);
    let lines: Vec<&str> = tree.lines().collect();
    assert_eq!(lines.len(), 2, "{tree}");
    assert!(lines[0].contains("boss v1") && lines[1].starts_with("└─ ") && lines[1].contains("helper v1"), "{tree}");
    assert!(err.contains("2 execution(s)"), "{err}");
    // A sub-run's id draws the same tree, from the top.
    let child_id = child["id"].as_str().unwrap();
    assert_eq!(home.ok(&["tree", child_id]).0, tree);
    let shown = home.ok(&["show", &root]).0;
    assert!(shown.contains("Sub-runs   1") && shown.contains("(team duo)"), "{shown}");
    assert!(home.ok(&["show", child_id]).0.contains("Part of"));
    assert!(home.json(&["tree", &root]).as_array().unwrap().len() == 2);
}

#[test]
fn an_approval_asked_by_a_sub_run_is_answered_from_the_run_you_started() {
    let home = Home::new();
    home.duo();
    let first = home.script(
        "first",
        &format!(r#"[{DELEGATE}, {{"call": {{"calls": [{{"tool": "notes.put", "arguments": {{"key": "k", "value": "v"}}}}]}}}}]"#),
    );
    // Left to wait: the person is told which sub-run asks and how to answer it.
    let out = home.zorix(&["run", "--team", "duo", "--model", &first, "--approve", "never", "Save a note."]);
    assert_eq!(out.status.code(), Some(3), "{}", text(&out.stderr));
    let err = text(&out.stderr);
    assert!(err.contains("notes.put") && err.contains("zorix approve"), "{err}");
    let root = home.json(&["list"])[0]["id"].as_str().unwrap().to_string();
    assert!(home.ok(&["show", &root]).0.contains("waiting_for_children"));

    // Answering names the run you started, not the sub-run.
    let second = home.script("second", r#"[{"say": "Saved it."}, {"say": "Final: saved."}]"#);
    let out = home.zorix(&["approve", &root, "--model", &second]);
    assert_eq!(out.status.code(), Some(0), "{}{}", text(&out.stdout), text(&out.stderr));
    assert!(
        text(&out.stderr).contains("Approving notes.put") && text(&out.stderr).contains("asked by helper"),
        "{}",
        text(&out.stderr)
    );
    assert!(text(&out.stdout).contains("Final: saved."), "{}", text(&out.stdout));
    assert!(home.fails(&["approve", &root]).contains("not waiting for an approval"));
}

#[test]
fn run_options_that_do_not_fit_together_are_refused() {
    let home = Home::new();
    assert!(
        !home.zorix(&["run", "--agent", "a", "--team", "t", "hi"]).status.success(),
        "an agent or a team, not both"
    );
    assert!(home.fails(&["run", "--team", "nobody", "hi"]).contains("no team called nobody"));
    assert!(
        home.fails(&["tree", "01a1f6b3-5ab2-71d5-a20e-bba0ee5ba61c"]).contains("zorix"),
        "an unknown id is an error"
    );
}
