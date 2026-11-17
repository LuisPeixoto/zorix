//! The command line, run as a real program against a throwaway data folder.

use std::io::Write;
use std::path::PathBuf;
use std::process::{Command, Output, Stdio};
use std::sync::mpsc;
use std::time::Duration;

use serde_json::Value;

struct Zorix {
    dir: tempfile::TempDir,
}

fn text(bytes: &[u8]) -> String {
    String::from_utf8_lossy(bytes).into_owned()
}

impl Zorix {
    fn new() -> Self {
        Self { dir: tempfile::tempdir().unwrap() }
    }

    fn home(&self) -> PathBuf {
        self.dir.path().join("home")
    }

    /// Run `zorix --home <home> <args>` and wait, but never forever.
    fn run_with(&self, args: &[&str], stdin: &str) -> Output {
        let mut command = Command::new(env!("CARGO_BIN_EXE_zorix"));
        command
            .arg("--home")
            .arg(self.home())
            .args(args)
            .env_remove("ZORIX_HOME")
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped());
        let mut child = command.spawn().expect("start zorix");
        let mut pipe = child.stdin.take().unwrap();
        let input = stdin.to_string();
        std::thread::spawn(move || {
            let _ = pipe.write_all(input.as_bytes());
        });
        let (sender, receiver) = mpsc::channel();
        std::thread::spawn(move || {
            let _ = sender.send(child.wait_with_output());
        });
        receiver.recv_timeout(Duration::from_secs(60)).expect("zorix finished within a minute").expect("output")
    }

    fn run(&self, args: &[&str]) -> Output {
        self.run_with(args, "")
    }

    /// The id of the newest execution.
    fn newest_id(&self) -> String {
        let out = self.run(&["list", "--json"]);
        let list: Value = serde_json::from_slice(&out.stdout).expect("list json");
        list[0]["id"].as_str().expect("an execution").to_string()
    }

    fn write_script(&self, turns: &str) -> String {
        std::fs::create_dir_all(self.dir.path()).unwrap();
        let path = self.dir.path().join("script.json");
        std::fs::write(&path, turns).unwrap();
        format!("script:{}", path.display())
    }
}

fn code(output: &Output) -> i32 {
    output.status.code().expect("exited normally")
}

#[test]
fn data_dir_prints_the_folder() {
    let z = Zorix::new();
    let out = z.run(&["data-dir"]);
    assert_eq!(code(&out), 0);
    assert_eq!(text(&out.stdout).trim(), z.home().display().to_string());
}

#[test]
fn a_bad_command_line_is_refused_before_the_data_folder_is_touched() {
    let z = Zorix::new();
    for args in [
        vec!["show", "not-an-id"],
        vec!["run", ""],
        vec!["run", "x", "--max-iterations", "0"],
        vec!["run", "x", "--allow", "bad tool"],
        vec!["run", "x", "--allow-read", "../etc"],
        vec!["run", "x", "--model", "GPT 5"],
        vec!["list", "--state", "sleeping"],
        vec!["approve", "xyz"],
        vec!["--tenant", "a/b", "list"],
    ] {
        let out = z.run(&args);
        assert_eq!(code(&out), 2, "{args:?}: {}", text(&out.stderr));
    }
    assert!(!z.home().exists(), "nothing was created");
    let out = z.run(&["show", "not-an-id"]);
    assert!(text(&out.stderr).contains("not a valid ExecutionId"), "{}", text(&out.stderr));
}

#[test]
fn the_tools_are_listed_with_how_careful_they_are() {
    let z = Zorix::new();
    let out = z.run(&["tools"]);
    assert_eq!(code(&out), 0);
    let listing = text(&out.stdout);
    assert!(listing.contains("math.evaluate") && listing.contains("read-only"));
    let put = listing.lines().find(|l| l.starts_with("notes.put")).unwrap();
    assert!(put.contains("asks first"), "{put}");
    let json = z.run(&["--json", "tools"]);
    assert_eq!(serde_json::from_slice::<Value>(&json.stdout).unwrap().as_array().unwrap().len(), 8);
}

