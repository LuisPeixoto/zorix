//! The skill library and its tools: what is imported, what is refused, and what a run can do with a skill.

use std::collections::HashSet;
use std::io::Write;
use std::path::Path;
use std::sync::Arc;

use serde_json::{Value, json};
use tokio_util::sync::CancellationToken;
use zorix_agents::SkillLibrary;
use zorix_agents::skills::{content_hash, skill_tools};
use zorix_core::{
    Action, AgentBinding, AgentSlug, AgentSpec, AgentStore, ExecutionId, Grants, Permission, SkillFile, SkillPin,
    TenantId, Timestamp, Tool, ToolCallId, ToolContext,
};
use zorix_sandbox::Sandbox;
use zorix_store::SqliteAgentStore;

const NOW: Timestamp = Timestamp(5);

fn write(dir: &Path, rel: &str, text: &str) {
    let path = dir.join(rel);
    std::fs::create_dir_all(path.parent().unwrap()).unwrap();
    std::fs::write(path, text).unwrap();
}

fn skill_md(name: &str, description: &str) -> String {
    format!("---\nname: {name}\ndescription: {description}\n---\n\n# Steps\n\n1. Do the thing.\n")
}

/// A skill folder named `name` under a fresh parent.
fn skill_folder(parent: &Path, name: &str) -> std::path::PathBuf {
    let dir = parent.join(name);
    write(&dir, "SKILL.md", &skill_md(name, "Does a thing. Use when a task needs the thing."));
    dir
}

fn library() -> (tempfile::TempDir, SkillLibrary) {
    let home = tempfile::tempdir().unwrap();
    let library = SkillLibrary::new(home.path());
    (home, library)
}

fn invalid(error: zorix_agents::SkillError) -> String {
    error.to_string()
}

// ---------------------------------------------------------------------------------------------------------------------
// The library

#[test]
fn a_skill_folder_is_kept_by_its_content_and_importing_it_again_changes_nothing() {
    let (_home, library) = library();
    let parent = tempfile::tempdir().unwrap();
    let dir = skill_folder(parent.path(), "pdf-tools");
    write(&dir, "references/REFERENCE.md", "Details.");
    write(&dir, "scripts/extract.py", "print('hi')");
    let record = library.import_folder(&dir, None, NOW).unwrap();
    assert_eq!(record.name, "pdf-tools");
    assert_eq!(record.hash.len(), 64);
    assert_eq!(
        record.files.iter().map(|f| f.path.as_str()).collect::<Vec<_>>(),
        ["SKILL.md", "references/REFERENCE.md", "scripts/extract.py"]
    );
    assert!(library.has(&record.hash));
    assert_eq!(library.read(&record.hash, "scripts/extract.py", 1000).unwrap(), b"print('hi')");

    let again = library.import_folder(&dir, None, Timestamp(9)).unwrap();
    assert_eq!(again.hash, record.hash, "the same content is the same skill");
    assert_eq!(std::fs::read_dir(library.root()).unwrap().count(), 1, "and nothing was stored twice");

    write(&dir, "references/REFERENCE.md", "Details, changed.");
    let changed = library.import_folder(&dir, None, NOW).unwrap();
    assert_ne!(changed.hash, record.hash);
    assert!(library.has(&record.hash) && library.has(&changed.hash), "the old content stays for whoever pinned it");
}

#[test]
fn the_hash_ignores_order_and_follows_every_part_of_a_file() {
    let a = SkillFile { path: "a.md".into(), size: 1, sha256: "11".repeat(32) };
    let b = SkillFile { path: "b.md".into(), size: 2, sha256: "22".repeat(32) };
    assert_eq!(content_hash(&[a.clone(), b.clone()]), content_hash(&[b.clone(), a.clone()]));
    let mut moved = a.clone();
    moved.path = "c.md".into();
    assert_ne!(content_hash(&[a.clone(), b.clone()]), content_hash(&[moved, b.clone()]));
    let mut bigger = a.clone();
    bigger.size = 9;
    assert_ne!(content_hash(&[a.clone(), b.clone()]), content_hash(&[bigger, b.clone()]));
    let mut other = a.clone();
    other.sha256 = "33".repeat(32);
    assert_ne!(content_hash(&[a, b.clone()]), content_hash(&[other, b]));
}

