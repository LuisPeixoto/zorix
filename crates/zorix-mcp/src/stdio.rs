//! A server that is a program on this computer, spoken to over its standard streams (ADR-0018).
//!
//! It runs as the user, with a clean environment plus what a person approved, in its own process group so that closing
//! it also ends what it started. Messages are lines; a line longer than the limit closes the connection.

use std::collections::{HashMap, VecDeque};
use std::path::PathBuf;
use std::process::Stdio;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, AtomicI64, Ordering};
use std::time::Duration;

use async_trait::async_trait;
use parking_lot::Mutex;
use serde_json::{Value, json};
use tokio::io::{AsyncBufRead, AsyncBufReadExt, AsyncWriteExt, BufReader};
use tokio::process::Command;
use tokio::sync::{Notify, broadcast, mpsc, oneshot};
use tokio_util::sync::CancellationToken;
use zorix_core::redact_text;

use crate::error::{METHOD_NOT_FOUND, McpError};
use crate::sandbox::Confinement;
use crate::transport::{LOG_BYTES, MAX_MESSAGE_BYTES, ServerEvent, Transport};
use crate::wire::{self, Incoming, RpcError};

/// Variables passed to every server, because programs cannot start without them. Nothing else of ours is.
const PASSED_THROUGH: &[&str] = &[
    "PATH",
    "HOME",
    "USER",
    "LOGNAME",
    "LANG",
    "LC_ALL",
    "TMPDIR",
    "TMP",
    "TEMP",
    "SYSTEMROOT",
    "SYSTEMDRIVE",
    "USERPROFILE",
    "APPDATA",
    "LOCALAPPDATA",
    "PROGRAMFILES",
    "PROGRAMDATA",
    "PATHEXT",
    "COMSPEC",
];

/// How to start a server.
#[derive(Clone, Debug)]
pub struct StdioConfig {
    /// The program.
    pub command: String,
    /// Its arguments.
    pub args: Vec<String>,
    /// Variables to give it, with their values already resolved.
    pub env: Vec<(String, String)>,
    /// Where to start it.
    pub cwd: Option<PathBuf>,
}

type Pending = HashMap<i64, oneshot::Sender<Result<Value, RpcError>>>;

struct Ring {
    lines: VecDeque<String>,
    bytes: usize,
}

impl Ring {
    fn push(&mut self, line: String) {
        self.bytes += line.len();
        self.lines.push_back(line);
        while self.bytes > LOG_BYTES {
            match self.lines.pop_front() {
                Some(old) => self.bytes -= old.len(),
                None => break,
            }
        }
    }
}

struct Shared {
    pending: Mutex<Pending>,
    next: AtomicI64,
    alive: AtomicBool,
    events: broadcast::Sender<ServerEvent>,
    log: Mutex<Ring>,
    writer: Mutex<Option<mpsc::Sender<String>>>,
    kill: CancellationToken,
    /// Cancelled once the process is gone and what it wrote on its error stream has been read.
    settled: CancellationToken,
    exited: Notify,
}

impl Shared {
    fn fail_all(&self, error: McpError) {
        let waiting: Vec<_> = self.pending.lock().drain().collect();
        // Dropping the senders wakes every waiter with "closed"; the reason is delivered through the event.
        drop(waiting);
        let _ = self.events.send(ServerEvent::Closed(error.to_string()));
    }

    fn end(&self, reason: McpError) {
        if self.alive.swap(false, Ordering::SeqCst) {
            self.fail_all(reason);
        }
    }

    async fn send(&self, line: String) -> Result<(), McpError> {
        let sender = self.writer.lock().clone();
        match sender {
            Some(sender) if self.alive.load(Ordering::SeqCst) => sender.send(line).await.map_err(|_| McpError::Closed),
            _ => Err(McpError::Closed),
        }
    }
}

/// A running stdio server.
pub struct StdioTransport {
    shared: Arc<Shared>,
}

enum Line {
    Complete,
    Eof,
    TooLarge,
}