#[test]
fn a_run_that_needs_no_approval_prints_the_answer_and_the_story() {
    let z = Zorix::new();
    let out = z.run(&["run", "What is 17*23?", "--allow", "math.evaluate"]);
    assert_eq!(code(&out), 0, "{}", text(&out.stderr));
    assert!(text(&out.stdout).contains("391"), "{}", text(&out.stdout));
    let story = text(&out.stderr);
    for wanted in ["created", "model.requested", "tool.completed", "completed"] {
        assert!(story.contains(wanted), "{wanted} in\n{story}");
    }
    assert!(!story.contains(" 0 tokens"), "token counts must survive redaction:\n{story}");
}

#[test]
fn the_whole_acceptance_scenario_across_separate_processes() {
    let z = Zorix::new();
    // 1. The activity runs, calls a tool, then needs a person for the second tool.
    let first = z.run(&[
        "run",
        "What is 17*23 and save it as result",
        "--allow",
        "math.evaluate",
        "--allow",
        "notes.put",
        "--approve",
        "never",
    ]);
    assert_eq!(code(&first), 3, "exit 3 means waiting for approval:\n{}", text(&first.stderr));
    assert!(text(&first.stderr).contains("zorix approve"), "it says how to answer");
    let id = z.newest_id();

    let listed = text(&z.run(&["list"]).stdout);
    assert!(listed.contains("waiting_for_approval") && listed.contains(&id), "{listed}");
    let shown = text(&z.run(&["show", &id]).stdout);
    assert!(shown.contains("Waiting    approval") && shown.contains("notes.put"), "{shown}");

    // 2. A different process answers. The execution continues from the saved state.
    let approved = z.run(&["approve", &id]);
    assert_eq!(code(&approved), 0, "{}", text(&approved.stderr));
    assert!(text(&approved.stdout).contains("Done.") && text(&approved.stdout).contains("notes.put"));

    // 3. The record is complete and in order.
    let events = text(&z.run(&["events", &id]).stdout);
    let order = [
        "created",
        "queued",
        "started",
        "tool.completed",
        "waiting_approval",
        "approval.granted",
        "tool.completed",
        "completed",
    ];
    let mut at = 0;
    for line in events.lines() {
        if at < order.len() && line.contains(order[at]) {
            at += 1;
        }
    }
    assert_eq!(at, order.len(), "events out of order:\n{events}");

    let json: Value = serde_json::from_slice(&z.run(&["--json", "show", &id]).stdout).unwrap();
    assert_eq!(json["execution"]["state"], "completed");
    assert_eq!(json["summary"]["tool_calls"].as_array().unwrap().len(), 2);
    assert!(json["summary"]["usage"]["input_tokens"].as_u64().unwrap() > 0);

    // 4. What was saved is still there for a later run.
    let recall = z.run(&["run", "recall result", "--allow", "notes.get"]);
    assert!(text(&recall.stdout).contains("17*23 = 391"), "{}", text(&recall.stdout));
}

#[test]
fn refusing_an_approval_means_the_tool_never_runs() {
    let z = Zorix::new();
    z.run(&[
        "run",
        "What is 2+2 and save it as sum",
        "--allow",
        "math.evaluate",
        "--allow",
        "notes.put",
        "--approve",
        "never",
    ]);
    let id = z.newest_id();
    let denied = z.run(&["deny", &id, "--note", "not today"]);
    assert_eq!(code(&denied), 0, "{}", text(&denied.stderr));
    assert!(text(&denied.stdout).contains("could not complete"), "{}", text(&denied.stdout));
    assert!(text(&denied.stderr).contains("approval.denied"));
    let recall = z.run(&["run", "recall sum", "--allow", "notes.get"]);
    assert!(
        text(&recall.stdout).contains("There is no note called sum"),
        "nothing was saved:\n{}",
        text(&recall.stdout)
    );
}

#[test]
fn approval_can_be_answered_on_the_terminal() {
    for (answer, saved) in [("y\n", true), ("n\n", false), ("", false)] {
        let z = Zorix::new();
        let out = z.run_with(
            &[
                "run",
                "What is 3+4 and save it as seven",
                "--allow",
                "math.evaluate",
                "--allow",
                "notes.put",
                "--approve",
                "ask",
            ],
            answer,
        );
        assert_eq!(code(&out), 0, "answer {answer:?}: {}", text(&out.stderr));
        assert!(text(&out.stderr).contains("Allow this?"));
        let recall = z.run(&["run", "recall seven", "--allow", "notes.get"]);
        assert_eq!(text(&recall.stdout).contains("3+4 = 7"), saved, "answer {answer:?}");
    }
}

