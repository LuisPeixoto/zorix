//! The OCR models: fetched once, checked against fixed hashes, refused when anything is off, and never kept half-written (ADR-0048).
//! A server on this computer stands in for the real one; the real files are checked only by the opt-in test at the end.

use std::collections::HashMap;
use std::path::Path;
use std::sync::Arc;
use std::sync::atomic::{AtomicU64, AtomicUsize, Ordering};

use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::TcpListener;
use zorix_core::bytes_hash;
use zorix_hub::ocr::{self, ModelFile, Progress};

/// What the stand-in server answers for a path.
#[derive(Clone)]
enum Reply {
    /// The bytes, with a right Content-Length.
    Body(Vec<u8>),
    /// A status and nothing else.
    Status(u16),
    /// A redirect to an address.
    Redirect(String),
    /// A body longer than it said (a Content-Length smaller than what follows is not honoured by a client, so this is chunked).
    Chunked(Vec<u8>),
}

struct Server {
    port: u16,
    hits: Arc<AtomicUsize>,
}

async fn serve(routes: HashMap<String, Reply>) -> Server {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let port = listener.local_addr().unwrap().port();
    let hits = Arc::new(AtomicUsize::new(0));
    let counter = hits.clone();
    tokio::spawn(async move {
        loop {
            let Ok((mut socket, _)) = listener.accept().await else { return };
            let (routes, counter) = (routes.clone(), counter.clone());
            tokio::spawn(async move {
                let mut request = Vec::new();
                let mut buffer = [0u8; 2048];
                while !request.windows(4).any(|w| w == b"\r\n\r\n") {
                    match socket.read(&mut buffer).await {
                        Ok(0) | Err(_) => return,
                        Ok(n) => request.extend_from_slice(&buffer[..n]),
                    }
                }
                counter.fetch_add(1, Ordering::SeqCst);
                let text = String::from_utf8_lossy(&request).to_string();
                let path = text.split_whitespace().nth(1).unwrap_or("/").to_string();
                let answer = match routes.get(&path) {
                    Some(Reply::Body(bytes)) => {
                        let mut out =
                            format!("HTTP/1.1 200 OK\r\ncontent-length: {}\r\nconnection: close\r\n\r\n", bytes.len())
                                .into_bytes();
                        out.extend_from_slice(bytes);
                        out
                    }
                    Some(Reply::Chunked(bytes)) => {
                        let mut out =
                            b"HTTP/1.1 200 OK\r\ntransfer-encoding: chunked\r\nconnection: close\r\n\r\n".to_vec();
                        out.extend(format!("{:x}\r\n", bytes.len()).into_bytes());
                        out.extend_from_slice(bytes);
                        out.extend_from_slice(b"\r\n0\r\n\r\n");
                        out
                    }
                    Some(Reply::Status(code)) => {
                        format!("HTTP/1.1 {code} Nope\r\ncontent-length: 0\r\nconnection: close\r\n\r\n").into_bytes()
                    }
                    Some(Reply::Redirect(to)) => format!(
                        "HTTP/1.1 302 Found\r\nlocation: {to}\r\ncontent-length: 0\r\nconnection: close\r\n\r\n"
                    )
                    .into_bytes(),
                    None => b"HTTP/1.1 404 Not Found\r\ncontent-length: 0\r\nconnection: close\r\n\r\n".to_vec(),
                };
                let _ = socket.write_all(&answer).await;
                let _ = socket.shutdown().await;
            });
        }
    });
    Server { port, hits }
}

fn leak(text: String) -> &'static str {
    Box::leak(text.into_boxed_str())
}

/// A model made of `bytes`, to be fetched from `path` on the server.
fn model(name: &'static str, port: u16, path: &str, bytes: &[u8]) -> ModelFile {
    ModelFile {
        name,
        url: leak(format!("http://127.0.0.1:{port}{path}")),
        sha256: leak(bytes_hash(bytes)),
        size: bytes.len() as u64,
    }
}

fn detection_bytes() -> Vec<u8> {
    (0..4_000u32).map(|i| (i % 251) as u8).collect()
}

fn recognition_bytes() -> Vec<u8> {
    (0..9_000u32).map(|i| (i % 241) as u8).collect()
}

