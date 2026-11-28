# Security policy

ZORIX runs AI models that can ask to use tools on your computer, so security is part of the design, not an add-on.

## Supported versions

Only the latest release receives security fixes while the project is below 1.0.

## Reporting a vulnerability

**Please do not open a public issue.** Use GitHub's private reporting instead:

<https://github.com/LuisPeixoto/zorix/security/advisories/new>

Include, where possible: a description and impact, steps to reproduce or a proof of concept, and the affected versions
and operating systems. You will get an acknowledgement as soon as the maintainer can review it, and credit in the
release notes unless you prefer to stay anonymous.

## What is in scope

- A model request (including text injected through tool output or files) that gets around a permission, a policy or a
  required approval
- An approval that can be forged, replayed, or applied to a different tool call than the one shown to the user
- A built-in tool reaching files outside its workspace root
- Secrets (API keys, tokens) appearing in logs, events, traces, the database, `providers.toml` or the interface
- A provider call that sends a key or a conversation somewhere other than the configured host, or ignores `--local-only`
- Data from one tenant or execution becoming visible to another
- Memory safety or parsing flaws in the persistence layer

## Dependencies and releases

CI audits the dependencies on every change (`cargo deny check` with the policy in `deny.toml`, `npm audit` for the
interface) and runs CodeQL weekly. A release is not tagged while an advisory that affects ZORIX is open; an advisory that
does not affect it is accepted in `deny.toml` with the reason.

## Design notes

These notes grow with each feature that handles data, files, network or permissions. The long form is in
[docs/permissions.md](docs/permissions.md), [docs/tools.md](docs/tools.md) and [docs/persistence.md](docs/persistence.md).

- **Deny by default.** An execution only has the permissions it was created with. Tools are checked when they are
  *run*, not only when they are offered to the model, and the policy is evaluated again after an approval.
- **The model is untrusted.** Its output cannot change grants, policy or approvals. Tool results are added to the
  context labelled as untrusted data, wrapped so they cannot close their own wrapper.
- **Approvals are bound to one call.** An approval names the runtime-issued id and the hash of the tool and its exact
  arguments. Only the engine's `decide` can resolve one; it can be answered once, by the right tenant, for the call that
  is waiting, and an answer that does not name the call's hash is refused (before 0.10.0 it was accepted). If the stored
  call is edited while it waits, the altered call does not run on the answer given to the original: it asks again with its own hash. A call that was running when the process stopped and cannot be repeated safely asks a person.
- **File tools are confined to a workspace folder, per execution.** Paths must be relative, with forward slashes, no
  `..`, no drive letters, no control characters, no Windows reserved names and nothing in `.zorix`. The real path,
  after following links, must stay inside the root; links that point outside are not followed and files are not written
  through a link. Writes go through a temporary file and a rename. Reads, listings and writes are size-limited. The check and the open are two steps, so a program that can change
  the workspace at that very moment could redirect one call; this is stated in the [tools guide](docs/tools.md), and a workspace should not be a
  folder another program writes to. This is
  *path* confinement. Code runs only in the [WebAssembly sandbox](docs/sandbox.md); a stdio MCP server is a separate program that runs as you.
- **Bounded everything.** Iterations, tool calls, tokens, cost, working time, model and tool time, tool output size,
  context size, note count and size, file sizes, calculator length and depth all have limits, listed in the docs.
- **Secrets stay out.** Event payloads pass through a redaction filter (secret-looking keys, and words that look like
  API keys, GitHub, GitLab, Google, Stripe and AWS keys, JWTs and `Bearer` tokens, `password=…` pairs and PEM private keys). Trace spans and logs carry identifiers and names only,
  never prompts, arguments or results, and a test fails if they do. Redaction is **best effort**: a secret in free text
  with no recognisable shape can slip through, so do not paste secrets into prompts. The arguments of a call are kept
  as they are in the local database, because the tool must receive them.
- **Local data.** The database and the notes live in the data folder (`ZORIX_HOME`, or the platform default). They hold
  prompts, tool arguments and results: treat the folder as sensitive. ZORIX sends no telemetry unless you give it a collector address
  (off by default; see below). It connects only to the providers you configure, and only when a run uses them; the desktop interface itself can make no network request (its
  content security policy allows none), so every call goes through the Rust side.
- **Telemetry is yours to turn on, and says what leaves.** With a collector address set ([ADR-0046](docs/adr/0046-opentelemetry-export.md)), the spans of
  each run go to it: the model, token counts, cost, tool names, how a call or run ended, and ids. A prompt, an answer, a tool's arguments or its result never do,
  and a test checks the bytes sent. The address must be `https` (or a collector on this computer) and cannot hold a password; the header a collector needs is kept in the
  system keychain. Export failures are not reported.
- **MCP servers are third parties.** A server you add is added **off** and needs your confirmation; a stdio server is a program
  that runs **as you with no sandbox**, started without a shell, with a clean environment plus the variables you named. Its
  tools are pinned: nothing is offered until you approve what it says, a changed description withdraws the tool until you
  review it, and a run keeps the definitions it started with. Every MCP tool asks first; the server's own hints never grant.
  What a server returns is data. ZORIX refuses sampling, elicitation and roots. OAuth uses PKCE, a state value, the resource
  indicator and issuer checks; tokens are in the keychain, sent only to their server, and never logged. Details in
  [docs/mcp.md](docs/mcp.md).
