//! Ways a program might reach what it must not (ADR-0022, Phase 12): paths that name the host, links that lead out of a folder it was
//! shown, a mount that tries to take another's place, and writes where only reading was allowed.
//!
//! The list of paths is written for every system. On Windows a backslash, a drive letter or a `\\?\` prefix are ways into the host's
//! files (a sandbox escape of that kind was fixed in a dependency in 0.9.x); on macOS and Linux they are only odd file names. Either
//! way the program must find nothing, and the same test runs on all three in CI.

use std::path::Path;
use std::time::Duration;

use tokio_util::sync::CancellationToken;
use zorix_sandbox::{Language, Limits, Outcome, Run, Sandbox, SandboxError};

const SECRET: &str = "TOP-SECRET-OF-THE-HOST";

fn quick() -> Limits {
    Limits { time: Duration::from_secs(30), ..Limits::default() }
}

async fn go(
    sandbox: &Sandbox,
    code: &str,
    workspace: Option<&Path>,
    mounts: Vec<(&Path, &str)>,
) -> Result<Outcome, SandboxError> {
    let run = Run {
        language: Language::JavaScript,
        code: code.into(),
        stdin: None,
        workspace: workspace.map(Path::to_path_buf),
        mounts: mounts.into_iter().map(|(p, g)| (p.to_path_buf(), g.to_string())).collect(),
    };
    sandbox.run(run, quick(), &CancellationToken::new()).await
}

/// A host folder holding the secret, and inside it the folder a program is shown.
struct Host {
    _dir: tempfile::TempDir,
    outside: std::path::PathBuf,
    shown: std::path::PathBuf,
}

fn host() -> Host {
    let dir = tempfile::tempdir().unwrap();
    let outside = dir.path().join("secret.txt");
    std::fs::write(&outside, SECRET).unwrap();
    let shown = dir.path().join("shown");
    std::fs::create_dir(&shown).unwrap();
    std::fs::write(shown.join("ok.txt"), "visible").unwrap();
    Host { outside, shown, _dir: dir }
}

#[tokio::test]
async fn a_mount_cannot_take_the_place_of_the_scratch_folder_or_the_workspace_or_be_a_relative_path() {
    let dir = tempfile::tempdir().unwrap();
    let sandbox = Sandbox::new(dir.path()).unwrap();
    let folder = tempfile::tempdir().unwrap();
    for guest in ["/work", "/workspace", "work", "skill", "", "C:/x"] {
        let refused = go(&sandbox, "1", None, vec![(folder.path(), guest)]).await;
        assert!(matches!(refused, Err(SandboxError::Failed(_))), "{guest:?} was accepted");
    }
    // A mount that is fine is there, read-only.
    let ok = go(&sandbox, "console.log(std.loadFile('/skill/ok.txt'))", None, vec![(&host().shown, "/skill")])
        .await
        .unwrap();
    assert_eq!(ok.stdout, "visible\n", "{ok:?}");
}

#[tokio::test]
async fn paths_that_could_name_the_host_find_nothing_and_never_the_secret() {
    let dir = tempfile::tempdir().unwrap();
    let sandbox = Sandbox::new(dir.path()).unwrap();
    let host = host();
    let secret = host.outside.to_string_lossy().replace('\\', "\\\\");
    let paths = [
        "/workspace/../secret.txt",
        "/workspace/./../secret.txt",
        "/workspace/a/../../secret.txt",
        "/workspace/..\\secret.txt",
        "/workspace\\..\\secret.txt",
        "\\workspace\\..\\secret.txt",
        "/work/../secret.txt",
        "/work/../../secret.txt",
        "/../secret.txt",
        "/../../../../etc/passwd",
        "/etc/passwd",
        "/proc/self/environ",
        "/proc/self/cwd/secret.txt",
        "C:\\Windows\\win.ini",
        "C:/Windows/win.ini",
        "/C:/Windows/win.ini",
        "\\\\?\\C:\\Windows\\win.ini",
        "\\\\.\\C:\\Windows\\win.ini",
        "//server/share/x",
        "\\\\server\\share\\x",
        "/workspace/C:\\Windows\\win.ini",
        "/workspace/\\\\?\\C:\\Windows\\win.ini",
        "/workspace/..%2fsecret.txt",
        "/workspace/ok.txt/../../secret.txt",
        "~/.ssh/id_rsa",
        "$HOME/.ssh/id_rsa",
        "",
        "\0",
    ];
    let list = paths.iter().map(|p| format!("{p:?}")).collect::<Vec<_>>().join(", ");
    let code = format!(
        "const secret = {secret:?};\n\
         for (const p of [{list}, secret]) {{\n\
           let got = null; try {{ got = std.loadFile(p); }} catch (e) {{ got = null; }}\n\
           console.log((got === null ? 'denied ' : 'READ ') + JSON.stringify(p));\n\
         }}\n\
         let listed = []; for (const p of ['/', '/workspace/..', '/work/..', '/workspace/../..']) {{ try {{ const r = os.readdir(p); listed.push(p + ' -> ' + (r[1] === 0 ? JSON.stringify(r[0].filter(n => n !== '.' && n !== '..')) : 'error')); }} catch (e) {{ listed.push(p + ' -> error'); }} }}\n\
         console.log(listed.join('\\n'));"
    );
    let out = go(&sandbox, &code, Some(&host.shown), vec![]).await.unwrap();
    assert_eq!(out.exit_code, 0, "{}", out.stderr);
    assert!(!out.stdout.contains(SECRET), "the secret was read: {}", out.stdout);
    assert!(!out.stdout.contains("READ "), "a path reached a file of the host: {}", out.stdout);
    for line in out.stdout.lines().filter(|l| l.contains(" -> ")) {
        // Only the folders the program was given are ever listed, never the host's.
        assert!(!line.contains("secret.txt") && !line.contains("shown"), "{line}");
    }
}