async fn read_line<R: AsyncBufRead + Unpin>(reader: &mut R, buf: &mut Vec<u8>, cap: usize) -> std::io::Result<Line> {
    buf.clear();
    loop {
        let available = reader.fill_buf().await?;
        if available.is_empty() {
            return Ok(if buf.is_empty() { Line::Eof } else { Line::Complete });
        }
        match available.iter().position(|b| *b == b'\n') {
            Some(at) => {
                if buf.len() + at > cap {
                    return Ok(Line::TooLarge);
                }
                buf.extend_from_slice(&available[..at]);
                reader.consume(at + 1);
                return Ok(Line::Complete);
            }
            None => {
                let n = available.len();
                if buf.len() + n > cap {
                    return Ok(Line::TooLarge);
                }
                buf.extend_from_slice(available);
                reader.consume(n);
            }
        }
    }
}

/// Throw away what is left of a line that was too long, up to and including its newline, and not a byte after it. (Reading "until a
/// short read" instead ate whatever followed the end of the line, so the next line came out cut or missing.)
async fn skip_line<R: AsyncBufRead + Unpin>(reader: &mut R) -> std::io::Result<()> {
    loop {
        let available = reader.fill_buf().await?;
        if available.is_empty() {
            return Ok(());
        }
        match available.iter().position(|b| *b == b'\n') {
            Some(at) => {
                reader.consume(at + 1);
                return Ok(());
            }
            None => {
                let n = available.len();
                reader.consume(n);
            }
        }
    }
}

/// On Windows a bare `npx` is `npx.cmd`; look for the real file the way the shell would.
#[cfg(windows)]
fn resolve(command: &str) -> String {
    let path = std::env::var_os("PATH").unwrap_or_default();
    let exts = std::env::var("PATHEXT").unwrap_or_else(|_| ".COM;.EXE;.BAT;.CMD".into());
    if std::path::Path::new(command).extension().is_some() || command.contains(['\\', '/']) {
        return command.to_string();
    }
    for dir in std::env::split_paths(&path) {
        for ext in exts.split(';') {
            let candidate = dir.join(format!("{command}{ext}"));
            if candidate.is_file() {
                return candidate.to_string_lossy().into_owned();
            }
        }
    }
    command.to_string()
}

#[cfg(not(windows))]
fn resolve(command: &str) -> String {
    command.to_string()
}

/// Send `signal` to the server's process group. The server leads its own group (see `process_group(0)` below), so this reaches what
/// it started too, and it can still be done after the leader has gone: a group lives as long as one process is in it.
#[cfg(unix)]
fn signal_group(pid: u32, signal: libc::c_int) {
    // SAFETY: `kill` takes plain integers and has no memory effects; a negative pid addresses a process group.
    unsafe {
        libc::kill(-(pid as i32), signal);
    }
}

#[cfg(unix)]
fn kill_group(pid: u32) {
    signal_group(pid, libc::SIGKILL);
}

#[cfg(unix)]
fn terminate_group(pid: u32) {
    signal_group(pid, libc::SIGTERM);
}

/// How long a server and what it started get to leave after they are asked to, before they are made to.
#[cfg(unix)]
const TERMINATE_GRACE: Duration = Duration::from_secs(2);

impl StdioTransport {
    /// Start the server.
    pub fn spawn(config: StdioConfig) -> Result<Self, McpError> {
        Self::spawn_confined(config, None)
    }

