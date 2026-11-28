//! `zorix mcp`, `zorix connect` and `zorix sandbox`, run as real programs. The MCP server under test is `zorix mcp serve`
//! itself, so the whole path is exercised: add, enable, list tools, review, trust, and a run that calls a tool.

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

fn code(output: &Output) -> i32 {
    output.status.code().expect("exited normally")
}

fn all(output: &Output) -> String {
    format!("{}{}", text(&output.stdout), text(&output.stderr))
}

impl Zorix {
    fn new() -> Self {
        Self { dir: tempfile::tempdir().unwrap() }
    }

    fn home(&self) -> PathBuf {
        self.dir.path().join("home")
    }

    fn run_with(&self, args: &[&str], stdin: &str, vars: &[(&str, &str)]) -> Output {
        let mut command = Command::new(env!("CARGO_BIN_EXE_zorix"));
        command
            .arg("--home")
            .arg(self.home())
            .args(args)
            .env_remove("ZORIX_HOME")
            .env("ZORIX_SECRETS", "env")
            .env("ZORIX_TEST_SECRETS_FILE", self.dir.path().join("secrets.json"))
            .env_remove("ANTHROPIC_API_KEY")
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
        let (sender, receiver) = mpsc::channel();
        std::thread::spawn(move || {
            let _ = sender.send(child.wait_with_output());
        });
        receiver.recv_timeout(Duration::from_secs(90)).expect("zorix finished in time").expect("output")
    }

    fn run(&self, args: &[&str]) -> Output {
        self.run_with(args, "", &[])
    }

    fn script(&self, turns: &str) -> String {
        std::fs::create_dir_all(self.dir.path()).unwrap();
        let path = self.dir.path().join("script.json");
        std::fs::write(&path, turns).unwrap();
        format!("script:{}", path.display())
    }

    /// Add `zorix mcp serve` on this same data folder as a server called `self`.
    fn add_self_server(&self) {
        let zorix = env!("CARGO_BIN_EXE_zorix");
        let home = self.home().display().to_string();
        let added = self.run(&[
            "mcp",
            "add",
            "self",
            "--command",
            zorix,
            "--arg",
            "--home",
            "--arg",
            &home,
            "--arg",
            "mcp",
            "--arg",
            "serve",
            "--set",
            "ZORIX_SECRETS=env",
        ]);
        assert_eq!(code(&added), 0, "{}", all(&added));
        // A sandboxed server may not read ZORIX's data folder, and this one is meant to serve it: a person turns its sandbox off.
        assert!(
            text(&added.stderr).contains("Sandbox    native"),
            "a new local server starts sandboxed: {}",
            all(&added)
        );
        let off = self.run(&["mcp", "sandbox", "self", "--mode", "off"]);
        assert_eq!(code(&off), 0, "{}", all(&off));
        assert!(text(&off.stdout).contains("Sandbox    off"), "{}", all(&off));
    }
}

#[test]
fn the_catalogue_lists_and_searches() {
    let z = Zorix::new();
    let listed = z.run(&["mcp", "catalog"]);
    assert_eq!(code(&listed), 0);
    assert!(text(&listed.stdout).contains("github") && text(&listed.stdout).contains("remote"), "{}", all(&listed));
    let json = z.run(&["--json", "mcp", "catalog", "github"]);
    let entries: Value = serde_json::from_slice(&json.stdout).unwrap();
    assert!(entries.as_array().unwrap().iter().all(|e| e.to_string().to_lowercase().contains("github")));
    assert!(!entries.as_array().unwrap().is_empty());
}

