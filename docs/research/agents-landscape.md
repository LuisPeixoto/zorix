# Agents, skills and evaluations: what exists and what ZORIX takes

Research for Phase 5 (agents). Checked on 2026-11-19. Items marked *verified* were read from the source that day; the rest is
from public documentation as understood, and is marked so where it matters.

## The Agent Skills format (verified)

Source: the specification at agentskills.io/specification. A skill is a directory with a required `SKILL.md` and optional
`scripts/`, `references/`, `assets/` and any other files. `SKILL.md` is YAML frontmatter followed by Markdown.

| Field | Rule |
|---|---|
| `name` (required) | 1 to 64 characters; lowercase alphanumerics and hyphens (the text says *unicode* lowercase alphanumerics); must not start or end with a hyphen; **no consecutive hyphens**; **must match the parent directory name** |
| `description` (required) | 1 to 1024 characters; says what the skill does **and when to use it** |
| `license` | Short: a name or a bundled file |
| `compatibility` | 1 to 500 characters; environment needs (products, system packages, network) |
| `metadata` | A map of string keys to string values |
| `allowed-tools` | A **space-separated string** of pre-approved tools; marked *experimental* |

Progressive disclosure is part of the format: about 100 tokens of metadata per skill at start, the body (under 5,000 tokens,
under 500 lines recommended) when the skill is activated, and other files only when needed. File references are relative to
the skill root and one level deep. There is a reference validator, `skills-ref validate`.

What ZORIX takes: all of it. It is stricter in two places: names are ASCII only (so they are safe in tool arguments, paths and
hashes), and `allowed-tools` is shown to the person but never turned into a grant (ZORIX's grants are the only permission).

What the format leaves open, and ZORIX decides: how a script is run. Published skills often assume a shell and `pip`. ZORIX runs
scripts only in its WebAssembly sandbox (no network, no packages beyond the standard library), says so when a script cannot
run, and never runs one anywhere else.

## How others model an agent

From public documentation, not re-verified today.

| System | The agent is… | Notes for ZORIX |
|---|---|---|
| OpenAI Assistants and GPTs | A named configuration: instructions, model, tools, files; versions were not first-class | The picker and the card gallery are familiar; ZORIX adds immutable versions and per-run version recording |
| Claude Code subagents | A Markdown file with YAML frontmatter (name, description, tools, model) and the prompt as the body | The same file shape ZORIX uses for export and import, so agents are reviewable in git |
| Claude skills | The Agent Skills format above | Adopted as is |
| CrewAI | A role, a goal and a backstory plus tools; a crew coordinates agents | Role-play fields add little; instructions cover them. Coordination is Phase 6 |
| LangGraph | A graph of nodes, with state and checkpoints | ZORIX already checkpoints; graphs belong to workflows (Phase 7) |
| Google ADK | Agent classes in code, with sub-agents | Code-defined agents do not fit a person editing in an app |

The common core is small: **identity, instructions, tools, model, and optionally files or skills**. What is rare, and what
ZORIX adds because it is a harness: grants and limits that bound the agent, a model allow-list that fallbacks cannot escape,
memory the agent owns but a person approves, immutable versions, and evaluations.

## Evaluating agents

Public tools such as promptfoo and OpenAI's evals describe a case as an input plus assertions: contains, regex, JSON
schema, similarity, a model-graded rubric, cost and latency. Two lessons carry over. Deterministic assertions are cheap,
repeatable and enough for most cases; a model judge is useful for style and relevance but must be optional, explicit about its
model and cost, and treated as untrusted-input-consuming (the answer it grades can contain instructions). ZORIX adds checks
only a harness can make: which **tools were used or forbidden**, steps and tool calls, and that **approvals are refused
automatically** while an evaluation runs.

## The YAML parser (verified on crates.io)

| Crate | Latest | Updated | Notes |
|---|---|---|---|
| `serde_yaml` | 0.9.34+deprecated | 2024-03 | Archived and deprecated: not used |
| `serde_yaml_ng` | 0.10.0 | 2024-05 | A fork; quiet for a year |
| `yaml-rust2` | 0.13.0 | 2026-09 | Maintained; a pure parser with an event API |
| `saphyr` | 0.3.4 | 2026-11 | The newer line of the same family; younger, fewer users |

Choice: **`yaml-rust2`** through its event interface. ZORIX reads only a strict subset (scalars, lists, maps, folded and
literal blocks) into `serde_json::Value`, and **refuses** anchors and aliases (they allow expansion attacks), documents larger
than 16 KB and nesting deeper than 4. If `saphyr` matures it is a drop-in change behind the same function.

## What REA showed in Claude.app

Strings of the shipped app (patterns only; no code or assets were taken):

- Adding a skill asks first: *"Upload this skill to Claude?"*, and says that **the files are sent** to a named host and saved
  to the person's skills.
- A skill file has a size cap (30 MB there), with specific errors: *file too large*, *file not found*, *could not open*.
- Things brought in from elsewhere (plugins, saved skills, scheduled tasks) arrive **turned off**, and a toast says where to
  find them.
- Skills and plugins are managed on one screen ("Customize › Skills") separate from chat.

What ZORIX takes: confirm before import, say what will be stored and where, specific errors, a smaller cap (10 MB), and
imported agents and skills grant nothing until a person attaches them to an agent.

## What ZORIX decides

| Question | Decision | ADR |
|---|---|---|
| Where is an agent stored | In SQLite, with immutable versions; Markdown file for export and import | [0024](../adr/0024-agents-and-versions.md) |
| How a run knows its agent | A typed binding frozen on the execution, like the tool pins | 0024 |
| Skills | Agent Skills, content-addressed, loaded on demand; scripts only in the sandbox | [0025](../adr/0025-skills.md) |
| Memory | A new `agent` scope; people still approve everything | [0026](../adr/0026-agent-memory-scope.md) |
| Which models an agent may use | An allow-list enforced inside the router; live text deltas | [0027](../adr/0027-model-allow-list-and-live-text.md) |
| Evaluations | Deterministic checks plus an optional judge; approvals refused | [0028](../adr/0028-evals.md) |

## Open items for the implementation branches

- Whether WASI Python can run `runpy.run_path` on a file in a read-only preopen at `/skill` (branch `feat/skills` tests it first).
- Provider streaming: each adapter aggregates its stream internally; the text-delta hook must not change the aggregated result
  (branch `feat/live-text`).
