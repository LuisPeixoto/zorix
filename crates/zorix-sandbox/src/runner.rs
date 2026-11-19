//! Running a module: limits, mounts and what comes back.

use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::{Duration, Instant};

use parking_lot::Mutex;
use sha2::{Digest, Sha256};
use tokio_util::sync::CancellationToken;
use wasmtime::{Config, Engine, Linker, Module, Store, StoreLimits, StoreLimitsBuilder};
use wasmtime_wasi::p1::{WasiP1Ctx, add_to_linker_async};
use wasmtime_wasi::p2::pipe::{MemoryInputPipe, MemoryOutputPipe};
use wasmtime_wasi::{FsPerms, I32Exit, WasiCtxBuilder};

/// QuickJS (quickjs-ng 0.18.0, MIT) as a WASI command, shipped inside ZORIX.
const QUICKJS: &[u8] = include_bytes!("../wasm/qjs-wasi.wasm");
/// Put in front of a JavaScript program (on one line, so line numbers in errors stay the program's own).
const JS_PRELUDE: &str = "if (typeof console.error !== 'function') { const e = (...a) => std.err.puts(a.map(String).join(' ') + '\\n'); console.error = e; console.warn = e; } if (typeof console.info !== 'function') { console.info = console.log; } if (typeof console.debug !== 'function') { console.debug = console.log; } ";
/// Its SHA-256, checked when it is loaded and by a test.
pub const QUICKJS_SHA256: &str = "2c218412cb8b17e46ee91bea7556ec38f5b9b6bd6312fddd3ceeed14aa71eaaa";

/// What can be run.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Language {
    /// JavaScript, with QuickJS.
    JavaScript,
    /// Python 3.12, from a module that is downloaded once.
    Python,
}

impl Language {
    /// The name used in grants and arguments.
    pub fn as_str(&self) -> &'static str {
        match self {
            Language::JavaScript => "javascript",
            Language::Python => "python",
        }
    }

    /// Read a name.
    pub fn parse(text: &str) -> Option<Self> {
        match text {
            "javascript" => Some(Language::JavaScript),
            "python" => Some(Language::Python),
            _ => None,
        }
    }

    fn file(&self) -> &'static str {
        match self {
            Language::JavaScript => "main.js",
            Language::Python => "main.py",
        }
    }
}

/// How much a run may use.
#[derive(Clone, Copy, Debug)]
pub struct Limits {
    /// Most memory the guest may have.
    pub memory_bytes: usize,
    /// Longest a run may take.
    pub time: Duration,
    /// Most bytes of standard output and of standard error, each.
    pub output_bytes: usize,
    /// Most bytes in the scratch folder (checked while it runs, so a burst can briefly go over).
    pub scratch_bytes: u64,
}

impl Default for Limits {
    fn default() -> Self {
        Self {
            memory_bytes: 256 * 1024 * 1024,
            time: Duration::from_secs(10),
            output_bytes: 1024 * 1024,
            scratch_bytes: 50 * 1024 * 1024,
        }
    }
}

/// A run to make.
#[derive(Clone, Debug)]
pub struct Run {
    /// The language.
    pub language: Language,
    /// The program text.
    pub code: String,
    /// Text for the program's standard input.
    pub stdin: Option<String>,
    /// A folder to show the program read-only as `/workspace`.
    pub workspace: Option<PathBuf>,
}

/// Why a run was stopped before it finished.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum StopReason {
    /// It ran longer than the limit.
    Time,
    /// It wanted more memory than the limit.
    Memory,
    /// It wrote more than the output limit.
    Output,
    /// Its scratch folder grew past the limit.
    Disk,
    /// The call was cancelled.
    Cancelled,
}

impl StopReason {
    /// A sentence for people and models.
    pub fn explain(&self, limits: &Limits) -> String {
        match self {
            StopReason::Time => format!("It ran longer than {} seconds and was stopped.", limits.time.as_secs()),
            StopReason::Memory => {
                format!("It needed more than {} MB of memory and was stopped.", limits.memory_bytes / 1024 / 1024)
            }
            StopReason::Output => {
                format!("It wrote more than {} KB of output and was stopped.", limits.output_bytes / 1024)
            }
            StopReason::Disk => {
                format!("It wrote more than {} MB of files and was stopped.", limits.scratch_bytes / 1024 / 1024)
            }
            StopReason::Cancelled => "The call was cancelled.".into(),
        }
    }
}