#[test]
fn automatic_modes_are_explicit() {
    let z = Zorix::new();
    let yes = z.run(&[
        "run",
        "What is 5+5 and save it as ten",
        "--allow",
        "math.evaluate",
        "--allow",
        "notes.put",
        "--approve",
        "yes",
    ]);
    assert_eq!(code(&yes), 0);
    assert!(text(&yes.stderr).contains("approval.granted") && text(&yes.stderr).contains("cli:auto"));
    let no = z.run(&[
        "run",
        "What is 6+6 and save it as twelve",
        "--allow",
        "math.evaluate",
        "--allow",
        "notes.put",
        "--approve",
        "no",
    ]);
    assert!(text(&no.stderr).contains("approval.denied"));
}

#[test]
fn a_tool_the_execution_was_not_given_is_refused_even_when_the_model_asks() {
    let z = Zorix::new();
    let model = z.write_script(
        r#"[{"call": {"calls": [{"tool": "math.evaluate", "arguments": {"expression": "1+1"}}]}}, {"say": "I was blocked."}]"#,
    );
    let out = z.run(&["run", "do math", "--model", &model]);
    assert_eq!(code(&out), 0, "{}", text(&out.stderr));
    let story = text(&out.stderr);
    assert!(story.contains("tool.denied") && story.contains("no permission"), "{story}");
    assert!(!story.contains("tool.completed"));
}

#[test]
fn limits_stop_a_loop_and_the_exit_code_says_it_failed() {
    let z = Zorix::new();
    let call = r#"{"call": {"calls": [{"tool": "clock.now", "arguments": {}}]}}"#;
    let model = z.write_script(&format!("[{}]", [call; 10].join(",")));
    let out = z.run(&["run", "loop forever", "--model", &model, "--allow", "clock.now", "--max-iterations", "2"]);
    assert_eq!(code(&out), 1);
    assert!(text(&out.stderr).contains("Stopped after 2 model steps"), "{}", text(&out.stderr));
}

#[test]
fn file_tools_stay_inside_the_workspace_and_need_permission_and_approval() {
    let z = Zorix::new();
    let workspace = z.dir.path().join("work");
    std::fs::create_dir_all(&workspace).unwrap();
    let ws = workspace.to_str().unwrap();
    let escape = z.dir.path().join("escape.txt");

    let ok = z.run(&[
        "run",
        "Write \"hello\" to out.txt",
        "--workspace",
        ws,
        "--allow",
        "workspace.write",
        "--allow-write",
        "out.txt",
        "--approve",
        "yes",
    ]);
    assert_eq!(code(&ok), 0, "{}", text(&ok.stderr));
    assert_eq!(std::fs::read_to_string(workspace.join("out.txt")).unwrap(), "hello");

    let outside = z.run(&[
        "run",
        "Write \"x\" to ../escape.txt",
        "--workspace",
        ws,
        "--allow",
        "workspace.write",
        "--allow-write",
        "*",
        "--approve",
        "yes",
    ]);
    assert!(text(&outside.stderr).contains("may not go up"), "{}", text(&outside.stderr));
    assert!(!escape.exists(), "nothing was written outside the workspace");

    let no_grant = z.run(&[
        "run",
        "Write \"y\" to other.txt",
        "--workspace",
        ws,
        "--allow",
        "workspace.write",
        "--approve",
        "yes",
    ]);
    assert!(text(&no_grant.stderr).contains("no permission"), "{}", text(&no_grant.stderr));
    assert!(!workspace.join("other.txt").exists());

    let none = z.run(&[
        "run",
        "Write \"z\" to third.txt",
        "--allow",
        "workspace.write",
        "--allow-write",
        "*",
        "--approve",
        "yes",
    ]);
    assert!(text(&none.stderr).contains("No workspace folder"), "{}", text(&none.stderr));

    let missing = z.run(&["run", "x", "--workspace", z.dir.path().join("nope").to_str().unwrap()]);
    assert_eq!(code(&missing), 1);
    assert!(text(&missing.stderr).contains("cannot be used"));
}