#[test]
fn a_server_is_added_off_confirmed_to_turn_on_and_removed_with_its_secrets() {
    let z = Zorix::new();
    let added = z.run_with(
        &[
            "mcp",
            "add",
            "tools",
            "--command",
            "some-server",
            "--arg",
            "--flag",
            "--env",
            "API_KEY",
            "--set",
            "MODE=fast",
        ],
        "",
        &[("API_KEY", "sk-abcdefghijklmnopqrstuvwxyz")],
    );
    assert_eq!(code(&added), 0, "{}", all(&added));
    assert!(text(&added.stderr).contains("It is off for now") && text(&added.stderr).contains("some-server --flag"));
    assert!(!all(&added).contains("sk-abcdef"), "a secret is never echoed");
    let shown = text(&z.run(&["mcp", "show", "tools"]).stdout);
    assert!(shown.contains("API_KEY (secret, in the keychain)") && shown.contains("MODE=fast"), "{shown}");
    assert!(!shown.contains("sk-abcdef"));
    assert!(text(&z.run(&["mcp", "list"]).stdout).contains("off"));

    // Turning it on needs a person: no terminal and no --yes is refused.
    let refused = z.run(&["mcp", "enable", "tools"]);
    assert_ne!(code(&refused), 0);
    assert!(text(&refused.stderr).contains("confirmation"), "{}", all(&refused));
    assert!(text(&refused.stderr).contains("can do anything you can do"), "the warning comes first");
    assert_eq!(code(&z.run(&["mcp", "enable", "tools", "--yes"])), 0);
    assert!(text(&z.run(&["mcp", "list"]).stdout).contains(" on "));
    assert_eq!(code(&z.run(&["mcp", "disable", "tools"])), 0);

    assert_eq!(code(&z.run(&["mcp", "remove", "tools"])), 0);
    assert_ne!(code(&z.run(&["mcp", "show", "tools"])), 0);
    // Names and shapes are checked.
    assert_ne!(code(&z.run(&["mcp", "add", "Bad Name", "--command", "x"])), 0);
    assert_ne!(code(&z.run(&["mcp", "add", "x"])), 0, "nothing to add");
    assert_eq!(code(&z.run(&["mcp", "add", "x", "--url", "http://example.com/mcp"])), 1, "plain http to another host");
}

#[test]
fn a_catalogue_server_asks_for_what_it_needs_and_says_what_to_do_next() {
    let z = Zorix::new();
    let folder = z.dir.path().join("docs");
    std::fs::create_dir_all(&folder).unwrap();
    let missing = z.run(&["mcp", "add", "--catalog", "filesystem"]);
    assert_ne!(code(&missing), 0);
    assert!(all(&missing).contains("is needed"), "{}", all(&missing));
    let added = z.run(&["mcp", "add", "--catalog", "filesystem", "--input", &format!("folder={}", folder.display())]);
    assert_eq!(code(&added), 0, "{}", all(&added));
    assert!(text(&added.stderr).contains("zorix mcp enable"), "{}", all(&added));
    assert_ne!(code(&z.run(&["mcp", "add", "--catalog", "nonexistent"])), 0);
    // A remote server that signs in with the browser says so.
    let remote = z.run(&["mcp", "add", "linear", "--catalog", "linear"]);
    assert_eq!(code(&remote), 0, "{}", all(&remote));
    assert!(text(&remote.stderr).contains("zorix mcp login linear"), "{}", all(&remote));
}

