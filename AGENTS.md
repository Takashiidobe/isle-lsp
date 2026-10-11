# Agent Instructions

This project uses **bd** (beads) for issue tracking. Run `bd prime` for full
workflow context.

> **Architecture in one line:** Issues live in a local Dolt database
> (`.beads/dolt/`); cross-machine sync uses `bd dolt push/pull` (a
> git-compatible protocol), stored under `refs/dolt/data` on your git remote —
> separate from `refs/heads/*` where your code lives. `.beads/issues.jsonl` is a
> passive export, not the wire protocol.
>
> See
> [sync-concepts](https://github.com/gastownhall/beads/blob/main/docs/core-concepts/sync-concepts.md)
> for the one-screen overview and anti-patterns (don't treat JSONL as the source
> of truth; don't `bd import` during normal operation; don't reach for
> third-party Dolt hosting before trying the default).

## Quick Reference

```bash
bd ready              # Find available work
bd show <id>          # View issue details
bd update <id> --claim  # Claim work atomically
bd close <id>         # Complete work
bd dolt push          # Push beads data to remote
```

## Non-Interactive Shell Commands

**ALWAYS use non-interactive flags** with file operations to avoid hanging on
confirmation prompts.

Shell commands like `cp`, `mv`, and `rm` may be aliased to include `-i`
(interactive) mode on some systems, causing the agent to hang indefinitely
waiting for y/n input.

**Use these forms instead:**

```bash
# Force overwrite without prompting
cp -f source dest           # NOT: cp source dest
mv -f source dest           # NOT: mv source dest
rm -f file                  # NOT: rm file

# For recursive operations
rm -rf directory            # NOT: rm -r directory
cp -rf source dest          # NOT: cp -r source dest
```

**Other commands that may prompt:**

- `scp` - use `-o BatchMode=yes` for non-interactive
- `ssh` - use `-o BatchMode=yes` to fail instead of prompting
- `apt-get` - use `-y` flag
- `brew` - use `HOMEBREW_NO_AUTO_UPDATE=1` env var

<!-- BEGIN BEADS INTEGRATION v:1 profile:minimal hash:1105d646 -->

## Beads Issue Tracker

This project uses **bd (beads)** for issue tracking. Run `bd prime` to see full
workflow context and commands.

### Quick Reference

```bash
bd ready              # Find available work
bd show <id>          # View issue details
bd update <id> --claim  # Claim work
bd close <id>         # Complete work
```

### Rules

- Use `bd` for ALL task tracking — do NOT use TodoWrite, TaskCreate, or markdown
  TODO lists
- Run `bd prime` for detailed command reference and session close protocol
- Use `bd remember` for persistent knowledge — do NOT use MEMORY.md files

**Architecture in one line:** issues live in a local Dolt DB; sync uses
`refs/dolt/data` on your git remote; `.beads/issues.jsonl` is a passive export.
See
https://github.com/gastownhall/beads/blob/main/docs/core-concepts/sync-concepts.md
for details and anti-patterns.

## Agent Context Profiles

The managed Beads block is task-tracking guidance, not permission to override
repository, user, or orchestrator instructions.

- **Conservative (default)**: Use `bd` for task tracking. Do not run git
  commits, git pushes, or Dolt remote sync unless explicitly asked. At handoff,
  report changed files, validation, and suggested next commands.
- **Minimal**: Keep tool instruction files as pointers to `bd prime`; use the
  same conservative git policy unless active instructions say otherwise.
- **Team-maintainer**: Only when the repository explicitly opts in, agents may
  close beads, run quality gates, commit, and push as part of session close. A
  current "do not commit" or "do not push" instruction still wins.

## Session Completion

This protocol applies when ending a Beads implementation workflow. It is
subordinate to explicit user, repository, and orchestrator instructions.

1. **File issues for remaining work** - Create beads for anything that needs
   follow-up
2. **Run quality gates** (if code changed) - Tests, linters, builds
3. **Update issue status** - Close finished work, update in-progress items
4. **Handle git/sync by active profile**:
5. **Hand off** - Summarize changes, validation, issue status, and any blocked
   sync/commit/push step

**Critical rules:**

- Explicit user or orchestrator instructions override this Beads block.
- Do not commit or push without clear authority from the active profile or the
  current user request.
- If a required sync or push is blocked, stop and report the exact command and
error.
<!-- END BEADS INTEGRATION -->

## Working Style

The user reads edits as they happen, so make them visible.

- Change files with the Edit and Write tools, not with `sed`, `python`,
  `cat <<EOF` or other shell scripts run through Bash. A shell edit hides the
  diff, and the user cannot review what changed.
- Use the LSP tool for code navigation (definitions, references, hover,
  diagnostics, symbols) before falling back to grep.
- Keep Bash calls to one small step each, with a description that says what the
  step is for. Do not chain an edit, a build, a test run and a commit into one
  command.
- Before a multi-step change, say in a sentence or two what is about to happen.

## Build & Test

```bash
cargo build
cargo test --test fixtures   # runs every tests/fixtures/**/*.isle through its mode/check directives
tools/update_filecheck.py [globs]   # bless failing fixtures in place; -n shows the diff only
```

`tools/update_filecheck.py` replaces the `check:`/`nextln:` lines of a failing
fixture with the full actual output, so it turns hand-picked checks into a
complete dump. Use it when the new output is right and the old checks are stale;
then trim the result by hand if a smaller check reads better. Always read the
diff (`-n`) before blessing, since a wrong output blessed once is a passing
test. Port it from `../clif-lsp/tools/update_filecheck.py` when the fixture
harness lands.

## Architecture Overview

Language server for ISLE, Cranelift's instruction selection and rewrite DSL,
aimed at people writing optimization rules. It is modeled on `../clif-lsp`
(same module layout, tower-lsp-server front end, ropey documents, fixture
harness, HTML views) and should stay close to it unless ISLE needs otherwise.

- A tolerant lexer and CST of our own handle editing-time input. Upstream's
  parser stops at the first error and its lexer drops comments, so it cannot
  drive editing features.
- `cranelift-isle` supplies the semantic layer (sema, overlap checking, decision
  trie, codegen) and runs only when the earlier layers are clean.
- Depend on the crates.io `cranelift-isle` pinned to `=0.136.2`, never on a
  path checkout of wasmtime, so the crate stays publishable. Rely on upstream
  Cranelift 0.136.2 for anything about which rules fire.
- Feature order is tracked in beads: tier 1 (core editing loop) before tier 2
  (rule-centric tooling) before tier 3 (HTML views).

## Conventions & Patterns

### Commits

- Commit once at the end of each bead task, after the quality gates pass and the
  bead is closed.
- One-line message, imperative, no body (e.g.
  `Add tolerant lexer for ISLE forms`).
- This overrides the Conservative profile above for task-end commits only. Do
  not push unless asked.

### Testing

- No unit tests: no `#[test]`, no `#[cfg(test)]` modules, no proptest.
  Correctness is checked by fixtures.
- Every behavior is covered by a `.isle` fixture under `tests/fixtures/` run
  through the fixture harness (`cargo test --test fixtures`), which pipes the
  tool's output through FileCheck-style `; check:`, `; nextln:`, `; sameln:`,
  `; not:` directives (the `filecheck` crate).
- A bead that adds or changes behavior is not done until a fixture with `check:`
  lines covers it.
- `; check:`, `; mode:` and other directive lines are ordinary `;` comments to
  the lexer/parser. Keep them as comments in the syntax tree.
