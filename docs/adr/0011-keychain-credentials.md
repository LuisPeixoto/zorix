# ADR-0011: API keys live in the operating system's keychain

Status: accepted (Phase 2)

## Context

Cloud providers need API keys. Files and databases are easy to leak, back up or paste into a bug report.

## Decision

Keys are stored with the platform's credential store (macOS Keychain, Windows Credential Manager, Linux Secret Service)
through the `keyring` crate, behind a `SecretStore` trait. Environment variables (`ANTHROPIC_API_KEY`, `OPENAI_API_KEY`,
`GEMINI_API_KEY`, or a per-provider name) are read as an alternative and win when set, so CI and servers need no keychain.
A key is read at call time into a `Secret` (redacted `Debug`/`Display`/`Serialize`), sent only in an auth header, and
never written to the database, events, spans, logs or the interface. Keys are entered with hidden input (CLI) or a
password field that hands the value straight to Rust (desktop).

## Consequences

- Nothing secret is in the data folder, so copying or backing it up does not leak keys.
- A machine without a keychain service (some Linux servers) must use environment variables.
- Tests use an in-memory store; the keychain implementation has an opt-in test.
