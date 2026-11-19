//! JavaScript in the sandbox: it works, and it cannot do what it must not (ADR-0022).

use std::time::{Duration, Instant};

use tokio_util::sync::CancellationToken;
use zorix_sandbox::{Language, Limits, Outcome, Run, Sandbox, StopReason};

fn run(code: &str) -> Run {
    Run { language: Language::JavaScript, code: code.into(), stdin: None, workspace: None }
}

async fn go(sandbox: &Sandbox, code: &str, limits: Limits) -> Outcome {
    sandbox.run(run(code), limits, &CancellationToken::new()).await.unwrap()
}

fn quick() -> Limits {
    Limits { time: Duration::from_secs(3), ..Limits::default() }
}

#[tokio::test]
async fn a_program_runs_and_its_output_and_exit_code_come_back() {
    let dir = tempfile::tempdir().unwrap();
    let sandbox = Sandbox::new(dir.path()).unwrap();
    let started = Instant::now();
    let first = go(&sandbox, "console.log('hello', 6 * 7); console.error('careful');", Limits::default()).await;
    eprintln!("first run (compiles the module): {:?}", started.elapsed());
    assert_eq!(
        (first.stdout.as_str(), first.stderr.as_str(), first.exit_code, first.stopped),
        ("hello 42\n", "careful\n", 0, None)
    );
    let started = Instant::now();
    let second = go(&sandbox, "console.log([1,2,3].map(x => x * 2).join(','))", Limits::default()).await;
    eprintln!("second run: {:?}", started.elapsed());
    assert_eq!(second.stdout, "2,4,6\n");
}

#[tokio::test]
async fn a_program_that_never_stops_is_stopped_at_the_time_limit_and_can_be_cancelled() {
    let dir = tempfile::tempdir().unwrap();
    let sandbox = Sandbox::new(dir.path()).unwrap();
    go(&sandbox, "1", quick()).await; // compile once, so the timing below is the run's
    let started = Instant::now();
    let out = go(&sandbox, "while (true) {}", Limits { time: Duration::from_millis(500), ..Limits::default() }).await;
    assert_eq!(out.stopped, Some(StopReason::Time));
    assert!(started.elapsed() < Duration::from_secs(3), "{:?}", started.elapsed());
    assert!(StopReason::Time.explain(&Limits::default()).contains("10 seconds"));

    let token = CancellationToken::new();
    let stopper = token.clone();
    tokio::spawn(async move {
        tokio::time::sleep(Duration::from_millis(200)).await;
        stopper.cancel();
    });
    let started = Instant::now();
    let out = sandbox.run(run("while (true) {}"), Limits::default(), &token).await.unwrap();
    assert_eq!(out.stopped, Some(StopReason::Cancelled));
    assert!(started.elapsed() < Duration::from_secs(3));
}

#[tokio::test]
async fn a_program_that_wants_too_much_memory_or_writes_too_much_is_stopped() {
    let dir = tempfile::tempdir().unwrap();
    let sandbox = Sandbox::new(dir.path()).unwrap();
    let small = Limits {
        memory_bytes: 64 * 1024 * 1024,
        time: Duration::from_secs(8),
        output_bytes: 64 * 1024,
        ..Limits::default()
    };
    let hungry = go(&sandbox, "const keep = []; while (true) { keep.push(new Array(500000).fill(7)); }", small).await;
    // Either the runtime refuses the growth and the program dies, or the host stops it: it never gets past the limit.
    assert!(hungry.stopped == Some(StopReason::Memory) || hungry.exit_code != 0, "{hungry:?}");
    assert!(hungry.elapsed < Duration::from_secs(8), "{:?}", hungry.elapsed);

    let loud = go(&sandbox, "while (true) { console.log('x'.repeat(1000)); }", small).await;
    assert_eq!(loud.stopped, Some(StopReason::Output), "{:?}", loud.stderr);
    assert!(loud.stdout.len() <= 64 * 1024, "the output is capped at the limit, not at what was written");
    assert!(StopReason::Output.explain(&small).contains("64 KB"));
}

#[tokio::test]
async fn a_program_sees_no_environment_no_host_files_and_no_network() {
    let dir = tempfile::tempdir().unwrap();
    let sandbox = Sandbox::new(dir.path()).unwrap();
    let secret = dir.path().join("secret.txt");
    std::fs::write(&secret, "TOP-SECRET").unwrap();
    let code = format!(
        "console.log('home=' + std.getenv('HOME')); console.log('path=' + std.getenv('PATH')); console.log('user=' + std.getenv('USER'));\n\
         console.log('passwd=' + (std.loadFile('/etc/passwd') === null)); console.log('secret=' + (std.loadFile({:?}) === null));\n\
         console.log('up=' + (std.loadFile('/work/../secret.txt') === null)); console.log('root=' + JSON.stringify(os.readdir('/')[0].filter(n => n !== '.' && n !== '..')));\n\
         console.log('fetch=' + typeof fetch); console.log('socket=' + typeof os.socket);",
        secret.to_string_lossy()
    );
    let out = go(&sandbox, &code, quick()).await;
    assert_eq!(out.exit_code, 0, "{}", out.stderr);
    for expected in [
        "home=undefined",
        "path=undefined",
        "user=undefined",
        "passwd=true",
        "secret=true",
        "up=true",
        "fetch=undefined",
        "socket=undefined",
    ] {
        assert!(out.stdout.contains(expected), "{expected} in {:?}", out.stdout);
    }
    assert!(out.stdout.contains("root=[]"), "nothing of the host is listed: {}", out.stdout);
}

