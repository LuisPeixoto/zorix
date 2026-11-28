//! The two models the OCR engine runs (ADR-0048).
//!
//! Reading scanned pages needs a detection model (where the lines of text are) and a recognition model (what the letters say), about 12 MB
//! together. They are not in the program: turning on "Reading scans" fetches them once into `<home>/models/ocr/`. A model is code of a kind, the
//! parser of its file runs on whatever it is given, so nothing is loaded that does not hash to the value fixed here: a file that was swapped, cut
//! off or served by something else is refused and nothing is kept. The same check runs every time the models are loaded, so a file changed on disk
//! afterwards is not used either. The files can also be put there by hand (for a computer with no way out): the same check decides.

use std::io::Read;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::Duration;

use url::Url;
use zorix_core::bytes_hash;
use zorix_memory::MemoryStack;

use crate::HubError;

/// One model file: where it is fetched from and what it must be.
#[derive(Clone, Copy, Debug)]
pub struct ModelFile {
    /// The name it has on disk.
    pub name: &'static str,
    /// Where it is fetched from: `https`, or `http` for a server on this computer (tests).
    pub url: &'static str,
    /// Its SHA-256, in lowercase hex.
    pub sha256: &'static str,
    /// Its size in bytes. A body of any other size is refused without reading the rest.
    pub size: u64,
}

/// The model that finds the lines of text on a page.
pub const DETECTION: ModelFile = ModelFile {
    name: "text-detection.onnx",
    url: "https://ocrs-models.s3-accelerate.amazonaws.com/text-detection.onnx",
    sha256: "a917b23dbd9524b465df7e922641b3ff2981623df4ded5a0234004ef2fee7cfe",
    size: 2_499_479,
};

/// The model that reads the letters of a line.
pub const RECOGNITION: ModelFile = ModelFile {
    name: "text-recognition.onnx",
    url: "https://ocrs-models.s3-accelerate.amazonaws.com/text-recognition.onnx",
    sha256: "86c145c2edb96c8caed5b1ebb8f44d706408922211451c309c625157dd6061c5",
    size: 9_713_177,
};

/// Both models, detection first.
pub const MODELS: [ModelFile; 2] = [DETECTION, RECOGNITION];

/// The two model files, in place and checked.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ModelFiles {
    /// The detection model.
    pub detection: PathBuf,
    /// The recognition model.
    pub recognition: PathBuf,
}

/// How far a download has come, over both files.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Progress {
    /// Bytes received.
    pub done: u64,
    /// Bytes expected.
    pub total: u64,
}

/// Where the models live.
pub fn folder(home: &Path) -> PathBuf {
    home.join("models").join("ocr")
}

/// The model files, when both are in place and each is what it must be.
pub fn installed(home: &Path) -> Option<ModelFiles> {
    installed_in(&folder(home), &MODELS)
}

/// [`installed`] for a folder and a list of models (tests use their own).
pub fn installed_in(dir: &Path, models: &[ModelFile; 2]) -> Option<ModelFiles> {
    let ok = |model: &ModelFile| is_valid(&dir.join(model.name), model);
    (ok(&models[0]) && ok(&models[1]))
        .then(|| ModelFiles { detection: dir.join(models[0].name), recognition: dir.join(models[1].name) })
}

/// Whether the file is there with the size and the hash the model says. Reads at most one byte more than the size.
fn is_valid(path: &Path, model: &ModelFile) -> bool {
    let Ok(file) = std::fs::File::open(path) else { return false };
    let Ok(meta) = file.metadata() else { return false };
    if !meta.is_file() || meta.len() != model.size {
        return false;
    }
    let mut bytes = Vec::with_capacity(model.size as usize);
    if file.take(model.size + 1).read_to_end(&mut bytes).is_err() {
        return false;
    }
    bytes.len() as u64 == model.size && bytes_hash(&bytes) == model.sha256
}

/// Whether reading scans is working.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum State {
    /// Turned off.
    Off,
    /// On, and the engine is loaded.
    Ready,
    /// On, but the model files are not there (or are not the right ones): nothing is read until they are downloaded again.
    Missing,
}

