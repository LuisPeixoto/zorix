# ADR-0013: Four kinds of memory, three scopes, and a person approves every write

Status: accepted (Phase 3)

## Context

Executions start empty. People expect a conversation to continue, preferences to stick and documents to be usable, but
anything remembered is also an attack surface (a web page that tells the model to "remember" something) and a privacy
question (what exactly is kept, and can it really be deleted).

## Decision

- Memory is **conversation, long-term, knowledge and operational**, each its own `ContextSource` with its own trust and
  budget. They are not one pool.
- A long-term item has a **scope**: global, project (the canonical path of the run's workspace folder) or conversation. A run
  sees global items, the project's items and its conversation's. **Every query carries the tenant.**
- **The model can only propose** a long-term item, through the tool `memory.remember`, which always requires approval and is
  never answered by an automatic approval mode. People can add and edit items by hand. Nothing enters unseen.
- Items keep their origin (a person, or the execution, call and approval that created them), their dates, an optional
  expiry and the time they were last confirmed. An item with the same subject, or one named in `replaces`, becomes
  *superseded* instead of being edited or deleted, so history is not lost.
- Content that looks like a credential is refused. Deleting an item deletes it everywhere ZORIX keeps it (rows, full-text
  index, vectors, frozen snapshots).

## Consequences

- A poisoned document cannot write memory without a person seeing the text and where it came from.
- Approval cannot edit a proposal (it is bound to the call's hash); a person refuses with a note or approves and edits.
- Deletion cannot reach text that already reached a transcript or a provider's logs; the interface says so and lists the
  runs that used the item.
- The notes demo tool stays in 0.3 and is superseded by `memory.*`.
