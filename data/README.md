# Vendored game data

`db.json` and `breeding.json` are the Palworld game database from
[tylercamp/palcalc](https://github.com/tylercamp/palcalc)
(`PalCalc.Model/`), MIT-licensed — see `LICENSE-palcalc.txt`. PalCalc
generates them from local game files via its `PalCalc.GenDB` pipeline;
we vendor the generated output and do no extraction ourselves
(ARCHITECTURE.md "Source of truth").

| | |
|---|---|
| Upstream commit | `c59712e24b839a0bedef16b06a1a0117e8741fe3` |
| Database version | `v27` (db.json `Version` field) |
| Vendored | 2026-08-02 |

The two files are a matched pair generated from the same game build —
always refresh them together, never one alone. `breeding.json` carries
no version field of its own; it is trusted because it rides with the
`db.json` it was generated alongside.

## Refreshing after a game patch

1. Pick the new upstream commit and pull both files:

   ```sh
   SHA=<upstream-commit>
   curl -sSL -o data/db.json       "https://raw.githubusercontent.com/tylercamp/palcalc/$SHA/PalCalc.Model/db.json"
   curl -sSL -o data/breeding.json "https://raw.githubusercontent.com/tylercamp/palcalc/$SHA/PalCalc.Model/breeding.json"
   ```

2. Update `SUPPORTED_VERSION` in `crates/pal-core/src/db.rs` to the new
   `Version` field (the loader hard-fails on a mismatch by design).
3. Update the table above (commit, version, date).
4. `cargo test` — the loader cross-checks referential integrity, so a
   schema change upstream fails loudly here, not at runtime.

A refresh that keeps the schema is not a structural change; a schema
change is (ARCHITECTURE.md "Structural criteria").