    /// Start the server, restricted to what `confinement` allows (ADR-0050).
    pub fn spawn_confined(config: StdioConfig, mut confinement: Option<Confinement>) -> Result<Self, McpError> {
        let mut command = match &mut confinement {
            Some(confinement) => {
                confinement.prepare()?;
                // The program is found before it is confined: the sandbox must know where it sits to let it be read, and a name that
                // is not there is reported as such instead of as whatever the wrapper would say.
                // A container has its own programs; only a native sandbox needs the one here.
                let program = if confinement.container.is_some() {
                    PathBuf::from(&config.command)
                } else {
                    let name = resolve(&config.command);
                    crate::sandbox::find_program(&name, config.cwd.as_deref())
                        .ok_or_else(|| McpError::Spawn(format!("“{}” was not found on the PATH.", config.command)))?
                };
                let forward: Vec<String> = config.env.iter().map(|(name, _)| name.clone()).collect();
                confinement.command(&program, &config.args, &forward)?
            }
            None => {
                let mut command = Command::new(resolve(&config.command));
                command.args(&config.args);
                command
            }
        };
        command.env_clear().stdin(Stdio::piped()).stdout(Stdio::piped()).stderr(Stdio::piped()).kill_on_drop(true);
        for name in PASSED_THROUGH {
            if let Some(value) = std::env::var_os(name) {
                command.env(name, value);
            }
        }
        if let Some(confinement) = &confinement {
            for (name, value) in confinement.environment() {
                command.env(name, value);
            }
            // Where the container program finds its daemon.
            if confinement.container.is_some() {
                for name in ["DOCKER_HOST", "DOCKER_CONFIG", "DOCKER_CONTEXT", "CONTAINER_HOST", "XDG_RUNTIME_DIR"] {
                    if let Some(value) = std::env::var_os(name) {
                        command.env(name, value);
                    }
                }
            }
        }
        for (name, value) in &config.env {
            command.env(name, value);
        }
        match (&config.cwd, &confinement) {
            (Some(cwd), _) => {
                command.current_dir(cwd);
            }
            (None, Some(confinement)) => {
                command.current_dir(&confinement.server_dir);
            }
            (None, None) => {}
        }
        #[cfg(unix)]
        command.process_group(0);
        #[cfg(windows)]
        command.creation_flags(0x0800_0000); // CREATE_NO_WINDOW

        let mut child = command.spawn().map_err(|e| {
            let hint = if e.kind() == std::io::ErrorKind::NotFound {
                format!(" “{}” was not found on the PATH.", config.command)
            } else {
                String::new()
            };
            McpError::Spawn(format!("{e}.{hint}"))
        })?;
        let (stdin, stdout, stderr) = match (child.stdin.take(), child.stdout.take(), child.stderr.take()) {
            (Some(i), Some(o), Some(e)) => (i, o, e),
            _ => return Err(McpError::Spawn("the standard streams of the server could not be opened".into())),
        };
        let (events, _) = broadcast::channel(256);
        let (write_tx, mut write_rx) = mpsc::channel::<String>(256);
        let shared = Arc::new(Shared {
            pending: Mutex::new(HashMap::new()),
            next: AtomicI64::new(1),
            alive: AtomicBool::new(true),
            events,
            log: Mutex::new(Ring { lines: VecDeque::new(), bytes: 0 }),
            writer: Mutex::new(Some(write_tx)),
            kill: CancellationToken::new(),
            settled: CancellationToken::new(),
            exited: Notify::new(),
        });

        // Writer: one line at a time; closing the channel closes the server's stdin, which is how it is told to stop.
        let mut stdin = stdin;
        let for_writer = shared.clone();
        tokio::spawn(async move {
            while let Some(line) = write_rx.recv().await {
                if stdin.write_all(line.as_bytes()).await.is_err()
                    || stdin.write_all(b"\n").await.is_err()
                    || stdin.flush().await.is_err()
                {
                    for_writer.end(McpError::Transport("the server stopped reading".into()));
                    return;
                }
            }
            let _ = stdin.shutdown().await;
        });

        // Reader.
        let for_reader = shared.clone();
        tokio::spawn(async move {
            let mut reader = BufReader::new(stdout);
            let mut buf = Vec::new();
            loop {
                match read_line(&mut reader, &mut buf, MAX_MESSAGE_BYTES).await {
                    Ok(Line::Complete) => {
                        let text = String::from_utf8_lossy(&buf).into_owned();
                        if text.trim().is_empty() {
                            continue;
                        }
                        Self::handle(&for_reader, &text).await;
                    }
                    Ok(Line::Eof) => {
                        for_reader.end(McpError::Closed);
                        return;
                    }
                    Ok(Line::TooLarge) => {
                        for_reader.end(McpError::TooLarge { limit: MAX_MESSAGE_BYTES });
                        for_reader.kill.cancel();
                        return;
                    }
                    Err(e) => {
                        for_reader.end(McpError::Transport(e.to_string()));
                        return;
                    }
                }
            }
        });

        // Standard error: what the server says about itself. Kept, redacted, bounded.
        let for_log = shared.clone();
        let read_to_the_end = CancellationToken::new();
        let done = read_to_the_end.clone().drop_guard();
        tokio::spawn(async move {
            let _done = done;
            let mut reader = BufReader::new(stderr);
            let mut buf = Vec::new();
            while let Ok(line) = read_line(&mut reader, &mut buf, 16 * 1024).await {
                match line {
                    Line::Complete => {
                        let text = redact_text(String::from_utf8_lossy(&buf).trim_end());
                        for_log.log.lock().push(text);
                    }
                    Line::TooLarge => {
                        // Drop the rest of the long line, and only that line.
                        if skip_line(&mut reader).await.is_err() {
                            return;
                        }
                        for_log.log.lock().push("[a very long line was cut]".into());
                    }
                    Line::Eof => return,
                }
            }
        });

        // On Windows the server and everything it starts go in one job, so ending the server ends all of it (and so does ZORIX ending).
        #[cfg(windows)]
        let job = match (child.raw_handle(), crate::job::Job::new(confinement.is_some())) {
            (Some(handle), Ok(job)) => match job.add(handle) {
                Ok(()) => Some(job),
                Err(e) => {
                    shared.log.lock().push(format!("[the server could not be grouped with what it starts: {e}]"));
                    None
                }
            },
            (_, Err(e)) => {
                shared.log.lock().push(format!("[a job for the server could not be made: {e}]"));
                None
            }
            (None, _) => None,
        };

        // Ending the container program does not end its container, so the container is removed once the program is gone.
        let cleanup = confinement.as_ref().and_then(Confinement::cleanup);

        // Waiter: owns the child, ends it when asked, and says when it is gone.
        let for_waiter = shared.clone();
        tokio::spawn(async move {
            let pid = child.id();
            let mut left_by_itself = None;
            tokio::select! {
                status = child.wait() => left_by_itself = status.ok(),
                _ = for_waiter.kill.cancelled() => {
                    // Ask first, so a server and what it started can save what they hold; then make them.
                    #[cfg(unix)]
                    if let Some(pid) = pid {
                        terminate_group(pid);
                        if tokio::time::timeout(TERMINATE_GRACE, child.wait()).await.is_err() {
                            kill_group(pid);
                        }
                    }
                    #[cfg(windows)]
                    if let Some(job) = &job {
                        job.terminate();
                    }
                    let _ = child.start_kill();
                    let _ = child.wait().await;
                }
            }
            // A server that left by itself may have left what it started behind (a grandchild keeps a group, or a job, alive). Whatever
            // is still there goes with it.
            #[cfg(unix)]
            if let Some(pid) = pid {
                kill_group(pid);
            }
            #[cfg(windows)]
            if let Some(job) = &job {
                job.terminate();
            }
            #[cfg(not(unix))]
            let _ = pid;
            if let Some((runtime, args)) = cleanup {
                let removal = Command::new(runtime)
                    .args(args)
                    .stdin(Stdio::null())
                    .stdout(Stdio::null())
                    .stderr(Stdio::null())
                    .status();
                let _ = tokio::time::timeout(Duration::from_secs(15), removal).await;
            }
            // Its last words are still in the pipe when the process is already gone.
            let _ = tokio::time::timeout(Duration::from_millis(500), read_to_the_end.cancelled()).await;
            if let Some(status) = left_by_itself.filter(|status| !status.success()) {
                for_waiter.log.lock().push(format!("[the program ended with {status}]"));
            }
            for_waiter.settled.cancel();
            for_waiter.end(McpError::Closed);
            for_waiter.exited.notify_waiters();
        });

        Ok(Self { shared })
    }

