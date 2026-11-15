//! The `zorix` command line.

mod app;
mod cli;
mod commands;
mod follow;
mod render;

use std::process::ExitCode;

use clap::Parser;

use crate::cli::Cli;

fn run(cli: Cli) -> ExitCode {
    let runtime = match tokio::runtime::Builder::new_multi_thread().enable_all().build() {
        Ok(runtime) => runtime,
        Err(error) => {
            eprintln!("zorix: could not start: {error}");
            return ExitCode::from(1);
        }
    };
    match runtime.block_on(commands::dispatch(cli)) {
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