#[test]
fn what_is_not_a_skill_is_refused_with_a_reason() {
    let (_home, library) = library();
    let parent = tempfile::tempdir().unwrap();

    let empty = parent.path().join("empty");
    std::fs::create_dir_all(&empty).unwrap();
    assert!(invalid(library.import_folder(&empty, None, NOW).unwrap_err()).contains("SKILL.md at its top"));

    let wrong = parent.path().join("some-folder");
    write(&wrong, "SKILL.md", &skill_md("another-name", "x"));
    assert!(invalid(library.import_folder(&wrong, None, NOW).unwrap_err()).contains("must match the folder"));

    assert!(
        invalid(library.import_folder(&parent.path().join("missing"), None, NOW).unwrap_err())
            .contains("cannot be read")
    );

    let many = skill_folder(parent.path(), "many-files");
    for i in 0..200 {
        write(&many, &format!("f/{i}.txt"), "x");
    }
    assert!(invalid(library.import_folder(&many, None, NOW).unwrap_err()).contains("more than 200 files"));

    let big = skill_folder(parent.path(), "big-file");
    std::fs::write(big.join("data.bin"), vec![0u8; 2 * 1024 * 1024 + 1]).unwrap();
    assert!(invalid(library.import_folder(&big, None, NOW).unwrap_err()).contains("larger than 2 MB"));

    let heavy = skill_folder(parent.path(), "heavy");
    for i in 0..6 {
        std::fs::write(heavy.join(format!("part{i}.bin")), vec![1u8; 1_900_000]).unwrap();
    }
    assert!(invalid(library.import_folder(&heavy, None, NOW).unwrap_err()).contains("larger than 10 MB"));

    let deep = skill_folder(parent.path(), "deep");
    write(&deep, "a/b/c/d/e/f/g/h/i/j/k.txt", "x");
    assert!(invalid(library.import_folder(&deep, None, NOW).unwrap_err()).contains("deeper than"));

    let odd = skill_folder(parent.path(), "odd-name");
    write(&odd, "con.txt", "x");
    assert!(invalid(library.import_folder(&odd, None, NOW).unwrap_err()).contains("reserved name"));

    let binary = parent.path().join("binary-md");
    std::fs::create_dir_all(&binary).unwrap();
    std::fs::write(binary.join("SKILL.md"), [0xff, 0xfe, 0x00]).unwrap();
    assert!(invalid(library.import_folder(&binary, None, NOW).unwrap_err()).contains("UTF-8"));
    assert_eq!(
        std::fs::read_dir(library.root()).map(|d| d.count()).unwrap_or(0),
        0,
        "nothing half-made is left behind"
    );
}

#[cfg(unix)]
#[test]
fn a_link_inside_a_skill_is_refused_and_junk_files_are_left_out() {
    let (_home, library) = library();
    let parent = tempfile::tempdir().unwrap();
    let dir = skill_folder(parent.path(), "linky");
    std::os::unix::fs::symlink("/etc/hosts", dir.join("hosts")).unwrap();
    assert!(invalid(library.import_folder(&dir, None, NOW).unwrap_err()).contains("is a link"));
    std::fs::remove_file(dir.join("hosts")).unwrap();
    write(&dir, ".git/config", "secret");
    write(&dir, ".DS_Store", "x");
    write(&dir, "__pycache__/x.pyc", "x");
    let record = library.import_folder(&dir, None, NOW).unwrap();
    assert_eq!(record.files.len(), 1, "{:?}", record.files);
}

fn zip_of(path: &Path, entries: &[(&str, &[u8], Option<u32>)]) {
    let file = std::fs::File::create(path).unwrap();
    let mut writer = zip::ZipWriter::new(file);
    for (name, bytes, mode) in entries {
        let mut options = zip::write::SimpleFileOptions::default().compression_method(zip::CompressionMethod::Deflated);
        if *mode == Some(0o120777) {
            // A link: the target is what a link points at.
            writer.add_symlink(*name, String::from_utf8_lossy(bytes), options).unwrap();
            continue;
        }
        if let Some(mode) = mode {
            options = options.unix_permissions(*mode);
        }
        writer.start_file(*name, options).unwrap();
        writer.write_all(bytes).unwrap();
    }
    writer.finish().unwrap();
}

