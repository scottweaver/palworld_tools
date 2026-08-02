//! Breeding-path solver over the pal-core model, following the shape
//! of palcalc's `PalCalc.Solver` (see ARCHITECTURE.md).
//!
//! Current slice: single-pair primitives — [`child`] answers which
//! species a male × female pairing produces, [`passives`] computes
//! passive-inheritance probabilities. Both are ported from palcalc at
//! the same upstream commit as the vendored database and are
//! parity-tested against it. Multi-step path search builds on these.

pub mod child;
pub mod passives;
