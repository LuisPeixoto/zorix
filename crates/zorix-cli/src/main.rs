//! The `zorix` command line.

mod app;
mod cli;
mod commands;
mod follow;
mod integrations;
mod mcp_serve;
mod memory;
mod providers;
mod render;

use std::process::ExitCode;

use clap::Parser;

use crate::cli::Cli;

fn run(cli: Cli) -> ExitCode {
    // Logs go to stderr, so they never mix with the answer on stdout. Prompts, arguments and results are never logged.
    tracing_subscriber::fmt()
        .with_env_filter(tracing_subscriber::EnvFilter::try_from_env("ZORIX_LOG").unwrap_or_else(|_| "warn".into()))
        .with_writer(std::io::stderr)
        .init();
    let runtime = match tokio::runtime::Builder::new_multi_thread().enable_all().build() {
        Ok(runtime) => runtime,
        Err(error) => {
            eprintln!("zorix: could not start: {error}");
            return ExitCode::from(1);
        }
    };
    let outcome = runtime.block_on(commands::dispatch(cli));
    // Reading standard input happens on a thread that cannot be interrupted; do not wait for it when we are done.
    runtime.shutdown_timeout(std::time::Duration::from_secs(2));
    match outcome {
        Ok(exit) => ExitCode::from(exit as u8),
        Err(error) => {
            eprintln!("zorix: {error}");
            ExitCode::from(1)
        }
    }
}

fn main() -> ExitCode {
    // Parse first: a bad command line is reported (exit code 2) before anything else happens.
    let cli = Cli::parse();
    // The default stack of the main thread is only 1 MB on Windows. Work on a thread with room to spare.
    let worker = std::thread::Builder::new().name("zorix".into()).stack_size(16 * 1024 * 1024).spawn(move || run(cli));
    match worker {
        Ok(handle) => handle.join().unwrap_or(ExitCode::from(1)),
        Err(error) => {
            eprintln!("zorix: could not start: {error}");
            ExitCode::from(1)
        }
    }
}