    async fn handle(shared: &Arc<Shared>, text: &str) {
        match wire::parse(text) {
            Ok(Incoming::Response { id, result }) => {
                if let Some(sender) = wire::id_number(&id).and_then(|id| shared.pending.lock().remove(&id)) {
                    let _ = sender.send(result);
                }
            }
            Ok(Incoming::Request { id, method, .. }) => {
                // Legacy servers may ask things of us. We answer `ping` and nothing else (ADR-0018).
                let line = if method == "ping" {
                    wire::result_line(&id, &json!({}))
                } else {
                    let _ = shared.events.send(ServerEvent::Refused { method: method.clone() });
                    wire::error_line(&id, METHOD_NOT_FOUND, "ZORIX does not answer requests from servers.")
                };
                let _ = shared.send(line).await;
            }
            Ok(Incoming::Notification { method, params }) => {
                let _ = shared.events.send(ServerEvent::Notification { method, params });
            }
            Err(error) => {
                shared.log.lock().push(format!("[ignored a line that was not valid: {error}]"));
            }
        }
    }
}

#[async_trait]
impl Transport for StdioTransport {
    async fn request_with(
        &self,
        method: &str,
        params: Value,
        _headers: &[(String, String)],
        timeout: Duration,
        cancel: &CancellationToken,
    ) -> Result<Value, McpError> {
        let id = self.shared.next.fetch_add(1, Ordering::SeqCst);
        let (tx, rx) = oneshot::channel();
        self.shared.pending.lock().insert(id, tx);
        if let Err(e) = self.shared.send(wire::request(id, method, &params)).await {
            self.shared.pending.lock().remove(&id);
            return Err(e);
        }
        // The specification forbids cancelling `initialize`: the request is simply given up on.
        let cancellable = method != "initialize";
        let stop = |reason: &str| {
            let shared = self.shared.clone();
            let line = wire::notification("notifications/cancelled", &json!({"requestId": id, "reason": reason}));
            async move {
                shared.pending.lock().remove(&id);
                if cancellable {
                    let _ = shared.send(line).await;
                }
            }
        };
        tokio::select! {
            answer = rx => match answer {
                Ok(Ok(value)) => Ok(value),
                Ok(Err(error)) => Err(error.into()),
                Err(_) => Err(McpError::Closed),
            },
            _ = cancel.cancelled() => { stop("cancelled by ZORIX").await; Err(McpError::Cancelled) }
            _ = tokio::time::sleep(timeout) => {
                stop("timed out").await;
                Err(McpError::Timeout(format!("{method} took longer than {} seconds", timeout.as_secs().max(1))))
            }
        }
    }

