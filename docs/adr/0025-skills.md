# ADR-0025: Skills follow the Agent Skills format, load on demand, and run only in the sandbox

Status: accepted (Phase 5)

## Context

Agents need reusable know-how that does not bloat every prompt. The open Agent Skills format (a folder with `SKILL.md` and
files) is already used by other products and has a published specification.

## Decision

- ZORIX adopts the format: `name` (1–64, lowercase alphanumerics and single hyphens, equal to the folder name), `description`
  (1–1024), and optional `license`, `compatibility` (≤ 500), `metadata` (string map) and `allowed-tools` (space separated).
  Stricter in one way: names are ASCII.
- Skills are **content-addressed**: `<home>/skills/<sha256>/`, where the hash covers the sorted list of path, size and file
  hash. An agent version pins `name@hash`, so editing a skill never changes an old version.
- The model sees only the **name and description** of the skills of its agent. Three read-only tools do the rest, and each
  checks the skill against the run's pins: `skill.load` (the body and the file list), `skill.read` (a text file ≤ 256 KB) and
  `skill.run` (a script).
- **Scripts run only in the WebAssembly sandbox**, with the skill mounted read-only at `/skill`, and only with the
  `execute code:<language>` grant. There is no shell and no network. A script that needs a package or a network says so.
- `allowed-tools` is shown to the person and **never** becomes a grant.
- Import (folder or zip) is bounded: 200 files, 10 MB, 2 MB per file, `SKILL.md` ≤ 100 KB; no symlinks, absolute paths or `..`;
  atomic (temporary folder, then rename). The frontmatter parser reads a strict YAML subset: no anchors or aliases, ≤ 16 KB,
  nesting ≤ 4. Importing grants nothing.

## Consequences

- Many published skills assume `pip` or a shell and will load but not run their scripts. The error says why.
- Skill folders accumulate; `zorix skill gc` removes only hashes nothing references.
