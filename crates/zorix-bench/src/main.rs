//! Builds a realistic data folder (many runs, many events, many agents, many memories) through the real stores, then times what a person
//! waits for: the run list and its filters, replaying a run, the Inbox's queries, search over memory and opening the store.
//!
//! `zorix-bench --scale 1` is the full size (10,000 runs, 1,000,000 events, 200 agents, 5,000 facts and 50,000 pieces of documents; the
//! facts and agents are at the most the product keeps). CI runs a tenth of it with
//! `--check`, which compares each time with a generous limit in `limits.json`, so only a large regression fails.

use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

use clap::Parser;
use serde_json::{Value, json};
use zorix_core::memory::text_hash;
use zorix_core::{
    AgentBinding, AgentSlug, AgentSpec, AgentStore, ConversationId, Corpus, DocState, DocStatus, DocumentWrite,
    ExecutionRepository, ExecutionState, ListFilter, MemoryKind, MemoryOrigin, MemoryStore, NewChunk, NewMemory,
    NewRoot, Scope, TenantId, Timestamp, Visibility,
};
use zorix_runtime::testing::{event, sample_execution};
use zorix_store::{SqliteAgentStore, SqliteMemoryStore, SqliteRepository};

#[derive(Parser)]
#[command(about = "Times what people wait for, on a data folder of realistic size")]
struct Args {
    /// 1 is the full size; 0.1 is a tenth.
    #[arg(long, default_value_t = 1.0)]
    scale: f64,
    /// Write the numbers here as JSON.
    #[arg(long)]
    json: Option<PathBuf>,
    /// Compare with these limits (milliseconds per measurement) and exit 1 when one is passed.
    #[arg(long)]
    check: Option<PathBuf>,
    /// How many times each measurement is made.
    #[arg(long, default_value_t = 15)]
    repeat: usize,
}

/// A small, fixed generator so every run builds the same data.
struct Lcg(u64);

impl Lcg {
    fn next(&mut self) -> u64 {
        self.0 = self.0.wrapping_mul(6364136223846793005).wrapping_add(1442695040888963407);
        self.0 >> 33
    }

    fn below(&mut self, n: u64) -> u64 {
        self.next() % n.max(1)
    }
}

const WORDS: [&str; 64] = [
    "deploy",
    "release",
    "database",
    "backup",
    "invoice",
    "customer",
    "report",
    "incident",
    "latency",
    "budget",
    "contract",
    "review",
    "pipeline",
    "schema",
    "migration",
    "support",
    "ticket",
    "billing",
    "metric",
    "alert",
    "queue",
    "cache",
    "index",
    "query",
    "policy",
    "approval",
    "workflow",
    "agent",
    "memory",
    "document",
    "search",
    "vector",
    "provider",
    "route",
    "model",
    "token",
    "cost",
    "limit",
    "sandbox",
    "network",
    "secret",
    "rotate",
    "audit",
    "security",
    "patch",
    "upgrade",
    "rollback",
    "monitor",
    "dashboard",
    "logging",
    "tracing",
    "planning",
    "roadmap",
    "sprint",
    "estimate",
    "design",
    "prototype",
    "feedback",
    "survey",
    "interview",
    "analysis",
    "summary",
    "draft",
    "publish",
];

fn sentence(rng: &mut Lcg, words: u64) -> String {
    (0..words).map(|_| WORDS[rng.below(64) as usize]).collect::<Vec<_>>().join(" ")
}

#[derive(Default)]
struct Report {
    sizes: Vec<(String, u64)>,
    timings: Vec<(String, f64, f64)>,
}

impl Report {
    fn size(&mut self, name: &str, value: u64) {
        println!("  {name:<28} {value:>10}");
        self.sizes.push((name.into(), value));
    }

    fn record(&mut self, name: &str, mut times: Vec<Duration>) {
        times.sort();
        let ms = |d: Duration| d.as_secs_f64() * 1000.0;
        let (median, worst) = (ms(times[times.len() / 2]), ms(*times.last().unwrap()));
        println!("  {name:<44} median {median:>9.2} ms   worst {worst:>9.2} ms");
        self.timings.push((name.into(), median, worst));
    }
}

