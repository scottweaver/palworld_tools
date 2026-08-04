//! Palworld save-file import: `Level.sav` in, typed owned pals out.
//!
//! Layered like the rest of the workspace: [`level`] is the wire
//! boundary — it parses the Palworld GVAS container (via the `gvas`
//! crate's native Palworld support) and extracts raw character
//! entries from `worldSaveData.CharacterSaveParameterMap`; [`import`]
//! resolves those raw entries against a [`pal_core::model::PalDb`]
//! into typed pals plus an honest report of everything skipped.
//!
//! Real save files are private user data: they are read in place and
//! never committed to this repository (see ARCHITECTURE.md); CI
//! exercises the layers below the file boundary with synthetic data.

pub mod container;
pub mod import;
pub mod level;

/// Whether `bytes` look like a Palworld `.sav` container rather than
/// e.g. a `pals.toml`. Checks the `PlZ`/`PlM`/`CNK` magic at offset 8.
#[must_use]
pub fn looks_like_sav(bytes: &[u8]) -> bool {
    container::has_save_magic(bytes)
}