#[test]
fn a_zip_is_read_with_or_without_a_folder_around_the_skill() {
    let (_home, library) = library();
    let dir = tempfile::tempdir().unwrap();
    let md = skill_md("zipped-skill", "Zipped. Use when zipped.");
    let with_folder = dir.path().join("a.zip");
    zip_of(
        &with_folder,
        &[
            ("zipped-skill/SKILL.md", md.as_bytes(), None),
            ("zipped-skill/notes/n.md", b"n", None),
            ("__MACOSX/zipped-skill/._SKILL.md", b"junk", None),
        ],
    );
    let a = library.import_zip(&with_folder, None, NOW).unwrap();
    assert_eq!((a.name.as_str(), a.files.len()), ("zipped-skill", 2));

    let flat = dir.path().join("b.zip");
    zip_of(&flat, &[("SKILL.md", md.as_bytes(), None), ("notes/n.md", b"n", None)]);
    let b = library.import_zip(&flat, None, NOW).unwrap();
    assert_eq!(b.hash, a.hash, "the same files are the same skill, however they were zipped");

    let wrong = dir.path().join("c.zip");
    zip_of(&wrong, &[("other-folder/SKILL.md", md.as_bytes(), None)]);
    assert!(invalid(library.import_zip(&wrong, None, NOW).unwrap_err()).contains("must match the folder"));
}

#[test]
fn a_hostile_zip_is_refused() {
    let (_home, library) = library();
    let dir = tempfile::tempdir().unwrap();
    let md = skill_md("evil-skill", "x");

    let slip = dir.path().join("slip.zip");
    zip_of(&slip, &[("SKILL.md", md.as_bytes(), None), ("../outside.txt", b"x", None)]);
    assert!(invalid(library.import_zip(&slip, None, NOW).unwrap_err()).contains("unsafe path"));
    assert!(!dir.path().parent().unwrap().join("outside.txt").exists());

    let absolute = dir.path().join("abs.zip");
    zip_of(&absolute, &[("SKILL.md", md.as_bytes(), None), ("/etc/evil", b"x", None)]);
    // A leading slash is made relative by the zip reader, so the file lands inside the skill, never at /etc.
    match library.import_zip(&absolute, None, NOW) {
        Err(_) => {}
        Ok(record) => assert!(record.files.iter().all(|f| !f.path.starts_with('/')), "{:?}", record.files),
    }
    assert!(!Path::new("/etc/evil").exists());

    let link = dir.path().join("link.zip");
    zip_of(&link, &[("SKILL.md", md.as_bytes(), None), ("hosts", b"/etc/hosts", Some(0o120777))]);
    assert!(invalid(library.import_zip(&link, None, NOW).unwrap_err()).contains("is a link"));

    let bomb = dir.path().join("bomb.zip");
    zip_of(&bomb, &[("SKILL.md", md.as_bytes(), None), ("zeros.bin", &vec![0u8; 3 * 1024 * 1024], None)]);
    assert!(std::fs::metadata(&bomb).unwrap().len() < 100_000, "it is small on disk");
    assert!(invalid(library.import_zip(&bomb, None, NOW).unwrap_err()).contains("larger than 2 MB"));

    let notzip = dir.path().join("not.zip");
    std::fs::write(&notzip, "this is text").unwrap();
    assert!(invalid(library.import_zip(&notzip, None, NOW).unwrap_err()).contains("not a readable zip"));
    // Only the zip whose absolute path was made relative was kept; every other hostile zip left nothing behind.
    assert!(std::fs::read_dir(library.root()).map(|d| d.count()).unwrap_or(0) <= 1);

    // Other ways to climb or to name a place outside: none of them gets a file out of the skill's folder.
    for (n, name) in [
        "a/../../outside2.txt",
        "a/./../../outside3.txt",
        "..\\outside4.txt",
        "C:\\outside5.txt",
        "C:/outside6.txt",
        "//host/share/x",
    ]
    .iter()
    .enumerate()
    {
        let path = dir.path().join(format!("slip{n}.zip"));
        zip_of(&path, &[("SKILL.md", md.as_bytes(), None), (name, b"x", None)]);
        let outcome = library.import_zip(&path, None, NOW);
        if let Ok(record) = &outcome {
            assert!(
                record
                    .files
                    .iter()
                    .all(|f| !f.path.starts_with('/') && !f.path.contains("..") && !f.path.contains(':')),
                "{name}: {:?}",
                record.files
            );
        }
        for stray in ["outside2.txt", "outside3.txt", "outside4.txt", "outside5.txt", "outside6.txt"] {
            assert!(!dir.path().parent().unwrap().join(stray).exists(), "{name} wrote {stray} outside");
            assert!(!dir.path().join(stray).exists(), "{name} wrote {stray} beside the zip");
        }
    }
}

