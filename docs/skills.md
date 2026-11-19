# Skills

A **skill** is a folder that teaches an [agent](agents.md) how to do one kind of task: a `SKILL.md` with a name, a description and
instructions, plus any files it refers to (references, templates, scripts). ZORIX reads the open **Agent Skills** format, so a
skill written for another tool imports as it is. The design is in ADR [0025](adr/0025-skills.md).

```
citing-sources/
├── SKILL.md            # required: front matter (name, description) and the instructions
└── references/
    └── checklist.md
```

```markdown
---
name: citing-sources
description: "How to cite sources in an answer. Use whenever an answer states facts that came from a page or a paper."
license: MIT
---

# Citing sources
…
```

## What the model sees

Only each pinned skill's **name and description**, as an index in the instructions. When the task fits one, the model calls
**`skill.load`** and gets the full instructions and the list of files; **`skill.read`** returns one text file (up to 256 KB);
**`skill.run`** runs a `scripts/*.py` or `scripts/*.js`. The skill tools are read-only, and see **only the skills pinned to that run**,
so an agent cannot reach a skill it was not given.

## Scripts run in the sandbox, and only there

`skill.run` uses the [code sandbox](sandbox.md): WebAssembly, no network, limits on memory and time, the skill's own folder mounted
read-only at `/skill`, **the standard library only**. It needs `code:python` or `code:javascript` in the agent's access. A skill that
needs a package from PyPI cannot run its scripts here; the error says so. A skill script cannot see the workspace: to run code on a
workspace file, the agent reads the script with `skill.read` and runs it with `code.run_in_workspace`, or passes text as `stdin`.

## Import

`zorix skill import PATH` takes a folder (whose name must equal the skill's `name`) or a `.zip`. In the app: *Skills › Import a folder*.

| Rule | Limit |
|---|---|
| Files per skill | 200 |
| Total size | 10 MB |
| One file | 2 MB |
| `SKILL.md` | 100 KB |
| Folder depth | 8 |
| Symbolic links, absolute paths, `..` | refused |
| `name` | ASCII, kebab-case, up to 64 characters, equal to the folder name |
| `description` | 1 to 1024 characters |

Junk files (`.DS_Store`, `.git`) are skipped. The import is atomic: a temporary folder, then a rename. The header is read with a
strict YAML subset: no anchors, aliases, tags or several documents, at most 16 KB and 4 levels deep. Keys ZORIX does not know are
noted, not refused. Quote a `description` that contains a colon.

`allowed-tools` in a skill is **shown to you and never granted**: a skill cannot give itself permissions.

## The library

Skills live in `<data>/skills/<sha256>/`, named by the hash of their files (sorted `path`, size and SHA-256 lines), so the same content
is stored once and an agent that **pins** `name@sha256:…` always gets exactly the files it was tested with. Changing a skill makes a
new hash; an agent moves to it when you attach the new one (*Use newer* in the app), which saves a new agent version.
`zorix skill gc` deletes only folders that no agent version and no current skill refers to.

## Ready-made skills

The four templates each come with one: `citing-sources` (researcher), `review-checklist` (code reviewer), `csv-profiling` (data analyst,
with a Python script that profiles a CSV from text or a file) and `plain-style` (writer).
