//! The vendored database pair embedded at compile time, so frontend
//! binaries ship self-contained (the same choice palcalc makes).
//! Enabled by the `vendored-data` feature.

use crate::db::{self, ParseError};
use crate::model::{BreedingDb, PalDb};

const DB_JSON: &str = include_str!(concat!(env!("CARGO_MANIFEST_DIR"), "/../../data/db.json"));
const BREEDING_JSON: &str = include_str!(concat!(
    env!("CARGO_MANIFEST_DIR"),
    "/../../data/breeding.json"
));

/// Parses the embedded `db.json`.
///
/// # Errors
///
/// Fails only when the embedded data and the loader disagree — i.e. a
/// vendored-data refresh that skipped the `data/README.md` runbook.
pub fn pal_db() -> Result<PalDb, ParseError> {
    db::parse_pal_db(DB_JSON)
}

/// Parses the embedded `breeding.json` against `pal_db`.
///
/// # Errors
///
/// Same failure mode as [`pal_db`]: embedded data out of sync with
/// the loader.
pub fn breeding_db(pal_db: &PalDb) -> Result<BreedingDb, ParseError> {
    db::parse_breeding_db(BREEDING_JSON, pal_db)
}
