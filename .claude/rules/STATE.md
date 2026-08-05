# Project State

The rehydration document. Any agent starting a session reads this
first to learn where the project stands right now. It answers "where
are we" — never "how does this work" (that's ARCHITECTURE.md and the
code) and never "how should we work" (that's METHODOLOGIES.md).

Last updated: 2026-08-05

## Active workstream

The MVP breeding calculator is feature-complete as a TUI and
battle-tested against the user's real save (PRs #1–#35, 2026-08-02
… 08-05). Stack: **pal-core** (vendored palcalc db v27 @ `c59712e`,
typed loader, embeddable via `vendored-data`); **pal-solver**
(parity-anchored search — arena + frontier expansion +
species-group pruning + incumbent cut, rayon; depth ≤ 24;
progenitor-anchor, wild-capture, and IV-minimum modes); **pal-save** (`Level.sav`
import incl. PlM/Oodle containers and self-discovering GVAS hints;
validated: 771 pals, all with IVs); **pal-tui** (reactive
planner — auto re-search on any change with searches on a background
worker (spinner in the Plans title, latest-wins supersession), mouse
+ ⇧click progenitors, tier-colored passives, pinned selections,
family-tree plans, wild opt-in via F2, saved-plan library incl.
Ctrl+S/Ctrl+L and x-delete, F6 pool reload, h/a/d IV floors, `:`
command prompt — :help/:readme embedded docs plus :w/:o/:dd/:clear
working verbs). **GUI stack (egui vs Tauri) deliberately
deferred** until pal-gui starts. 107 tests, CI runs `--all-features`. Scope
(binding): umbrella Rust toolset for Palworld — breeding calculator
(done as TUI), save-file tools (import done), server admin tools,
pal data website; thin frontends over shared library crates, pal-gui
the remaining stub.

## Branches in flight

| Branch | Purpose | Status |
|---|---|---|
| `main` | trunk | at `5eda6bb`, CI green, 107 tests |

## Next up