/// What a run produced.
#[derive(Clone, Debug)]
pub struct Outcome {
    /// What it wrote to standard output.
    pub stdout: String,
    /// What it wrote to standard error.
    pub stderr: String,
    /// The exit code (`0` is success; `-1` when it was stopped).
    pub exit_code: i32,
    /// Why it was stopped, if it was.
    pub stopped: Option<StopReason>,
    /// How long it took.
    pub elapsed: Duration,
}

/// Why a run could not even start.
#[derive(Debug, thiserror::Error)]
pub enum SandboxError {
    /// Python has not been installed.
    #[error("Python is not installed. Install it once with `zorix sandbox install python`.")]
    PythonMissing,
    /// The module could not be loaded.
    #[error("The {0} runtime could not be loaded: {1}")]
    Load(&'static str, String),
    /// Something else went wrong.
    #[error("{0}")]
    Failed(String),
}

struct Host {
    wasi: WasiP1Ctx,
    limits: StoreLimits,
}

/// Where Python comes from: one pinned file, accepted only if its checksum matches (ADR-0022).
pub const PYTHON_URL: &str = "https://github.com/vmware-labs/webassembly-language-runtimes/releases/download/python%2F3.12.0%2B20231211-040d5a6/python-3.12.0.wasm";
/// Its SHA-256.
pub const PYTHON_SHA256: &str = "e5dc5a398b07b54ea8fdb503bf68fb583d533f10ec3f930963e02b9505f7a763";
/// Its size in bytes.
pub const PYTHON_BYTES: u64 = 26_267_204;
/// What it is called in the data folder.
pub const PYTHON_FILE: &str = "python-3.12.0.wasm";

struct Inner {
    engine: Engine,
    data_dir: PathBuf,
    modules: Mutex<std::collections::HashMap<Language, Module>>,
}

/// Runs programs in WebAssembly.
pub struct Sandbox {
    inner: Arc<Inner>,
}

fn fail(error: impl std::fmt::Display) -> SandboxError {
    SandboxError::Failed(error.to_string())
}

fn hex(bytes: &[u8]) -> String {
    bytes.iter().map(|b| format!("{b:02x}")).collect()
}

/// The size of a folder's files, counting at most a little over `stop_at` bytes.
fn folder_size(path: &Path, stop_at: u64) -> u64 {
    let mut total = 0;
    let mut pending = vec![path.to_path_buf()];
    while let Some(dir) = pending.pop() {
        let Ok(entries) = std::fs::read_dir(&dir) else { continue };
        for entry in entries.flatten() {
            let Ok(meta) = entry.metadata() else { continue };
            if meta.is_dir() {
                pending.push(entry.path());
            } else {
                total += meta.len();
                if total > stop_at {
                    return total;
                }
            }
        }
    }
    total
}

impl Inner {
    fn python_path(&self) -> PathBuf {
        self.data_dir.join(PYTHON_FILE)
    }

    fn bytes_of(&self, language: Language) -> Result<Vec<u8>, SandboxError> {
        match language {
            Language::JavaScript => {
                if hex(&Sha256::digest(QUICKJS)) != QUICKJS_SHA256 {
                    return Err(SandboxError::Load(
                        "JavaScript",
                        "the embedded module does not match its checksum".into(),
                    ));
                }
                Ok(QUICKJS.to_vec())
            }
            Language::Python => {
                let bytes = std::fs::read(self.python_path()).map_err(|_| SandboxError::PythonMissing)?;
                if hex(&Sha256::digest(&bytes)) != PYTHON_SHA256 {
                    return Err(SandboxError::Load(
                        "Python",
                        "the installed module does not match its checksum; install it again".into(),
                    ));
                }
                Ok(bytes)
            }
        }
    }

