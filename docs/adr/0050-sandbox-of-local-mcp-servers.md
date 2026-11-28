# ADR-0050: A sandbox for local MCP servers

Status: accepted (Phase 12)

## Context

A stdio MCP server is a program ZORIX starts on the person's computer. Until now it ran as the person: it could read every file,
the SSH keys and cloud credentials included, write anywhere, open any connection and ask other programs to act. Tool approval
limits what a *run* asks of it, not what the program itself does, and most servers are packages fetched from a registry.

## Decision

- **Two modes, `native` and `off`** (a container mode follows). A new local server starts `native`, decided by the store so no way
  of adding one (command line, catalogue, import, app) can forget it. A server added before has no choice recorded and keeps running
  as before, with a notice; the schema moved to 17 so an older ZORIX, which would ignore the column and run servers unconfined,
  refuses the data instead.
- **The server's own folder** under the data folder is its home, temporary folder and package caches. It reads the system and the
  language toolchains it runs on, the folders a person gave it, and nothing else of theirs; it writes only in its own folder and the
  folders given for writing. A folder named to a catalogue server that asks for one (the *Filesystem* folder, the *Git*
  repository) is given to it for writing; the whole disk and credential folders are never accepted.
- **The network is on only for launchers that fetch what they run** (`npx`, `uvx` and the like) and off for other commands; a person
  can change it.
- **macOS:** a Seatbelt profile through `sandbox-exec`. System services are refused by name except the few needed to resolve users,
  names and certificates, so `open`, AppleScript, `launchctl`, the keychain and the pasteboard are out of reach. Credential folders
  are refused even inside a given folder. Local services over Unix sockets are refused (the name resolver aside).
- **Linux:** Landlock for files (a kernel without it refuses to start the server, never runs it open) and a seccomp filter that
  refuses Unix sockets (the keyring over D-Bus, the Docker socket, the SSH agent), internet sockets when the network is off, and
  io_uring.
- **Windows:** the Job Object limits memory and the number of processes and ends them with the server. Files and the network are
  **not** confined, and the interface and `zorix mcp doctor` say so.

## Security review

- **Escapes tried in tests** (`zorix-mcp/tests/sandbox.rs`, macOS and Linux in CI): reading keys, notes, ZORIX's database and
  other folders; writing outside and into read-only folders; TCP and Unix connections; reaching a credential folder through a wider
  folder that was given; and the same attempts without a sandbox succeeding, so they are real. Tried by hand on macOS: `open`
  (it **did** start an application until services were refused by default), AppleScript, `launchctl`, the keychain and the pasteboard.
- **Found while building it:** a Seatbelt rule for one operation wins over a later rule for its family, so refusing `file-read*`
  did not refuse the keys inside a given folder until the refusal named `file-read-data` too. A test holds it.
- **Not covered:** it is a restriction, not a virtual machine; a kernel flaw is outside it. With the network on, a server can send
  anywhere what it can read, private addresses included. The toolchain folders it may read are listed in `sandbox.rs`. A package
  that must be compiled on first use may fail inside it (turn it off for that start). `sandbox-exec` is deprecated by Apple but
  still present; if it goes, the start is refused.
