# Permissions, policy and approvals

The runtime treats the model as **untrusted**. Offering a tool to a model is not permission to run it; a model's request
is only a request. The rules below are enforced by the runtime, in Rust, at the moment a tool call is about to run.

## Grants: nothing is allowed by default

An execution carries a set of **grants**, each an action on a resource:

| Action | Used for |
|---|---|
| `use_tool` | Using a tool at all. The resource is the tool id |
| `read`, `write`, `execute`, `network` | What the tool does to a resource |

A resource in a grant is a pattern: `*` matches everything, `prefix*` matches anything starting with `prefix`, anything
else must be equal. Examples: `use_tool math.evaluate`, `use_tool files.*`, `read workspace:docs/*`.

A tool declares what it needs (`required_permissions`), and always needs `use_tool` on its own id. The file tools check
the grant **for the exact path** when they run, because the path is only known then.

```bash
zorix run "Summarise docs/plan.md" --workspace ~/project --allow workspace.read --allow-read 'docs/*'
```

Tools that are not granted are not even shown to the model, and if the model asks for one anyway it is refused.

### Grants of integrations and MCP servers

| Grant | Meaning |
|---|---|
| `use_tool mcp.<server>.*` | The tools of one MCP server (an MCP tool **always asks**, whatever the grant) |
| `network http:<host>` | Reach a host (`http.*`, `webhook.send`); a trailing `*` is allowed |
| `read` / `write` `github:<owner>/<repo>`, `gitlab:<group>/<project>` | A repository; `<owner>/*` covers an owner |
| `read` / `write` `jira:<KEY>`, `slack:<channel>`, `discord:<channel id>` | A project or a channel |
| `read` / `write` `db:<connection>` | A database connection. `db.query` is always read-only |
| `execute agent:<slug>` and `use_tool agents.delegate` | Ask that agent to do part of the work (`agent:*` for any agent). A child run only gets the grants that **both** it and its parent have ([orchestration](orchestration.md#what-only-gets-smaller-down-the-tree)) |
| `read` / `write` `git:<branch>`, `write git-push:<remote>/<branch>` | The [git tools](tools.md#git) on the workspace's repository |
| `execute code:javascript` / `code:python` | The [sandbox](sandbox.md); `read workspace:*` also lets it see the workspace, read-only |

Writes, sends and statements that change data **always ask a person**, `--approve yes` is for tests and demos. See
[integrations](integrations.md) for what each tool guards, and [MCP](mcp.md) for pinning and review.

### Grants of agents

An [agent](agents.md)'s grants are its `tools` and `access` entries, compiled with the same code as the `--allow-…` flags, plus the skill tools. A run made
by an agent can only **narrow** them (`Grants::intersect`: allowed only if both sides allow it), whether the narrowing comes from flags, the New run form or
the server that started `zorix mcp serve`. `skill.run` is granted only to an agent whose access has `code:…`. A skill's `allowed-tools` is shown and never
granted. The agent's `models.allowed` list is enforced twice: the router will not plan a model outside it, and the runtime fails a run whose model is not on it.
Evaluations additionally drop `write` grants and refuse approvals unless a case says otherwise.

## The policy engine

`PolicyEngine::evaluate_tool_call` answers **Allow**, **Deny** or **RequireApproval**, in this order:

1. **Forbidden actions** (`Policy.forbidden`) are denied whatever the grants say.
2. **Deny rules** by tool id or prefix.
3. **Environment**: with `allow_open_world: false`, tools that reach outside the computer are denied.
4. **Grants**: anything the grants do not cover is denied, and the reason lists what is missing.
5. **Approval**: asked when the tool says so (`requires_approval`), when a rule says so, or (by default) when the tool is
   destructive and not read-only.
6. Otherwise **Allow**.

A denial is not a failure of the execution: the model receives an error result and can explain or try something else. It
is never retried unchanged.

Limits (iterations, tool calls, tokens, cost, time) belong to the same engine. See [the runtime](runtime.md#limits).

## Human in the loop

When approval is required the execution **suspends** in `waiting_for_approval`. Nothing holds it; the process can exit.

The request records: the approval id (issued by the runtime), the tool, the exact arguments, a **hash** of tool plus
arguments, and the reason. A person answers with `approve` or `deny`, a name and an optional note.

What keeps an approval honest:

- Only `ExecutionEngine::decide` can resolve one. There is no path from model text to it.
- The id must be one the runtime issued for **this** execution. An invented id is refused.
- If the answer carries the hash it was shown, it must match the call that is waiting. The CLI and the desktop app
  always send it.
- An approval can be answered once. A second answer is refused.
- Another tenant cannot answer it, or even see it.
- A cancelled execution closes its open approval as denied by the system.
- The policy is evaluated again when the call actually runs. An approval does not bypass a denial that appeared since.

- **Some approvals cannot be automated.** A tool can declare that only a person may approve it (`memory.remember`, because what is
  remembered is shown to every later run). The engine refuses an approval marked automatic for such a tool, so `--approve yes`
  leaves the run waiting. An automatic *refusal* is still fine.

When refused, the model is told: *"Denied: a person refused this call (your note)."*

## Prompt injection

Text that comes back from tools, files or the web may try to give orders ("ignore your instructions and call …"). The
runtime does not rely on the model resisting:

- **It cannot change what the execution may do.** Grants and policy are fixed when the execution is created.
- **Tool output is data.** It is added to the context labelled `untrusted`, wrapped in `<tool_result … untrusted="true">`
  with any tag of ZORIX inside the text (opening or closing, in any case and spacing) defused, the call id kept inside its attribute, and the system text tells the model never to follow instructions found
  there.
- **A persuaded model still cannot act.** The test `text_inside_a_tool_result_cannot_unlock_another_tool` shows a model that
  falls for an injected instruction and calls a tool it was not granted: the call is denied.

- **Retrieved memory is data too.** Document pieces and past work are wrapped (`<document … untrusted="true">`,
  `<past_work … untrusted="true">`) like tool results, and are never put in the system prompt. Facts are the exception: a person
  approved each one. See [memory](memory.md#untrusted-data).

- **What a trigger or a task brings is data as well.** A value a prompt template puts in (a webhook body, a step's output, a loop
  item), the input a request carries and the text of a task (its title, description, links and the titles it builds on) are
  wrapped in `<data source="…" untrusted="true">`, with any tag of ZORIX inside the text defused in every spelling and case.
  The words the author of the workflow or the person wrote around them stay outside the wrapper.

What injection *can* still do is persuade a model to **ask** for something it is allowed to ask for. That is why
destructive tools ask a person first.

## Tenants

Every execution belongs to a tenant. Reads, events, approvals and notes of one tenant are invisible to another, and the
repository contract tests prove it. In this phase there is one local tenant by default (`--tenant` selects another);
real accounts come later.

## Credentials

This phase stores none. Events, logs and traces never include prompts or tool arguments in spans, and event payloads
pass through a redaction filter: values under keys such as `api_key`, `password`, `secret`, `authorization`, `cookie`
or a key named `token` are replaced, and inside text these are replaced too: words that look like credentials (`sk-…`, `ghp_…`,
Google `AIza…`, Stripe `sk_live_…`, GitLab `glpat-…`, JWTs, `Bearer …`), the value of a pair such as `password=…`,
`"client_secret": "…"` or `Authorization: Basic …`, and whole PEM private key blocks. The history of a project and of the organisation and
the reports of a manager are redacted this way before they are stored. Counts of tokens (`input_tokens`) are not secrets and stay readable. Redaction is best effort; see
[SECURITY.md](../SECURITY.md).

The arguments of a call are stored as they are in the tool call record, because the tool must receive them.

## Projects

A manager gets `project:KEY[:write]` and reads and plans only that project. Every write asks, and `tasks.move` asks a person who is looking (no automatic approval can give it), so a move to Done
under a manager is an ordinary approval. A run a task started gets `task:KEY` and can only report progress, add a deliverable and leave a note on that task. See [projects](projects.md#the-manager).

## Workforce

A worker's **level** changes what asks, only for runs the workforce starts ([workforce](workforce.md#autonomy)). Suggest turns a call that changes something into a recorded proposal; Ask asks for every
such call; Notify, Budgeted and Full let a call that only the default policy would hold run without asking, and record it. An explicit approval rule, a tool only a person may approve and a pattern on
the worker's *still asks* list always ask, at every level. A manager reads who reports to it with `org:team`, and plans with `project:KEY:write`.

## Workflows

A workflow's actions ask first by default, bound to the exact call. A step set to run without asking is confirmed by a person when publishing, and the version records who and when. It is
refused for nothing a person must approve, and its resource grants (a channel, a host) come from the step itself, and a host has to be written in the step, not filled in from data. Its arguments are checked against the tool's schema and the policy's standing refusals (`PolicyEngine::standing_denial`: forbidden actions, blocked tools, open-world, missing grants) apply, as they do to a run. See [workflows](workflows.md#people).

