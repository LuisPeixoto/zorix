# ADR-0046: OpenTelemetry export

Status: accepted (Phase 11)

## Context

The runtime already opens three `tracing` spans named after the OpenTelemetry GenAI conventions (`invoke_agent`, `chat`, `execute_tool`), and the
roadmap asked for an OTLP exporter so a team can see its runs in the tools it already uses. ZORIX has promised, and says in SECURITY.md, that it has
no telemetry: a feature that sends data off the computer has to keep that promise as far as it can, and say plainly where it stops.

## Decision

- **Off, and only to an address a person gives.** `telemetry.endpoint` is empty by default. It must be `https`, or `http` for a collector on this
  computer (`localhost`, `127.0.0.0/8`, `::1`); it may not carry a name, a password, a query or a fragment (`https://user:pw@…` is refused); an address with
  no path becomes `/v1/traces`. A header the collector needs (`authorization`) has its **name** in the settings and its **value in the system keychain**,
  never in the database, a log, an error or a screen; the interface cannot read it back.
- **Spans only, and only the runtime's.** The export sits behind a filter made when the subscriber is built: a metadata filter that lets through
  spans (never an event, so no message with an error text) whose target is `zorix_runtime`. Nothing another crate opens, and nothing written inside a
  span, can reach the exporter. The runtime's spans carry the model, token counts, the cost, the tool's name, how a call or a run ended, and ids.
  They never carry a prompt, an answer, a tool's arguments or its result. A test runs a real engine against a collector on loopback and checks that
  none of the words of a prompt, an argument and an answer appear anywhere in the bytes received, that an event and a foreign span are not exported, and that
  every attribute key is one of ZORIX's own or OpenTelemetry's.
- **Nothing of the machine.** The resource is a service name and a version; the host name, the user, the environment's resource attributes and the
  SDK's detectors are not used.
- **HTTP with protobuf, official crates.** `opentelemetry`, `opentelemetry_sdk`, `opentelemetry-otlp` (no gRPC, so no `tonic` server stack) and
  `tracing-opentelemetry`, with `rustls` like every other connection in ZORIX. Twelve crates were added to the lock file; `cargo deny` passes.
- **A bounded export that costs a run nothing.** Spans go through a batch processor with a queue of 2,048 (more are dropped, never waited for), a
  batch every five seconds and a ten-second limit per request. A dead collector costs a run nothing (a test proves it). What is queued is sent when
  telemetry is turned off and when the program ends, within five seconds.
- **Live.** The desktop app applies a change at once (the subscriber's layer is swapped); the command line reads the setting when it starts.

## Consequences

- When the setting is on, the **names of tools, the model, the token counts, the cost and timings of every run leave the computer.** The card says so
  in those words. A collector's operator can learn what is run and how much it costs; they cannot learn what was said.
- ZORIX does not report that a collector is unreachable. A batch that fails is dropped. Check the collector's own logs.
- The standard environment variables of OpenTelemetry (`OTEL_EXPORTER_OTLP_*`) are not read for the address, and the SDK's resource detectors are
  off; other SDK defaults that read the environment (such as `OTEL_BSP_*` limits) can still apply.
- Metrics and logs are not exported, only traces.