#[test]
fn a_server_goes_from_new_to_approved_to_used_by_a_run() {
    let z = Zorix::new();
    assert_eq!(code(&z.run(&["memory", "add", "Releases happen on Thursdays."])), 0);
    z.add_self_server();
    // Tools of a server that is off cannot be listed.
    assert_ne!(code(&z.run(&["mcp", "tools", "self"])), 0);
    assert_eq!(code(&z.run(&["mcp", "enable", "self", "--yes"])), 0);

    // 1. Its tools arrive as new and are not offered.
    let tools = z.run(&["mcp", "tools", "self"]);
    assert_eq!(code(&tools), 0, "{}", all(&tools));
    let listing = text(&tools.stdout);
    assert!(listing.contains("memory.search") && listing.contains("new"), "{listing}");
    assert!(text(&tools.stderr).contains("not offered to runs until you approve"), "{}", all(&tools));
    let script = z.script(
        r#"[{"call": {"calls": [{"tool": "mcp.self.memory.search", "arguments": {"query": "releases"}}]}}, {"say": "Noted."}]"#,
    );
    let unapproved =
        z.run(&["run", "when are releases?", "--model", &script, "--allow", "mcp.self.*", "--approve", "never"]);
    let events = text(&z.run(&["events", &newest(&z)]).stdout);
    assert!(
        events.contains("tool.denied") && events.contains("There is no tool called"),
        "an unapproved tool is not offered:\n{events}\n{}",
        all(&unapproved)
    );

    // 2. A person reads what it says and approves.
    let review = z.run(&["mcp", "review", "self"]);
    assert!(text(&review.stdout).contains("memory.search — new"), "{}", all(&review));
    assert_eq!(code(&z.run(&["mcp", "review", "self", "--approve", "memory.search"])), 0);
    assert!(text(&z.run(&["mcp", "tools", "self"]).stdout).contains("pinned"));

    // 3. A run that was not granted the server cannot use it.
    let ungranted = z.run(&["run", "when are releases?", "--model", &script, "--approve", "never"]);
    let events = text(&z.run(&["events", &newest(&z)]).stdout);
    assert!(events.contains("tool.denied"), "{events}\n{}", all(&ungranted));

    // 4. Granted, the call asks first (every MCP tool does) and waits for a person.
    let waiting =
        z.run(&["run", "when are releases?", "--model", &script, "--allow", "mcp.self.*", "--approve", "never"]);
    assert_eq!(code(&waiting), 3, "{}", all(&waiting));
    assert!(text(&waiting.stderr).contains("mcp.self.memory.search"), "{}", all(&waiting));
    let approved = z.run(&["approve", &newest(&z)]);
    assert_eq!(code(&approved), 0, "{}", all(&approved));
    assert!(text(&approved.stdout).contains("Thursdays"), "{}", all(&approved));

    // 5. A person may let a read-only tool run without asking.
    assert_eq!(code(&z.run(&["mcp", "trust", "self", "--tool", "memory.search", "--read-only-without-asking"])), 0);
    let free = z.run(&["run", "when are releases?", "--model", &script, "--allow", "mcp.self.*", "--approve", "never"]);
    assert_eq!(code(&free), 0, "{}", all(&free));
    assert!(text(&free.stdout).contains("Noted."));
    let id = newest(&z);
    let events = text(&z.run(&["events", &id]).stdout);
    assert!(events.contains("tool.completed") && events.contains("mcp"), "{events}");
    let shown = text(&z.run(&["show", &id]).stdout);
    assert!(shown.contains("mcp.self.memory.search"), "{shown}");
    // Asking again after switching back.
    assert_eq!(code(&z.run(&["mcp", "trust", "self", "--tool", "memory.search", "--ask"])), 0);
    // The server logs are reachable.
    assert_eq!(code(&z.run(&["mcp", "logs", "self"])), 0);
}

fn newest(z: &Zorix) -> String {
    let out = z.run(&["list", "--json"]);
    let list: Value = serde_json::from_slice(&out.stdout).expect("list json");
    list[0]["id"].as_str().expect("an execution").to_string()
}

#[test]
fn servers_can_be_imported_from_a_claude_config_after_a_preview() {
    let z = Zorix::new();
    let config = z.dir.path().join("claude_desktop_config.json");
    std::fs::write(
        &config,
        r#"{"mcpServers": {"Files": {"command": "npx", "args": ["-y", "@modelcontextprotocol/server-filesystem", "/tmp"]}, "broken": {"nothing": true}}}"#,
    )
    .unwrap();
    let path = config.display().to_string();
    let preview = z.run(&["mcp", "import", "--from", "claude", "--path", &path]);
    assert_eq!(code(&preview), 0, "{}", all(&preview));
    assert!(
        text(&preview.stdout).contains("add   files") && text(&preview.stdout).contains("skip  broken"),
        "{}",
        all(&preview)
    );
    assert!(text(&preview.stderr).contains("Nothing was changed"));
    assert!(text(&z.run(&["mcp", "list"]).stderr).contains("No MCP servers yet"));
    let done = z.run(&["mcp", "import", "--from", "claude", "--path", &path, "--yes"]);
    assert_eq!(code(&done), 0, "{}", all(&done));
    let listed = text(&z.run(&["mcp", "list"]).stdout);
    assert!(listed.contains("files") && listed.contains("off"), "imported servers start off:\n{listed}");
}

