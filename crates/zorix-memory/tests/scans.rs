//! Scanned PDFs: left out with a reason while nothing can read them, read once something can, and not read again and again when they cannot be.
//! The engine itself is replaced by a stand-in that answers from a script; what is tested here is everything around it (ADR-0048).
#![cfg(feature = "ocr")]

use std::path::Path;
use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};

use lopdf::{Dictionary, Document, Object, Stream, dictionary};
use parking_lot::Mutex;
use tokio_util::sync::CancellationToken;
use zorix_core::{
    Corpus, DocStatus, KnowledgeRoot, MemoryStore, OCR_KIND, SCAN_NOTE, ScanCounts, Scope, TenantId, Visibility,
};
use zorix_memory::index::{Indexer, PageReader, SyncReport};
use zorix_store::SqliteMemoryStore;

struct Env {
    _dir: tempfile::TempDir,
    folder: std::path::PathBuf,
    store: Arc<SqliteMemoryStore>,
}

fn env() -> Env {
    let dir = tempfile::tempdir().unwrap();
    let folder = dir.path().join("docs");
    std::fs::create_dir_all(&folder).unwrap();
    Env { store: Arc::new(SqliteMemoryStore::open(&dir.path().join("zorix.sqlite")).unwrap()), folder, _dir: dir }
}

/// A PDF whose pages are one grey picture each and carry no text.
fn scan(pages: usize, shade: u8) -> Vec<u8> {
    let mut doc = Document::with_version("1.5");
    let pages_id = doc.new_object_id();
    let mut kids = Vec::new();
    for _ in 0..pages {
        let image = doc.add_object(Stream::new(
            dictionary! {
                "Type" => "XObject", "Subtype" => "Image", "Width" => 2, "Height" => 2,
                "ColorSpace" => "DeviceGray", "BitsPerComponent" => 8,
            },
            vec![shade; 4],
        ));
        let mut resources = Dictionary::new();
        resources.set("XObject", dictionary! { "Im0" => image });
        let content = doc.add_object(Stream::new(dictionary! {}, b"q 100 0 0 100 0 0 cm /Im0 Do Q".to_vec()));
        let page = doc.add_object(dictionary! {
            "Type" => "Page", "Parent" => pages_id, "Contents" => content, "Resources" => resources,
            "MediaBox" => vec![0.into(), 0.into(), 100.into(), 100.into()],
        });
        kids.push(Object::Reference(page));
    }
    doc.objects.insert(
        pages_id,
        Object::Dictionary(dictionary! { "Type" => "Pages", "Kids" => kids, "Count" => pages as i64 }),
    );
    let catalog = doc.add_object(dictionary! { "Type" => "Catalog", "Pages" => pages_id });
    doc.trailer.set("Root", catalog);
    let mut out = Vec::new();
    doc.save_to(&mut out).unwrap();
    out
}

/// A stand-in for the OCR engine: it counts its calls and answers what it was told to.
struct Script {
    calls: AtomicUsize,
    answer: Mutex<Result<String, String>>,
}

impl Script {
    fn saying(text: &str) -> Arc<Self> {
        Arc::new(Self { calls: AtomicUsize::new(0), answer: Mutex::new(Ok(text.into())) })
    }
    fn failing(why: &str) -> Arc<Self> {
        Arc::new(Self { calls: AtomicUsize::new(0), answer: Mutex::new(Err(why.into())) })
    }
    fn calls(&self) -> usize {
        self.calls.load(Ordering::SeqCst)
    }
}

impl PageReader for Script {
    fn read(&self, _: u32, _: u32, _: &[u8]) -> Result<String, String> {
        self.calls.fetch_add(1, Ordering::SeqCst);
        self.answer.lock().clone()
    }
}

fn tenant() -> TenantId {
    TenantId::local()
}

async fn add(env: &Env, indexer: &Indexer) -> KnowledgeRoot {
    indexer.add_root(&tenant(), Scope::Global, &env.folder, vec![], vec![], false).await.unwrap()
}