#[test]
fn reading_stays_inside_the_skill_and_cleaning_up_keeps_what_is_still_used() {
    let (home, library) = library();
    let parent = tempfile::tempdir().unwrap();
    let dir = skill_folder(parent.path(), "reader");
    write(&dir, "data.txt", "twelve bytes");
    let record = library.import_folder(&dir, None, NOW).unwrap();
    for bad in ["../SKILL.md", "/etc/passwd", "a/../../b", "", "..", "C:\\x", "nothing.txt"] {
        assert!(library.read(&record.hash, bad, 100).is_err(), "{bad:?}");
    }
    assert!(invalid(library.read(&record.hash, "data.txt", 5).unwrap_err()).contains("12 bytes"));
    assert!(library.path("../../etc").is_err() && library.path("short").is_err());

    write(&dir, "data.txt", "changed");
    let newer = library.import_folder(&dir, None, NOW).unwrap();
    std::fs::create_dir_all(home.path().join("skills/not-a-hash")).unwrap();
    let keep: HashSet<String> = [newer.hash.clone()].into();
    let report = library.gc(&keep).unwrap();
    assert_eq!(report.removed, 1);
    assert!(report.bytes > 0);
    assert!(!library.has(&record.hash) && library.has(&newer.hash));
    assert!(home.path().join("skills/not-a-hash").exists(), "folders that are not skills are not touched");
}

// ---------------------------------------------------------------------------------------------------------------------
// The tools

struct World {
    _home: tempfile::TempDir,
    store: Arc<SqliteAgentStore>,
    library: Arc<SkillLibrary>,
    tools: Vec<Arc<dyn Tool>>,
    binding: Option<AgentBinding>,
    scratch: tempfile::TempDir,
}

async fn world(extra: &[(&str, &str)], pin: &[&str]) -> World {
    let home = tempfile::tempdir().unwrap();
    let store = Arc::new(SqliteAgentStore::open_in_memory().unwrap());
    let library = Arc::new(SkillLibrary::new(home.path()));
    let sandbox = Arc::new(Sandbox::new(home.path()).unwrap());
    let scratch = tempfile::tempdir().unwrap();
    let dir = skill_folder(scratch.path(), "helper");
    write(&dir, "references/guide.md", "Guide text.");
    write(
        &dir,
        "scripts/hello.js",
        "console.log('hello ' + scriptArgs.join(',') + ' ' + std.loadFile('/skill/references/guide.md'));",
    );
    write(
        &dir,
        "scripts/hello.py",
        "import sys\nprint('hello', sys.argv[1:], open('/skill/references/guide.md').read())",
    );
    write(&dir, "scripts/tool.sh", "echo no");
    std::fs::write(dir.join("logo.bin"), [0xffu8, 0x00, 0xfe]).unwrap();
    for (rel, text) in extra {
        write(&dir, rel, text);
    }
    let record = library.import_folder(&dir, None, NOW).unwrap();
    store.record_skill(&TenantId::local(), record.clone()).await.unwrap();
    let mut spec = AgentSpec::new(AgentSlug::new("worker").unwrap(), "Worker", "demo");
    spec.skills = pin.iter().map(|name| SkillPin { name: name.to_string(), hash: record.hash.clone() }).collect();
    let (agent, version) = store.create_agent(&TenantId::local(), spec, None, NOW).await.unwrap();
    let binding = Some(AgentBinding::of(&agent, &version));
    let tools = skill_tools(store.clone(), library.clone(), sandbox);
    World { _home: home, store, library, tools, binding, scratch }
}