#[test]
fn the_same_idempotency_key_runs_once() {
    let z = Zorix::new();
    let args = ["run", "What is 9*9?", "--allow", "math.evaluate", "--idempotency-key", "nine"];
    let first = z.run(&args);
    let second = z.run(&args);
    assert_eq!((code(&first), code(&second)), (0, 0));
    assert!(text(&second.stdout).contains("81"));
    let list: Value = serde_json::from_slice(&z.run(&["list", "--json"]).stdout).unwrap();
    assert_eq!(list.as_array().unwrap().len(), 1);
    assert_eq!(text(&second.stderr).matches("model.requested").count(), 2, "the second run only replayed the record");
}

#[test]
fn json_mode_prints_one_json_value_per_line_and_ends_with_the_result() {
    let z = Zorix::new();
    let out = z.run(&["--json", "run", "What is 2*21?", "--allow", "math.evaluate"]);
    assert_eq!(code(&out), 0);
    let lines: Vec<Value> = text(&out.stdout).lines().map(|l| serde_json::from_str(l).expect("a JSON line")).collect();
    assert!(lines.len() > 5);
    assert_eq!(lines.last().unwrap()["type"], "result");
    assert_eq!(lines.last().unwrap()["execution"]["state"], "completed");
    assert!(lines[..lines.len() - 1].iter().all(|l| l["kind"].as_str().unwrap().starts_with("execution.")));
}

#[test]
fn a_waiting_execution_can_be_cancelled_and_events_can_be_followed_to_the_end() {
    let z = Zorix::new();
    z.run(&[
        "run",
        "What is 1+1 and save it as two",
        "--allow",
        "math.evaluate",
        "--allow",
        "notes.put",
        "--approve",
        "never",
    ]);
    let id = z.newest_id();
    let follow = z.run(&["events", &id, "--follow"]);
    assert_eq!(code(&follow), 0, "it stops when the execution waits for a person");
    assert!(text(&follow.stdout).contains("waiting_approval"));

    let cancelled = z.run(&["cancel", &id, "--reason", "changed my mind"]);
    assert_eq!(code(&cancelled), 4, "{}", text(&cancelled.stderr));
    let json: Value = serde_json::from_slice(&z.run(&["--json", "show", &id]).stdout).unwrap();
    assert_eq!(json["execution"]["state"], "cancelled");
    assert_eq!(json["execution"]["cancel_reason"], "changed my mind");
    let again = z.run(&["cancel", &id]);
    assert_eq!(code(&again), 1, "a finished execution cannot be cancelled again");
}

#[test]
fn approving_something_that_is_not_waiting_explains_itself() {
    let z = Zorix::new();
    z.run(&["run", "What is 1+2?", "--allow", "math.evaluate"]);
    let id = z.newest_id();
    let out = z.run(&["approve", &id]);
    assert_eq!(code(&out), 1);
    assert!(text(&out.stderr).contains("not waiting for an approval"), "{}", text(&out.stderr));
}

#[test]
fn recovery_with_nothing_to_do_says_so() {
    let z = Zorix::new();
    let out = z.run(&["recover"]);
    assert_eq!(code(&out), 0);
    assert!(text(&out.stderr).contains("Nothing to recover."));
}

#[test]
fn only_one_zorix_program_works_on_a_data_folder_at_a_time() {
    let z = Zorix::new();
    let _held = zorix_store::DataDirLock::acquire(&z.home()).unwrap();
    let out = z.run(&["run", "What is 1+1?", "--allow", "math.evaluate"]);
    assert_eq!(code(&out), 1);
    assert!(text(&out.stderr).contains("Another ZORIX program is using the data folder"), "{}", text(&out.stderr));
    // Reading is allowed while someone else works.
    assert_eq!(code(&z.run(&["list"])), 0);
}

#[test]
fn another_tenant_sees_nothing() {
    let z = Zorix::new();
    z.run(&["--tenant", "acme", "run", "What is 4+4?", "--allow", "math.evaluate"]);
    let mine = text(&z.run(&["--tenant", "acme", "list"]).stdout);
    assert!(mine.contains("What is 4+4?"));
    let theirs = z.run(&["--tenant", "globex", "list"]);
    assert!(!text(&theirs.stdout).contains("4+4"));
    let id = mine.split_whitespace().next().unwrap().to_string();
    let peek = z.run(&["--tenant", "globex", "show", &id]);
    assert_eq!(code(&peek), 1);
    assert!(text(&peek.stderr).contains("was not found"));
}

