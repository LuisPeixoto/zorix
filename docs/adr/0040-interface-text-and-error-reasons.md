# ADR-0040: Interface text is a catalogue, and errors carry reasons

Status: accepted (Phase 10)

## Context

The desktop interface has about 2,500 user-facing strings written in its components and helpers, plurals written by hand
(`n === 1 ? "task" : "tasks"`), about eight hand-written money formatters, six duration formatters, a hard-coded French locale
for token counts, and no use of `Intl`. The Rust side turns every error into an English sentence that the interface shows
as it is. The [engineering guidelines](../engineering-guidelines.md#11-accessibility-and-internationalisation) require text to
be centralised and translatable, and Phase 15 adds Portuguese, Spanish and Chinese. Doing the migration once, strictly, is
cheaper than finding stray strings later.

## Decision

- **One catalogue in the interface, no new dependency.** `src/lib/i18n/messages.en.ts` is a typed nested object, English is the
  source. `t(key, args)` handles interpolation, plurals through `Intl.PluralRules` (every message with a count lists its
  forms) and a choice between named variants. A key that does not exist is a type error.
- **One formatter for everything that has a locale.** Dates, times, relative times, numbers, percentages, money (USD, from
  millionths of a dollar, with today's precision rules), durations and lists go through `Intl` with the active locale. No
  component formats a value by hand.
- **The lint is strict from the start.** `tools/check-text.mjs` follows `check-design.mjs`: it rejects literal text and literal
  `aria-label`, `title`, `placeholder`, `label` and `hint` values in components, and prose string literals in the helpers that
  build interface text. An exception needs a reason in its allowlist (a brand name, a key symbol, a code sample).
- **A pseudo-locale (`en-XA`)** with accented, about 40 % longer text is part of the tests and can be turned on for
  screenshots, so cut-off text is found before a real translation arrives.
- **Errors carry a reason (what was done, and what is left).** A reason is a stable `code` (`task.not_found`) and its `args`. The English sentence is built in
  Rust from a table next to the code, so the command line prints exactly what it prints today. Desktop commands return
  `{code, args, message}`; the interface builds the sentence from its catalogue and uses `message` only for a code it does
  not know (a newer backend). A generated `reasons.json` and a test keep the Rust table and the catalogue in step.
- **Text from outside is shown as it is.** The operating system's, SQLite's and a provider's own messages are arguments of a
  generic code (`io.failed`, `storage.failed`, `provider.failed`), never translated or rewritten.
- **The migration was done by a program** (`tools/i18n-codemod.mjs`, removed once it had run): it read each component's syntax tree, turned text between tags, the attributes a person reads and the prose literals of the scripts into catalogue messages (a run of text
  and expressions is one sentence with placeholders; a `n === 1 ? "task" : "tasks"` becomes plural forms), and added the import. English output was held identical by the existing helper tests and by reading the diff of the screens; where it could not
  do it safely (a sentence with two counts) a person did it. Its keys and placeholder names are mechanical, and Phase 15 reads each message in its context before it is translated.
- **One screen is exempt: the design-system showcase** (⌘⇧D), a developer screen that documents the kit in English. It is listed in the lint's allowlist with that reason.
- **Units are messages** (`{value} min`, `{h} h {m} min`), because their words and order belong to a language, and money, numbers, percentages, dates, weekdays and lists use `Intl`. English lists are written without the serial comma, as the interface always did.
- **Phase 10 adds no language.** The picker and the translations are Phase 15; the locale store holds English only.

### What this version did with errors

Every typed error has a code (`zorix_core::Reason`, from the `Coded` trait: the core's `Error`, `ModelError` by kind, `ToolError`, `IdError`), the desktop commands reject with a `UiError { code, args, message }`, and `errorText` writes the sentence from the
catalogue (`reason.<code>`), falling back to the English `message` for a code it does not know. `crates/zorix-core/reasons.json` is generated from the code table, and a test compares it with the interface's catalogue word for word. The command line
prints exactly what it printed.

**What is not done:** about 170 validation sentences are plain `String`s (`Error::InvalidInput(String)` and the validators of the core and the services). They travel as the codes `invalid`, `missing` and `message` with their English sentence as the value, so a
translated interface shows them in English. Moving each to a code of its own is the work of the audits of Phases 11 to 14, one area at a time, tracked in [#26](https://github.com/LuisPeixoto/zorix/issues/26); doing all of it here would have been a rewrite of every
service with nothing to show for it in English.

## Consequences

- Every component changes once, and the diff is large but mechanical, one commit per area. English output must stay
  identical: the existing helper tests keep their English assertions through the catalogue, and the command line's messages
  are covered by snapshot tests.
- Adding a language is a new file and a glossary, with a check that every key is present.
- A new interface string without a key fails the lint; a new error without a code fails a test.
- Messages the user wrote (names, notes) and model output are data, and are not in the catalogue.