async fn sync(indexer: &Indexer, root: &KnowledgeRoot) -> SyncReport {
    indexer.sync(root, &|_| {}, &CancellationToken::new()).await.unwrap()
}

fn plain(env: &Env) -> Indexer {
    Indexer::new(env.store.clone(), None)
}

fn reading(env: &Env, reader: &Arc<Script>) -> Indexer {
    let reader: Arc<dyn PageReader> = reader.clone();
    Indexer::new(env.store.clone(), None).with_ocr(Some(reader))
}

/// Change a file so that the next sync notices, even on a coarse clock.
fn change(path: &Path, bytes: &[u8]) {
    let before = std::fs::metadata(path).unwrap().modified().unwrap();
    std::fs::write(path, bytes).unwrap();
    let file = std::fs::OpenOptions::new().write(true).open(path).unwrap();
    file.set_modified(before + std::time::Duration::from_secs(5)).unwrap();
}

#[tokio::test]
async fn a_scan_is_left_out_with_a_reason_until_something_reads_it_and_then_it_is_read_without_the_file_changing() {
    let env = env();
    std::fs::write(env.folder.join("contract.pdf"), scan(2, 200)).unwrap();
    let root = add(&env, &plain(&env)).await;

    // Nothing reads scans: the file is left out and the reason says what to do.
    let report = sync(&plain(&env), &root).await;
    assert_eq!((report.indexed, report.skipped), (0, 1), "{report:?}");
    let problems = env.store.problem_documents(root.id).await.unwrap();
    assert!(problems[0].2.starts_with(SCAN_NOTE) && problems[0].2.contains("Settings"), "{problems:?}");
    assert_eq!(env.store.scan_counts(root.id).await.unwrap(), ScanCounts { read: 0, unread: 1 });

    // Reading scans is turned on. The file is the same, and still it is read.
    let reader = Script::saying("Payment is due within thirty days of the invoice");
    let report = sync(&reading(&env, &reader), &root).await;
    assert_eq!((report.indexed, report.skipped, report.unchanged), (1, 0, 0), "{report:?}");
    assert_eq!(reader.calls(), 2, "one call a page");
    assert_eq!(env.store.scan_counts(root.id).await.unwrap(), ScanCounts { read: 1, unread: 0 });
    assert!(env.store.problem_documents(root.id).await.unwrap().is_empty());

    // The text is found, with the page it came from.
    let visibility = Visibility::tenant(tenant());
    let hits = env.store.text_search(&visibility, Corpus::Knowledge, &["invoice".to_string()], 5).await.unwrap();
    assert!(!hits.is_empty());
    let chunk = &env.store.chunks(&tenant(), &[hits[0].id.parse().unwrap()]).await.unwrap()[0];
    assert!(chunk.citation().starts_with("contract.pdf#page="), "{}", chunk.citation());
    assert_eq!(env.store.document_states(root.id).await.unwrap()["contract.pdf"].status, DocStatus::Indexed);

    // The next sync leaves it alone: it is no longer waiting for a reader.
    let report = sync(&reading(&env, &reader), &root).await;
    assert_eq!((report.indexed, report.unchanged), (0, 1), "{report:?}");
    assert_eq!(reader.calls(), 2);
}

#[tokio::test]
async fn what_was_read_stays_when_reading_scans_is_turned_off_again() {
    let env = env();
    std::fs::write(env.folder.join("contract.pdf"), scan(1, 200)).unwrap();
    let reader = Script::saying("Termination needs ninety days of notice");
    let root = add(&env, &plain(&env)).await;
    sync(&reading(&env, &reader), &root).await;
    let report = sync(&plain(&env), &root).await;
    assert_eq!((report.indexed, report.skipped, report.unchanged), (0, 0, 1), "{report:?}");
    assert_eq!(env.store.scan_counts(root.id).await.unwrap(), ScanCounts { read: 1, unread: 0 });
}

