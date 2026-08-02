//! Palworld data model and the loader for the vendored game database.
//!
//! Data provenance: `data/db.json` + `data/breeding.json` at the
//! workspace root, vendored from tylercamp/palcalc (MIT) — see
//! `data/README.md` for the pinned upstream commit and refresh policy.
//!
//! [`db`] is the JSON boundary: raw files are parsed there exactly
//! once into [`model`] types, which downstream code trusts without
//! re-checking.

pub mod db;
pub mod model;
#[cfg(feature = "vendored-data")]
pub mod vendored;