#[test]
fn connections_keep_secrets_out_of_sight_and_are_tested() {
    let z = Zorix::new();
    let db = z.dir.path().join("app.db");
    // The smallest SQLite file: the store ZORIX makes itself is one.
    assert_eq!(code(&z.run(&["connect", "add", "sqlite", "app", &db.display().to_string()])), 1, "no such file yet");
    std::fs::create_dir_all(z.home()).unwrap();
    let existing = z.home().join("zorix.sqlite");
    let _ = z.run(&["list"]);
    assert!(existing.exists() || std::fs::read_dir(z.home()).unwrap().count() > 0);
    let file = std::fs::read_dir(z.home())
        .unwrap()
        .filter_map(Result::ok)
        .map(|e| e.path())
        .find(|p| p.extension().is_some_and(|e| e == "sqlite"))
        .expect("a database file");
    let added = z.run(&["connect", "add", "sqlite", "app", &file.display().to_string()]);
    assert_eq!(code(&added), 0, "{}", all(&added));
    let tested = z.run(&["connect", "test", "app"]);
    assert_eq!(code(&tested), 0, "{}", all(&tested));
    assert!(text(&tested.stderr).contains("opens read-only"), "{}", all(&tested));
    assert!(text(&z.run(&["connect", "list"]).stdout).contains("tested ok"));

    let slack = z.run_with(
        &["connect", "add", "slack", "team", "--secret-env", "SLACK_TOKEN"],
        "",
        &[("SLACK_TOKEN", "xoxb-1111111111-abcdefghij")],
    );
    assert_eq!(code(&slack), 0, "{}", all(&slack));
    assert!(!all(&slack).contains("xoxb-"));
    let listed = z.run(&["--json", "connect", "list"]);
    assert!(!text(&listed.stdout).contains("xoxb-"), "{}", text(&listed.stdout));
    // A secret is required where it is, and never taken from the command line.
    let missing = z.run(&["connect", "add", "slack", "other"]);
    assert_ne!(code(&missing), 0);
    assert!(all(&missing).contains("--secret-env"), "{}", all(&missing));
    let stdin =
        z.run_with(&["connect", "add", "discord", "bot", "--secret-stdin"], "discord-token-abcdefghijklmnop\n", &[]);
    assert_eq!(code(&stdin), 0, "{}", all(&stdin));

    // A webhook gets a signing key that is shown once.
    let hook = z.run(&["connect", "add", "webhook", "ci", "--url", "https://hooks.example.com/x"]);
    assert_eq!(code(&hook), 0, "{}", all(&hook));
    assert!(text(&hook.stderr).contains("Signing key (shown once"), "{}", all(&hook));
    assert!(!text(&z.run(&["connect", "list"]).stdout).contains("Signing key"));
    assert_ne!(code(&z.run(&["connect", "add", "webhook", "plain", "--url", "http://example.com/x"])), 0, "HTTPS only");

    assert_eq!(code(&z.run(&["connect", "remove", "team"])), 0);
    assert_ne!(code(&z.run(&["connect", "test", "team"])), 0);
}

