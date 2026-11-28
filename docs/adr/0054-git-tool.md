# ADR-0054: A git tool that uses the system's git, made deaf to the repository

Status: accepted (Phase 12).

## Context

Agents that change code need to see what changed, commit it and push it. Git itself will run programs that a repository
configures (hooks, filters, an SSH command, diff drivers), so pointing it at a repository a model or a stranger can write to is
running their code.

## Decision

- Tools `git.status`, `git.diff`, `git.log`, `git.branch`, `git.commit` and `git.push`, run with the person's installed git (so
  their identity, credential helpers and SSH setup apply) on the repository whose **top folder is the workspace**.
- Grants: `read git:<branch>`, `write git:<branch>`, `write git-push:<remote>/<branch>`; flags `--allow-git BRANCH[:write]`
  and `--allow-git-push REMOTE/BRANCH`, access entries `git:` and `git-push:`. Files also need the workspace grants.
- `git.commit` takes only the named files. `git.push` sends **one full commit hash** to a configured remote, and fails if the
  branch is no longer at it, so an approval covers exactly what it showed. There is no force.
- Commits and branch changes ask. A push asks a person who saw it: no automatic mode approves it.

## Security review

- **Nothing the repository says runs.** Hooks, fsmonitor, the pager, submodule recursion and the `ext` transport are turned off
  on the command line, which wins over every file. A repository whose own configuration (included files too) sets a filter, a
  diff or merge driver, an SSH or askpass command, a credential helper, a signing program, `receivepack`/`uploadpack`/`vcs`,
  URL rewrites, protocol rules, `core.worktree` or an include is **not used at all**, with the key named. The same settings in
  your global configuration are yours and apply.
- **Only this repository.** The top folder must be the workspace, and its git folder must be inside it, so a `.git` file cannot
  point at another repository on the computer. Variables that point git elsewhere are removed.
- **Inputs.** Branch and remote names are letters, digits, `/ - _ .`, never starting like an option or a refspec; paths follow
  the workspace rules, are passed after `--` with literal pathspecs, and nothing in `.git` can be committed.
- **Reading** shows only files the run's grants let it read (status and diff say how many were left out). Log messages and
  authors are shown with a branch grant alone.
- **Switching** branches needs the write grant on every file the switch would change.
- **Writing into `.git`** is refused for the file tools too ([audit F34](../audits/phase-12-tools-integrations-sandbox.md)).
- Tried in `crates/zorix-tools/tests/git.rs` with a local bare remote: hooks and hostile configuration do not run, a stale or
  rejected push changes nothing.

## Consequences

- Repositories with local filters (some setups of Git LFS put them there) are not used; LFS installed globally works.
- Worktrees and submodules whose git folder is outside the workspace are not used.
- The commit approval shows the diff at that moment; a program of yours changing the files between approval and commit would
  commit what it changed.
- Hooks do not run, so checks a repository relies on (formatting, secret scanning) are not applied to these commits.
