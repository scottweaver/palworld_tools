# Project State

The rehydration document. Any agent starting a session reads this
first to learn where the project stands right now. It answers "where
are we" — never "how does this work" (that's ARCHITECTURE.md and the
code) and never "how should we work" (that's METHODOLOGIES.md).

Last updated: 2026-08-03

## Active workstream

MVP build-out underway. PR #1 (merged 2026-08-02) landed the cargo
workspace (pal-core / pal-solver / pal-tui / pal-gui), the vendored
palcalc database pair (`data/db.json` + `data/breeding.json` @
upstream `c59712e`, v27), and pal-core's typed loader —
version-pinned, cross-reference-checked, 3 integration tests green.
The solver is functionally complete for the MVP core (PRs #5 + #7,
both merged 2026-08-02): `ChildIndex` + `PassiveOdds` single-pair
primitives, `steps` reachability (BFS matrix parity-matches the
vendored `MinBreedingSteps` on all 89,401 pairs; upstream's 10000
unreachable-sentinel normalized to `None` in the pal-core loader),
and `search::find_paths` (beam-pruned multi-step search over an
owned-pal pool, ranked by expected eggs, gender-reroll costs
modeled). The first frontend shipped in PR #9
(merged 2026-08-02): a ratatui TUI (species picker, ≤4-passive
picker, ranked plan trees) over an owned pool from `pals.toml`, with
the vendored data embedded in the binary via pal-core's
`vendored-data` feature (12 MB release binary, self-contained).
Frontend decisions 2026-08-02: TUI first; **GUI stack (egui vs
Tauri) deliberately deferred** until pal-gui work starts. PRs #11 +
#13 (merged 2026-08-03; #13 superseded the auto-closed #12) made the
TUI a full planner: ←/→ search depth 1–8, F2 wild-capture mode, F4
progenitor anchors with required-bitmask solver support, Del
clear-all, pinned marks, and family-tree plan rendering. 47 tests,
CI runs `--all-features`. Scope (binding):
umbrella Rust toolset for Palworld — breeding calculator first
(ported from tylercamp/palcalc's design, C#/MIT), then save-file
tools, server admin tools, pal data website. MVP ships two thin
frontends (TUI + desktop GUI) over shared library crates; pal-gui is
the remaining stub.

## Branches in flight

| Branch | Purpose | Status |
|---|---|---|
| `main` | trunk | at `8d299d8`, CI green, 47 tests |
| `feat/save-import` | pal-save crate: Level.sav → owned pals | validated on real save, 54 tests green, PR pending |

## Next up

1. Merge the `feat/save-import` PR (validated 2026-08-03 on the real
   save: 706/706 entries, 704 pals/221 species, 0.4s parse). Design
   decisions: gvas crate + own layer; TUI auto-detects .sav at the
   existing pool argument; all guild-owned pals import.
2. pal-gui: run the deferred egui-vs-Tauri stack dialog, then mirror
   the TUI slice over the same library APIs.
3. Solver refinements toward palcalc parity (IVs, time-based effort,
   capture-effort costing for wild pals) and TUI follow-ups (in-app
   pool editing; search off the UI thread — worst-case searches are
   ~2s now, enough to warrant a worker for responsiveness).
4. PROJECT.md carve-out gap: docs PRs carry generated
   `.cursor/rules/*.mdc` mirrors, which sit outside the
   `.claude/rules/` auto-merge carve-out — extend the binding or
   exclude mirrors (flagged during PR #1 wrap-up).

## Most recent meaningful progress

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
- **2026-08-02 — First frontend: ratatui TUI (PR #9, merged).**
  pal-tui is a real
  calculator now: filterable species/passive pickers, F5 search,
  ranked plan trees with per-step carry annotations; owned pool from
  `pals.toml` (display-name resolution via new `PalDb::find_pal`/
  `find_passive`); vendored data embedded behind pal-core's
  `vendored-data` feature. CI upgraded to `--all-features`. Why:
  proves the whole stack — loader → solver → UI — as a usable tool.
  Risk: search runs on the UI thread and rebuilds `SpeciesAdjacency`
  per search (~100ms release) — fine now, needs a worker thread
  before bigger pools/steps.
- **2026-08-02 — Path search + reachability slice (PR #7,
  merged).** `pal-solver`
  gains `steps::MinStepsTable` (BFS parity with the vendored matrix,
  all 89,401 pairs) and `search::find_paths` (expected-eggs-ranked
  plans; gender rerolls and passive carry-through costed per node).
  pal-core now normalizes upstream's 10000 "unreachable" sentinel to
  `None` at the boundary. Why: the MVP calculator core is now
  end-to-end — pool in, ranked breeding plans out. Risk: cost model
  is deliberately simplified vs palcalc (eggs not wall-clock, no
  wild pals, bred parents contribute only carried passives) —
  documented in search.rs; revisit before claiming palcalc parity in
  the UI.
- **2026-08-02 — First solver slice: child lookup + passive odds
  (PR #5, merged).**
  `pal-solver` gains `ChildIndex` (every ordered species pair
  resolves; the one gendered combo, Katress × Wixen, verified per
  arrangement) and `PassiveOdds` (upstream formula ported; validated
  entry point sums exact-total probabilities over final counts).
  Why: these are the two primitives every breeding-path search step
  costs out. Risk: formula parity rests on ported math + hand-derived
  vectors, not on running PalCalc itself — if upstream changes its
  solver, our numbers silently diverge from theirs (acceptable: the
  vendored data pins the game build).
- **2026-08-02 — CI gate landed (PR #3, merged).** GitHub Actions
  workflow running fmt + clippy `-D warnings` + `cargo test
  --workspace` on every PR and `main`; first run green in 30s. Why:
  the docs auto-merge carve-out and every future PR now have a real
  gate instead of a vacuous one. Risk: none noted — mirrors the
  local tooling floor exactly.
- **2026-08-02 — Workspace scaffold + vendored game DB (PR #1,
  merged).** Cargo
  workspace (4 crates), palcalc data pair vendored with MIT
  attribution + refresh runbook (`data/README.md`), and pal-core's
  boundary loader (newtyped model, version pin `v27`, full
  referential cross-checks; 3 integration tests). Why: unblocks the
  solver — typed, trusted game data is the input to everything else.
  Risk: db schema is upstream's; a palcalc refresh can break the
  loader — mitigated by the version pin and loud parse errors.
- **2026-08-02 — Rules layer bootstrapped.** PROJECT.md (GitHub
  tracker) plus METHODOLOGIES, RUST_BEST_PRACTICES, STATE,
  ARCHITECTURE, and CLAUDE.md installed; public repo
  `scottweaver/palworld_tools` created. Why: portable skills and
  agent rehydration work from day one. Risk: architecture constraints
  are young — expect renegotiation as the MVP solidifies.

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