#[test]
fn a_database_run_needs_its_grant_and_reads_through_a_connection() {
    let z = Zorix::new();
    let _ = z.run(&["list"]);
    let file = std::fs::read_dir(z.home())
        .unwrap()
        .filter_map(Result::ok)
        .map(|e| e.path())
        .find(|p| p.extension().is_some_and(|e| e == "sqlite"))
        .expect("a database file");
    assert_eq!(code(&z.run(&["connect", "add", "sqlite", "app", &file.display().to_string()])), 0);
    let script = z.script(
        r#"[{"call": {"calls": [{"tool": "db.query", "arguments": {"connection": "app", "sql": "SELECT count(*) AS n FROM sqlite_master"}}]}}, {"say": "Counted."}]"#,
    );
    let denied = z.run(&["run", "count", "--model", &script, "--allow", "db.query", "--approve", "never"]);
    let id = newest(&z);
    let shown = text(&z.run(&["show", &id]).stdout);
    assert!(
        shown.contains("Failed") || shown.contains("failed") || all(&denied).contains("permission"),
        "{shown}\n{}",
        all(&denied)
    );

    let granted = z.run(&["run", "count", "--model", &script, "--allow-db", "app", "--approve", "never"]);
    assert_eq!(code(&granted), 0, "{}", all(&granted));
    assert!(text(&granted.stdout).contains("Counted."));
    let events = text(&z.run(&["events", &newest(&z)]).stdout);
    assert!(events.contains("integration:") || events.contains("tool.completed"), "{events}");
}

#[test]
fn code_runs_in_the_sandbox_only_when_granted() {
    let z = Zorix::new();
    let status = z.run(&["sandbox", "status"]);
    assert_eq!(code(&status), 0);
    assert!(
        text(&status.stdout).contains("javascript   ready") && text(&status.stdout).contains("python"),
        "{}",
        all(&status)
    );
    let script = z.script(
        r#"[{"call": {"calls": [{"tool": "code.run", "arguments": {"language": "javascript", "code": "console.log(6 * 7)"}}]}}, {"say": "Computed."}]"#,
    );
    let granted = z.run(&["run", "compute", "--model", &script, "--allow-code", "javascript", "--approve", "never"]);
    assert_eq!(code(&granted), 0, "{}", all(&granted));
    assert!(text(&granted.stdout).contains("Computed."));
    let events = text(&z.run(&["events", &newest(&z)]).stdout);
    assert!(events.contains("tool.completed") && events.contains("code.run"), "{events}");
    // Python is a separate language grant.
    let python = z.script(
        r#"[{"call": {"calls": [{"tool": "code.run", "arguments": {"language": "python", "code": "print(1)"}}]}}, {"say": "Done."}]"#,
    );
    let denied = z.run(&["run", "compute", "--model", &python, "--allow-code", "javascript", "--approve", "never"]);
    let events = text(&z.run(&["events", &newest(&z)]).stdout);
    assert!(events.contains("tool.denied") || events.contains("tool.failed"), "{events}\n{}", all(&denied));
}

#[test]
fn grant_flags_are_checked_before_anything_starts() {
    let z = Zorix::new();
    for args in [
        vec!["run", "x", "--allow-repo", "nonsense"],
        vec!["run", "x", "--allow-http", "bad host"],
        vec!["run", "x", "--allow-jira", "e n g"],
        vec!["run", "x", "--allow-chat", "teams:x"],
        vec!["run", "x", "--allow-db", "Not A Name"],
        vec!["run", "x", "--allow-code", "ruby"],
    ] {
        let out = z.run(&args);
        assert_eq!(code(&out), 2, "{args:?}: {}", all(&out));
    }
}

