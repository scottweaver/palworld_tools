# palworld_tools

A Rust toolbox for [Palworld](https://www.pocketpair.jp/palworld). The
centerpiece is a **breeding-path calculator**: give it a target pal
(and optionally the passive skills you want on it), and it computes
ranked, multi-generation breeding plans — starting from your actual
box, imported straight from your save file.

The breeding model and game database are ported from
[tylercamp/palcalc](https://github.com/tylercamp/palcalc) (C#, MIT),
with the search engine, save import, and terminal UI built fresh in
Rust.

```text
┌ Pals ────────────┐┌ Passives (2/4) ─┐┌ Plans — Loomen · depth ≤ 3 · wild off ────┐
│ /azur            ││ /                ││ 1. 1.67 expected eggs, 1 step(s)          │
│ [P] Azurmane     ││ [x] Diamond Body ││ 2. 2.50 expected eggs, 1 step(s)          │
│     Azurobe      ││ [x] Demon God    ││                                           │
│     Azurobe Cryst││ [ ] Abnormal     ││ 🥚 Loomen · hatch for Diamond Body, De…   │
│                  ││ [ ] Aggressive   ││ ├─ ♂ 🎒 Azurmane · Diamond Body, Demon…   │
│                  ││ [ ] Artisan      ││ ╰─ ♀ 🎒 Helzephyr · Swift                 │
└──────────────────┘└──────────────────┘└───────────────────────────────────────────┘
 5 plan(s) found
 Tab panes · type to filter · Enter/click select · F4/⇧click progenitor · …
```

## Features

- **Plan from your real box** — point the app at your `Level.sav` and
  every owned pal (species, gender, passive skills) becomes breeding
  stock. Supports current Oodle-compressed saves (`PlM`, Palworld
  0.6+), the older zlib era (`PlZ`), and Xbox containers (`CNK`).
  `F6` re-imports the file in place, so hatching progress flows in
  after each in-game save without restarting.
- **Passive-skill planning** — pick up to 4 target passives; plans
  are ranked by **expected eggs**, computed with palcalc's
  inheritance-probability model (verified against its formulas).
- **Progenitor anchoring** — mark one or more pals as required
  ancestors and every plan is guaranteed to breed *through* them.
- **IV minimums** — require the target to hit per-stat IV floors
  (e.g. Attack ≥ 90); plans route through parents that carry the
  stat and the cost absorbs the inheritance odds (palcalc's model).
- **Wild-capture mode** — optionally let plans recruit catchable wild
  pals as partners (off by default; your box is the source of truth).
- **Deep searches, fast** — plans up to 24 breeding steps; typical
  searches against a 700-pal box complete in well under a second.
- **Reactive UI** — every change (target, passives, marks, depth,
  wild) re-plans instantly; the Plans pane never shows stale results.
  Searches run on a background worker: a spinner in the Plans title
  marks one in flight, the UI stays responsive throughout, and
  changing anything mid-search supersedes it.
- Fully self-contained binary: the game database is embedded, and
  `:help` / `:readme` render the documentation in-app.

## Installation

Requires [Rust](https://rustup.rs/) (stable, edition 2024).

```sh
git clone https://github.com/scottweaver/palworld_tools
cd palworld_tools
cargo build --release
# binary at target/release/pal-tui
```

## Quick start

```sh
# Plan from your save (recommended):
pal-tui /path/to/Level.sav

# …or from a hand-written pool:
cp pals.example.toml pals.toml
pal-tui              # reads ./pals.toml by default
pal-tui my-pals.toml # or any explicit path
```

The single optional argument is sniffed by content: Palworld `.sav`
containers import directly, anything else is parsed as TOML.

**Where is my save?** For Steam on Windows:
`%LOCALAPPDATA%\Pal\Saved\SaveGames\<SteamID>\<WorldID>\Level.sav`.
Copies of the file work fine — the app only ever reads it.

### The pals.toml format

```toml
[[pals]]
species = "Lamball"      # display or internal name, case-insensitive
gender = "male"          # male/female (or m/f)
passives = ["Swift"]     # optional; display or internal names
ivs = { hp = 85, attack = 92, defense = 70 }  # optional; 0 if omitted

[[pals]]
species = "Cattiva"
gender = "female"
```

## Using the TUI

Three panes, left to right: **Pals** (pick the target species and
mark progenitors), **Passives** (pick up to 4 desired passive
skills), **Plans** (ranked results with a family-tree breakdown).

Searches run automatically whenever the question changes. There is
nothing to submit — pick a target and watch the Plans pane.

Documentation travels with the binary: `:` opens a vim-style command
prompt, `:help` shows the key and feature reference in-app, and
`:readme` renders this README in a scrollable viewer (`Esc`/`q`
closes it, `:q` quits).

### Keys and mouse

| Input | Effect |
|---|---|
| `Tab` / `Shift+Tab` | Cycle pane focus |
| type / `Backspace` | Edit the focused pane's filter |
| `↑` / `↓` | Move the cursor |
| `Enter` or **click** | Pals: set target · Passives: toggle · Results: re-search |
| `F4` or **⇧+click** | Toggle a progenitor mark (Pals pane) |
| `Ctrl+D` (or `Delete`) | Clear progenitor marks; library: delete the plan (`Backspace` too) |
| `←` / `→` | Search depth (1–24, shown in the Plans title) |
| `h` / `a` / `d` (Plans pane) | Raise the HP/Attack/Defense IV floor by 10 (⇧ lowers; below 10 = off) |
| `F2` | Toggle wild-capture mode |
| `F5` | Re-run the search and jump to Results |
| `F6` | Reload the pool from the save/toml on disk (no restart needed) |
| `Ctrl+S` (or `F8`) | Save the highlighted plan to the library |
| `Ctrl+L` (or `F9`) | Toggle the Plans pane between live results and the saved-plan library |
| `:` | Open the command prompt: `:help` · `:readme` · `:q`/`:quit` |
| `Esc` | Quit (an open prompt or viewer closes first) |

Selected passives and marked progenitors **pin to the top** of their
lists (even when filtered out), so every active choice stays one
keypress from undoing. If ⇧+click doesn't register, your terminal is
reserving it for text selection — use `F4`.

### Reading a plan

```text
🥚 Fuack · hatch for Swift
├─ ♂ 🎒 Lamball · Swift
╰─ ♀ 🥚 Daedream
   ├─ ♂ ⭐ Anubis · your progenitor
   ╰─ ♀ 🌿 Cattiva · catch
```

| Symbol | Meaning |
|---|---|
| 🥚 | A breeding step — re-hatch until the child has the listed passives |
| 🎒 | A pal you own (from the save/toml) |
| ⭐ | A progenitor you marked |
| 🌿 | A wild pal to catch |
| ♂ / ♀ | Which parent slot the pal fills (intermediates re-hatch until the needed gender) |

Passive names are colored by in-game tier: **teal** for the special
"rainbow" tier (Legend, Lucky, …), **gold** for regular beneficial
passives, **red** for detrimental ones. When IV minimums are active,
owned leaves also show the pal's `hp/attack/defense` IVs, and the
Plans title shows the floors (`IV ≥ 80/90/–`).

**Expected eggs** is the plan's cost: the statistically expected
number of eggs you'll hatch across every step, accounting for
passive-inheritance odds and gender re-rolls of intermediates. Lower
is better; plans are ranked by it.

### Progenitor mode

Marking pals with `F4`/⇧+click switches the search to *anchored*
plans: every result must breed through **all** marked pals. Wild
partners are recruited automatically around your anchors (regardless
of the F2 setting — anchors need partners), while the rest of your
box steps aside. Note that anchors contribute their species only;
if you also select passives, the passives must come from somewhere,
so prefer un-anchored searches for passive goals.

### The saved-plan library

Found a plan you'll execute over several play sessions? `Ctrl+S`
(or `F8`) saves the highlighted plan into your library. It lives at a stable
per-user path regardless of where you launch from — macOS:
`~/Library/Application Support/palworld_tools/plans.json`, Linux:
`~/.local/share/palworld_tools/plans.json`, Windows:
`%LOCALAPPDATA%\palworld_tools\plans.json` (a legacy `./plans.json`
migrates there automatically; the library's status line shows the
exact path). `Ctrl+L` (or `F9`) flips the Plans pane to the library,
where `↑`/`↓`/click
browse and `Ctrl+D`/`Backspace` removes the highlighted entry.
Saved plans are fully self-contained — the whole tree with species,
genders, and passives — so they stay readable even after your box
changes.

The library shows the **original** plan, with a staleness banner:
`✓` when every pal in it still exists in your current box,
`⚠ N pal(s) no longer in your box` when the world has moved. Pressing
`Enter` on a saved plan **re-plans it**: the saved goal (target,
passives, progenitor marks) is restored and searched against your
current box, and the status line compares the fresh best cost with
what you saved. The original stays in the library untouched.
Changing anything about the live search flips you back to live
results.

### Wild-capture mode

Off by default: plans use only what you own. Toggle `F2` and any
species with wild spawns may join plans as a free `🌿 catch` leaf —
useful when your box lacks a bridge species. Wild pals contribute no
passive skills, and capture effort isn't costed yet (a caught
partner looks free next to hatching eggs).

## How the math works

The model follows palcalc:

- **Child species** is deterministic per parent pair, from the
  game-extracted combo table (44,851 combos incl. the gendered
  Katress × Wixen special case).
- **Passives**: a child rolls how many passives to inherit from the
  parents' combined pool (40/30/20/10% for 1–4) plus random
  additions; the probability that all desired passives land is
  computed exactly, per palcalc's formulas.
- **Gender** re-rolls of bred intermediates cost
  `1 / P(needed gender)` extra eggs.
- **IVs**: a child inherits 1–3 of its three IV stats from the
  parents (1/2, 1/3, 1/6), each copied from a random parent. A
  minimum is *met* through parents at/above it: both parents meeting
  a stat costs only the category roll, one parent adds a 50/50
  right-parent chance, and a stat no parent meets cannot be met at
  all (random rolls are ignored, matching palcalc) — wild pals and
  progenitors never satisfy IV minimums.

Known simplifications versus PalCalc proper (roadmapped): effort is
measured in eggs rather than wall-clock time, wild-pal capture is
free, IV values collapse to met/not-met against the active minimums,
and bred parents contribute exactly their target passives to the
next pool.

## Game-data coverage

The embedded database is palcalc's (`v27`, pinned upstream commit in
[`data/README.md`](data/README.md) with a refresh runbook). If a
game update adds pals or passives, they'll be missing until the
vendored pair is refreshed; save import reports anything it cannot
resolve rather than guessing. Save parsing self-adapts to unknown
GVAS structures and fails loudly — never silently misreads — when
the format truly moves.

## Workspace layout

| Crate | Role |
|---|---|
| `pal-core` | Game-data model + loaders (the only code that parses the database) |
| `pal-solver` | Breeding search: child lookup, passive odds, reachability, path search |
| `pal-save` | `Level.sav` import: container, GVAS, character extraction |
| `pal-tui` | The terminal UI |
| `pal-gui` | Desktop GUI (planned) |

Development gates, CI-enforced: `cargo fmt`,
`cargo clippy --all-targets --all-features -- -D warnings`, and
`cargo test --workspace --all-features`.
Real-save validation harnesses live in
`crates/pal-save/tests/real_save.rs` and
`crates/pal-tui/tests/repro.rs` (opt-in via `PAL_SAVE_PATH`; real
saves are never committed).

## Roadmap

- Desktop GUI (`pal-gui`)
- Solver refinements: capture-effort costing, wall-clock effort
- Background search worker for the heaviest queries
- Dedicated-server admin tools; pal data website

## Credits and license

MIT. The breeding model, probability formulas, and game database are
from [PalCalc](https://github.com/tylercamp/palcalc) by Tyler Camp
(MIT — see [`data/LICENSE-palcalc.txt`](data/LICENSE-palcalc.txt)).
Palworld is a trademark of Pocketpair, Inc.; this project is
unaffiliated fan tooling.