impl Zorix {
    /// Like `run`, with only environment-variable keys (never the real keychain).
    fn run_env(&self, args: &[&str], stdin: &str, vars: &[(&str, &str)]) -> Output {
        let mut command = Command::new(env!("CARGO_BIN_EXE_zorix"));
        command
            .arg("--home")
            .arg(self.home())
            .args(args)
            .env_remove("ZORIX_HOME")
            .env("ZORIX_SECRETS", "env")
            .env_remove("ANTHROPIC_API_KEY")
            .env_remove("OPENAI_API_KEY")
            .env_remove("GEMINI_API_KEY")
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped());
        for (k, v) in vars {
            command.env(k, v);
        }
        let mut child = command.spawn().expect("start zorix");
        let mut pipe = child.stdin.take().unwrap();
        let input = stdin.to_string();
        std::thread::spawn(move || {
            let _ = pipe.write_all(input.as_bytes());
        });
        child.wait_with_output().expect("output")
    }
}

#[test]
fn providers_are_listed_with_their_key_state_and_a_key_is_never_printed() {
    let z = Zorix::new();
    let out = z.run_env(&["provider", "list"], "", &[("ANTHROPIC_API_KEY", "sk-ant-secret-0123456789abcdef")]);
    assert_eq!(code(&out), 0, "{}", text(&out.stderr));
    let listing = text(&out.stdout);
    assert!(listing.lines().any(|l| l.starts_with("anthropic") && l.contains("key stored")), "{listing}");
    assert!(listing.lines().any(|l| l.starts_with("ollama") && l.contains("local")), "{listing}");
    assert!(!listing.contains("secret-0123") && !text(&out.stderr).contains("secret-0123"));
}

#[test]
fn set_key_refuses_unknown_providers_and_bad_keys_without_echoing_them() {
    let z = Zorix::new();
    let out = z.run_env(&["provider", "set-key", "nobody", "--stdin"], "sk-x\n", &[]);
    assert_eq!(code(&out), 1);
    assert!(text(&out.stderr).contains("There is no provider called"), "{}", text(&out.stderr));
    let out = z.run_env(&["provider", "set-key", "openai", "--stdin"], "two words\n", &[]);
    assert_eq!(code(&out), 1);
    assert!(text(&out.stderr).contains("paste only the key"), "{}", text(&out.stderr));
    assert!(!text(&out.stderr).contains("two words"));
}

#[test]
fn models_lists_the_catalogue_and_routes_are_empty_without_a_file() {
    let z = Zorix::new();
    let out = z.run_env(&["models"], "", &[]);
    assert_eq!(code(&out), 0);
    assert!(text(&out.stdout).contains("anthropic/claude-opus-5-5"), "{}", text(&out.stdout));
    let out = z.run_env(&["route", "list"], "", &[]);
    assert_eq!(code(&out), 0);
    assert!(text(&out.stderr).contains("No routes or aliases"), "{}", text(&out.stderr));
}

#[test]
fn a_run_with_an_unknown_provider_fails_clearly_and_demo_still_works() {
    let z = Zorix::new();
    let out = z.run_env(&["run", "hi", "--model", "nobody/x", "--approve", "never"], "", &[]);
    assert_ne!(code(&out), 0, "{}", text(&out.stdout));
    let all = format!("{}{}", text(&out.stdout), text(&out.stderr));
    assert!(all.contains("There is no provider called"), "{all}");
    let out = z.run_env(&["run", "What is 2+2?", "--allow", "math.evaluate", "--approve", "never"], "", &[]);
    assert_eq!(code(&out), 0, "{}{}", text(&out.stdout), text(&out.stderr));
}

// ---------------------------------------------------------------------------------------------------------------------
// memory, knowledge, embeddings

impl Zorix {
    /// Run a command that needs no keychain and read what it printed.
    fn env_run(&self, args: &[&str]) -> Output {
        self.run_env(args, "", &[])
    }