async fn two_models(routes: &mut HashMap<String, Reply>) -> ([ModelFile; 2], Server) {
    let (d, r) = (detection_bytes(), recognition_bytes());
    routes.entry("/d".into()).or_insert(Reply::Body(d.clone()));
    routes.entry("/r".into()).or_insert(Reply::Body(r.clone()));
    let server = serve(routes.clone()).await;
    ([model("det.onnx", server.port, "/d", &d), model("rec.onnx", server.port, "/r", &r)], server)
}

fn files_in(dir: &Path) -> Vec<String> {
    let mut names: Vec<String> = std::fs::read_dir(dir)
        .map(|rd| rd.flatten().map(|e| e.file_name().to_string_lossy().to_string()).collect())
        .unwrap_or_default();
    names.sort();
    names
}

fn quiet() -> impl Fn(Progress) + Send + Sync {
    |_| {}
}

#[tokio::test]
async fn the_models_are_downloaded_checked_and_kept_and_a_second_call_does_not_download_again() {
    let dir = tempfile::tempdir().unwrap();
    let mut routes = HashMap::new();
    let (models, server) = two_models(&mut routes).await;
    assert!(ocr::installed_in(dir.path(), &models).is_none());

    let seen = Arc::new(AtomicU64::new(0));
    let sink = seen.clone();
    let files = ocr::install_into(dir.path(), &models, &move |p: Progress| {
        assert!(p.done <= p.total);
        sink.store(p.done, Ordering::SeqCst);
    })
    .await
    .unwrap();
    assert_eq!(std::fs::read(&files.detection).unwrap(), detection_bytes());
    assert_eq!(std::fs::read(&files.recognition).unwrap(), recognition_bytes());
    assert_eq!(files_in(dir.path()), vec!["det.onnx".to_string(), "rec.onnx".to_string()], "no temporary file is left");
    assert_eq!(seen.load(Ordering::SeqCst), 13_000, "the progress reached the end");
    assert_eq!(server.hits.load(Ordering::SeqCst), 2);

    let again = ocr::install_into(dir.path(), &models, &quiet()).await.unwrap();
    assert_eq!(again, files);
    assert_eq!(server.hits.load(Ordering::SeqCst), 2, "nothing was fetched the second time");
    assert_eq!(ocr::installed_in(dir.path(), &models), Some(files));
}

#[tokio::test]
async fn a_file_with_another_hash_is_refused_and_nothing_is_kept() {
    let dir = tempfile::tempdir().unwrap();
    let mut routes = HashMap::new();
    let mut swapped = detection_bytes();
    swapped[10] ^= 0xff;
    routes.insert("/d".to_string(), Reply::Body(swapped));
    let (models, _server) = two_models(&mut routes).await;
    let error = ocr::install_into(dir.path(), &models, &quiet()).await.unwrap_err().to_string();
    assert!(error.contains("det.onnx") && error.contains("SHA-256") && error.contains("Nothing was kept"), "{error}");
    assert!(files_in(dir.path()).is_empty(), "{:?}", files_in(dir.path()));
    assert!(ocr::installed_in(dir.path(), &models).is_none());
}

#[tokio::test]
async fn a_body_longer_or_shorter_than_expected_is_refused() {
    for (what, bytes) in
        [("larger", [detection_bytes(), vec![1; 50]].concat()), ("shorter", detection_bytes()[..3_000].to_vec())]
    {
        let dir = tempfile::tempdir().unwrap();
        let mut routes = HashMap::new();
        routes.insert("/d".to_string(), Reply::Chunked(bytes));
        let (models, _server) = two_models(&mut routes).await;
        let error = ocr::install_into(dir.path(), &models, &quiet()).await.unwrap_err().to_string();
        assert!(error.contains(what) && error.contains("Nothing was kept"), "{what}: {error}");
        assert!(files_in(dir.path()).is_empty(), "{what}: {:?}", files_in(dir.path()));
    }
}

#[tokio::test]
async fn a_server_that_declares_another_size_is_refused_before_the_body_is_read() {
    let dir = tempfile::tempdir().unwrap();
    let mut routes = HashMap::new();
    routes.insert("/d".to_string(), Reply::Body(vec![0; 10]));
    let (models, _server) = two_models(&mut routes).await;
    let error = ocr::install_into(dir.path(), &models, &quiet()).await.unwrap_err().to_string();
    assert!(error.contains("not the size expected"), "{error}");
}