    /// The compiled module of a language: from memory, from the cache on disk, or compiled now (once).
    fn module(&self, language: Language) -> Result<Module, SandboxError> {
        if let Some(module) = self.modules.lock().get(&language) {
            return Ok(module.clone());
        }
        let bytes = self.bytes_of(language)?;
        let name = format!(
            "{}-{}-{}.cwasm",
            language.as_str(),
            &hex(&Sha256::digest(&bytes))[..16],
            env!("CARGO_PKG_VERSION")
        );
        let cache = self.data_dir.join("cache").join(name);
        let module = match Self::load_cached(&self.engine, &cache) {
            Some(module) => module,
            None => {
                let module = Module::new(&self.engine, &bytes)
                    .map_err(|e| SandboxError::Load(language.as_str(), e.to_string()))?;
                if let (Ok(compiled), Some(dir)) = (module.serialize(), cache.parent()) {
                    let _ = std::fs::create_dir_all(dir);
                    let _ = std::fs::write(&cache, compiled);
                }
                module
            }
        };
        self.modules.lock().insert(language, module.clone());
        Ok(module)
    }

    #[allow(unsafe_code)]
    fn load_cached(engine: &Engine, path: &Path) -> Option<Module> {
        if !path.is_file() {
            return None;
        }
        // SAFETY: the file is one this program wrote under its own data folder, named by the hash of the module it was made from
        // and the version of ZORIX; wasmtime itself refuses a file made by another wasmtime or with other settings. Whoever can
        // change files in the data folder can already do more than that.
        unsafe { Module::deserialize_file(engine, path).ok() }
    }
}

impl Sandbox {
    /// A sandbox that keeps what it downloads and compiles under `data_dir/sandbox`.
    pub fn new(data_dir: &Path) -> Result<Self, SandboxError> {
        let mut config = Config::new();
        config.epoch_interruption(true);
        let engine = Engine::new(&config).map_err(fail)?;
        let inner =
            Arc::new(Inner { engine, data_dir: data_dir.join("sandbox"), modules: Mutex::new(Default::default()) });
        // The epoch ticks, so a program that never yields can be interrupted. The thread ends when the sandbox is dropped.
        let ticker = Arc::downgrade(&inner);
        std::thread::Builder::new()
            .name("zorix-sandbox-epoch".into())
            .spawn(move || {
                while let Some(inner) = ticker.upgrade() {
                    inner.engine.increment_epoch();
                    drop(inner);
                    std::thread::sleep(Duration::from_millis(10));
                }
            })
            .map_err(fail)?;
        Ok(Self { inner })
    }

    /// Whether Python has been installed.
    pub fn python_installed(&self) -> bool {
        self.inner.python_path().is_file()
    }

    /// Where the Python module is (or would be).
    pub fn python_file(&self) -> PathBuf {
        self.inner.python_path()
    }

    /// The folder this sandbox keeps its files in.
    pub fn folder(&self) -> &Path {
        &self.inner.data_dir
    }