    fn docs(&self) -> PathBuf {
        let docs = self.dir.path().join("docs");
        std::fs::create_dir_all(docs.join("guide")).unwrap();
        std::fs::write(docs.join("guide/deploy.md"), "# Deploys\nDeploys reach staging first, then production.\n")
            .unwrap();
        std::fs::write(docs.join("notes.txt"), "The cafeteria serves lunch at noon.").unwrap();
        std::fs::write(docs.join(".env"), "KEY=sk-abcdefghijklmnopqrstuvwxyz0123456789").unwrap();
        docs
    }
}

fn first_id(output: &Output) -> String {
    let all = format!("{}{}", text(&output.stdout), text(&output.stderr));
    let start = all.find("m:").expect("a memory id");
    all[start..].split(|c: char| c.is_whitespace() || c == '.' || c == ',').next().unwrap().to_string()
}

#[test]
fn memory_can_be_added_listed_changed_confirmed_and_forgotten() {
    let z = Zorix::new();
    let added = z.env_run(&["memory", "add", "Answer in Portuguese.", "--kind", "preference", "--subject", "language"]);
    assert_eq!(code(&added), 0, "{}", text(&added.stderr));
    let id = first_id(&added);
    let listed = z.env_run(&["memory", "list"]);
    assert!(
        text(&listed.stdout).contains(&id)
            && text(&listed.stdout).contains("preference")
            && text(&listed.stdout).contains("Answer in Portuguese."),
        "{}",
        text(&listed.stdout)
    );
    // The same text again changes nothing.
    assert!(text(&z.env_run(&["memory", "add", "Answer in Portuguese."]).stderr).contains("Already remembered"));
    // A newer memory about the same subject replaces it.
    let newer = z.env_run(&["memory", "add", "Answer in English.", "--kind", "preference", "--subject", "language"]);
    assert!(text(&newer.stderr).contains("replacing"), "{}", text(&newer.stderr));
    let json: Value = serde_json::from_slice(&z.env_run(&["--json", "memory", "list"]).stdout).unwrap();
    assert_eq!(json.as_array().unwrap().len(), 1, "only the one in force");
    assert_eq!(json[0]["content"], "Answer in English.");
    let all = text(&z.env_run(&["memory", "list", "--all"]).stdout);
    assert!(all.contains("(replaced by m:") && all.contains("Answer in Portuguese."), "{all}");
    // Edit, confirm, expiry.
    let new_id = json[0]["id"].as_str().unwrap().to_string();
    let edited =
        z.env_run(&["memory", "edit", &format!("m:{new_id}"), "--text", "Answer in Spanish.", "--until", "2099-12-31"]);
    assert_eq!(code(&edited), 0, "{}", text(&edited.stderr));
    assert!(text(&edited.stdout).contains("Answer in Spanish.") && text(&edited.stdout).contains("(until 2099-12-31)"));
    assert_eq!(code(&z.env_run(&["memory", "confirm", &new_id])), 0, "a bare id works too");
    assert_eq!(code(&z.env_run(&["memory", "edit", &new_id])), 1, "nothing to change");
    // Forgetting needs a yes when nobody is there to ask.
    let refused = z.env_run(&["memory", "forget", &new_id]);
    assert_eq!(code(&refused), 1);
    assert!(text(&refused.stderr).contains("--yes"));
    assert_eq!(code(&z.env_run(&["memory", "forget", &new_id, "--yes"])), 0);
    assert!(!text(&z.env_run(&["memory", "list"]).stdout).contains("Spanish"));
    assert_eq!(code(&z.env_run(&["memory", "forget", &new_id, "--yes"])), 1, "already gone");
}

#[test]
fn memory_refuses_secrets_bad_dates_and_bad_ids_with_a_reason() {
    let z = Zorix::new();
    let secret = z.env_run(&["memory", "add", "The key is sk-abcdefghijklmnopqrstuvwxyz0123456789"]);
    assert_eq!(code(&secret), 1);
    assert!(text(&secret.stderr).contains("credential") && !text(&secret.stderr).contains("sk-abcdef"));
    assert!(text(&z.env_run(&["memory", "add", "x", "--until", "2001-01-01"]).stderr).contains("past"));
    assert!(text(&z.env_run(&["memory", "add", "x", "--until", "tomorrow"]).stderr).contains("YYYY-MM-DD"));
    assert!(text(&z.env_run(&["memory", "forget", "nope", "--yes"]).stderr).contains("not a memory id"));
    assert_eq!(
        code(&z.env_run(&["memory", "add", "x", "--scope", "planet"])),
        2,
        "the scope is checked before anything is touched"
    );
    assert!(text(&z.env_run(&["memory", "list"]).stderr).contains("Nothing is remembered"));
}