impl World {
    fn tool(&self, id: &str) -> &Arc<dyn Tool> {
        self.tools.iter().find(|t| t.spec().id == id).unwrap()
    }

    fn ctx(&self, grants: Grants, agent: bool) -> ToolContext {
        ToolContext {
            execution_id: ExecutionId::new(),
            tenant: TenantId::local(),
            call_id: ToolCallId("c".into()),
            grants,
            cancel: CancellationToken::new(),
            workspace_root: None,
            conversation: None,
            agent: if agent { self.binding.clone() } else { None },
        }
    }

    async fn call(&self, id: &str, grants: Grants, args: Value) -> Result<Value, String> {
        self.tool(id).call(&self.ctx(grants, true), args).await.map_err(|e| e.to_string())
    }
}

fn code(language: &str) -> Grants {
    Grants::none().with(Permission::new(Action::Execute, format!("code:{language}")))
}

#[tokio::test]
async fn loading_a_skill_gives_its_instructions_without_the_frontmatter_and_lists_its_other_files() {
    let w = world(&[], &["helper"]).await;
    let loaded = w.call("skill.load", Grants::none(), json!({"name": "helper"})).await.unwrap();
    assert_eq!(loaded["name"], "helper");
    let instructions = loaded["instructions"].as_str().unwrap();
    assert!(instructions.starts_with("# Steps") && !instructions.contains("name: helper"), "{instructions}");
    let files: Vec<&str> = loaded["files"].as_array().unwrap().iter().map(|f| f["path"].as_str().unwrap()).collect();
    assert!(
        files.contains(&"references/guide.md") && files.contains(&"scripts/hello.js") && !files.contains(&"SKILL.md"),
        "{files:?}"
    );
    assert!(w.tool("skill.load").describe_call(&json!({"name": "helper"})).unwrap().contains("helper"));
}

#[tokio::test]
async fn a_run_sees_only_the_skills_its_agent_pinned() {
    let w = world(&[], &[]).await;
    let error = w.call("skill.load", Grants::none(), json!({"name": "helper"})).await.unwrap_err();
    assert!(error.contains("is not one of your skills") && error.contains("Yours: none"), "{error}");
    let w = world(&[], &["helper"]).await;
    let error = w.call("skill.load", Grants::none(), json!({"name": "other"})).await.unwrap_err();
    assert!(error.contains("Yours: helper"), "{error}");
    // A run with no agent has no skills at all.
    let no_agent = w
        .tool("skill.load")
        .call(&w.ctx(Grants::none(), false), json!({"name": "helper"}))
        .await
        .unwrap_err()
        .to_string();
    assert!(no_agent.contains("belong to an agent"), "{no_agent}");
    // A skill that was removed from the library says what to do.
    w.store.remove_skill(&TenantId::local(), "helper").await.unwrap();
    let error = w.call("skill.load", Grants::none(), json!({"name": "helper"})).await.unwrap_err();
    assert!(error.contains("import it again") || error.contains("not in the library"), "{error}");
}

#[tokio::test]
async fn reading_a_file_needs_a_path_the_skill_lists_and_text() {
    let w = world(&[], &["helper"]).await;
    let ok =
        w.call("skill.read", Grants::none(), json!({"name": "helper", "path": "references/guide.md"})).await.unwrap();
    assert_eq!(ok["text"], "Guide text.");
    for bad in ["../../etc/passwd", "references/missing.md", "/etc/hosts", "scripts/../SKILL.md"] {
        let error = w.call("skill.read", Grants::none(), json!({"name": "helper", "path": bad})).await.unwrap_err();
        assert!(error.contains("has no file") || error.contains("not allowed"), "{bad}: {error}");
    }
    let error = w.call("skill.read", Grants::none(), json!({"name": "helper", "path": "logo.bin"})).await.unwrap_err();
    assert!(error.contains("not a text file"), "{error}");
    assert!(w.library.has(&w.binding.as_ref().unwrap().skills[0].hash));
    let _ = &w.scratch;
}