impl State {
    /// The word the screens and the command line use.
    pub fn as_str(self) -> &'static str {
        match self {
            State::Off => "off",
            State::Ready => "ready",
            State::Missing => "missing",
        }
    }
}

/// Make the memory stack read scans, or stop it, to match `on`. With `on` it loads the engine from the models in `<home>/models/ocr/`, after
/// checking them. Loading takes a moment and happens off the async threads.
pub async fn attach(stack: &MemoryStack, home: &Path, on: bool) -> Result<State, HubError> {
    if !on {
        stack.set_ocr(None);
        return Ok(State::Off);
    }
    let dir = home.to_path_buf();
    let found = tokio::task::spawn_blocking(move || installed(&dir)).await.map_err(|e| HubError(e.to_string()))?;
    let Some(files) = found else {
        stack.set_ocr(None);
        return Ok(State::Missing);
    };
    let reader = tokio::task::spawn_blocking(move || {
        zorix_memory::index::ocr::OcrReader::from_files(&files.detection, &files.recognition)
    })
    .await
    .map_err(|e| HubError(e.to_string()))?
    .map_err(HubError)?;
    stack.set_ocr(Some(Arc::new(reader)));
    Ok(State::Ready)
}

/// Turn reading scans on: fetch the models if they are not there, check them and load the engine.
pub async fn enable(
    stack: &MemoryStack,
    home: &Path,
    progress: &(dyn Fn(Progress) + Send + Sync),
) -> Result<(), HubError> {
    install(home, progress).await?;
    match attach(stack, home, true).await? {
        State::Ready => Ok(()),
        _ => Err(HubError("The OCR models were downloaded but could not be loaded.".into())),
    }
}

/// Whether the address may be fetched: `https`, or `http` for a server on this computer; no name or password in it.
fn allowed(url: &Url) -> bool {
    let loopback = match url.host() {
        Some(url::Host::Domain(name)) => name.eq_ignore_ascii_case("localhost"),
        Some(url::Host::Ipv4(ip)) => ip.is_loopback(),
        Some(url::Host::Ipv6(ip)) => ip.is_loopback(),
        None => return false,
    };
    (url.scheme() == "https" || (url.scheme() == "http" && loopback))
        && url.username().is_empty()
        && url.password().is_none()
}

/// Fetch the models into `<home>/models/ocr/`, unless they are there already, and check them.
pub async fn install(home: &Path, progress: &(dyn Fn(Progress) + Send + Sync)) -> Result<ModelFiles, HubError> {
    install_into(&folder(home), &MODELS, progress).await
}

/// [`install`] for a folder and a list of models (tests use their own).
pub async fn install_into(
    dir: &Path,
    models: &[ModelFile; 2],
    progress: &(dyn Fn(Progress) + Send + Sync),
) -> Result<ModelFiles, HubError> {
    let total: u64 = models.iter().map(|m| m.size).sum();
    if let Some(files) = installed_in(dir, models) {
        progress(Progress { done: total, total });
        return Ok(files);
    }
    for model in models {
        let url = Url::parse(model.url).map_err(|_| HubError("The address of an OCR model is not valid.".into()))?;
        if !allowed(&url) {
            return Err(HubError("The OCR models must be fetched over https.".into()));
        }
    }
    tokio::fs::create_dir_all(dir)
        .await
        .map_err(|e| HubError(format!("The folder for the OCR models could not be made: {e}.")))?;
    // Like every client in ZORIX: the provider is installed once, and an error means one is already there.
    let _ = rustls::crypto::ring::default_provider().install_default();
    let client = reqwest::Client::builder()
        .user_agent("zorix")
        .connect_timeout(Duration::from_secs(15))
        .timeout(Duration::from_secs(900))
        .redirect(reqwest::redirect::Policy::custom(|attempt| {
            if attempt.previous().len() < 5 && allowed(attempt.url()) { attempt.follow() } else { attempt.stop() }
        }))
        .build()
        .map_err(|e| HubError(format!("The download could not be prepared: {e}.")))?;
    let mut done = 0u64;
    for model in models {
        let target = dir.join(model.name);
        if is_valid(&target, model) {
            done += model.size;
            progress(Progress { done, total });
            continue;
        }
        fetch(&client, model, &target, done, total, progress).await?;
        done += model.size;
    }
    installed_in(dir, models)
        .ok_or_else(|| HubError("The OCR models were downloaded but could not be read back.".into()))
}

