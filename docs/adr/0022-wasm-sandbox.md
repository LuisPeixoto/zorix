# ADR-0022: Code runs in WebAssembly

Status: accepted (Phase 4)

## Context

"Run this code" is the most dangerous tool. A platform-specific sandbox (sandbox-exec, bubblewrap, AppContainer) has different
guarantees on each system; a container needs Docker.

## Decision

- `code.run` executes JavaScript or Python inside **wasmtime** with WASI. The same module behaves the same on all three systems.
- **JavaScript** is `qjs-wasi.wasm` from quickjs-ng v0.18.0, embedded in the binary, checked against its SHA-256 at build time.
  **Python** is CPython 3.12 as a WASI build (`python-3.12.0.wasm`), **downloaded on demand** into the data folder by
  `zorix sandbox install python`, only from the pinned URL, accepted only if the SHA-256 matches.
- Limits per call: 256 MB memory (`StoreLimits`), 10 s of CPU through epoch interruption plus a wall-clock watchdog, 1 MB of
  combined output, 50 MB of scratch disk. No network, no environment variables, no clock beyond WASI's, no host paths except
  a fresh scratch directory (and, only if the run has `read` workspace grants and asks for it, the workspace mounted read-only).
- Grants: `execute code:javascript` / `code:python`; approval only when a workspace is mounted.
- The module is instantiated fresh for every call; nothing persists between calls.
- Not supported: native extensions (numpy…), package installation, threads, sockets.

## Consequences

- A wasmtime bug could escape; we pin the version and update it on advisories.
- Python is slow to start (tens of milliseconds to seconds) and 26 MB on disk; JavaScript is instant.