#[test]
fn memory_scopes_and_export_work() {
    let z = Zorix::new();
    let project = z.dir.path().join("proj");
    std::fs::create_dir_all(&project).unwrap();
    let scope = format!("project:{}", project.display());
    z.env_run(&["memory", "add", "Global fact."]);
    z.env_run(&["memory", "add", "Project fact.", "--scope", &scope]);
    let only_project = text(&z.env_run(&["memory", "list", "--scope", &scope]).stdout);
    assert!(only_project.contains("Project fact.") && !only_project.contains("Global fact."), "{only_project}");
    let out = z.dir.path().join("memories.json");
    let exported = z.env_run(&["memory", "export", "--out", out.to_str().unwrap()]);
    assert_eq!(code(&exported), 0);
    let file: Value = serde_json::from_str(&std::fs::read_to_string(out).unwrap()).unwrap();
    assert_eq!(file.as_array().unwrap().len(), 2);
}

#[test]
fn a_folder_is_added_indexed_searched_listed_and_removed() {
    let z = Zorix::new();
    let docs = z.docs();
    let added = z.env_run(&["knowledge", "add", docs.to_str().unwrap()]);
    assert_eq!(code(&added), 0, "{}", text(&added.stderr));
    assert!(text(&added.stderr).contains("2 indexed"), "the private file is not read: {}", text(&added.stderr));
    let found = text(&z.env_run(&["knowledge", "search", "staging production"]).stdout);
    assert!(found.contains("guide/deploy.md:L1-L2") && found.contains("Deploys reach staging"), "{found}");
    assert!(
        text(&z.env_run(&["knowledge", "search", "abcdefghijklmnopqrstuvwxyz0123456789"]).stderr)
            .contains("Nothing found")
    );
    let list = text(&z.env_run(&["knowledge", "list"]).stdout);
    assert!(list.contains("2 files") && list.contains("global"), "{list}");
    let files = text(&z.env_run(&["knowledge", "files", docs.to_str().unwrap()]).stdout);
    assert!(files.contains("indexed  guide/deploy.md") && !files.contains(".env"), "{files}");
    // Nothing changed: a sync does nothing.
    assert!(text(&z.env_run(&["knowledge", "sync"]).stderr).contains("0 indexed, 2 unchanged"));
    // Adding it twice, and a pattern that is not valid, are refused.
    assert_eq!(code(&z.env_run(&["knowledge", "add", docs.to_str().unwrap()])), 1);
    let removed = z.env_run(&["knowledge", "remove", docs.to_str().unwrap()]);
    assert_eq!(code(&removed), 0);
    assert!(docs.join("guide/deploy.md").exists(), "the files stay");
    assert!(text(&z.env_run(&["knowledge", "search", "staging"]).stderr).contains("Nothing found"));
    assert_eq!(code(&z.env_run(&["knowledge", "sync", "nowhere"])), 1);
}

#[test]
fn embeddings_are_chosen_tried_used_and_turned_off() {
    let z = Zorix::new();
    assert!(text(&z.env_run(&["embeddings", "show"]).stdout).contains("keywords only"));
    // A cloud provider needs a deliberate yes, because document text goes to it.
    let cloud = z.env_run(&["embeddings", "set", "openai/text-embedding-3-small"]);
    assert_eq!(code(&cloud), 1);
    assert!(
        text(&cloud.stderr).contains("--cloud") && text(&cloud.stderr).contains("sent to it"),
        "{}",
        text(&cloud.stderr)
    );
    assert_eq!(code(&z.env_run(&["embeddings", "set", "all-minilm"])), 1, "it must be provider/model");
    assert_eq!(code(&z.env_run(&["embeddings", "test"])), 1, "nothing is set yet");
    let docs = z.docs();
    z.env_run(&["knowledge", "add", docs.to_str().unwrap()]);
    let set = z.env_run(&["embeddings", "set", "sim/hash-embed"]);
    assert_eq!(code(&set), 0, "{}", text(&set.stderr));
    assert!(text(&set.stderr).contains("64 dimensions"));
    assert!(text(&z.env_run(&["embeddings", "show"]).stdout).contains("0 of 2 texts"));
    let again = z.env_run(&["embeddings", "reembed"]);
    assert_eq!(code(&again), 0, "{}", text(&again.stderr));
    let show = text(&z.env_run(&["embeddings", "show"]).stdout);
    assert!(show.contains("sim/hash-embed") && show.contains("2 of 2 texts") && show.contains("not allowed"), "{show}");
    let tried = text(&z.env_run(&["embeddings", "test"]).stdout);
    assert!(tried.contains("64 dimensions"), "{tried}");
    assert_eq!(code(&z.env_run(&["embeddings", "off"])), 0);
    assert!(text(&z.env_run(&["embeddings", "show"]).stdout).contains("keywords only"));
    // The list of models never fails because one server is down.
    assert_eq!(code(&z.env_run(&["embeddings", "models"])), 0);
}