    /// Run a program.
    pub async fn run(&self, run: Run, limits: Limits, cancel: &CancellationToken) -> Result<Outcome, SandboxError> {
        let started = Instant::now();
        let language = run.language;
        let module = {
            let inner = self.inner.clone();
            tokio::task::spawn_blocking(move || inner.module(language)).await.map_err(fail)??
        };

        let scratch = tempfile::tempdir().map_err(fail)?;
        let code = match language {
            // QuickJS's WASI build has no console.error; the program gets one that writes to standard error.
            Language::JavaScript => format!("{JS_PRELUDE}{}", run.code),
            Language::Python => run.code.clone(),
        };
        std::fs::write(scratch.path().join(language.file()), code).map_err(fail)?;
        let stdout = MemoryOutputPipe::new(limits.output_bytes);
        let stderr = MemoryOutputPipe::new(limits.output_bytes);
        let mut wasi = WasiCtxBuilder::new();
        wasi.stdout(stdout.clone()).stderr(stderr.clone());
        wasi.stdin(MemoryInputPipe::new(run.stdin.clone().unwrap_or_default().into_bytes()));
        let program = format!("/work/{}", language.file());
        match language {
            Language::JavaScript => wasi.args(&["qjs", "--std", program.as_str()]),
            Language::Python => wasi.args(&["python", program.as_str()]),
        };
        if language == Language::Python {
            wasi.env("PYTHONDONTWRITEBYTECODE", "1").env("PYTHONUNBUFFERED", "1");
        }
        wasi.preopened_dir(scratch.path(), "/work", FsPerms::ReadWrite).map_err(fail)?;
        if let Some(folder) = &run.workspace {
            wasi.preopened_dir(folder, "/workspace", FsPerms::ReadOnly).map_err(fail)?;
        }
        let host = Host {
            wasi: wasi.build_p1(),
            limits: StoreLimitsBuilder::new()
                .memory_size(limits.memory_bytes)
                .instances(1)
                .memories(1)
                .tables(8)
                .trap_on_grow_failure(true)
                .build(),
        };
        let mut store = Store::new(&self.inner.engine, host);
        store.limiter(|h| &mut h.limits);
        // Yield to the executor every 10 ms so the time limit (and a cancel) can end a program that never gives up.
        store.set_epoch_deadline(1);
        store.epoch_deadline_async_yield_and_update(1);
        let mut linker = Linker::new(&self.inner.engine);
        add_to_linker_async(&mut linker, |h: &mut Host| &mut h.wasi).map_err(fail)?;

        let program_run = async {
            let instance = linker.instantiate_async(&mut store, &module).await?;
            let start = instance.get_typed_func::<(), ()>(&mut store, "_start")?;
            start.call_async(&mut store, ()).await
        };
        // The scratch folder is watched while the program runs.
        let watch_path = scratch.path().to_path_buf();
        let watch_limit = limits.scratch_bytes;
        let disk = async move {
            loop {
                tokio::time::sleep(Duration::from_millis(100)).await;
                let path = watch_path.clone();
                if tokio::task::spawn_blocking(move || folder_size(&path, watch_limit)).await.unwrap_or(0) > watch_limit
                {
                    return;
                }
            }
        };
        // A program that keeps writing after its output stream is full would otherwise run until the time limit.
        let (watch_out, watch_err, cap) = (stdout.clone(), stderr.clone(), limits.output_bytes);
        let output = async move {
            loop {
                tokio::time::sleep(Duration::from_millis(50)).await;
                if watch_out.contents().len() >= cap || watch_err.contents().len() >= cap {
                    return;
                }
            }
        };
        enum End {
            Done(wasmtime::Result<()>),
            Stopped(StopReason),
        }
        let end = tokio::select! {
            result = program_run => End::Done(result),
            _ = tokio::time::sleep(limits.time) => End::Stopped(StopReason::Time),
            _ = disk => End::Stopped(StopReason::Disk),
            _ = output => End::Stopped(StopReason::Output),
            _ = cancel.cancelled() => End::Stopped(StopReason::Cancelled),
        };
        let text_of = |pipe: &MemoryOutputPipe| String::from_utf8_lossy(&pipe.contents()).into_owned();
        let (exit_code, stopped, crash) = match end {
            End::Stopped(reason) => (-1, Some(reason), None),
            End::Done(Ok(())) => (0, None, None),
            End::Done(Err(error)) => {
                if let Some(exit) = error.downcast_ref::<I32Exit>() {
                    (exit.0, None, None)
                } else {
                    let text = format!("{error:#}");
                    if text.contains("write beyond capacity") {
                        (-1, Some(StopReason::Output), None)
                    } else if text.contains("epoch") || text.contains("interrupt") {
                        (-1, Some(StopReason::Time), None)
                    } else if text.contains("memory")
                        && (text.contains("grow") || text.contains("limit") || text.contains("exceed"))
                    {
                        (-1, Some(StopReason::Memory), None)
                    } else {
                        // A crash in the program: an unreachable instruction, an out-of-bounds access, a failed allocation.
                        (134, None, Some(text))
                    }
                }
            }
        };
        let mut err_text = text_of(&stderr);
        // A program that gives up when its output stream fills (Python raises BrokenPipeError) still hit the output limit.
        let stopped = stopped.or_else(|| {
            (stdout.contents().len() >= limits.output_bytes || stderr.contents().len() >= limits.output_bytes)
                .then_some(StopReason::Output)
        });
        if let Some(why) = crash {
            tracing::debug!(%why, "a sandboxed program crashed");
            err_text.push_str("\n[the program crashed]");
        }
        Ok(Outcome { stdout: text_of(&stdout), stderr: err_text, exit_code, stopped, elapsed: started.elapsed() })
    }
}
