# ADR-0050: A sandbox for local MCP servers

Status: accepted (Phase 12)

## Context

A stdio MCP server is a program ZORIX starts on the person's computer. Until now it ran as the person: it could read every file,
the SSH keys and cloud credentials included, write anywhere, open any connection and ask other programs to act. Tool approval
limits what a *run* asks of it, not what the program itself does, and most servers are packages fetched from a registry.

## Decision

- **Three modes: `native`, `container` and `off`.** A new local server starts `native`, decided by the store so no way
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

- **Container:** Docker, else Podman. `--read-only`, `--cap-drop ALL`, `no-new-privileges`, process and memory limits, no network
  unless chosen, only the server's folder and the given folders mounted (at their own paths; on Windows elsewhere, with the arguments
  that name them rewritten), the server's variables handed in by name so no value is written on a command line, and the person's own
  user id with Docker. Images are pinned by digest (`NODE_IMAGE` for `npx` and `node`, `UV_IMAGE` for `uvx`, `uv` and `python`),
  reviewed with each release; other commands are refused. The container is removed when the server stops, because ending the
  `docker` program does not end it. No container program: the server is refused with the reason, never started unconfined.

## Security review

- **Escapes tried in tests** (`zorix-mcp/tests/sandbox.rs`, macOS and Linux in CI): reading keys, notes, ZORIX's database and
  other folders; writing outside and into read-only folders; TCP and Unix connections; reaching a credential folder through a wider
  folder that was given; and the same attempts without a sandbox succeeding, so they are real. Tried by hand on macOS: `open`
  (it **did** start an application until services were refused by default), AppleScript, `launchctl`, the keychain and the pasteboard.
- **Found while building it:** a Seatbelt rule for one operation wins over a later rule for its family, so refusing `file-read*`
  did not refuse the keys inside a given folder until the refusal named `file-read-data` too. A test holds it.
- **Container, tried on Linux in CI** (`tests/container.rs`): a host file not given, a read-only folder, the system files and the
  network are all refused, and no container is left behind. Not tried here: Docker Desktop on macOS and Windows, and Podman.
- **Not covered:** a container shares the host's kernel, and the container program's daemon runs with its own privileges; anyone who
  can start containers already can do more than a server in one. A ZORIX that is killed outright may leave a container running until
  its input closes.
- **Not covered:** it is a restriction, not a virtual machine; a kernel flaw is outside it. With the network on, a server can send
  anywhere what it can read, private addresses included. The toolchain folders it may read are listed in `sandbox.rs`. A package
  that must be compiled on first use may fail inside it (turn it off for that start). `sandbox-exec` is deprecated by Apple but
  still present; if it goes, the start is refused.