#[test]
fn a_conversation_carries_over_and_show_context_says_what_memory_added() {
    let z = Zorix::new();
    z.env_run(&["memory", "add", "The team is called Platform."]);
    let first = z.env_run(&[
        "run",
        "What is 2+2?",
        "--allow",
        "math.evaluate",
        "--approve",
        "never",
        "--conversation",
        "chat-1",
    ]);
    assert_eq!(code(&first), 0, "{}", text(&first.stderr));
    let second = z.env_run(&[
        "run",
        "And what is 3+3?",
        "--allow",
        "math.evaluate",
        "--approve",
        "never",
        "--conversation",
        "chat-1",
    ]);
    assert_eq!(code(&second), 0);
    let id = z.newest_id();
    let shown = text(&z.env_run(&["show", &id, "--context"]).stdout);
    assert!(shown.contains("Context") && shown.contains("earlier"), "{shown}");
    assert!(shown.contains("fact") && shown.contains("The team is called Platform."), "{shown}");
    assert!(shown.contains("conversation") && shown.contains("long_term") && shown.contains("kept"), "{shown}");
    // Switching things off leaves them out.
    z.env_run(&[
        "run",
        "And 4+4?",
        "--allow",
        "math.evaluate",
        "--approve",
        "never",
        "--conversation",
        "chat-1",
        "--no-memory",
        "--no-conversation",
        "--no-knowledge",
    ]);
    let bare = text(&z.env_run(&["show", &z.newest_id(), "--context"]).stdout);
    assert!(bare.contains("Memory was not used"), "{bare}");
    let json: Value = serde_json::from_slice(&z.env_run(&["--json", "show", &id, "--context"]).stdout).unwrap();
    assert!(json["context"]["recall"]["items"].as_array().unwrap().iter().any(|i| i["kind"] == "exchange"));
}

#[test]
fn a_proposal_to_remember_waits_for_a_person_even_with_approve_yes() {
    let z = Zorix::new();
    let model = z.write_script(r#"[{"call": {"calls": [{"tool": "memory.remember", "arguments": {"content": "Releases happen on Thursdays.", "kind": "fact", "scope": "global"}}]}}, {"say": "Noted."}]"#);
    let out = z.env_run(&[
        "run",
        "Remember when we release",
        "--model",
        &model,
        "--allow",
        "memory.remember",
        "--approve",
        "yes",
    ]);
    assert_eq!(code(&out), 3, "it is waiting: {}", text(&out.stderr));
    assert!(text(&out.stderr).contains("must be approved by a person"), "{}", text(&out.stderr));
    assert!(text(&z.env_run(&["memory", "list"]).stderr).contains("Nothing is remembered"));
    // A person approves it, and then it is remembered.
    let id = z.newest_id();
    let approved = z.env_run(&["approve", &id, "--model", "demo"]);
    assert_eq!(code(&approved), 0, "{}", text(&approved.stderr));
    assert!(text(&z.env_run(&["memory", "list"]).stdout).contains("Releases happen on Thursdays."));
}

#[test]
fn calibration_starts_empty_and_says_so() {
    let z = Zorix::new();
    assert!(text(&z.env_run(&["calibration"]).stderr).contains("Nothing learned yet"));
    assert_eq!(code(&z.env_run(&["--json", "calibration"])), 0);
}