#[tokio::test]
async fn a_scan_that_could_not_be_read_is_not_tried_again_until_its_file_changes() {
    let env = env();
    let path = env.folder.join("blurred.pdf");
    std::fs::write(&path, scan(1, 90)).unwrap();
    let broken = Script::failing("the engine could not make out any lines");
    let root = add(&env, &plain(&env)).await;

    let report = sync(&reading(&env, &broken), &root).await;
    assert_eq!((report.indexed, report.skipped), (0, 1), "{report:?}");
    assert_eq!(broken.calls(), 1);
    let problems = env.store.problem_documents(root.id).await.unwrap();
    assert!(
        problems[0].2.starts_with("the scan could not be read") && problems[0].2.contains("page 1"),
        "{problems:?}"
    );
    assert_eq!(env.store.scan_counts(root.id).await.unwrap(), ScanCounts { read: 0, unread: 1 });

    // A sync after a sync (the watcher does this all day) costs nothing: the file is not read again.
    let report = sync(&reading(&env, &broken), &root).await;
    assert_eq!((report.skipped, report.unchanged), (0, 1), "{report:?}");
    assert_eq!(broken.calls(), 1, "the same scan is not read again");

    // A new version of the file is tried again.
    let better = Script::saying("Now it is a clean copy of the policy");
    change(&path, &scan(1, 250));
    let report = sync(&reading(&env, &better), &root).await;
    assert_eq!((report.indexed, report.skipped), (1, 0), "{report:?}");
    assert_eq!(better.calls(), 1);
    assert_eq!(env.store.scan_counts(root.id).await.unwrap(), ScanCounts { read: 1, unread: 0 });
}

#[tokio::test]
async fn only_the_scan_is_counted_as_read_by_the_engine_and_other_files_never_reach_it() {
    let env = env();
    std::fs::write(env.folder.join("scan.pdf"), scan(1, 10)).unwrap();
    std::fs::write(env.folder.join("notes.txt"), "Plain notes about the quarterly review").unwrap();
    let reader = Script::saying("Quarterly figures");
    let root = add(&env, &plain(&env)).await;
    let report = sync(&reading(&env, &reader), &root).await;
    assert_eq!(report.indexed, 2, "{report:?}");
    assert_eq!(reader.calls(), 1, "the text file was not sent to the engine");
    assert_eq!(env.store.scan_counts(root.id).await.unwrap(), ScanCounts { read: 1, unread: 0 });
    assert_eq!(OCR_KIND, "pdf-ocr");
}

#[test]
fn an_attached_scan_is_read_with_the_engine_and_refused_with_the_way_out_without_it() {
    let env = env();
    let path = env.folder.join("receipt.pdf");
    std::fs::write(&path, scan(2, 120)).unwrap();

    let without = zorix_memory::attach::read_attachments(std::slice::from_ref(&path), None).unwrap_err().to_string();
    assert!(without.starts_with("“receipt.pdf” cannot be attached") && without.contains("reading scans"), "{without}");

    let reader = Script::saying("Total due 42.00");
    let engine: Arc<dyn PageReader> = reader.clone();
    let attached = zorix_memory::attach::read_attachments(&[path], Some(engine)).unwrap();
    assert_eq!(reader.calls(), 2);
    assert_eq!(attached[0].text, "[page 1]\nTotal due 42.00\n\n[page 2]\nTotal due 42.00");
}

#[tokio::test]
async fn a_scan_with_too_many_pages_is_left_out_and_counted_as_not_read() {
    let env = env();
    std::fs::write(env.folder.join("archive.pdf"), scan(101, 10)).unwrap();
    let reader = Script::saying("x");
    let root = add(&env, &plain(&env)).await;
    let report = sync(&reading(&env, &reader), &root).await;
    assert_eq!((report.indexed, report.skipped), (0, 1), "{report:?}");
    assert_eq!(reader.calls(), 0);
    let problems = env.store.problem_documents(root.id).await.unwrap();
    assert!(problems[0].2.starts_with("the scan has 101 pages"), "{problems:?}");
    assert_eq!(env.store.scan_counts(root.id).await.unwrap().unread, 1);
}

