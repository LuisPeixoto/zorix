# ADR-0009: Provider adapters over raw HTTP, in one crate

Status: accepted (Phase 2)

## Context

Phase 2 connects four API families (Anthropic, OpenAI Responses, OpenAI-style Chat Completions, Gemini). Rust has no
official SDK for the first and the third-party frameworks normalise conversations in ways that lose provider-specific
state (thinking blocks, thought signatures, reasoning items) and hide the error classes the router must act on.

## Decision

One crate, `zorix-providers`, with one adapter per API family, each implementing `ModelGateway` over `reqwest` (rustls).
HTTPS is required except for loopback hosts; requests have timeouts and responses have size caps. Each adapter maps the
provider's errors to a small `ModelErrorKind` (auth, not found, context too long, rate limited, unavailable, refused,
invalid, cancelled) and keeps the provider's raw assistant turn on the message. Adapters are tested against local mock
servers; live tests are opt-in.

## Consequences

- We own the wire formats and must follow API changes; the catalogue and adapter tests make drift visible.
- No dependency on a framework's abstraction of tools or messages.
- Adding a provider means adding one adapter and a catalogue entry; the runtime does not change.

## Addendum: Ollama uses its native API

Ollama also serves the Chat Completions protocol, but its native `/api/chat` reports the real context length and
capabilities through `/api/show`, lets a request fix the window with `num_ctx` (otherwise a small default silently
truncates the conversation), and streams newline-delimited JSON. The adapter uses the native API, asks for at most
32 768 tokens of context, and reports that number as the model's window so the runtime's budget matches what is loaded.
Text is not streamed to the interface yet: adapters stream internally and return the complete answer.