#[cfg(unix)]
#[tokio::test]
async fn a_link_in_a_folder_the_program_was_shown_does_not_lead_out_of_it() {
    use std::os::unix::fs::symlink;
    let dir = tempfile::tempdir().unwrap();
    let sandbox = Sandbox::new(dir.path()).unwrap();
    let host = host();
    symlink("../secret.txt", host.shown.join("relative")).unwrap();
    symlink(&host.outside, host.shown.join("absolute")).unwrap();
    symlink("..", host.shown.join("up")).unwrap();
    symlink("/etc", host.shown.join("etc")).unwrap();
    symlink("ok.txt", host.shown.join("inside")).unwrap();
    let code = "for (const p of ['/workspace/relative', '/workspace/absolute', '/workspace/up/secret.txt', '/workspace/etc/passwd', '/skill/relative', '/skill/up/secret.txt']) {\n\
                  let got = null; try { got = std.loadFile(p); } catch (e) { got = null; }\n\
                  console.log((got === null ? 'denied ' : 'READ ') + p);\n\
                }\n\
                for (const p of ['/workspace/up', '/workspace/etc']) { try { const r = os.readdir(p); console.log((r[1] === 0 ? 'LISTED ' : 'blocked ') + p); } catch (e) { console.log('blocked ' + p); } }";
    let out = go(&sandbox, code, Some(&host.shown), vec![(&host.shown, "/skill")]).await.unwrap();
    assert_eq!(out.exit_code, 0, "{}", out.stderr);
    assert!(
        !out.stdout.contains(SECRET) && !out.stdout.contains("READ "),
        "a link led out of the folder: {}",
        out.stdout
    );
    assert!(!out.stdout.contains("LISTED "), "a link let the program list a folder outside: {}", out.stdout);
    assert!(out.stdout.contains("blocked /workspace/up"), "the folders were tried: {}", out.stdout);
}

#[cfg(unix)]
#[tokio::test]
async fn a_link_made_by_the_program_cannot_be_followed_out_of_its_scratch_folder() {
    let dir = tempfile::tempdir().unwrap();
    let sandbox = Sandbox::new(dir.path()).unwrap();
    let host = host();
    let secret = host.outside.to_string_lossy().into_owned();
    let code = format!(
        "const made = [];\n\
         for (const [target, name] of [[{secret:?}, 'host'], ['/etc/passwd', 'etc'], ['../..', 'up'], ['/', 'root']]) {{\n\
           try {{ made.push(name + '=' + (os.symlink(target, '/work/' + name) === 0 ? 'made' : 'no')); }} catch (e) {{ made.push(name + '=refused'); }}\n\
         }}\n\
         console.log(made.join(' '));\n\
         for (const p of ['/work/host', '/work/etc', '/work/up/secret.txt', '/work/root/etc/passwd']) {{\n\
           let got = null; try {{ got = std.loadFile(p); }} catch (e) {{ got = null; }}\n\
           console.log((got === null ? 'denied ' : 'READ ') + p);\n\
         }}"
    );
    let out = go(&sandbox, &code, None, vec![]).await.unwrap();
    assert_eq!(out.exit_code, 0, "{}", out.stderr);
    assert!(!out.stdout.contains(SECRET) && !out.stdout.contains("READ "), "{}", out.stdout);
}

#[tokio::test]
async fn nothing_can_be_written_where_only_reading_was_allowed() {
    let dir = tempfile::tempdir().unwrap();
    let sandbox = Sandbox::new(dir.path()).unwrap();
    let host = host();
    let code = "const tried = [];\n\
                for (const p of ['/workspace/new.txt', '/skill/new.txt', '/workspace/ok.txt', '/skill/ok.txt', '/workspace/dir', '/skill/dir']) {\n\
                  let wrote = 'no'; try {\n\
                    if (p.endsWith('dir')) { wrote = os.mkdir(p) === 0 ? 'yes' : 'no'; }\n\
                    else { const f = std.open(p, 'w'); if (f) { f.puts('overwritten'); f.close(); wrote = 'yes'; } }\n\
                  } catch (e) { wrote = 'no'; }\n\
                  tried.push(p + '=' + wrote);\n\
                }\n\
                for (const p of ['/workspace/ok.txt', '/skill/ok.txt']) { try { tried.push('rm ' + p + '=' + (os.remove(p) === 0 ? 'yes' : 'no')); } catch (e) { tried.push('rm ' + p + '=no'); } }\n\
                console.log(tried.join('\\n'));";
    let out = go(&sandbox, code, Some(&host.shown), vec![(&host.shown, "/skill")]).await.unwrap();
    assert_eq!(out.exit_code, 0, "{}", out.stderr);
    assert!(!out.stdout.contains("=yes"), "something was written or removed: {}", out.stdout);
    assert_eq!(std::fs::read_to_string(host.shown.join("ok.txt")).unwrap(), "visible", "the file is as it was");
    assert!(!host.shown.join("new.txt").exists() && !host.shown.join("dir").exists());
}