/// A one-page PDF whose page is the JPEG `jpeg`, as a scanner would make it.
fn scan_of_jpeg(jpeg: &[u8], width: i64, height: i64) -> Vec<u8> {
    let mut doc = Document::with_version("1.5");
    let pages_id = doc.new_object_id();
    let image = doc.add_object(Stream::new(
        dictionary! {
            "Type" => "XObject", "Subtype" => "Image", "Width" => width, "Height" => height,
            "ColorSpace" => "DeviceRGB", "BitsPerComponent" => 8, "Filter" => "DCTDecode",
        },
        jpeg.to_vec(),
    ));
    let mut resources = Dictionary::new();
    resources.set("XObject", dictionary! { "Im0" => image });
    let content = doc.add_object(Stream::new(dictionary! {}, b"q 400 0 0 85 0 0 cm /Im0 Do Q".to_vec()));
    let page = doc.add_object(dictionary! {
        "Type" => "Page", "Parent" => pages_id, "Contents" => content, "Resources" => resources,
        "MediaBox" => vec![0.into(), 0.into(), 400.into(), 85.into()],
    });
    doc.objects.insert(
        pages_id,
        Object::Dictionary(dictionary! { "Type" => "Pages", "Kids" => vec![Object::Reference(page)], "Count" => 1 }),
    );
    let catalog = doc.add_object(dictionary! { "Type" => "Catalog", "Pages" => pages_id });
    doc.trailer.set("Root", catalog);
    let mut out = Vec::new();
    doc.save_to(&mut out).unwrap();
    out
}

/// The real engine on a real picture of text. It needs the two model files, which are not in the repository: point `ZORIX_OCR_MODELS` at a
/// folder holding `text-detection.onnx` and `text-recognition.onnx` (Settings downloads them to `<home>/models/ocr`), and run it with `--ignored`.
#[tokio::test]
#[ignore = "needs the OCR models: set ZORIX_OCR_MODELS to the folder that holds them"]
async fn the_real_engine_reads_a_scanned_page() {
    let models = std::env::var("ZORIX_OCR_MODELS").expect("set ZORIX_OCR_MODELS");
    let models = std::path::PathBuf::from(models);
    let reader = zorix_memory::index::ocr::OcrReader::from_files(
        &models.join("text-detection.onnx"),
        &models.join("text-recognition.onnx"),
    )
    .unwrap();
    let env = env();
    let jpeg = include_bytes!("fixtures/scan-page.jpg");
    std::fs::write(env.folder.join("invoice.pdf"), scan_of_jpeg(jpeg, 800, 170)).unwrap();
    let reader: Arc<dyn PageReader> = Arc::new(reader);
    let indexer = Indexer::new(env.store.clone(), None).with_ocr(Some(reader));
    let root = add(&env, &indexer).await;
    let started = std::time::Instant::now();
    let report = sync(&indexer, &root).await;
    eprintln!("read the scan in {:?}", started.elapsed());
    assert_eq!((report.indexed, report.skipped, report.failed), (1, 0, 0), "{report:?}");
    let visibility = Visibility::tenant(tenant());
    for word in ["payment", "thirty", "invoice"] {
        let hits = env.store.text_search(&visibility, Corpus::Knowledge, &[word.to_string()], 5).await.unwrap();
        assert!(!hits.is_empty(), "'{word}' was not found in what the engine read");
    }
    let hits = env.store.text_search(&visibility, Corpus::Knowledge, &["payment".to_string()], 1).await.unwrap();
    let chunk = &env.store.chunks(&tenant(), &[hits[0].id.parse().unwrap()]).await.unwrap()[0];
    eprintln!("the engine read: {:?}", chunk.text);
    assert_eq!(env.store.scan_counts(root.id).await.unwrap(), ScanCounts { read: 1, unread: 0 });
}