#[tokio::test]
async fn a_server_error_is_said_in_words() {
    let dir = tempfile::tempdir().unwrap();
    let mut routes = HashMap::new();
    routes.insert("/d".to_string(), Reply::Status(503));
    let (models, _server) = two_models(&mut routes).await;
    let error = ocr::install_into(dir.path(), &models, &quiet()).await.unwrap_err().to_string();
    assert!(error.contains("det.onnx") && error.contains("503"), "{error}");
    assert!(files_in(dir.path()).is_empty());
}

#[tokio::test]
async fn a_server_that_is_not_there_is_said_without_its_address() {
    let dir = tempfile::tempdir().unwrap();
    let port = {
        let l = TcpListener::bind("127.0.0.1:0").await.unwrap();
        l.local_addr().unwrap().port()
    };
    let models =
        [model("det.onnx", port, "/d", &detection_bytes()), model("rec.onnx", port, "/r", &recognition_bytes())];
    let error = ocr::install_into(dir.path(), &models, &quiet()).await.unwrap_err().to_string();
    assert!(error.contains("could not be reached") && !error.contains("127.0.0.1"), "{error}");
}

#[tokio::test]
async fn a_redirect_to_somewhere_that_is_not_allowed_is_not_followed() {
    let dir = tempfile::tempdir().unwrap();
    let mut routes = HashMap::new();
    routes.insert("/d".to_string(), Reply::Redirect("http://models.example.com/d".into()));
    let (models, _server) = two_models(&mut routes).await;
    let error = ocr::install_into(dir.path(), &models, &quiet()).await.unwrap_err().to_string();
    assert!(error.contains("det.onnx"), "{error}");
    assert!(files_in(dir.path()).is_empty());
}

#[tokio::test]
async fn an_address_that_is_not_https_is_refused_before_any_request() {
    let dir = tempfile::tempdir().unwrap();
    let hit = ModelFile { name: "a.onnx", url: "http://models.example.com/a.onnx", sha256: "00", size: 10_000 };
    let error = ocr::install_into(dir.path(), &[hit, hit], &quiet()).await.unwrap_err().to_string();
    assert!(error.contains("https"), "{error}");
    assert!(!dir.path().join("a.onnx").exists());
}

#[tokio::test]
async fn a_file_that_was_changed_on_disk_is_not_used_and_is_replaced() {
    let dir = tempfile::tempdir().unwrap();
    let mut routes = HashMap::new();
    let (models, server) = two_models(&mut routes).await;
    ocr::install_into(dir.path(), &models, &quiet()).await.unwrap();
    assert!(ocr::installed_in(dir.path(), &models).is_some());

    // Same size, one byte different: the size alone would let it through.
    let path = dir.path().join("rec.onnx");
    let mut bytes = std::fs::read(&path).unwrap();
    bytes[5] ^= 1;
    std::fs::write(&path, &bytes).unwrap();
    assert!(ocr::installed_in(dir.path(), &models).is_none(), "a changed file is not trusted");

    ocr::install_into(dir.path(), &models, &quiet()).await.unwrap();
    assert_eq!(std::fs::read(&path).unwrap(), recognition_bytes());
    assert_eq!(server.hits.load(Ordering::SeqCst), 3, "only the file that was changed was fetched again");
}

#[tokio::test]
async fn a_file_put_there_by_hand_is_used_when_it_is_the_right_one_and_a_wrong_one_is_not() {
    let dir = tempfile::tempdir().unwrap();
    let port = 9; // never contacted: the files are already there
    let models =
        [model("det.onnx", port, "/d", &detection_bytes()), model("rec.onnx", port, "/r", &recognition_bytes())];
    std::fs::write(dir.path().join("det.onnx"), detection_bytes()).unwrap();
    std::fs::write(dir.path().join("rec.onnx"), recognition_bytes()).unwrap();
    assert!(ocr::installed_in(dir.path(), &models).is_some());
    assert!(ocr::install_into(dir.path(), &models, &quiet()).await.is_ok());

    std::fs::write(dir.path().join("rec.onnx"), b"not a model").unwrap();
    assert!(ocr::installed_in(dir.path(), &models).is_none());
    // A directory in the place of a file, a missing one, and an empty one are all "not installed".
    std::fs::remove_file(dir.path().join("rec.onnx")).unwrap();
    std::fs::create_dir(dir.path().join("rec.onnx")).unwrap();
    assert!(ocr::installed_in(dir.path(), &models).is_none());
    assert!(ocr::installed_in(&dir.path().join("missing"), &models).is_none());
}