#[tokio::test]
async fn files_go_to_a_scratch_folder_that_does_not_survive_and_a_workspace_is_read_only() {
    let dir = tempfile::tempdir().unwrap();
    let sandbox = Sandbox::new(dir.path()).unwrap();
    let workspace = tempfile::tempdir().unwrap();
    std::fs::write(workspace.path().join("notes.txt"), "from the workspace").unwrap();
    let write = "const f = std.open('/work/out.txt', 'w'); f.puts('kept?'); f.close(); console.log('wrote');";
    assert_eq!(go(&sandbox, write, quick()).await.stdout, "wrote\n");
    let check = "console.log('later=' + (std.loadFile('/work/out.txt') === null));";
    assert_eq!(go(&sandbox, check, quick()).await.stdout, "later=true\n", "nothing is left for the next run");

    let with_files = Run {
        language: Language::JavaScript,
        code: "console.log(std.loadFile('/workspace/notes.txt')); let wrote = 'yes'; try { const f = std.open('/workspace/new.txt', 'w'); wrote = f ? 'yes' : 'no'; if (f) { f.puts('x'); f.close(); } } catch (e) { wrote = 'no'; } console.log('wrote=' + wrote);".into(),
        stdin: None,
        workspace: Some(workspace.path().to_path_buf()),
    };
    let out = sandbox.run(with_files, quick(), &CancellationToken::new()).await.unwrap();
    assert!(out.stdout.starts_with("from the workspace\n"), "{out:?}");
    assert!(!workspace.path().join("new.txt").exists(), "the workspace was not changed: {out:?}");
    assert_eq!(std::fs::read_to_string(workspace.path().join("notes.txt")).unwrap(), "from the workspace");
}

#[tokio::test]
async fn a_program_that_fills_its_scratch_folder_is_stopped() {
    let dir = tempfile::tempdir().unwrap();
    let sandbox = Sandbox::new(dir.path()).unwrap();
    let limits = Limits { scratch_bytes: 1024 * 1024, time: Duration::from_secs(8), ..Limits::default() };
    let out = go(&sandbox, "const f = std.open('/work/big.bin', 'w'); const chunk = 'x'.repeat(65536); while (true) { f.puts(chunk); f.flush(); }", limits).await;
    assert_eq!(out.stopped, Some(StopReason::Disk), "{out:?}");
}

#[tokio::test]
async fn standard_input_exit_codes_and_errors_work() {
    let dir = tempfile::tempdir().unwrap();
    let sandbox = Sandbox::new(dir.path()).unwrap();
    let echoed = sandbox
        .run(
            Run {
                language: Language::JavaScript,
                code: "console.log(std.in.readAsString().toUpperCase());".into(),
                stdin: Some("hello there".into()),
                workspace: None,
            },
            quick(),
            &CancellationToken::new(),
        )
        .await
        .unwrap();
    assert_eq!(echoed.stdout, "HELLO THERE\n");
    assert_eq!(go(&sandbox, "std.exit(3)", quick()).await.exit_code, 3);
    let broken = go(&sandbox, "function (", quick()).await;
    assert_eq!(broken.exit_code, 1);
    assert!(broken.stderr.contains("SyntaxError"), "{}", broken.stderr);
    let thrown = go(&sandbox, "throw new Error('boom')", quick()).await;
    assert!(thrown.exit_code != 0 && thrown.stderr.contains("boom"), "{thrown:?}");
}

#[tokio::test]
async fn many_runs_at_once_do_not_see_each_other() {
    let dir = tempfile::tempdir().unwrap();
    let sandbox = std::sync::Arc::new(Sandbox::new(dir.path()).unwrap());
    go(&sandbox, "1", quick()).await;
    let mut handles = Vec::new();
    for i in 0..8 {
        let sandbox = sandbox.clone();
        handles.push(tokio::spawn(async move {
            let code = format!("const f = std.open('/work/mine.txt', 'w'); f.puts('{i}'); f.close(); console.log(std.loadFile('/work/mine.txt'));");
            go(&sandbox, &code, quick()).await.stdout
        }));
    }
    for (i, handle) in handles.into_iter().enumerate() {
        assert_eq!(handle.await.unwrap(), format!("{i}\n"));
    }
}

#[test]
fn the_embedded_runtime_is_the_one_that_was_checked() {
    use sha2::{Digest, Sha256};
    let bytes = std::fs::read(concat!(env!("CARGO_MANIFEST_DIR"), "/wasm/qjs-wasi.wasm")).unwrap();
    let hex: String = Sha256::digest(&bytes).iter().map(|b| format!("{b:02x}")).collect();
    assert_eq!(hex, zorix_sandbox::runner::QUICKJS_SHA256);
}