#[tokio::test]
async fn running_a_script_needs_the_language_grant_a_listed_script_and_runs_in_the_sandbox_with_the_skill_read_only() {
    let w = world(&[], &["helper"]).await;
    let args = json!({"name": "helper", "script": "scripts/hello.js", "args": ["a", "b"]});
    let denied = w.call("skill.run", Grants::none(), args.clone()).await.unwrap_err();
    assert!(
        denied.contains("no permission to run javascript") && denied.contains("--allow-code javascript"),
        "{denied}"
    );

    let ran = w.call("skill.run", code("javascript"), args).await.unwrap();
    assert_eq!(ran["exit_code"], 0, "{ran}");
    assert_eq!(ran["stdout"], "hello a,b Guide text.\n", "{ran}");

    for (script, wants) in [
        ("scripts/tool.sh", "Only scripts/*.py and scripts/*.js"),
        ("references/guide.md", "Only scripts"),
        ("scripts/missing.js", "has no file"),
    ] {
        let error =
            w.call("skill.run", code("javascript"), json!({"name": "helper", "script": script})).await.unwrap_err();
        assert!(error.contains(wants), "{script}: {error}");
    }
    assert!(
        w.tool("skill.run")
            .describe_call(&json!({"name": "helper", "script": "scripts/hello.js"}))
            .unwrap()
            .contains("scripts/hello.js")
    );
}

#[tokio::test]
async fn a_script_cannot_change_its_skill_or_see_anything_else() {
    let w = world(
        &[(
            "scripts/mean.js",
            "let wrote = 'yes'; try { const f = std.open('/skill/references/guide.md', 'w'); wrote = f ? 'yes' : 'no'; if (f) { f.puts('x'); f.close(); } } catch (e) { wrote = 'no'; }\n\
             console.log('wrote=' + wrote); console.log('etc=' + (std.loadFile('/etc/passwd') === null)); console.log('work=' + (std.loadFile('/workspace/x') === null));",
        )],
        &["helper"],
    )
    .await;
    let ran =
        w.call("skill.run", code("javascript"), json!({"name": "helper", "script": "scripts/mean.js"})).await.unwrap();
    let out = ran["stdout"].as_str().unwrap();
    assert!(out.contains("wrote=no") && out.contains("etc=true") && out.contains("work=true"), "{ran}");
    let hash = &w.binding.as_ref().unwrap().skills[0].hash;
    assert_eq!(w.library.read(hash, "references/guide.md", 100).unwrap(), b"Guide text.", "the skill is as it was");
}

/// Python tests use a copy of the module you point at with `ZORIX_PYTHON_WASM` (see the sandbox tests); without it they pass.
#[tokio::test]
async fn a_python_script_runs_and_a_missing_package_comes_with_a_hint() {
    let Ok(source) = std::env::var("ZORIX_PYTHON_WASM") else { return };
    let w = world(&[("scripts/needs.py", "import numpy\nprint(numpy.__version__)")], &["helper"]).await;
    let sandbox_home = w._home.path().join("sandbox");
    std::fs::create_dir_all(&sandbox_home).unwrap();
    std::fs::copy(source, sandbox_home.join("python-3.12.0.wasm")).unwrap();
    let ran = w
        .call("skill.run", code("python"), json!({"name": "helper", "script": "scripts/hello.py", "args": ["x"]}))
        .await
        .unwrap();
    assert_eq!(ran["exit_code"], 0, "{ran}");
    assert!(ran["stdout"].as_str().unwrap().contains("['x'] Guide text."), "{ran}");
    let missing =
        w.call("skill.run", code("python"), json!({"name": "helper", "script": "scripts/needs.py"})).await.unwrap();
    assert_ne!(missing["exit_code"], 0);
    assert!(missing["hint"].as_str().unwrap().contains("standard library"), "{missing}");
}
