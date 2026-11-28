# Changelog

All notable changes are documented here. The format follows [Keep a Changelog](https://keepachangelog.com) and the
project follows [Semantic Versioning](https://semver.org).

## [Unreleased]

Phase 12: the tools, integrations and sandbox audited against the engineering guidelines (thirty findings, [the audit](docs/audits/phase-12-tools-integrations-sandbox.md)) and completed.

### Added

- **Git tools** ([ADR-0054](docs/adr/0054-git-tool.md)): `git.status`, `git.diff`, `git.log`, `git.branch`, `git.commit` and `git.push` on the repository of the workspace, with their own grants (`--allow-git`, `--allow-git-push`), nothing the repository configures ever run, and approvals that show the diff. A push sends exactly the approved commit and is never forced.
- **Sign in to GitHub, GitLab and Jira** ([ADR-0053](docs/adr/0053-sign-in-for-native-integrations.md)) instead of pasting a token: a device code for GitHub, your browser for GitLab and Jira, with ZORIX's app ids when a build has them or your own. Tokens are renewed and kept in the keychain; pasted tokens keep working. Slack keeps taking a bot token.
- **MCP sampling and elicitation** ([ADR-0051](docs/adr/0051-sampling-and-elicitation.md)): while one of its tools runs, a server may ask to use your model, ask you a question or ask you to open a page. Each request waits for your answer (a card in the run and the Inbox, or a prompt on the terminal), nothing is asked when nobody is watching, a model call is charged to the run, and refusals are in the server's log and the run's events ([#77](https://github.com/LuisPeixoto/zorix/issues/77)).
- **See, answer and stop runs from another MCP client** ([ADR-0052](docs/adr/0052-runs-and-approvals-over-mcp.md)): `zorix mcp serve --allow-runs` adds `zorix.runs`, `zorix.approvals` and `zorix.cancel`, and offers runs and agents as resources. With `--allow-approvals` the client can pass your answer to a waiting call, and each answer is confirmed by you, with the exact call shown; without that, approvals stay in ZORIX. The app builds the command in *Settings*.
- **Local MCP servers are sandboxed** ([ADR-0050](docs/adr/0050-sandbox-of-local-mcp-servers.md)): a new one reads the system, its own folder and the folders you give it, writes only there, cannot read your keys, credentials, keychain or ZORIX's data, cannot reach other programs' services (keyring, Docker, SSH agent), and has the network only when it has to download its package. macOS and Linux enforce it; Windows confines processes and memory only, and says so. In the drawer (*Details › Sandbox*), `zorix mcp sandbox` and `zorix mcp doctor`. Servers you added before keep running as before until you choose. **Changed:** the database moves to schema 17. A **container mode** runs a server with Docker or Podman instead (read-only, no capabilities, only its folders, images pinned by digest), which also confines files and the network on Windows.
- **Eleven more MCP servers in the catalogue** (22 in all): Neon, Supabase, Vercel, Asana, Stripe, Hugging Face, Context7, and the reference *Git*, *Memory*, *Sequential thinking* and *Time* servers, each tried against its real address or package on the day.
- **Local catalogue servers are pinned to one exact version** (`@playwright/mcp` was `@latest`, the others unpinned), so the code behind the tools you approved does not change under you. A server added before keeps its command ([#75](https://github.com/LuisPeixoto/zorix/issues/75)).
- **Sandbox escape tests on every system**: paths that name the host, links that lead out of a folder the program was shown, mounts that try to take another's place and writes where only reading was allowed are tried by a program inside the sandbox, on macOS, Linux and Windows ([#79](https://github.com/LuisPeixoto/zorix/issues/79)). None found a way out.

### Fixed

- **A run could plant a git hook**: `workspace.write` could write inside `.git` (a hook, or a `config` that names a program), which git then ran as you. Nothing is written inside a `.git` folder now, by name or through a link ([#84](https://github.com/LuisPeixoto/zorix/issues/84)).
- **A Jira search could be widened to other projects** by a query whose parentheses did not close (`x = 1) OR project = OTHER OR (x = 1`): the person's part is wrapped in a group, and a `)` ended the wrap. A query that does not balance, or a sort clause that is more than field names, is now refused before anything is sent, and the project must be a project key ([#52](https://github.com/LuisPeixoto/zorix/issues/52)).
- **A workflow's HTTP step could be aimed at any machine by the data that started it**: the step's grant came from its own address, which can be filled in from a webhook payload, so `https://{{trigger.payload.host}}/…` reached `localhost`, the cloud metadata address and the local network with nobody asked. The scheme and the host must now be written in the step (saving, starting and running all refuse it); text from data may only come after the host ([#53](https://github.com/LuisPeixoto/zorix/issues/53)).
- **A remote MCP server could make ZORIX call private addresses while signing in**: the addresses of discovery and of the token calls come from the server, and any HTTPS address (a private one, the cloud metadata address) or any loopback port was accepted. A server that is not on a private network may no longer name one; the name is resolved and checked, and the connection goes to the address that was checked ([#56](https://github.com/LuisPeixoto/zorix/issues/56)).
- **Any program on this computer could end an OAuth sign-in** by sending a request to the callback port: only an answer with the sign-in's own `state` ends the wait now ([#62](https://github.com/LuisPeixoto/zorix/issues/62)).
- **A `*` at the end of a host grant covered hosts somebody else can register**: `--allow-http 'api.example.com*'` also allowed `api.example.com.evil.net`. A `*` is now accepted alone, or after a `.` or a `:` (`docs.*`, `localhost:*`); a name with a `*` glued to it is refused with the reason, and a grant built that way by hand matches only itself. **Changed:** a stored grant of the refused form no longer loads; write the host in full ([#57](https://github.com/LuisPeixoto/zorix/issues/57)).
- **A connection could point at an address where no service lives**: the address of GitHub Enterprise, GitLab, Jira, Slack or Discord was checked only for HTTPS, so a mistyped or imported address could aim a token at the cloud metadata address and its neighbours. Link-local, multicast, broadcast and reserved addresses are refused, as written and as what a name resolves to; private networks stay allowed ([#59](https://github.com/LuisPeixoto/zorix/issues/59)).
- **Approving an MCP tool approved whatever the server listed at the click, not what you had read**: a server could change a tool's description between your reading it and pressing *Approve*. The approval now carries the hash of the definition on screen and is refused, with the reason, if the server lists another one ([#60](https://github.com/LuisPeixoto/zorix/issues/60)).
- **A read-only MCP tool that did not say whether it reaches outside the server could be set to run without asking**, although the specification's default is that it does. It must now say `openWorldHint: false`; earlier choices that do not qualify are withdrawn ([#61](https://github.com/LuisPeixoto/zorix/issues/61)).
- **`zorix mcp serve --allow-runs` let a client read any run of the data folder**: `zorix.status` and `zorix.result` accepted any execution id, so a client could read the prompts and answers of every run you ever made in ZORIX. They now follow only the runs that server started (and their sub-runs); `--allow-read-runs` opens the others, and a run that is not the client's is refused the same way as one that does not exist ([#63](https://github.com/LuisPeixoto/zorix/issues/63)).
- **The workspace path rules were not the same on every system**: `COM0`, `LPT0`, `CONIN$`, `CONOUT$`, the superscript-digit device names, `CON .txt` and the characters `< > " | ? *` were accepted, and a short Windows name such as `ZORIX~1` (which on a volume that keeps short names is `.zorix`) walked around the reserved folder. They are refused on every system now, so the rules do not depend on the volume ([#74](https://github.com/LuisPeixoto/zorix/issues/74)). A file named like `NOTES~1.TXT` cannot be used by its short form any more; use its own name.
- **A streamed MCP answer could grow without limit**: only each line was capped, so an event of endless `data:` lines, or a stream of small events that never answers, was read until the call timed out. One event is now at most 4 MiB and a whole streamed answer 16 MiB ([#64](https://github.com/LuisPeixoto/zorix/issues/64)).
- **An MCP result had no ceiling on the number of items or on its structured content**: a body full of empty items cost memory, and a huge `structuredContent` crowded out the text. At most 100 items are passed on (saying how many were left out) and structured content over 32 KiB becomes a note ([#65](https://github.com/LuisPeixoto/zorix/issues/65)).
- **A very long line on a server's error stream garbled the next one**: the rest of the long line is skipped to its newline, and no further ([#70](https://github.com/LuisPeixoto/zorix/issues/70)).
- **A stopped MCP server could leave what it had started running**: on Windows the real program behind `npx.cmd` outlived its server (no Job Object), and on macOS and Linux a server that left by itself or crashed left its helpers behind, and stopping never asked (SIGTERM) before it killed. Servers now go with everything they started ([#66](https://github.com/LuisPeixoto/zorix/issues/66)). The Windows part is checked by compiling and by the cross-platform test on the Windows runner of CI.
- **A stdio MCP server that named no folder started in the folder ZORIX was started from** (the comment said the data folder): each server now gets a folder of its own under `<data folder>/mcp` ([#67](https://github.com/LuisPeixoto/zorix/issues/67)).
- **A remote MCP server that forgot its session stayed broken until restarted by hand** (legacy Streamable HTTP): every call after the server's 404 failed. The connection now ends, and the manager starts a new one with a new `initialize` and no session id, as the specification asks ([#68](https://github.com/LuisPeixoto/zorix/issues/68)).
- **MCP conformance details**: a timeout no longer cancels `initialize` (the specification forbids it), a request given up on over HTTP now tells a server that has a session with `notifications/cancelled`, and a server that echoes an id as a string (`"7"` for `7`) is understood instead of timing out ([#69](https://github.com/LuisPeixoto/zorix/issues/69)). The documented "era is cached" was not true and is corrected: the era is probed at every start.
- **Two environment variables of an MCP server that differ only by case shared one secret**: `FOO` and `foo` are one variable on Windows and one keychain entry in ZORIX, so one value silently replaced the other. A server that lists both, or a name that is not a variable name, is now refused when it is added, imported or changed ([#72](https://github.com/LuisPeixoto/zorix/issues/72)).
- **A workflow's *Send message* step to Discord never worked**: it sent `channel` and `text`, and the Discord tool reads `channel_id` and `content`. Both are sent with their own names now, and a Discord id that is not 5 to 25 digits is refused inside the tool as well, so it can never carry a `/`, a `..` or a `?` into the path of a request ([#73](https://github.com/LuisPeixoto/zorix/issues/73)).
- **An HTTP step with an explicit port always failed**: the workflow granted `host:port` and the tool asked for `host`. A grant may name the host (any port) or the host and one port, and a step names the port it was written with ([#73](https://github.com/LuisPeixoto/zorix/issues/73)).
- **The database tool could hold a huge answer in memory before cutting it, and `db.execute` could write a copy of a SQLite database anywhere**: Postgres rows are now read as they arrive and reading stops at the limits (`SHOW` and `EXPLAIN` included), and `VACUUM INTO` (which wrote a copy wherever it was told) is refused in both SQLite tools ([#78](https://github.com/LuisPeixoto/zorix/issues/78)). A Postgres role that may call functions with effects outside the tables is still the person's to limit, and the guide says so.
- **A remote MCP server that asked for more permission got the same sign-in again**: after a `403 insufficient_scope`, signing in again asked for the scopes that had already been refused. The missing scope is remembered, and the next sign-in asks for what was granted and what was missing ([#71](https://github.com/LuisPeixoto/zorix/issues/71)).
- **The app took any `npx` server for the *Filesystem* entry of the catalogue** (they all start with `-y`): it showed its mark and marked it as added whatever you had added. A server is now recognised by the package it runs, whatever its version, and the catalogue marks the entry itself ([#81](https://github.com/LuisPeixoto/zorix/issues/81)).
- **An MCP server that failed to start gave no reason**: a program that wrote why (a missing dependency, a wrong argument) and exited was reported as *"The server is not running"* with an empty log. The reason now ends with the last lines it wrote (secrets removed) and the status it ended with, and the log keeps all of it ([#82](https://github.com/LuisPeixoto/zorix/issues/82)).
- **On Windows a file could not be read while another run was writing it**: `workspace.read` answered *outside the workspace* or *no such file* for the instant in which the file was being replaced. It now asks again for a few milliseconds ([#83](https://github.com/LuisPeixoto/zorix/issues/83)).
- **A symbolic link inside the workspace routed around a grant and the reserved folder**: with the grant `workspace:docs/*`, a link `docs/shortcut` to `private/notes.txt` was read, and a link to `.zorix` reached ZORIX's own folder, because the grant and the reservation were checked on the text of the request and only the containment on the real path. The place a link leads to now needs its own grant and cannot be reserved; a listing hides `.zorix` in any case ([#55](https://github.com/LuisPeixoto/zorix/issues/55)).
- **A write's temporary file followed a link and two files could share one**: its name was the target's with a new extension, so a link left at that name sent the write elsewhere, and `a.txt` and `a.md` used the same name. It is now created new, with a name of its own ([#58](https://github.com/LuisPeixoto/zorix/issues/58)).
- **A workflow action skipped the checks a model's tool call gets**: its arguments were not validated against the tool's schema (a Discord channel went into the request path as it came) and the policy's refusals did not apply. Both now do, through the same code as a run's calls ([#54](https://github.com/LuisPeixoto/zorix/issues/54)).

## [0.11.0] - 2026-11-27

Phase 11: the runtime, providers and memory audited against the engineering guidelines (thirteen findings, [the audit](docs/audits/phase-11-runtime-providers-memory.md)) and completed.

### Added

- **Files can be given to a run**: with the *Attach* button, with `@` in the prompt (a menu of indexed documents and files of the workspace folder), by dropping them on the window, or with `zorix run --attach FILE`, in New run and in the chat with an agent. Each file is checked before anything is read (a regular file, no private name even through a link, not under a folder of credentials, at most 1 MB of text or a 25 MB PDF, at most ten files), cleaned of secret-looking text, and given to the run after the task as an untrusted document that cannot close its wrapper or speak as you, cut in proportion when together they would take more than half the model's window. A file outside the workspace is allowed because you chose it, and the run still cannot open anything else. The estimate counts the files. Compacting an old run drops the text and keeps the name and the size ([ADR-0049](docs/adr/0049-files-attached-to-a-run.md)).
- **Scanned PDFs are read on this computer** (off until you turn on *Reading scans*): the text of a PDF that is pictures of pages is read out of them by an OCR engine in pure Rust, with two small models (about 12 MB) that are downloaded once, checked against fixed hashes and never used if they are not the exact files expected. A scan is cited by page like any PDF, a folder shows *N via OCR* and *N scans not read*, and a scan that failed is not tried again until its file changes. Pages, pixel counts, text and time are bounded before anything is allocated. Documents › Reading scans, `zorix settings set knowledge.ocr on` ([ADR-0048](docs/adr/0048-watching-folders-and-reading-scans.md)).
- **Folders of documents are watched**: a change is read again a few seconds after the files stop changing, while the app is open (or `zorix knowledge watch` runs), with one sync for a burst of changes, nothing for hidden files, and a ten-minute check for a folder the system cannot watch, which the screen explains. New folders are watched; mark an older one with *Watch* or `zorix knowledge add --watch`. Database schema 16 ([ADR-0048](docs/adr/0048-watching-folders-and-reading-scans.md)).
- **A vector index for large collections of documents**: above 20,000 visible pieces, search by meaning ranks one-bit codes kept in memory and in a file next to the database and rereads only the best few hundred vectors for the exact order, so it takes 7 ms over 50,000 pieces and 49 ms over 490,000 instead of scanning every vector (188 ms over 50,000), and no longer holds the store's connection while it scans. About 96 to 98% of the exact top ten is kept on the benchmark's data. The limit rises from 200,000 to 500,000 pieces per installation. Database schema 15 ([ADR-0047](docs/adr/0047-index-of-document-vectors.md)).
- **OpenTelemetry export** of the spans of each run (`invoke_agent`, `chat`, `execute_tool`) to a collector you run, over HTTP: off by default; `https`, or `http` for a collector on this computer; a header the collector needs is kept in the system keychain; and only the runtime's spans with the model, token counts, cost, tool names and outcomes, never a prompt, an answer or an argument (a test checks the bytes sent). Settings › General › Telemetry, `zorix settings set telemetry.endpoint` and `telemetry.header` ([ADR-0046](docs/adr/0046-opentelemetry-export.md)).
- **An estimate before a run starts**: what it will probably cost and how long it will take, from the median of your last similar runs, or a rough guess (and it says so) when there are fewer than three. A model with no price shows *cost unknown*, never $0. New run shows it in *Before you start* and gains *Limits* (budget, time, steps; an agent's can only be narrowed); `zorix run --estimate` prints it and `--max-cost` limits the spend ([ADR-0045](docs/adr/0045-estimates-before-a-run.md)).
- **Retention of finished runs** (off until you set a number of days): old runs are **compacted** (their events, steps, approvals and recall go; the request, the answer, the cost and the time stay, so spend and budgets do not change) or **deleted**, whole trees at a time and never what is still moving. It runs ten minutes after the app starts and then daily, and on demand with a preview first: Settings › General › Keep runs, `zorix retention show` and `zorix retention run`. Database schema 14 ([ADR-0044](docs/adr/0044-retention-of-finished-runs.md)).
- **Long conversations are summarised** instead of cut: when the history of a run no longer fits the model's window, the oldest turns become a summary that stands where they were, written by the run's model, kept in the run's checkpoint (so a resume shows the same text), shown to the model as untrusted data, and counted in the run's usage and limits. An `execution.context.summarised` event says how much was folded and what it cost. On by default; `zorix settings set context.summaries off` turns it off ([ADR-0043](docs/adr/0043-summaries-of-long-conversations.md)).

- A place for the program's settings in the database (schema 13), `zorix settings show` and `zorix settings set`, and the commands the desktop app will use: long-conversation summaries, how long runs are kept, a telemetry address and reading scanned pages. All of them are off, or conservative, until a person changes them.

### Changed

- **The database is upgraded to schema 16 the first time 0.11 opens it** (past work gets its agent, the records of past work are written again at the first start, three tables are added, and the document index and the watch switch get their columns). A 0.10 program refuses the upgraded data folder, so back it up first if you may go back ([persistence](docs/persistence.md)).
- The reason a scanned PDF is left out reads *it is a scan, with no text to read (turn on reading scans in Settings to read it)*; a script that matched the old words (*a scan needs OCR, which is not included*) needs the new ones.
- The limit of a model call is time **without progress**, not the whole call (see Fixed), and the interface's Settings > General holds the new cards (long conversations, how long runs are kept, telemetry).
- The cost shown for a model with no known price is *cost unknown*, never $0.00.

### Fixed

- Syncing a large folder no longer loads the text of every piece that needs a vector just to count them (it can be hundreds of megabytes), and writing a file no longer counts every piece of the tenant first. Both are measured by `zorix-bench` now.

- A cost limit (a run's, or a worker's weekly budget) never tripped on a model with no known price, because every call counted as zero. The run is now refused before anything is sent, with the way out; models on this computer, which cost nothing, are not affected.

- A document folder's sync cut a file into pieces again whenever its modification time changed, even when its content was the same (a copy, a checkout, a touch). The content hash is compared now, so such a file keeps its pieces and the next sync skips it without reading it.

- The 60-second limit of a model call counted the whole answer, so a long answer that was still arriving was cut and paid for again. It is now the time **without progress**: an answer that keeps arriving is never cut, one that goes quiet is, and a call ends after ten minutes whatever it does ([runtime](docs/runtime.md#limits)).

- A run whose lease was taken by another engine kept calling models and tools beside it; it stops now, writes nothing more and leaves the execution to the engine that holds the lease. Every commit a run makes is checked against the lease.

- **Security:** when a changed file was indexed again, its old pieces stayed in the saved context of the runs that had read them, so removing the folder later left an old version of the text behind. They are scrubbed now, as a removed file's are.

- **Security:** the past work of an agent that has memory of its own was recalled by every other agent and by plain runs in the same project. It now belongs to its agent ([memory](docs/memory.md#agent-memory)); records already stored are assigned to their agents when the database is opened. Database schema 11.
- **Security:** the record of a finished run kept the first arguments of each tool as plain text, so a value under a key like `password` or `authorization` was stored in past work and recalled into later runs. Values under secret keys are now replaced before the text is made. Records made before this release are written again from their runs when the database is opened (schema 12).
- **Security:** text redaction now also replaces the value of `password=…`, `"client_secret": "…"` and `Authorization: Basic …` pairs, PEM private key blocks, and Google, Stripe, GitLab and other GitHub token shapes. The history of a project, the history of the organisation and the reports of a manager were stored without any redaction; they pass through it now.

## [0.10.0] - 2026-11-24

The quality foundation: the tools every audit uses, and seven security fixes. See the [roadmap](docs/roadmap.md#-phase-10--quality-foundation--v0100).

### Changed

- The [roadmap](docs/roadmap.md) now puts quality and completion phases (10 to 15) before the Command Center (16) and the
  Marketplace (17). Documentation that described shipped features as missing was corrected.

### Fixed

- Schedules and the workforce's week no longer use the unmaintained `chrono` and `chrono-tz` crates, nor `croner`: they run on `jiff` with an evaluator of our own for cron (ADR-0041), which gives the same fires as before for every documented form (checked against 2,300 saved answers). **What changed:**
  a cron expression with `L`, `W`, `#` or `?`, or with an empty item in a list (`0 0 ,1 * *`), was accepted by the old library and is now refused with a message. Nothing in ZORIX used them.
- The lists of a conversation's runs, an agent's runs and the runs in one state are 20 to 200 times faster on a large history: each has an index that ends in the order the list is shown in. Database schema 10.
- **Security:** the data wrapper's defusing now sees a closing tag however many slashes and spaces sit between the `<` and its name (`<//data>` was left live). Found by the new fuzz target.
- **Security:** `.ZORIX` (and any other capital letters) was not recognised as the reserved `.zorix` folder, which is the same folder on macOS and Windows; the file tools now refuse it.
  A server's description of an MCP tool is cut at 4,000 characters, and the evaluation judge now defuses its `rubric`, `task` and `answer` wrappers in any case.
- A full disk is now said in words ("The disk is full, so ZORIX could not save…") with what to do, in every store. The run that was interrupted continues with `zorix recover` once there is room.
- A model that **declines** to answer, or whose answer **ran out of room**, no longer ends the execution as a success: a refusal fails it with the model's own words, and a cut-off answer
  is sent back once and then fails. Text that a cut stream had already written is forgotten before the retry, so a reader never sees it twice.
- **Security:** a webhook delivery that carries the sender's delivery id (`X-GitHub-Delivery` and the like) starts a workflow once: a retry or a replayed request answers `200` and
  starts nothing. Database schema 9.
- **Security:** the wrappers around data (tool results, documents, past work, task and trigger text) now defuse every tag of ZORIX in any case and spacing, opening
  as well as closing, and keep attribute values inside their attribute. Before, only the exact lower-case closing tag was defused. One implementation
  serves all four wrappers, and property tests check it against arbitrary hostile text.
- **Security:** the web tools now also refuse IPv6 addresses that tunnel to a private IPv4 address or to a network of their own (6to4, Teredo,
  ISATAP, IPv4-compatible and the local NAT64 prefix). They could pass the check as ordinary public addresses while a translator or tunnel
  on the network sent them to a private IPv4 address.
- **Security:** an approval answer must name the call it is for. Before, an answer without the call's hash was accepted, so it could have been
  applied to a call the person never saw. A test now edits the stored call and approval and shows that an altered call asks again.
- **Security:** what a webhook, a workflow step, a request's input or a task says now reaches the model wrapped as untrusted data. Before, a
  prompt template put those values in as if the workflow's author had written them, so a webhook body could give instructions.
- Dependencies with known vulnerabilities were updated: `wasmtime-wasi` (a WASI filesystem sandbox escape on Windows),
  `url` (a stack overflow on nested `blob:` URLs) and `cookie` (a repeated nonce after a fork).
- The documented minimum Rust version is 1.97, which the dependencies already required.

### Added

- A workflow that installs each installer silently on a fresh Linux, macOS and Windows runner, starts the installed app, uninstalls and checks nothing is left; and a per-release checklist for what only a person can see ([distribution](docs/distribution.md)).
- End-to-end tests of the built app: a WebDriver client of our own and five scenarios, run on Linux in CI; macOS and Windows start the app and wait for it to load. `ZORIX_SMOKE=1` makes an app say it has loaded and close ([testing](docs/testing.md#end-to-end-tests)).
- Errors have stable codes: a failed command in the app tells the interface a code and its values, so the sentence can be written in the person's language. The command line prints what it always did.
- **The interface's text is in one catalogue** ([ADR-0040](docs/adr/0040-interface-text-and-error-reasons.md), [guide](docs/desktop.md#text-and-languages)): about 1,900 sentences moved out of the components, with plural forms,
  a pseudo-locale (`en-XA`) that shows text that does not fit, `Intl` formatters for money, numbers, dates and lists, and a lint (`tools/check-text.mjs`) that fails on a sentence written in a component. The window's language is announced to the browser.
  The English the app shows is the same, except that numbers have the grouping of their language (token counts were written with French spaces, and are `12,345` now) and money has thousands separators.
- A coverage report per crate in CI, with the first numbers and what they show ([testing](docs/testing.md#coverage)).
- `zorix-bench`: a data folder of realistic size (10,000 runs, a million events, 50,000 pieces of documents) built through the real stores and timed, with a tenth of it checked in CI against limits ([testing](docs/testing.md#benchmarks)).
- Fuzzing: `fuzz/` has 14 targets for every reader of text or bytes from outside, with a seed corpus, and a weekly workflow runs them ([testing](docs/testing.md#fuzzing)).
- CI audits the dependencies (`cargo deny`, `npm audit`) and runs CodeQL.
- The [engineering guidelines](docs/engineering-guidelines.md) and `AGENTS.md`: the rules every change follows, with where
  ZORIX enforces each and the gaps that are still open. The pull request template carries their checklist.

## [0.9.0] - 2026-11-22

**Workforce**: your agents in an organisation, with levels of autonomy and weekly budgets ([guide](docs/workforce.md)).

### Added
- Workers: an agent with a role, a department, someone to answer to, a level, a weekly budget, a time for each task and what always asks. Every chain ends at you, and a worker cannot be freer or
  spend more than the one it reports to ([ADR-0038](docs/adr/0038-workers-are-records-over-agents.md)). Database schema 8.
- Autonomy decided by the policy: Suggest records a proposal instead of running a call, Ask asks for every change, and Notify, Budgeted and Full let a call that only the default would hold run, recorded with
  its arguments. A tool a person must approve, an explicit rule and the worker's own list always ask. Budgets narrow the run, and a spent week starts it at Ask
  ([ADR-0039](docs/adr/0039-autonomy-is-decided-by-the-policy.md)).
- A virtual manager plans the week and writes the report with the new `org.team` tool, on demand or on a schedule that is off until you turn it on. Plans, reports, proposals, unattended calls and budget
  warnings reach a new section of the Inbox.
- Figures from real data: open tasks, the share done on time (a task now remembers the day it went to Done), the latest eval, spend this week without counting anything twice, and what waits for a person.
- `zorix org` and `zorix worker`; the workforce ticker runs in the app and under `zorix serve`; `tools/live-tests.sh workforce`: 28 checks through the real command line.
- The Workforce screens: the org chart, a list, the activity, the worker panel and page, *Hire a worker* and *Org settings*; kit `OrgChart`, `WorkerCard`, `AutonomyDial`, `MetricTile` and `LogStream`; a
  status dot on avatars. The design system gained these screens (logged in `design/local-additions.md`).

## [0.8.0] - 2026-11-21

**Projects**: tasks for you and your agents, on a board, a list and a timeline ([guide](docs/projects.md)).

### Added
- Projects and tasks, with keys (`OPS-38`), four fixed lanes, priority, who has them, progress, dates, dependencies, links, deliverables and a history. A task given to an agent or a team starts an
  ordinary run when it moves to *In progress*; a finished run sends it to *Review* with the answer as a deliverable, a failed run says so, and a task waits for the tasks it depends on
  ([ADR-0036](docs/adr/0036-projects-and-tasks-are-records-that-create-runs.md)). Database schema 7.
- A manager: an agent that plans a project through the tools `tasks.list`, `tasks.get`, `tasks.create`, `tasks.update`, `tasks.move` and `tasks.comment`. Every write asks, a move asks a person who is
  looking, and the grant is one project. A task's own run can report progress and add deliverables with `task.*`, on that task only
  ([ADR-0037](docs/adr/0037-project-tools-and-the-manager.md)).
- `zorix project` and `zorix task`; the task ticker runs in the app and under `zorix serve`; `tools/live-tests.sh projects`: 17 checks through the real command line.
- The Projects screens: the index, the board (drag with the pointer or the keyboard), the list, the timeline with dependency arrows, deliverables, history and the task drawer; kit `TaskCard`,
  `TaskBoard`, `Timeline`, `AvatarStack` and a round avatar for people; tasks that need you in the Inbox; tasks in ⌘K search; *From task* on a run. The design system gained these screens and the
  timeline arrows (logged in `design/local-additions.md`).

### Fixed
- A write that read before it wrote could fail with "database is locked" when the engine wrote at the same moment; project writes now wait for each other.

## [0.7.0] - 2026-11-21

**Workflows**: triggers, steps and people joined into a graph that runs by itself and stops where a person must decide ([guide](docs/workflows.md)).

### Added
- Workflows are versioned graphs (a draft and published versions, a hash, rollback as a new version, and a readable diff) that create ordinary runs through the hub
  ([ADR-0033](docs/adr/0033-workflows-are-versioned-graphs-that-create-executions.md)). Steps: agent, team, condition, loop, wait, approval, question, message (Slack, Discord) and HTTP request.
  Values pass with `{{…}}` templates that fail loudly, and conditions use a small safe expression language. Database schema 6.
- Triggers: schedules (every N minutes, at times on chosen days, or cron, in a time zone), webhooks (a secret address, an optional signature, 1 MB cap, off by default), agent and team runs that
  ended, and another workflow ending, with a chain cut at 4. They run in the app while it is open or under the new **`zorix serve`**; a schedule missed while nothing ran is recorded, not
  replayed ([ADR-0034](docs/adr/0034-the-workflow-host-and-triggers.md)).
- People: approvals and questions are waits bound to a fingerprint of what was shown, with time-outs and an optional Slack reminder. Actions ask first; a step that runs without asking is confirmed
  when publishing and the version records it. An action interrupted by a stop asks instead of repeating ([ADR-0035](docs/adr/0035-people-steps-and-unattended-actions.md)).
- `zorix workflow` (list, show, create, edit, import, export, versions, diff, publish, rollback, enable, run, runs, answer, webhook, templates…), `.workflow.json` files, and three starters.
- The Workflows screens (list, editor with a canvas, run view), kit `TriggerChip`, `WorkflowNode` and `WorkflowCanvas`, workflow waits in the Inbox, *Run as › Workflow* and *Save as workflow* in New run,
  and Settings › General for webhooks. The design system gained the *Workflow run* screen (logged in `design/local-additions.md`).
- `tools/live-tests.sh workflows`: 17 checks through the real command line, including a signed webhook against `zorix serve`.

## [0.6.1] - 2026-11-21

**Design system v3**: the app follows the design system strictly, and every later phase will.

### Added
- The design system is vendored in `design/` (tokens, a specification for each component, the surfaces of the product and 17 assembled screens as images) and is the contract
  for every screen ([ADR-0032](docs/adr/0032-design-system-v3-is-the-contract.md)).
- `tools/design-tokens.mjs` writes the tokens of `app.css` from `design/tokens.json` (`--check` in the gates and CI), and `tools/check-design.mjs` fails on colour literals,
  arbitrary values for colour, type, radius, shadow and duration, transitions over 200 ms and emoji in the interface. New tokens: chart series, workflow node kinds and task
  priorities, plus twenty icons for the phases to come.
- Kit: `BrandMark` and `HealthGrid`; every other component corrected to its specification (Tabs underlined, Badge, Chip, Switch, Segmented, Card, Callout, Stat, Stepper, Kbd, Avatar,
  EmptyState and the primary button's surface).
- An **Inbox** with everything waiting for a person (tool approvals anywhere in a tree of runs, and servers whose tools or sign-in need a review) and an approvals bell that opens it.
- A **Runs** page, **Documents** and **Settings** (General, Appearance, About) as screens of their own, ⌘N for a new run, and spend today and runs in progress at the foot of the sidebar.
- Run screen: the stat row, tabs for Work tree, Steps, Events, Context and Cost, and a *This run* panel. New run: *Run as* plain, agent or team, chips that mark what asks, where the
  conversation goes, and *Before you start*. Providers: a models table. Memory: *To approve*. Integrations: failures and an attention filter. Agent chat: a rail with the team and the cost.

### Changed
- The sidebar follows the design system's groups (Work, Organisation, Connect, Operate) and lists only what has shipped. Documents moved out of Memory, and Teams out of the Agents switch.
- Attention counts and the *asks* mark pass WCAG AA contrast; "Review" names what it opens. [Known differences](docs/design-system.md#known-differences) from the design system's screens are listed with their reasons.
- The roadmap details phases 7 to 11 from the design system's screens.

## [0.6.0] - 2026-11-20

**Multi-Agent Orchestration**: agents that ask other agents, without ever being allowed more than the one who asked.

### Added
- **Delegation** with the `agents.delegate` tool: one to eight tasks per call, run at the same time, up to three levels deep and 32 runs below a root. Each task can carry
  structured input, an expected output (a JSON Schema), notes and workspace patterns. The parent receives, per task, the answer, the structured result, steps, tool calls, cost
  and the child's id. A child that failed or was cancelled is **reported**, not raised.
- A new lifecycle state, **`waiting_for_children`** (eleven states, fifteen triggers): a parent that waits holds no worker, so nested delegation cannot deadlock. Recovery reconciles
  parents whose children finished while ZORIX was stopped; a call is never delegated twice.
- Rules that only **shrink** down the tree: a child's grants are its agent's grants intersected with the parent's; the parent's remaining cost, tokens and tool calls are split
  among the children and rolled up when they finish; loops, archived agents and models the parent may not use are refused with a reason.
- **Approvals rise to the root**: a sub-run's approval keeps the whole tree waiting and is answered from the run you started (`zorix approve ROOT`, the app's *Waiting in the tree*).
  **Cancelling a root cancels its tree.**
- **Teams**: a lead and the agents it may ask, run with `zorix run --team`, from the app, or over MCP. The lead's instructions get an `<available_agents>` index. A ready-made
  **research desk** (`research-lead`, `researcher`, `writer`) installs only what is missing.
- Evaluations: a `delegated` check, and the whole tree refuses approvals during an evaluation.
- **Command line**: `zorix team …`, `run --team`, `tree`, `list` of root runs (`--all` for every run), `show` with sub-runs, a follow view that merges the streams of sub-runs.
- **MCP**: `team` on `zorix.run_agent`, teams in `zorix.agents`, sub-runs in `zorix.status`.
- **Desktop**: delegation cards in the chat with live text and inline approvals, a *Work tree* card and *Waiting in the tree* approvals in the run view, the Teams gallery, team page and
  editor, teams in *Run as* and ⌘K, and a sub-run count in the run list. Design system: TreeView, DelegationCard and Sparkbar.
- Database schema 5 (`parent` and `root` on executions, with indexes, and a `teams` table). A 0.5 database upgrades in one transaction.
- `tools/live-tests.sh orchestration`, and the guides [orchestration](docs/orchestration.md) and [teams](docs/teams.md), with ADRs 0029 to 0031.

### Fixed
- The in-memory repository now honours the agent filter like the SQLite one.
- An agent's versions list is shown newest first in the command line and the app.

## [0.5.0] - 2026-11-19

The **AI Agent Engine**: reusable agents that you can version, evaluate and trust to change.

### Added
- **Agents**: a name and face, instructions, skills, tools and access, memory of their own, the models they may use and limits. Stored in the database, **versioned**
  (a new version only when the content changed; rollback writes a new version) and exported and imported as one deterministic Markdown file. A run made by an
  agent records the agent, version and hash, and can only **narrow** the agent's grants, limits and memory.
- **Skills** in the open **Agent Skills** format (`SKILL.md` plus files), content-addressed in a library, pinned by hash, loaded on demand with `skill.load`,
  `skill.read` and `skill.run`; scripts run only in the WebAssembly sandbox with the skill mounted read-only. Imports from a folder or a zip are bounded and atomic.
- An **agent memory scope**: facts only one agent sees, approved by a person like every memory.
- A **model allow-list** per agent, enforced by the router (a fallback can never leave it) and again by the runtime.
- **Live text**: answers appear as they are written (Anthropic, OpenAI chat and responses, Gemini, Ollama), through a `TextSink` and `ExecutionEngine::live()`;
  never stored.
- **Evaluations**: deterministic checks (text, regex, JSON Schema, tools, steps, cost), an optional judge with a rubric, runs per version and model, comparison
  (fixed, broke), and a safe way to run them (no writes, no approvals, not recorded as past work).
- Four **starter agents** (researcher, code reviewer, data analyst, writer), each with a skill and an evaluation suite.
- **Agents over MCP**: `zorix.agents`, and `zorix.run_agent` with `--allow-runs`, limited to what the server's flags allow (new `--allow-access`).
- **Command line**: `zorix agent …`, `zorix skill …`, `zorix eval …`, `zorix run --agent`, `zorix list --agent`, `zorix memory --scope agent:<name>`.
- **Desktop**: the Agents gallery and agent page (chat with live text, tool cards and inline approvals; definition; skills; versions with a diff and rollback;
  evals with a score matrix; memory), a *Run as* picker in New run, an *agent · vN* chip in the run view, and an Agent filter in Memory.
- **Design system**: Avatar, Markdown (safe, never raw HTML), ChatBubble, Composer, ToolCallCard, ScoreMatrix and VersionTimeline.
- Database schema 4 (agents, versions, skills, evaluations, and an agent column on executions).

### Changed
- `ModelGateway` gained `generate_live`, and the engine contract `live()`, both with defaults, so existing gateways keep working.
- `zorix mcp serve` always offers `zorix.agents`.

### Security
- A skill's `allowed-tools` is shown and never granted; skill scripts have no network and no workspace.
- An agent file or skill cannot widen what a person allowed: grants only narrow, and an import whose pinned skill is missing is refused.
- Markdown from a model is rendered as a tree, never as HTML; only http, https and mailto links open.

## [0.4.0] - 2026-11-19

The **Tool & Integration Engine**: ZORIX reaches the outside world, and you decide how far.

### Added
- **MCP client** for both protocol eras (the 2026-07-28 stateless revision and the handshake revisions up to 2025-11-25), over
  stdio and Streamable HTTP, with OAuth 2.1 (PKCE, protected-resource and authorization-server metadata, dynamic client
  registration, refresh) and tokens in the keychain. Tool definitions are **pinned**; a changed description withdraws the tool
  until you review it; runs keep the definitions they started with. Servers are off until confirmed, every MCP tool asks first,
  sampling, elicitation and roots are refused. A reviewed catalogue, import from Claude Desktop, VS Code and Cursor.
- **Native integrations**: `http.get`/`http.request` (SSRF-guarded), signed outbound `webhook.send`, GitHub, GitLab, Jira, Slack,
  Discord, SQLite (read-only) and Postgres (verified TLS, read-only transactions), behind grants per resource; connections with
  tests and secrets in the keychain.
- **Code sandbox**: JavaScript (QuickJS) and Python (CPython for WASI, downloaded and checksum-verified) in WebAssembly, with no
  network and limits on memory, time, output and scratch files; the workspace is mounted read-only only when granted.
- **`zorix mcp serve`**: ZORIX's memory, documents and past work over MCP on standard input and output, read-only and lock-free
  by default; `--allow-runs` adds `zorix.run`, `zorix.status` and `zorix.result` (approvals stay in ZORIX).
- **Command line**: `zorix mcp …`, `zorix connect …`, `zorix sandbox …`, and `--allow-http`, `--allow-repo`, `--allow-jira`,
  `--allow-chat`, `--allow-db`, `--allow-code` on `run`. `zorix tools` shows every tool and where it comes from.
- **Desktop**: the Integrations screens (connected servers with live state, catalogue, connections, sandbox), a server drawer to
  review tools and changes, guided add/connect/import flows, a servers-and-services picker in New run, approval cards for MCP
  tools, web requests, repositories and tickets, messages, SQL and code, and a command palette (⌘K).
- **Design system v2**: depth tokens, glass, motion, and the Drawer, CommandPalette, BrandLogo, StatusPill, CodeBlock, DiffView,
  Stepper, KeyValue and ToggleRow components. Brand marks are CC0 Simple Icons ([NOTICE](NOTICE.md)).
- Database schema 3 (servers, tool pins, connections); events carry the tool's source, and approvals the pinned definition hash.
- `tools/live-tests.sh integrations`: the real command line against real servers.

### Changed
- `zorix tools` lists more than the built-in tools; commands that change integrations lock the data folder like the other
  writing commands.
- The tool registry can change while ZORIX runs; an execution pins the tools it was offered.

### Security
- A Postgres connection that asks for verified TLS no longer falls back to plain text when the server does not offer it.
- See SECURITY.md for the model of MCP servers, integrations and the sandbox.

### Known limits
- A stdio MCP server runs as you without a sandbox. Postgres is tested against a real server in CI, not locally. Legacy HTTP
  MCP servers do not push tool-list changes live (they are seen at start or on refresh). HTTP+SSE (2024-11-05) and MCP client
  ID metadata documents are not supported; Slack and GitHub OAuth need a pre-registered client id (the catalogue uses a token for
  GitHub). Python is the 3.12 build of December 2023. MCP messages over SSE are limited to 1 MB. The catalogue has eleven entries.

## [0.3.0] - 2026-11-17

The **Context & Memory Engine**: ZORIX remembers, and you decide what.

### Added

- **Memory** in four kinds: the **conversation** (earlier runs of the same conversation), **facts and preferences**, **documents**
  and **past work**, each with its own share of the context budget. Scopes: global, project, conversation. See [memory](docs/memory.md).
- **A model proposes, a person approves.** `memory.remember` can never be approved automatically (the engine refuses it, so
  `--approve yes` leaves the run waiting); `memory.search`, `knowledge.search` and `history.search` are read-only.
- **Hybrid search**: SQLite FTS5 (BM25) always, plus vectors when an embeddings model is chosen (Ollama, OpenAI, Gemini), fused by
  reciprocal rank fusion with a relevance gate so unrelated questions find nothing.
- **Frozen recall per run**: what was retrieved is saved and every step and resume sends the same text; the run view and
  `zorix show --context` show the facts, exchanges and document pieces used and what each part cost.
- **Documents**: folders of text, code, HTML and PDF (text layer; no OCR), respecting `.gitignore`, skipping private files,
  cut into pieces with citations, synced on demand with progress, cancel and resume. See [documents](docs/knowledge.md).
- **Token calibration**: real usage from providers teaches a factor per model, replacing characters ÷ 4 (`zorix calibration`).
- **Command line**: `memory`, `knowledge`, `embeddings`, `calibration`, `show --context`, and `run --no-memory --no-knowledge
  --no-conversation`; `--conversation` now carries earlier runs forward.
- **Desktop app**: a Memory screen (facts, documents, past work), a layout for approving a proposal, memory switches and Continue,
  a Context panel in the run view and an Embeddings card in Providers & keys.
- **Design system** for the desktop app: tokens, Light (default), Dark and System themes, bundled Inter and JetBrains Mono, a typed
  component kit, an app shell with a custom toolbar, and a Design system screen. See [design system](docs/design-system.md).
- Research, five ADRs (0013 to 0017) and opt-in live tests (`tools/live-tests.sh embeddings`).

### Changed

- The context builder keeps the start of the prompt stable: when any part is dropped while provider-native turns are being
  replayed, the history is treated as edited (fixes a latent cache and signature problem).
- Database schema 2 (opening a 0.2 database upgrades it). The demo model picks the task, not memory messages.

### Security

- Memory never enters without approval; retrieved text is untrusted data; a cloud embeddings model needs an explicit opt-in and
  receives your text; deleting a fact or folder removes it from every table and saved context (not from transcripts or providers).

## [0.2.0] - 2026-11-15

The **AI Provider Engine**: real models behind the same runtime.

### Added

- **Providers**: Anthropic (Messages API), OpenAI (Responses API), OpenAI-compatible servers (Chat Completions: OpenRouter,
  Groq, vLLM, LM Studio) and Google Gemini, all over raw HTTPS, plus **Ollama** for local models (native API, real
  context length and capabilities discovered).
- **Credentials** in the system keychain (or environment variables); `zorix provider set-key` reads from a hidden prompt.
  Keys never reach files, the database, events, logs or the interface.
- **Routing**: `provider/model`, aliases and `route:<name>` from `providers.toml`, ordered candidates with capability and
  context filters, **fallback** on provider failures with cooldowns, route phases (start, continue, retry) and `--local-only`.
- **Catalogue** with context windows, limits, capabilities and prices (as of 2026-11-15), overridable in `providers.toml`.
- **Runtime awareness**: the context budget comes from the model's window; usage counts cache and reasoning tokens; cost in
  millionths of a dollar when the price is known; `execution.model.fallback` events; provider and model per step.
- **Native turn replay** so providers that bind reasoning to the conversation (Anthropic thinking, Gemini thought
  signatures, OpenAI reasoning items) keep working across tool calls.
- **Command line**: `--model`, `--local-only`, `models [--available]`, `provider list|set-key|remove-key|test`, `route list`.
- **Desktop app**: a model picker, a Providers & keys screen, and the model and provider of every step in the run view.
- **Tests**: mock-server suites for every adapter (no network in CI) and opt-in live tests (`tools/live-tests.sh`).
- Documentation: [providers guide](docs/providers.md), research and four new ADRs (0009 to 0012).

### Changed

- `--model` accepts any model reference, not only `demo` and `script:<file>`. With no `--model`, a run uses the `default`
  alias, or Anthropic when it has a key, and the demo model otherwise, so 0.1 commands behave as before.
- Secrets and redaction moved to `zorix-core` (still re-exported by `zorix-runtime`).

### Known limits

- **Data leaves the computer** when a cloud provider is used; `--local-only` forbids it.
- Answers arrive complete; text is not shown live yet.
- Verified against a real server only for Ollama (qwen3:1.7b, including a forced fallback and a whole tool loop). The
  Anthropic, OpenAI and Gemini adapters are verified against mock servers built from their documentation, and their live
  tests run when you store a key.
- Installers are still **not signed or notarized**.

## [0.1.0] - 2026-11-15

The first release: the **Harness Runtime**, a command line and a minimal desktop app. No real model provider yet; the
runtime is driven by simulated models.

### Added

- **Executions** with a strict, validated lifecycle (`created`, `queued`, `running`, `waiting_for_model`,
  `waiting_for_tool`, `waiting_for_approval`, `paused`, `completed`, `failed`, `cancelled`).
- **The execution loop**: context building from ordered, budgeted sources; model calls behind a provider-neutral
  `ModelGateway`; retries with backoff; timeouts; limits on iterations, tool calls, tokens, cost and working time;
  cancel, pause and resume; validation of final answers, including structured output against a JSON Schema.
- **Tools**: an MCP-shaped contract, a registry, and an executor with schema validation, timeouts and output caps.
- **Permissions and policy**: deny-by-default grants, a policy engine evaluated when a call runs, and **human approval**
  that suspends an execution and is bound to the exact call it names.
- **Prompt-injection containment**: tool output is labelled untrusted and cannot change grants or policy.
- **Events**: an ordered, versioned, persisted stream that can be replayed and followed live without gaps.
- **Persistence and recovery** on SQLite: atomic commits, checkpoints, leases, idempotent creation, and crash recovery
  that never repeats a finished tool and asks a person before repeating one that is not safe to repeat.
- **Observability**: trace spans named after the OpenTelemetry GenAI conventions, secret redaction, and an execution
  summary with step timings and usage.
- **Simulated models**: scripted, failure-injecting and a rule-based demo model.
- **Built-in demo tools**: clock, calculator, text statistics, a per-tenant notebook, and file tools confined to a workspace.
- **`zorix` command line**: `run`, `list`, `show`, `events`, `approve`, `deny`, `cancel`, `resume`, `recover`, `tools`,
  `data-dir`, with JSON output and exit codes.
- **Desktop app** (Tauri 2, Svelte 5): runs list, new run form, live run view with an approval card, pause, resume and cancel.
- CI on Linux, macOS and Windows, and a release workflow for installers, command line archives and checksums.
- Documentation, eight architecture decision records, and a comparison of the agent runtimes studied.

### Known limits

- Installers are **not signed or notarized**, and there are no automatic updates yet.
- File tools confine paths; there is no sandbox for tools that run code, and no such tool exists yet.
- Token counts are estimates until real providers arrive.
- One program at a time per data folder; no accounts or shared store.
