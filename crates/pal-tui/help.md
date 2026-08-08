# pal-tui — help

## Keys and mouse

| Input | Effect |
|---|---|
| `Tab` / `Shift+Tab` | Cycle pane focus |
| type / `Backspace` | Edit the focused pane's filter |
| `↑` / `↓` | Move the cursor |
| `Enter` or click | Pals: set target · Passives: toggle · Plans: re-search |
| `F4` or `⇧`+click | Toggle a progenitor mark (Pals pane) |
| `Ctrl+D` / `Delete` | Clear progenitor marks (library: delete entry after y/n; `x` too) |
| `←` / `→` | Search depth (1–24, shown in the Plans title) |
| `h` / `a` / `d` | Plans pane: raise an IV minimum by 10 (uppercase lowers) |
| `F2` | Toggle wild-capture mode |
| `F5` | Re-run the search and jump to Plans |
| `F6` | Reload the pool from the save/toml on disk |
| `Ctrl+S` / `F8` | Save the highlighted plan to the library |
| `Ctrl+L` / `F9` | Toggle the saved-plan library |
| `:` | Open the command prompt |
| `Esc` | Quit (closes the prompt or this viewer first) |

## Commands

| Command | Effect |
|---|---|
| `:help` | This screen |
| `:readme` | The full README, rendered in-app |
| `:w` | Save the highlighted plan to the library (= `Ctrl+S`) |
| `:o` | Open the saved-plan library (= `Ctrl+L`; never closes it) |
| `:dd` | Delete the selected saved plan immediately — typed, so it skips the y/n gate |
| `:clear` | Clear the target, progenitor marks, and passives |
| `:reload` | Reload the pool from the save/toml on disk (= `F6`) |
| `:q` / `:quit` | Quit |

In this viewer: `↑`/`↓` and the mouse wheel scroll by line,
`PgUp`/`PgDn` by 15, `Home`/`End` jump, `Esc` or `q` closes.

## The calculator in one minute

Pick a target species in the **Pals** pane (type to filter, `Enter`
to select). Plans appear immediately in the **Plans** pane and
re-compute on every change — there is nothing to submit.

- **Passives** — pick up to 4 in the Passives pane; plans are ranked
  by expected eggs, the cost of hatching until every passive lands.
- **IV minimums** — in the Plans pane, `h`/`a`/`d` raise per-stat
  floors (HP / Attack / Defense); plans then route through parents
  that can actually meet them.
- **Search depth** — `←`/`→` bound how many breeding generations a
  plan may use.
- **Wild capture** (`F2`) — off by default; on, catchable wild pals
  may join plans as free partners (they carry no passives or IVs).
- **Progenitors** (`F4` / `⇧`+click) — mark pals that every plan
  must breed through. Anchors contribute species only, so combine
  with passives sparingly.
- **The library** (`Ctrl+S` save, `Ctrl+L` browse) — bookmark plans
  across sessions; `Enter` on a saved plan re-plans its goal against
  your current box and shows how the cost moved.
- **Pool reload** (`F6`) — after an in-game save, re-import the same
  save/toml without restarting.

Leaf icons in a plan tree: 🥚 breed · 🎒 from your box · 🌿 catch ·
⭐ your progenitor. Passive names are tier-colored: teal rainbow-tier,
gold regular, red detrimental.

For the full manual — pool file formats, save-file locations, the
cost model, and known simplifications — see `:readme`.
