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
- Secrets (API keys, tokens) appearing in logs, events, traces or the database
- Data from one tenant or execution becoming visible to another
- Memory safety or parsing flaws in the persistence layer

## Design notes

These notes grow with each feature that handles data, files, network or permissions. The long form is in
[docs/permissions.md](docs/permissions.md), [docs/tools.md](docs/tools.md) and [docs/persistence.md](docs/persistence.md).

- **Deny by default.** An execution only has the permissions it was created with. Tools are checked when they are
  *run*, not only when they are offered to the model, and the policy is evaluated again after an approval.
- **The model is untrusted.** Its output cannot change grants, policy or approvals. Tool results are added to the
  context labelled as untrusted data, wrapped so they cannot close their own wrapper.
- **Approvals are bound to one call.** An approval names the runtime-issued id and the hash of the tool and its exact
  arguments. Only the engine's `decide` can resolve one; it can be answered once, by the right tenant, for the call that
  is waiting. A call that was running when the process stopped and cannot be repeated safely asks a person.
- **File tools are confined to a workspace folder, per execution.** Paths must be relative, with forward slashes, no
  `..`, no drive letters, no control characters, no Windows reserved names and nothing in `.zorix`. The real path,
  after following links, must stay inside the root; links that point outside are not followed and files are not written
  through a link. Writes go through a temporary file and a rename. Reads, listings and writes are size-limited. This is
  *path* confinement, not a sandbox for code: no built-in tool runs code, and a tool that does will need real isolation first.
- **Bounded everything.** Iterations, tool calls, tokens, cost, working time, model and tool time, tool output size,
  context size, note count and size, file sizes, calculator length and depth all have limits, listed in the docs.
- **Secrets stay out.** Event payloads pass through a redaction filter (secret-looking keys, and words that look like
  API keys, GitHub tokens, AWS keys, JWTs and `Bearer` tokens). Trace spans and logs carry identifiers and names only,
  never prompts, arguments or results, and a test fails if they do. Redaction is **best effort**: a secret in free text
  with no recognisable shape can slip through, so do not paste secrets into prompts. The arguments of a call are kept
  as they are in the local database, because the tool must receive them.
- **Local data.** The database and the notes live in the data folder (`ZORIX_HOME`, or the platform default). They hold
  prompts, tool arguments and results: treat the folder as sensitive. ZORIX has no telemetry and makes no network
  connection of its own in this version; the desktop app's content security policy allows none.
- **One program per data folder.** The command line and the desktop app lock the folder. A database written by a newer
  version is refused rather than risked.
- **Tenants never mix.** Every read, event stream, approval and note is scoped to a tenant; another tenant gets "not found".
- **The command line validates before it acts.** Every argument is checked by a value parser before the data folder is
  touched. Nothing is sent through a shell.
- **Desktop shell.** The interface holds no rules; it calls commands that return plain errors. Permissions are limited to
  the window's core capabilities and the folder-picker dialog.
