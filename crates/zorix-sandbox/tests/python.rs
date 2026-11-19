//! Python in the sandbox. The module is 26 MB, so these tests use a copy you point at with `ZORIX_PYTHON_WASM=/path/to/python-3.12.0.wasm`
//! (CI downloads it with the installer's own checksum rules); without it they do nothing and pass.

use std::time::Duration;

use tokio_util::sync::CancellationToken;
use zorix_sandbox::{Language, Limits, Outcome, Run, Sandbox, StopReason};

fn sandbox_with_python() -> Option<(tempfile::TempDir, Sandbox)> {
    let source = std::env::var("ZORIX_PYTHON_WASM").ok()?;
    let dir = tempfile::tempdir().unwrap();
    let sandbox = Sandbox::new(dir.path()).unwrap();
    std::fs::create_dir_all(sandbox.folder()).unwrap();
    std::fs::copy(source, sandbox.python_file()).unwrap();
    Some((dir, sandbox))
}

async fn py(sandbox: &Sandbox, code: &str, limits: Limits) -> Outcome {
    sandbox
        .run(
            Run { language: Language::Python, code: code.into(), stdin: None, workspace: None },
            limits,
            &CancellationToken::new(),
        )
        .await
        .unwrap()
}

fn quick() -> Limits {
    Limits { time: Duration::from_secs(20), ..Limits::default() }
}

#[tokio::test]
async fn python_runs_with_its_standard_library_and_exit_codes() {
    let Some((_dir, sandbox)) = sandbox_with_python() else { return };
    let out = py(
        &sandbox,
        "import json, math, sys\nprint(json.dumps({'pi': round(math.pi, 3), 'v': sys.version_info[:2]}))",
        quick(),
    )
    .await;
    assert_eq!((out.stdout.as_str(), out.exit_code), ("{\"pi\": 3.142, \"v\": [3, 12]}\n", 0), "{out:?}");
    let failed = py(&sandbox, "raise SystemExit(4)", quick()).await;
    assert_eq!(failed.exit_code, 4);
    let broken = py(&sandbox, "def (:", quick()).await;
    assert!(broken.exit_code != 0 && broken.stderr.contains("SyntaxError"), "{broken:?}");
}

#[tokio::test]
async fn python_sees_no_environment_no_host_files_and_no_network_and_is_stopped_at_the_limits() {
    let Some((_dir, sandbox)) = sandbox_with_python() else { return };
    let out = py(
        &sandbox,
        "import os, socket\n\
         print('home', os.environ.get('HOME'), os.environ.get('PATH'))\n\
         try:\n    open('/etc/passwd').read(); print('passwd readable')\nexcept OSError as e:\n    print('passwd blocked')\n\
         try:\n    s = socket.socket(); s.connect(('1.1.1.1', 80)); print('network works')\nexcept Exception as e:\n    print('network blocked')\n\
         open('/work/out.txt', 'w').write('x'); print('scratch ok')",
        quick(),
    )
    .await;
    assert_eq!(out.exit_code, 0, "{out:?}");
    for expected in ["home None None", "passwd blocked", "network blocked", "scratch ok"] {
        assert!(out.stdout.contains(expected), "{expected} in {:?}", out.stdout);
    }
    let looped =
        py(&sandbox, "while True: pass", Limits { time: Duration::from_millis(700), ..Limits::default() }).await;
    assert_eq!(looped.stopped, Some(StopReason::Time));
    let flood = py(
        &sandbox,
        "while True: print('x' * 1000)",
        Limits { output_bytes: 32 * 1024, time: Duration::from_secs(10), ..Limits::default() },
    )
    .await;
    assert_eq!(flood.stopped, Some(StopReason::Output));
    let hungry = py(
        &sandbox,
        "a = []\nwhile True: a.append(bytearray(10_000_000))",
        Limits { memory_bytes: 128 * 1024 * 1024, time: Duration::from_secs(15), ..Limits::default() },
    )
    .await;
    assert!(hungry.stopped == Some(StopReason::Memory) || hungry.exit_code != 0, "{hungry:?}");
}

#[tokio::test]
async fn without_the_module_python_says_how_to_install_it() {
    let dir = tempfile::tempdir().unwrap();
    let sandbox = Sandbox::new(dir.path()).unwrap();
    assert!(!sandbox.python_installed());
    let error = sandbox
        .run(
            Run { language: Language::Python, code: "print(1)".into(), stdin: None, workspace: None },
            quick(),
            &CancellationToken::new(),
        )
        .await
        .unwrap_err();
    assert!(error.to_string().contains("zorix sandbox install python"), "{error}");
}

#[tokio::test]
async fn a_download_that_does_not_match_the_pinned_checksum_is_thrown_away() {
    use wiremock::matchers::method;
    use wiremock::{Mock, MockServer, ResponseTemplate};
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .respond_with(ResponseTemplate::new(200).set_body_bytes(vec![0u8; 5000]))
        .mount(&server)
        .await;
    let dir = tempfile::tempdir().unwrap();
    let sandbox = Sandbox::new(dir.path()).unwrap();
    // A plain-HTTP loopback address is only reachable here because the installer's client does not check the scheme of the first request.
    let error = sandbox.install_python_from(&format!("{}/python.wasm", server.uri()), |_, _| {}).await.unwrap_err();
    assert!(error.to_string().contains("checksum differs"), "{error}");
    assert!(!sandbox.python_installed(), "nothing was kept");
    assert!(!sandbox.folder().join("python-3.12.0.wasm.part").exists());
}