    async fn notify(&self, method: &str, params: Value) -> Result<(), McpError> {
        self.shared.send(wire::notification(method, &params)).await
    }

    fn subscribe(&self) -> broadcast::Receiver<ServerEvent> {
        self.shared.events.subscribe()
    }

    fn alive(&self) -> bool {
        self.shared.alive.load(Ordering::SeqCst)
    }

    fn log(&self) -> Vec<String> {
        self.shared.log.lock().lines.iter().cloned().collect()
    }

    async fn drained(&self) {
        let _ = tokio::time::timeout(Duration::from_secs(2), self.shared.settled.cancelled()).await;
    }

    async fn close(&self) {
        // Closing stdin is the polite request; a server that ignores it is ended after a few seconds.
        *self.shared.writer.lock() = None;
        let exited = self.shared.exited.notified();
        tokio::pin!(exited);
        if self.shared.alive.load(Ordering::SeqCst) {
            tokio::select! {
                _ = &mut exited => {}
                _ = tokio::time::sleep(Duration::from_secs(3)) => self.shared.kill.cancel(),
            }
        }
        self.shared.alive.store(false, Ordering::SeqCst);
    }
}

impl Drop for StdioTransport {
    fn drop(&mut self) {
        self.shared.kill.cancel();
    }
}

#[cfg(test)]
mod tests {
    use tokio::io::BufReader;

    use super::*;

    #[tokio::test]
    async fn a_line_that_is_too_long_is_skipped_to_its_end_and_the_next_line_is_whole() {
        // A long line, a normal one, a long one that never ends with a newline before the next chunk, and a last one.
        let long = "x".repeat(50_000);
        let text = format!("{long}\nnext line\n{long}\nlast\n");
        // A tiny buffer, so the skipping has to cross many reads.
        let mut reader = BufReader::with_capacity(64, text.as_bytes());
        let mut buf = Vec::new();
        let mut seen = Vec::new();
        loop {
            match read_line(&mut reader, &mut buf, 1_000).await.unwrap() {
                Line::Complete => seen.push(String::from_utf8_lossy(&buf).into_owned()),
                Line::TooLarge => {
                    skip_line(&mut reader).await.unwrap();
                    seen.push("[cut]".into());
                }
                Line::Eof => break,
            }
        }
        assert_eq!(seen, ["[cut]", "next line", "[cut]", "last"]);
    }

    #[tokio::test]
    async fn skipping_a_line_with_no_end_stops_at_the_end_of_the_stream() {
        let mut reader = BufReader::with_capacity(16, "y".repeat(10_000).into_bytes().leak() as &[u8]);
        skip_line(&mut reader).await.unwrap();
        let mut buf = Vec::new();
        assert!(matches!(read_line(&mut reader, &mut buf, 1_000).await.unwrap(), Line::Eof));
    }
}
