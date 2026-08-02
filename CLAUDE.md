# palworld_tools

A Rust toolbox for Palworld. The MVP is a breeding-path calculator
ported from the design and game database of
[tylercamp/palcalc](https://github.com/tylercamp/palcalc) (C#, MIT),
shipping thin TUI and desktop-GUI frontends over shared library
crates; save-file tools, dedicated-server admin tools, and a pal data
website follow. Goal: one well-factored cargo workspace where every
Palworld tool shares the same core data model and solver.

## Agent memory and rules

This file is the shared memory root for **all** agents working on the
project — Claude Code, Cowork, and any others. `AGENTS.md` is a
symlink to this file so tools reading either name get identical
content. Durable project knowledge belongs in this file or in
`.claude/rules/` — not in any tool-private memory store — so the
human and every agent can read and modify it.

All files in `.claude/rules/` are loaded each turn. Use this table to
decide which file is authoritative for a given decision.

| For decisions about...                                  | Authority                |
| ------------------------------------------------------- | ------------------------ |
| Rust style, type discipline, FP conventions             | `RUST_BEST_PRACTICES.md` |
| Workflow: branching, PRs, refactors, post-merge cleanup | `METHODOLOGIES.md`       |
| Binding architecture constraints                        | `ARCHITECTURE.md`        |
| Bindings for portable skills (stand-up, wrap-up)        | `PROJECT.md`             |
| Current project state, in-flight work, what's next      | `STATE.md`               |

`STATE.md` is the rehydration document: every agent session starts by
reading it. Stable architecture docs win over `STATE.md` on questions
of intended shape; `STATE.md` wins on what is actually in flight
right now.

**Non-Claude agents (Cursor, etc.):** `.cursor/rules/*.mdc` are
generated mirrors of `.claude/rules/` — edit the source, then run
`agent-sync` to regenerate; never edit the mirrors. Reusable
cross-project workflows ("skills") live at `~/.claude/skills/<name>/
SKILL.md`; any agent may read one and follow it when the user asks
for that workflow by name (e.g. wrap-up, daily-stand-up,
bootstrap-agent-rules).

## Top-level defaults

- Confirm before destructive operations (branch deletion, history
  rewrite, file deletion outside `/tmp`).
- Draft tickets in chat before creating them.
- New feature work starts on a new branch; PRs open only when the
  work moves the needle (see METHODOLOGIES.md).
- Non-trivial designs go through an interactive design dialog before
  implementation.
