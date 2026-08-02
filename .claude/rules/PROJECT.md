---
# Project skill bindings — consumed by ~/.claude/skills/daily-stand-up and ~/.claude/skills/wrap-up.
# Hand-edited rule file; the portable skills are project-agnostic, all the per-project values live here.
# Schema source of truth: ~/.claude/skills/bootstrap-project/SKILL.md

tracker: github

# Linear bindings (required when tracker: linear; ignored otherwise).
# linear:
#   workspace: <slug>
#   team_prefix: <PREFIX>
#   assignee_id: <uuid>
#   state_uuids:
#     in_progress: <uuid>
#     in_review:   <uuid>
#     done:        <uuid>

# GitHub bindings (required when tracker: github; ignored otherwise).
github:
  owner: scottweaver
  repo:  palworld_tools
  assignee: scottweaver

# Project state file consumed by daily-stand-up (T discovery) and wrap-up (step-4 refresh).
# Set state_file.path to null to disable both behaviours.
state_file:
  path: .claude/rules/STATE.md
  next_up_patterns:
    - "**Next: {ID}**"
    - "**next up: {ID}**"
    - "Next up in Phase N: {ID}"

# Stand-up-only settings.
standup:
  no_blockers_sentinel: ":none:"

# Wrap-up-only settings.
wrapup:
  docs_pr_branch_prefix: docs/state-post-
  docs_pr_commit_prefix: "docs(state):"
  auto_merge_carve_out_path: .claude/rules/
  state_refresh_authority: null
  audit_doc: null
---

# Project bindings — palworld_tools

Public GitHub repo `scottweaver/palworld_tools`, tracked with GitHub Issues
(ticket identifiers take the `#123` shape). Repo and these bindings were
created together at bootstrap time (2026-08-02); the project itself is
greenfield.

Notes for the skills and future editors:

- `state_file.path` points at `.claude/rules/STATE.md`, which does not exist
  yet — run `/bootstrap-agent-rules` (or create it by hand) before relying on
  next-up ticket discovery. Until it exists, daily-stand-up falls back to
  prompting inline for today's ticket.
- Wrap-up runs with stock defaults: docs-only PRs branch from
  `docs/state-post-<slug>`, commit as `docs(state): …`, and anything entirely
  under `.claude/rules/` is auto-merge-eligible. No state-refresh authority
  file and no audit doc are configured.
- No `agent_sync` block — the project has no rules-sync script.
