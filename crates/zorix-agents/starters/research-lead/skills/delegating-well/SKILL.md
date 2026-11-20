---
name: delegating-well
description: "How to split a request into tasks other agents can do at the same time, how to write each task so it stands alone, and how to combine what comes back. Use before calling agents.delegate."
license: MIT
metadata:
  author: zorix
  version: "1"
---

# Delegating well

Use this before you call `agents.delegate`.

## Decide whether to delegate

- **Delegate** when the request has several independent parts (compare A and B, check three sources, research and then write), or when someone on the team is better at a part.
- **Do it yourself** when it is one small step, a greeting, or something you already know. Delegation costs time and budget.

## Split the work

- One task per part that can run **at the same time**. Parts that depend on each other go in separate calls: ask for the first, read the result, then ask for the next.
- Prefer a few clear tasks to many small ones. Two to four is usual; never more than eight.

## Write each task so it stands alone

The agent does not see this conversation. Every task says:

1. **The goal**: what you want to know or have made, in a sentence.
2. **The form of the answer**: a list, a table, two paragraphs, JSON with these fields. When you need data, pass `expected_output` with a JSON Schema.
3. **Where to look and what to leave out**: sources to prefer, a time range, what is out of scope.
4. **Anything it needs from you**: pass notes in `context`, data in `input`.

See `references/task-template.md` for a template.

## Read what comes back

Each result has a `state` (`completed`, `failed` or `cancelled`), an `answer`, and sometimes `structured` data or an `error`. A failed part is information, not the end: ask again with a clearer task, or tell the person what is missing.

## Put it together

Write the final answer yourself. Keep the sources the researcher gave. Say what you could not confirm.