1. pal-gui: run the deferred egui-vs-Tauri stack dialog, then mirror
   the TUI slice over the same library APIs (save import included —
   pal-save shipped in PR #17).
2. Remaining solver refinements toward palcalc parity (time-based
   effort, capture-effort costing for wild pals) plus solver-level
   search cancellation — the TUI worker discards superseded results,
   but an in-flight deep search still runs to completion (~15s worst
   case); a cancel check in the search loop plus the priority-queue
   rework would end it early. TUI follow-up: in-app pool editing.
3. PROJECT.md carve-out gap: docs PRs carry generated
   `.cursor/rules/*.mdc` mirrors, which sit outside the
   `.claude/rules/` auto-merge carve-out — extend the binding or
   exclude mirrors (flagged during PR #1 wrap-up).

## Most recent meaningful progress

- **2026-08-05 — Search worker + spinner, command verbs (PRs #34 +
  #35, merged).** Searches moved off the UI thread: App snapshots
  each question into a generation-tagged request, a worker thread
  computes it, and only the latest generation may apply — parked
  requests replace (latest-wins), stale outcomes are discarded, and
  clearing the target cancels. A braille spinner animates in the
  Plans title; Inline mode keeps the old synchronous path for tests.
  #34 added prompt verbs (:w save, :o open library, :dd delete,
  :clear reset goal); a later fix made plain `x` the in-library
  delete key after `dd` collided with the d IV-floor key. Risk: no
  solver-level cancellation yet — a superseded deep search finishes
  before its successor starts (Next up #2).
- **2026-08-05 — Command prompt + Ctrl shortcuts (PRs #31 + #32,
  merged).** `:` opens a vim-style prompt: `:help` renders a new
  embedded key/feature reference (`crates/pal-tui/help.md`),
  `:readme` the full README, `:q`/`:quit` quits; the viewer is
  full-screen and scrollable (keys + wheel). Rendering is
  tui-markdown 0.3.9 (default-features off; picked for box-drawing
  tables), with soft breaks preprocessed to hard breaks outside code
  fences so the authored ~72-col layout survives unwrapped and
  scroll math stays exact. Overlay state is an enum (None / Prompt /
  Doc) that owns keys and clicks while open. #31 added Ctrl+S/Ctrl+L
  as save/library synonyms. Risk: help.md's key table duplicates the
  README's — both are now guarded by a ≤100-col rendered-width test,
  but content drift needs auditing on UX changes.
- **2026-08-04 — IV minimums (PR #29, merged).** palcalc's
  threshold model end to end: h/a/d set per-stat floors, plans route
  only through qualifying parents, costs absorb the 5/9-5/18-1/6
  category roll + right-parent coins, and search state collapses IVs
  to met/not-met bits so depth-24 stays ~0.5s. Save import extracts
  talents (ByteProperty in current saves — caught by the new
  IV-presence assert; 771/771 pals carry IVs). Risk: dedupe now
  splits IV-different profiles (704 → 751 on the real box) — watch
  pair-enumeration cost if boxes grow much larger.
- **2026-08-03 — Saved-plan library + F6 pool reload (PR #27,
  merged).** F8 bookmarks fully self-describing plan trees into a
  stable per-user store (platform data dir; legacy `./plans.json`
  auto-migrates); F9 browses; Ctrl+D/Backspace delete (Mac
  keyboards lack Del); Enter re-plans a saved goal against the
  current box, with staleness banners. F6 re-imports the save/toml
  in place — loading extracted to a shared `pool` module so reload
  can never drift from startup. `probe_real_store` answers "where
  are my plans and do they parse" headlessly; corrupt stores back
  up to `.bak`, never clobbered.
- **2026-08-03 — What-if pals in the repro harness (PR #25,
  merged).** PROBE_EXTRA injects hypothetical owned pals into
  real-save searches; used to show a clean 3-desired-passive pal
  beats its junk-carrying twin (17.14 → 14.64 eggs) and that blank
  slots beat junk passives, always.
- **2026-08-03 — README + LICENSE (PR #23, merged).** Full user
  documentation (install, both pool sources, key/mouse reference,
  plan-tree legend, cost-model semantics incl. documented
  simplifications, contributor notes) and the MIT license text
  Cargo.toml had been declaring without a file. Risk: docs drift as
  features land — treat README's key table and semantics sections as
  audit-worthy on UX changes.
- **2026-08-03 — TUI UX batch (PR #21, merged).** Mouse support
  (click selects, ⇧click marks progenitors), wild capture off by
  default, target relocated to the Plans title, selected passives and
  progenitors pinned to their list heads, reactive search (any
  change re-plans automatically; stale Plans impossible), plus the
  three items below. Permanent real-save diagnostics landed in
  crates/pal-tui/tests/repro.rs.
- **2026-08-03 — Import bug: every pal's passives silently dropped.**
  `PassiveSkillList` parses into gvas's `ArrayProperty::Names`,
  which the extractor didn't handle — all 704 pals imported
  passive-less (also why passive-constrained searches returned
  nothing). Found via a user report (Loomen + Diamond Body + Demon
  God → no plans); fixed, revalidated (731 pals, 716 with passives),
  and `real_save.rs` now asserts nonzero carriers. Lesson: "0
  unknown passives" was consistent with "0 passives extracted" —
  validation now checks presence, not just absence of errors.
- **2026-08-03 — Depth cap 8 → 24 + incumbent cut.** With wild off
  by default, closed-pool routing can legitimately exceed the old
  7+1 bound. Solver gains a branch-and-bound cut (candidates whose
  root cost + remaining-distance eggs can't beat the worst incumbent
  plan are dropped once max_results plans exist). Measured on the
  real save at depth 24: 0.13s wild-off / 0.28s wild-on. Risk: the
  pathological combo (deep + wild + 3 progenitor marks) still runs
  ~15s on the UI thread — the state-space churn needs a
  priority-queue search, queued with the worker-thread item.
- **2026-08-03 — Mouse support in the TUI.** Left-click focuses a
  pane and acts on the row under the pointer (target in Pals, toggle
  in Passives, plan selection in Results); ⇧click marks a progenitor.
  Hit-testing shares the draw geometry (`pane_areas`) and replicates
  ratatui's keep-selection-visible scroll offset. Risk: some
  terminals reserve ⇧click for native text selection — F4 remains
  the fallback.

## Maintenance

- **Refresh trigger:** any merge or milestone that changes what an
  incoming agent needs to know: workstream shifts, a branch opens or
  closes, "Next up" changes, something lands. Wired into
  METHODOLOGIES.md's post-merge routine (the "refresh STATE.md"
  step).
- **Always update:** "Last updated"; "Branches in flight"; prepend a
  progress entry (what / why / risk voice — a judgment edit, not a
  paste of the PR description).
- **As applicable:** "Active workstream" paragraph, "Next up",
  "Blocked / waiting".
- **Trim policy:** progress log holds at most 10 entries — drop the
  oldest when adding. Anything stable graduates out of this file
  into the appropriate rules doc; this file stays small because
  every agent loads it every turn.
- **Edit policy:** STATE.md is authored on feature branches,
  propagates through merges, and is refreshed (not deleted) on new
  branches. Never edit it directly on `main`; docs-only diffs under
  `.claude/rules/` ride the METHODOLOGIES.md docs-only carve-out.
- **Keep entries short:** each progress entry is a pointer — date,
  PR #, ticket, a sentence or two of judgment. If you're tempted to
  write more, the detail belongs in the PR, commit, or ticket.