/// Make a measurement `$repeat` times, each time evaluating `$body` (a future) from scratch.
macro_rules! time {
    ($report:expr, $name:expr, $repeat:expr, $body:expr) => {{
        let mut times = Vec::new();
        for _ in 0..$repeat.max(1) {
            let started = Instant::now();
            let _ = $body.await;
            times.push(started.elapsed());
        }
        $report.record($name, times);
    }};
}

fn tenant() -> TenantId {
    TenantId::local()
}

#[tokio::main(flavor = "multi_thread", worker_threads = 4)]
async fn main() {
    let args = Args::parse();
    let scale = args.scale.clamp(0.001, 10.0);
    let n = |full: f64| ((full * scale).round() as u64).max(1);
    let (runs, events_per_run, agents, facts, pieces) = (n(10_000.0), 100u64, n(200.0), n(5_000.0), n(50_000.0));

    let dir = tempfile::tempdir().expect("a temporary folder");
    let path = dir.path().join("zorix.sqlite");
    let mut report = Report::default();
    let mut rng = Lcg(42);

    println!("Building the data folder at scale {scale} ...");
    let built = Instant::now();
    let repo = SqliteRepository::open(&path).expect("open the store");
    let agent_store = SqliteAgentStore::open(&path).expect("open the agent store");
    let memory = SqliteMemoryStore::open(&path).expect("open the memory store");

    // Agents.
    let mut bindings: Vec<AgentBinding> = Vec::new();
    for i in 0..agents {
        let slug = AgentSlug::new(format!("agent-{i:04}")).unwrap();
        let mut spec = AgentSpec::new(slug.clone(), format!("Agent {i}"), "demo");
        spec.description = sentence(&mut rng, 8);
        spec.instructions = sentence(&mut rng, 60);
        let (agent, version) =
            agent_store.create_agent(&tenant(), spec, None, Timestamp(1_000 + i as i64)).await.unwrap();
        if bindings.len() < 50 {
            bindings.push(AgentBinding {
                agent: agent.id,
                slug,
                version: version.version,
                hash: version.hash.clone(),
                skills: Vec::new(),
                allowed_models: vec!["*".into()],
                agent_memory: true,
                team: None,
                autonomy: None,
            });
        }
    }

    // Runs, each with its own events: 90 days of work, a few hundred conversations, one run in a hundred waiting for a person.
    let now = 1_800_000_000_000i64;
    let day = 86_400_000i64;
    let conversations: Vec<ConversationId> =
        (0..(runs / 20).max(5)).map(|_| ConversationId::new(format!("c-{}", rng.next())).unwrap()).collect();
    let mut newest = Vec::new();
    let mut a_busy_run = None;
    for i in 0..runs {
        let mut execution = sample_execution("local", None);
        execution.request.prompt = sentence(&mut rng, 20);
        execution.created_at = Timestamp(now - rng.below(90 * day as u64) as i64);
        execution.agent = Some(bindings[(i % bindings.len() as u64) as usize].clone());
        execution.conversation = Some(conversations[rng.below(conversations.len() as u64) as usize].clone());
        execution.state = if i % 100 == 0 { ExecutionState::WaitingForApproval } else { ExecutionState::Completed };
        let count = if i == 0 { events_per_run * 100 } else { events_per_run };
        let events = (0..count)
            .map(|k| {
                let mut e = event(zorix_core::EventKind::Progress);
                e.payload = json!({"step": k, "note": sentence(&mut rng, 12)});
                e
            })
            .collect();
        if i == 0 {
            a_busy_run = Some(execution.id);
        }
        if i % 100 == 0 {
            newest.push(execution.id);
        }
        repo.create(execution, events).await.unwrap();
    }

    // Facts, the most a tenant keeps, each with words to find.
    for i in 0..facts {
        memory
            .remember(
                &tenant(),
                NewMemory {
                    scope: Scope::Global,
                    kind: MemoryKind::Fact,
                    subject: Some(format!("topic {}", i % 500)),
                    content: sentence(&mut rng, 14),
                    valid_until: None,
                    origin: MemoryOrigin::User { name: "bench".into() },
                    replaces: Vec::new(),
                },
                Timestamp(now),
            )
            .await
            .unwrap();
    }
    // Documents: one folder, files of fifty pieces each, and a vector for every piece (the search that reads them all).
    let folder = dir.path().join("docs").display().to_string();
    let root = memory
        .add_root(
            &tenant(),
            NewRoot {
                scope: Scope::Global,
                path: folder,
                include: Vec::new(),
                exclude: Vec::new(),
                max_file_bytes: 2_000_000,
                auto_sync: false,
            },
            Timestamp(now),
        )
        .await
        .unwrap();
    let model = "bench-embeddings";
    let dimensions = 128usize;
    let mut vectors: Vec<(String, Vec<f32>)> = Vec::new();
    let per_file = 50u64;
    for file in 0..pieces.div_ceil(per_file) {
        let chunks: Vec<NewChunk> = (0..per_file.min(pieces - file * per_file))
            .map(|k| {
                let text = sentence(&mut rng, 60);
                let text_hash = text_hash(&format!("{text} {file} {k}"));
                let mut v: Vec<f32> = (0..dimensions).map(|_| rng.below(2_000) as f32 / 1_000.0 - 1.0).collect();
                let norm = v.iter().map(|x| x * x).sum::<f32>().sqrt().max(1e-6);
                v.iter_mut().for_each(|x| *x /= norm);
                vectors.push((text_hash.clone(), v));
                NewChunk {
                    text,
                    text_hash,
                    heading: None,
                    line_start: Some(k as u32 * 5 + 1),
                    line_end: Some(k as u32 * 5 + 5),
                    page: None,
                    tokens: 80,
                }
            })
            .collect();
        memory
            .write_document(DocumentWrite {
                root: root.id,
                rel_path: format!("notes/file-{file:05}.md"),
                kind: "markdown".into(),
                state: DocState {
                    size: 4_000,
                    mtime_ns: 1,
                    content_hash: format!("{file:064x}"),
                    status: DocStatus::Indexed,
                },
                error: None,
                chunks,
                indexed_at: Timestamp(now),
            })
            .await
            .unwrap();
        if vectors.len() >= 5_000 {
            memory.put_vectors(model, std::mem::take(&mut vectors)).await.unwrap();
        }
    }
    if !vectors.is_empty() {
        memory.put_vectors(model, vectors).await.unwrap();
    }
    let with_vectors = pieces;
    println!("Built in {:.1} s.", built.elapsed().as_secs_f64());

    println!("\nSize");
    report.size("runs", runs);
    report.size("events", runs * events_per_run + events_per_run * 99);
    report.size("agents", agents);
    report.size("facts", facts);
    report.size("pieces of documents", pieces);
    report.size("vectors", with_vectors);
    report.size("file (MB)", std::fs::metadata(&path).map(|m| m.len() / 1_048_576).unwrap_or(0));

    println!("\nTimes");
    let r = args.repeat;
    let visible = Visibility { tenant: tenant(), project: None, conversation: None, agent: None };
    let one_agent = bindings[3].agent;
    let one_conversation = &conversations[2];
    let week_ago = Timestamp(now - 7 * day);
    let probe: Vec<f32> = (0..dimensions).map(|i| if i == 0 { 1.0 } else { 0.0 }).collect();

    time!(
        report,
        "runs: the newest page (100)",
        r,
        repo.list(&tenant(), &ListFilter { limit: 100, ..ListFilter::default() })
    );
    time!(
        report,
        "runs: one agent's",
        r,
        repo.list(&tenant(), &ListFilter { agent: Some(one_agent), limit: 100, ..ListFilter::default() })
    );
    time!(report, "runs: waiting for approval", r, async {
        repo.list(
            &tenant(),
            &ListFilter { states: vec![ExecutionState::WaitingForApproval], limit: 100, ..ListFilter::default() },
        )
        .await
    });
    time!(report, "runs: one conversation's", r, async {
        repo.list(
            &tenant(),
            &ListFilter { conversation: Some(one_conversation.clone()), limit: 100, ..ListFilter::default() },
        )
        .await
    });
    time!(
        report,
        "runs: since a week ago",
        r,
        repo.list(&tenant(), &ListFilter { since: Some(week_ago), limit: 1_000, ..ListFilter::default() })
    );
    time!(
        report,
        "runs: roots only (100)",
        r,
        repo.list(&tenant(), &ListFilter { roots_only: true, limit: 100, ..ListFilter::default() })
    );
    let busy = a_busy_run.unwrap();
    time!(report, "a run's events: a page (200)", r, repo.events(busy, 0, 200));
    time!(report, "a run's events: all of 10,000", 3, async {
        let mut after = 0u64;
        let mut total = 0usize;
        loop {
            let page = repo.events(busy, after, 1_000).await.unwrap();
            if page.is_empty() {
                break;
            }
            after = page.last().map(|e| e.seq).unwrap_or(after);
            total += page.len();
        }
        total
    });
    time!(report, "inbox: runs waiting for a person", r, repo.in_states(&[ExecutionState::WaitingForApproval]));
    time!(report, "inbox: the approvals of fifty of them", r, async {
        for id in newest.iter().take(50) {
            let _ = repo.approvals(*id).await;
        }
    });
    time!(report, "agents: all of them", r, agent_store.agents(&tenant(), false));
    let terms: Vec<String> = ["deploy", "release", "database"].iter().map(|s| s.to_string()).collect();
    time!(report, "facts: keyword search (3 words)", r, memory.text_search(&visible, Corpus::Memory, &terms, 10));
    time!(
        report,
        "documents: keyword search (3 words)",
        r,
        memory.text_search(&visible, Corpus::Knowledge, &terms, 10)
    );
    time!(
        report,
        "documents: search by meaning",
        r,
        memory.vector_search(&visible, Corpus::Knowledge, model, &probe, 10)
    );
    // A sync counts what still needs a vector (every piece has one here, so this is the walk over all of them), and every file it
    // writes checks the room left in the tenant's pieces.
    time!(
        report,
        "documents: counting the pieces without a vector",
        r,
        memory.count_chunks_without_vector(root.id, model)
    );
    let rewritten: Vec<NewChunk> = (0..50)
        .map(|k| {
            let text = sentence(&mut rng, 60);
            NewChunk {
                text_hash: text_hash(&format!("{text} rewritten {k}")),
                text,
                heading: None,
                line_start: Some(k * 5 + 1),
                line_end: Some(k * 5 + 5),
                page: None,
                tokens: 80,
            }
        })
        .collect();
    time!(report, "documents: writing one file of fifty pieces", r, async {
        memory
            .write_document(DocumentWrite {
                root: root.id,
                rel_path: "notes/file-00000.md".into(),
                kind: "markdown".into(),
                state: DocState {
                    size: 4_000,
                    mtime_ns: 2,
                    content_hash: format!("{:064x}", 1u64),
                    status: DocStatus::Indexed,
                },
                error: None,
                chunks: rewritten.clone(),
                indexed_at: Timestamp(now),
            })
            .await
    });
    time!(
        report,
        "facts: the list",
        r,
        memory.all_memory(&tenant(), &zorix_core::MemoryFilter::default(), Timestamp(now))
    );
    time!(report, "opening the store again", 5, async {
        let again = SqliteRepository::open(&path).expect("open");
        drop(again);
    });

    if let Some(out) = &args.json {
        let value = json!({
            "scale": scale,
            "sizes": report.sizes.iter().map(|(k, v)| json!({"name": k, "value": v})).collect::<Vec<_>>(),
            "timings": report.timings.iter().map(|(k, m, w)| json!({"name": k, "median_ms": m, "worst_ms": w})).collect::<Vec<_>>(),
        });
        std::fs::write(out, serde_json::to_string_pretty(&value).unwrap()).expect("write the numbers");
    }
    if let Some(limits) = &args.check {
        std::process::exit(check(limits, &report));
    }
}

/// Compare each median with its limit. A measurement with no limit is reported, so a new one cannot be forgotten.
fn check(limits: &Path, report: &Report) -> i32 {
    let text = std::fs::read_to_string(limits).expect("read the limits");
    let limits: Value = serde_json::from_str(&text).expect("the limits are JSON");
    let mut failed = 0;
    println!("\nLimits");
    for (name, median, _) in &report.timings {
        match limits[name.as_str()].as_f64() {
            Some(limit) if *median > limit => {
                println!("  FAIL {name}: {median:.2} ms is over {limit:.0} ms");
                failed += 1;
            }
            Some(limit) => println!("  ok   {name}: {median:.2} ms (limit {limit:.0})"),
            None => {
                println!("  FAIL {name}: no limit is set for it in limits.json");
                failed += 1;
            }
        }
    }
    i32::from(failed > 0)
}
