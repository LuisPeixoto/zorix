//! The trace spans and the logs. This lives in its own test binary because `tracing` caches, for the whole process, which
//! call sites someone is listening to: a subscriber installed for one test must not share a process with other tests.

mod common;

use std::io::Write;
use std::sync::Arc;

use common::*;
use parking_lot::Mutex;
use serde_json::json;
use tracing_subscriber::fmt::format::FmtSpan;
use zorix_core::ExecutionState as S;
use zorix_sim::Turn;

#[derive(Clone, Default)]
struct Captured(Arc<Mutex<Vec<u8>>>);

impl Write for Captured {
    fn write(&mut self, bytes: &[u8]) -> std::io::Result<usize> {
        self.0.lock().extend_from_slice(bytes);
        Ok(bytes.len())
    }

    fn flush(&mut self) -> std::io::Result<()> {
        Ok(())
    }
}

impl Captured {
    fn text(&self) -> String {
        String::from_utf8_lossy(&self.0.lock()).into_owned()
    }
}

#[tokio::test(flavor = "current_thread")]
async fn spans_follow_the_genai_names_and_nothing_secret_reaches_the_logs() {
    let captured = Captured::default();
    let writer = captured.clone();
    let subscriber = tracing_subscriber::fmt()
        .with_max_level(tracing::Level::TRACE)
        .with_span_events(FmtSpan::NEW)
        .with_ansi(false)
        .with_writer(move || writer.clone())
        .finish();
    let _guard = tracing::subscriber::set_default(subscriber);

    let secret = "sk-abcdefghijklmnopqrstuvwxyz0123";
    let prompt = "my private prompt about project Nightingale";
    let h = harness(vec![
        Turn::call("echo", json!({"text": format!("key {secret}")})),
        Turn::Say("the final answer is purple".into()),
    ]);
    let done = run(&h.engine, request(prompt, &["echo"])).await;
    assert_eq!(done.state, S::Completed);

    let log = captured.text();
    for wanted in [
        "invoke_agent",
        "chat",
        "execute_tool",
        "gen_ai.tool.name=echo",
        "gen_ai.tool.call.id=call_1_0",
        "gen_ai.request.model=sim",
    ] {
        assert!(log.contains(wanted), "missing {wanted} in:\n{log}");
    }
    for hidden in [secret, "Nightingale", "purple"] {
        assert!(!log.contains(hidden), "{hidden} leaked into the logs:\n{log}");
    }
}