- **Native integrations are bounded by grants.** A tool needs `use_tool` and a permission on the resource, and writes ask. Web
  requests are HTTPS-only, resolve and check every address (SSRF and DNS rebinding), follow granted redirects only, use no
  proxy, and the model cannot supply credentials. Jira searches are always confined to the granted project; Slack and Discord
  mass mentions are refused; SQLite opens read-only; Postgres uses verified TLS (never falling back to plain text) and
  read-only transactions for queries. See [docs/integrations.md](docs/integrations.md).
- **The code sandbox has no network and hard limits.** WebAssembly (wasmtime): 256 MB, 10 s, 1 MB of output, 50 MB of scratch
  files, no sockets; workspace files are visible only if granted and then read-only. The Python runtime is downloaded on request
  and checked against a fixed SHA-256. See [docs/sandbox.md](docs/sandbox.md).
- **`zorix mcp serve` is read-only by default** and holds no lock; with `--allow-runs` approvals still need a person in ZORIX.
- **Data leaves the computer when a cloud provider is used.** The conversation, including tool results, is sent to that
  provider. `--local-only` (or `local_only = true` in `providers.toml`) restricts routing to servers on loopback addresses.
  Choosing a provider is a decision about where data goes; read the provider's terms.
- **API keys** live in the operating system's keychain or an environment variable, are read when a call is made, and are
  held in a type that prints as `[redacted]`. They are sent only in an authentication header, never in a URL or a log.
  `providers.toml` is refused if it holds anything key-shaped. Provider error text is redacted before it reaches an event.
  Base URLs must be `https://` unless they are loopback; redirects are not followed, so a key cannot be bounced to
  another host. Response bodies and streams have size and idle limits.
- **Memory is written only with a person's approval.** A model can propose a fact with `memory.remember`; the approval cannot be
  automatic (the engine refuses it), shows the exact text, and credential-shaped text is refused. Approved facts are labelled to the
  model as possibly out of date. Memory is scoped per tenant, and a project's memory is visible only to runs in that folder.
- **Text from triggers and tasks is untrusted data too.** A webhook body, a step's output, a request's input and the text of a task
  reach a model wrapped in `<data … untrusted="true">`, with ZORIX's own tags defused in any case and spacing. (Before 0.10.0
  a prompt template put such values in as the author's own words; this is fixed.)
- **A webhook delivery is started once.** An id the sender gives a delivery is remembered for 24 hours, so a retry or a replay of a captured request starts nothing. A sender
  that gives no id cannot be told apart from a new delivery, and a captured signed request can then be replayed until the signing secret is replaced; say so to
  anyone who exposes the listener.
- **Retrieved documents and past work are untrusted data.** They are wrapped, their closing tags defused, and never placed in the
  system prompt. Indexing reads files only: `.gitignore`, hidden files, links and credential-like files (`.env*`, keys,
  `credentials*`) are skipped, text that looks like a secret is removed from pieces before storage, and PDFs are parsed on an
  isolated thread with size, page and time limits.
- **Reading scans runs on this computer, from models that are checked.** It is off until you turn it on ([ADR-0048](docs/adr/0048-watching-folders-and-reading-scans.md)).
  The two model files (about 12 MB) are downloaded once over `https` and used only when their size and SHA-256 match the values fixed in the program, at download and at every load, so a swapped or
  changed file is never given to the model parser. The pictures in a PDF are decoded with the pages, pixels, characters and time bounded before anything is allocated, on a thread whose crash
  leaves the file out and nothing else. Nothing is sent anywhere but the one download, which carries no data of yours. The text it reads is as untrusted as any document's.
- **A file attached to a run is data the person chose, and nothing more.** It is checked before anything is read (a regular file, no private name even through a link, not under a folder of credentials, at most 1 MB of text or a 25 MB PDF, at most ten files),
  cleaned of secret-looking text, stored with the run, and given to the model after the task as an untrusted `<document>` that cannot close its wrapper or speak as the person ([ADR-0049](docs/adr/0049-files-attached-to-a-run.md)). It adds no permission.
  Compacting a run drops its text. A cloud model receives the text, like any other part of the conversation.
- **A cloud embeddings model receives text.** The pieces of your documents, your facts and the questions of runs are sent to it,
  so it needs an explicit opt-in, is never used by a `--local-only` run, and never replaces another model silently.
- **Deleting is real, with limits.** Forgetting a fact or a folder deletes its rows, search entries and vectors, scrubs saved
  run contexts, and overwrites freed pages. It cannot reach the transcript of a run that already used it, a backup you made, or
  what a cloud provider received.
- **One program per data folder.** The command line and the desktop app lock the folder. A database written by a newer
  version is refused rather than risked.
- **Tenants never mix.** Every read, event stream, approval and note is scoped to a tenant; another tenant gets "not found".
- **The command line validates before it acts.** Every argument is checked by a value parser before the data folder is
  touched. Nothing is sent through a shell.
- **Desktop shell.** The interface holds no rules; it calls commands that return plain errors. Permissions are limited to
  the window's core capabilities and the folder-picker dialog.