#[test]
fn the_models_live_under_the_data_folder() {
    let home = Path::new("/data/zorix");
    assert_eq!(ocr::folder(home), home.join("models").join("ocr"));
    assert!(ocr::installed(&std::env::temp_dir().join("zorix-no-such-home-for-models")).is_none());
}

/// The real files, from the real server. Opt-in: it downloads about 12 MB (`cargo test -p zorix-hub --test ocr -- --ignored`).
#[tokio::test]
#[ignore = "downloads the real OCR models (about 12 MB)"]
async fn the_real_models_still_hash_to_the_pinned_values() {
    let dir = tempfile::tempdir().unwrap();
    let files = ocr::install_into(dir.path(), &ocr::MODELS, &quiet()).await.unwrap();
    assert_eq!(std::fs::metadata(&files.detection).unwrap().len(), ocr::DETECTION.size);
    assert_eq!(std::fs::metadata(&files.recognition).unwrap().len(), ocr::RECOGNITION.size);
}

// ---------------------------------------------------------------------------------------------------------------------
// Attaching the engine to the memory stack

fn stack(home: &Path) -> zorix_memory::MemoryStack {
    let path = home.join("zorix.sqlite");
    let store = Arc::new(zorix_store::SqliteMemoryStore::open(&path).unwrap());
    let repository = Arc::new(zorix_store::SqliteRepository::open(&path).unwrap());
    zorix_memory::MemoryStack::new(store, repository, None)
}

#[tokio::test]
async fn reading_scans_is_off_missing_or_ready_and_the_stack_follows() {
    let home = tempfile::tempdir().unwrap();
    let stack = stack(home.path());
    assert!(!stack.has_ocr());

    assert_eq!(ocr::attach(&stack, home.path(), false).await.unwrap(), ocr::State::Off);
    assert!(!stack.has_ocr());

    // On, but the models were never downloaded: nothing is read, and the state says why.
    assert_eq!(ocr::attach(&stack, home.path(), true).await.unwrap(), ocr::State::Missing);
    assert!(!stack.has_ocr());

    // Files that are not the models are not loaded either: the hash decides, before the model parser sees a byte.
    let dir = ocr::folder(home.path());
    std::fs::create_dir_all(&dir).unwrap();
    std::fs::write(dir.join("text-detection.onnx"), b"not a model").unwrap();
    std::fs::write(dir.join("text-recognition.onnx"), b"not a model either").unwrap();
    assert_eq!(ocr::attach(&stack, home.path(), true).await.unwrap(), ocr::State::Missing);
    assert!(!stack.has_ocr());
    assert_eq!(ocr::State::Off.as_str(), "off");
    assert_eq!(ocr::State::Ready.as_str(), "ready");
    assert_eq!(ocr::State::Missing.as_str(), "missing");
}

/// The real engine from files put in place by hand, then turned off again. Opt-in: set `ZORIX_OCR_MODELS` to a folder that holds
/// `text-detection.onnx` and `text-recognition.onnx`.
#[tokio::test]
#[ignore = "needs the OCR models: set ZORIX_OCR_MODELS to the folder that holds them"]
async fn the_real_models_placed_by_hand_are_loaded_and_unloaded() {
    let source = std::path::PathBuf::from(std::env::var("ZORIX_OCR_MODELS").expect("set ZORIX_OCR_MODELS"));
    let home = tempfile::tempdir().unwrap();
    let dir = ocr::folder(home.path());
    std::fs::create_dir_all(&dir).unwrap();
    for name in ["text-detection.onnx", "text-recognition.onnx"] {
        std::fs::copy(source.join(name), dir.join(name)).unwrap();
    }
    let stack = stack(home.path());
    assert_eq!(ocr::attach(&stack, home.path(), true).await.unwrap(), ocr::State::Ready);
    assert!(stack.has_ocr());
    assert_eq!(ocr::attach(&stack, home.path(), false).await.unwrap(), ocr::State::Off);
    assert!(!stack.has_ocr());
}
