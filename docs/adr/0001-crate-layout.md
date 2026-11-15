# ADR-0001: A workspace of small crates, one deployable app

Status: accepted (Phase 1)

## Context

The runtime must be independent of providers, interfaces and storage, and must build and test on macOS, Windows and
Linux from one code base. Microservices would add distributed-systems cost with no benefit at this stage.

## Decision

A Cargo workspace of small crates with one job each: `zorix-core` (model and contracts, no I/O), `zorix-runtime`
(engine and default components), `zorix-store` (SQLite), `zorix-sim` (simulated models), `zorix-tools` (demo tools),
`zorix-cli`, plus the Tauri shell in `apps/desktop`. Dependencies point inwards: everything depends on `zorix-core`;
`zorix-core` depends on nothing of ours. The CLI and the desktop shell are thin faces over the same `Engine`.

## Consequences

- A component can be moved behind a process or network boundary later without changing its contract.
- Platform-specific code stays out of core and runtime, so they test on every OS.
- More crates mean more `Cargo.toml` files; accepted for the clear boundaries.
