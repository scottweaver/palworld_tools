# Project State

The rehydration document. Any agent starting a session reads this
first to learn where the project stands right now. It answers "where
are we" — never "how does this work" (that's ARCHITECTURE.md and the
code) and never "how should we work" (that's METHODOLOGIES.md).

Last updated: 2026-08-03

## Active workstream

The MVP breeding calculator is feature-complete as a TUI and
battle-tested against the user's real save (PRs #1–#21, 2026-08-02
… 08-03). Stack: **pal-core** (vendored palcalc db v27 @ `c59712e`,
typed loader, embeddable via `vendored-data`); **pal-solver**
(parity-anchored search — arena + frontier expansion +
species-group pruning + incumbent cut, rayon; depth ≤ 24;
progenitor-anchor and wild-capture modes); **pal-save** (`Level.sav`
import incl. PlM/Oodle containers and self-discovering GVAS hints;
validated: 731 pals, 716 with passives); **pal-tui** (reactive
planner — auto re-search on any change, mouse + ⇧click progenitors,
tier-colored passives, pinned selections, family-tree plans, wild
opt-in via F2). **GUI stack (egui vs Tauri) deliberately deferred**
until pal-gui starts. 60 tests, CI runs `--all-features`. Scope
(binding): umbrella Rust toolset for Palworld — breeding calculator
(done as TUI), save-file tools (import done), server admin tools,
pal data website; thin frontends over shared library crates, pal-gui
the remaining stub.

## Branches in flight

| Branch | Purpose | Status |
|---|---|---|
| `main` | trunk | at `0eb3fee`, CI green, 60 tests |

## Next up

1. pal-gui: run the deferred egui-vs-Tauri stack dialog, then mirror
   the TUI slice over the same library APIs (save import included —
   pal-save shipped in PR #17).
2. Solver refinements toward palcalc parity (IVs, time-based effort,
   capture-effort costing for wild pals) and TUI follow-ups (in-app
   pool editing; search off the UI thread — worst-case searches are
   ~2s now, enough to warrant a worker for responsiveness).
3. PROJECT.md carve-out gap: docs PRs carry generated
   `.cursor/rules/*.mdc` mirrors, which sit outside the
   `.claude/rules/` auto-merge carve-out — extend the binding or
   exclude mirrors (flagged during PR #1 wrap-up).

## Most recent meaningful progress

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
- **2026-08-03 — Passive tier colors in the TUI (PR #19, merged).**
  Passives render
  in the game's palette everywhere they appear (Passives pane + plan
  trees): detrimental (rank < 0) red, regular (1–3) gold, "rainbow"
  tier (4+) teal/cyan; plan trees moved from plain strings to styled
  spans to carry it. Risk: none noted — data-driven off
  `PassiveSkill.rank`.
- **2026-08-03 — Save-file import built (pal-save crate).**
  `Level.sav` → typed pals: gvas crate parses the Palworld container
  natively; `level.rs` extracts `CharacterSaveParameterMap` raw
  blobs with **self-discovering struct hints** (MissingHint errors
  drive retry, Guid backtracking, discovered hints reported);
  `import.rs` resolves species (BOSS_/role prefixes stripped),
  gender, and passives against PalDb with a full skip report. TUI
  pool argument auto-detects .sav vs toml (identical profiles
  deduped). **Validated on the real save**: PlM/Oodle container
  support added (oozextract; the save era post-dates gvas's zlib
  path), nested blobs decode under the file's custom versions
  (12-vs-24-byte vector widths), 706/706 entries → 704 pals, 221
  species, player detected, 0.4s. Why: real pools, not hand-typed
  toml. Risk: hint seeds + prefixes track game version — discovery
  self-heals and `decode_issues`/`malformed_entries` make drift
  loud.
- **2026-08-03 — Search rewritten for speed.** `Solver` struct
  (adjacency precomputed once, per-goal distance memo); append-only
  record arena (plan trees materialized only for results, never
  cloned mid-search); frontier expansion (only pairs touching a
  candidate added last round); species-group-first pair enumeration
  with group-level child/reachability rejection; rayon across
  groups. Depth-6 × 3-progenitor × wild went from unfinishable in
  10+ minutes to ~1.8s (perf.rs, ignored test). Why: user-reported
  stalls at high depth/multiple progenitors. Risk: frontier
  expansion relies on beam costs only improving over rounds —
  documented in search.rs; semantics pinned by the unchanged
  47-test suite.
- **2026-08-03 — Progenitor picking (the headline flow), fixed to
  anchor semantics (PR #13, merged).** F4 in the Pals pane marks progenitors (`[P]`
  rows, pinned to the top; Del clears all). Marks are required
  anchors: every plan must include each marked pal, with wild
  partners recruited around them (`BreedingGoal.progenitors` +
  required-bitmask beam state in the solver). First cut wrongly used
  a closed pool — Anubis→Knocklem found nothing; both user examples
  are now verbatim tests. Plans render as a family tree (box-drawing
  + emoji leaf tags). Risk: progenitors carry no passives by design;
  passive goals in this mode report guidance instead of plans.
- **2026-08-02 — Wild-pal capture mode.** pal-core now parses
  `MinWildLevel`/`MaxWildLevel` (`Pal::wild_levels`, `None` for the
  13 raid/special forms); `find_paths` gains `allow_wild_pals`:
  wild-spawning species join as free-capture leaves (any gender,
  zero eggs, no passives), so plans read "catch X" where useful; TUI
  toggles it with F2 (default on). Why: progenitor→target planning
  no longer limited to the toml pool — owned pals supply passives,
  wilds bridge species. Risk: capture effort is unmodeled, so
  wild-heavy plans look free; palcalc-style time costing queued.
- **2026-08-02 — TUI search depth adjustable.** ←/→ set
  `max_breeding_steps` (1–8, shown in the Plans title; 8 = data max
  species distance 7 + passive-consolidation headroom). Why: deep
  progenitor→descendant chains were capped at a hard-coded 3. Risk:
  depth 7–8 searches on the UI thread can stall on big pools —
  worker-thread item queued in next-up.
## Blocked / waiting

- *(nothing)*

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
