//! Breeding-path solver over the pal-core model, following the shape
//! of palcalc's `PalCalc.Solver` (see ARCHITECTURE.md).
//!
//! [`child`] answers which species a male × female pairing produces;
//! [`passives`] computes passive-inheritance probabilities — both
//! ported from palcalc at the same upstream commit as the vendored
//! database and parity-tested against it. [`steps`] derives species
//! reachability (parity-tested against the vendored min-steps
//! matrix), and [`search`] runs the multi-step breeding-path search
//! over an owned-pal pool on top of all three.

pub mod child;
pub mod passives;
pub mod search;
pub mod steps;