/// Download one model to a temporary file next to its place, check it and move it there. A failure leaves nothing behind.
async fn fetch(
    client: &reqwest::Client,
    model: &ModelFile,
    target: &Path,
    before: u64,
    total: u64,
    progress: &(dyn Fn(Progress) + Send + Sync),
) -> Result<(), HubError> {
    let mut response = client
        .get(model.url)
        .send()
        .await
        .map_err(|e| HubError(format!("The OCR model {} could not be downloaded: {}.", model.name, reason(&e))))?;
    if !response.status().is_success() {
        return Err(HubError(format!(
            "The OCR model {} could not be downloaded: the server answered {}.",
            model.name,
            response.status().as_u16()
        )));
    }
    if response.content_length().is_some_and(|n| n != model.size) {
        return Err(wrong(model, "it is not the size expected"));
    }
    let mut body: Vec<u8> = Vec::with_capacity(model.size as usize);
    loop {
        let chunk = response
            .chunk()
            .await
            .map_err(|e| HubError(format!("The OCR model {} could not be downloaded: {}.", model.name, reason(&e))))?;
        let Some(chunk) = chunk else { break };
        if body.len() as u64 + chunk.len() as u64 > model.size {
            return Err(wrong(model, "it is larger than expected"));
        }
        body.extend_from_slice(&chunk);
        progress(Progress { done: before + body.len() as u64, total });
    }
    if body.len() as u64 != model.size {
        return Err(wrong(model, "it is shorter than expected"));
    }
    if bytes_hash(&body) != model.sha256 {
        return Err(wrong(model, "its SHA-256 is not the one that is fixed for it"));
    }
    let part = target.with_extension("part");
    let kept = async {
        tokio::fs::write(&part, &body).await?;
        tokio::fs::rename(&part, target).await
    }
    .await;
    if let Err(error) = kept {
        let _ = tokio::fs::remove_file(&part).await;
        return Err(HubError(format!("The OCR model {} could not be saved: {error}.", model.name)));
    }
    Ok(())
}

fn wrong(model: &ModelFile, why: &str) -> HubError {
    HubError(format!(
        "The file downloaded for the OCR model {} is not the one ZORIX expects ({why}). Nothing was kept.",
        model.name
    ))
}

/// What went wrong with a request, without the address (it can carry a token behind a proxy).
fn reason(error: &reqwest::Error) -> &'static str {
    if error.is_timeout() {
        "it took too long"
    } else if error.is_connect() {
        "the server could not be reached"
    } else if error.is_redirect() {
        "it was sent somewhere that is not allowed"
    } else {
        "the connection failed"
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_pinned_values_have_the_shape_of_a_sha256_and_a_size() {
        for model in MODELS {
            assert_eq!(model.sha256.len(), 64, "{}", model.name);
            assert!(model.sha256.chars().all(|c| c.is_ascii_hexdigit() && !c.is_ascii_uppercase()), "{}", model.name);
            assert!(model.size > 1_000_000 && model.size < 50_000_000, "{}", model.name);
            assert!(model.url.starts_with("https://") && model.url.ends_with(model.name), "{}", model.name);
        }
        assert_ne!(DETECTION.sha256, RECOGNITION.sha256);
    }

    #[test]
    fn only_https_or_this_computer_may_be_fetched() {
        let ok = |s: &str| allowed(&Url::parse(s).unwrap());
        assert!(ok("https://models.example.com/a.onnx"));
        assert!(ok("http://127.0.0.1:9/a.onnx"));
        assert!(ok("http://localhost/a.onnx"));
        assert!(ok("http://[::1]/a.onnx"));
        assert!(!ok("http://models.example.com/a.onnx"));
        assert!(!ok("ftp://127.0.0.1/a.onnx"));
        assert!(!ok("file:///etc/passwd"));
        assert!(!ok("https://user:pass@models.example.com/a.onnx"));
        assert!(!ok("http://127.0.0.1.example.com/a.onnx"));
    }
}