#[test]
fn the_value_of_a_telemetry_header_goes_to_the_keychain_and_never_to_the_database_or_the_screen() {
    let z = Zorix::new();
    let secret = "Bearer s3cret-telemetry-value";
    let set = z.run(&["settings", "set", "telemetry.header", &format!("authorization: {secret}")]);
    assert_eq!(code(&set), 0, "{}", all(&set));
    assert!(!all(&set).contains("s3cret"), "the value was printed: {}", all(&set));

    // The settings show the name only, as JSON and as text.
    let shown = z.run(&["settings", "show"]);
    assert!(text(&shown.stdout).contains("authorization (the value is in the keychain)"), "{}", text(&shown.stdout));
    let json = z.run(&["settings", "show", "--json"]);
    assert!(
        !text(&json.stdout).contains("s3cret") && text(&json.stdout).contains("\"header\":\"authorization\""),
        "{}",
        text(&json.stdout)
    );
    // The database does not hold it; the keychain (a file in this test) does.
    let database = std::fs::read(z.home().join("zorix.sqlite")).unwrap();
    assert!(!database.windows(6).any(|w| w == b"s3cret"), "the value is in the database");
    assert!(std::fs::read_to_string(z.dir.path().join("secrets.json")).unwrap().contains("s3cret-telemetry-value"));

    // A header that is not one is refused, and `off` removes the value.
    assert_ne!(code(&z.run(&["settings", "set", "telemetry.header", "bad name: x"])), 0);
    assert_ne!(code(&z.run(&["settings", "set", "telemetry.header", "no-colon-here"])), 0);
    assert_eq!(code(&z.run(&["settings", "set", "telemetry.header", "off"])), 0);
    assert!(!std::fs::read_to_string(z.dir.path().join("secrets.json")).unwrap_or_default().contains("s3cret"));
    assert!(text(&z.run(&["settings", "show"]).stdout).contains("telemetry.header     off"));
}

#[test]
fn a_servers_sandbox_is_shown_and_changed_from_the_command_line() {
    let z = Zorix::new();
    let folder = z.dir.path().join("notes");
    std::fs::create_dir_all(&folder).unwrap();
    assert_eq!(code(&z.run(&["mcp", "add", "files", "--command", "npx", "--arg", "-y", "--arg", "some-server"])), 0);
    let shown = z.run(&["mcp", "sandbox", "files"]);
    assert!(text(&shown.stdout).contains("Sandbox    native, network on"), "{}", all(&shown));
    let changed =
        z.run(&["mcp", "sandbox", "files", "--network", "off", "--folder-write", &folder.display().to_string()]);
    assert_eq!(code(&changed), 0, "{}", all(&changed));
    let out = text(&changed.stdout);
    assert!(out.contains("network off") && out.contains("reads and writes") && out.contains("notes"), "{out}");
    // A folder of credentials is never given, and a remote server has nothing to confine.
    let secrets = z.dir.path().join(".ssh");
    std::fs::create_dir_all(&secrets).unwrap();
    let refused = z.run(&["mcp", "sandbox", "files", "--folder", &secrets.display().to_string()]);
    assert_ne!(code(&refused), 0, "{}", all(&refused));
    assert!(text(&refused.stderr).contains("credentials"), "{}", all(&refused));
    assert_eq!(code(&z.run(&["mcp", "add", "web", "--url", "https://mcp.example.com/mcp"])), 0);
    assert_ne!(code(&z.run(&["mcp", "sandbox", "web", "--mode", "off"])), 0);
    let doctor = z.run(&["mcp", "doctor"]);
    assert!(text(&doctor.stdout).contains("Files") && text(&doctor.stdout).contains("Processes"), "{}", all(&doctor));
}

#[test]
fn signing_in_says_what_it_needs_before_opening_anything() {
    let z = Zorix::new();
    let jira = z.run(&[
        "connect",
        "add",
        "jira",
        "work",
        "--site",
        "https://acme.atlassian.net",
        "--oauth",
        "--client-id",
        "a",
    ]);
    assert_ne!(code(&jira), 0);
    assert!(text(&jira.stderr).contains("--client-secret-env"), "{}", all(&jira));
    let slack = z.run(&["connect", "add", "slack", "team", "--oauth"]);
    assert_ne!(code(&slack), 0, "Slack is not signed in to");
    let both = z.run(&["connect", "add", "github", "gh", "--oauth", "--secret-env", "X"]);
    assert!(text(&both.stderr).contains("cannot be used with"), "{}", all(&both));
    if option_env!("ZORIX_GITHUB_CLIENT_ID").is_none() {
        let none = z.run(&["connect", "add", "github", "gh", "--oauth"]);
        assert_ne!(code(&none), 0);
        assert!(text(&none.stderr).contains("client id"), "{}", all(&none));
    }
    assert!(text(&z.run(&["connect", "list"]).stderr).contains("No connections yet"));
}
