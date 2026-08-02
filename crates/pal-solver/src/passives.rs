//! Passive-skill inheritance probabilities for a single breeding pair.
//!
//! Ported term-for-term from palcalc — the weight normalization from
//! `PalCalc.Model/BreedingMechanics.cs`, the probability sum from
//! `PalCalc.Solver/Probabilities/Passives.cs` — so results
//! parity-match upstream for the same inputs.

use std::collections::BTreeMap;

use pal_core::model::{BreedingMechanics, PassiveName};

/// A pal carries at most this many passive skills (upstream
/// `GameConstants.MaxTotalPassives`).
pub const MAX_TOTAL_PASSIVES: usize = 4;

const TABLE_LEN: usize = MAX_TOTAL_PASSIVES + 1;

/// Normalized probability tables derived from the database's raw
/// inheritance weights, indexed by passive count `0..=4`.
#[derive(Clone, Debug)]
pub struct PassiveOdds {
    /// P(the inherit roll selects exactly `n` passives from the parents).
    direct: [f64; TABLE_LEN],
    /// P(exactly `n` random passives are added).
    random_exact: [f64; TABLE_LEN],
    /// P(at least `n` random passives are added) — used when the child
    /// is already at the passive cap, where surplus rolls are discarded.
    random_at_least: [f64; TABLE_LEN],
}

#[derive(Debug, PartialEq, Eq, thiserror::Error)]
pub enum PassiveOddsError {
    #[error("{table} weights sum to zero; cannot normalize")]
    ZeroWeightSum { table: &'static str },
}

#[derive(Debug, PartialEq, Eq, thiserror::Error)]
pub enum DesiredPassivesError {
    #[error("desired passive {0} is not in the parent pool")]
    NotInPool(PassiveName),
    #[error("desired passive {0} listed more than once")]
    Duplicate(PassiveName),
    #[error("{count} desired passives exceed the {MAX_TOTAL_PASSIVES}-passive cap")]
    TooMany { count: usize },
}

/// The deduplicated union of the two parents' passive skills — the
/// set a child can inherit from.
#[derive(Clone, PartialEq, Eq, Debug)]
pub struct ParentPool(Vec<PassiveName>);

impl ParentPool {
    #[must_use]
    pub fn from_parents(first: &[PassiveName], second: &[PassiveName]) -> Self {
        let mut distinct = Vec::new();
        for passive in first.iter().chain(second) {
            if !distinct.contains(passive) {
                distinct.push(passive.clone());
            }
        }
        Self(distinct)
    }

    #[must_use]
    pub fn len(&self) -> usize {
        self.0.len()
    }

    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.0.is_empty()
    }

    #[must_use]
    pub fn contains(&self, passive: &PassiveName) -> bool {
        self.0.contains(passive)
    }
}

impl PassiveOdds {
    /// # Errors
    ///
    /// Fails if either weight table in `mechanics` sums to zero.
    pub fn from_mechanics(mechanics: &BreedingMechanics) -> Result<Self, PassiveOddsError> {
        let direct = normalize(
            &mechanics.passive_inheritance_weights,
            "passive inheritance",
        )?;
        let random_exact = normalize(&mechanics.passive_random_weights, "passive random")?;

        let mut random_at_least = [0.0; TABLE_LEN];
        let mut acc = 0.0;
        for count in (0..TABLE_LEN).rev() {
            acc += random_exact[count];
            random_at_least[count] = acc;
        }

        Ok(Self {
            direct,
            random_exact,
            random_at_least,
        })
    }

    /// Probability that a child of parents whose combined pool is
    /// `pool` carries all `desired` passives, at any final passive
    /// count.
    ///
    /// # Errors
    ///
    /// Fails if `desired` has duplicates, exceeds the passive cap, or
    /// names a passive outside the pool.
    pub fn all_desired_probability(
        &self,
        pool: &ParentPool,
        desired: &[PassiveName],
    ) -> Result<f64, DesiredPassivesError> {
        if desired.len() > MAX_TOTAL_PASSIVES {
            return Err(DesiredPassivesError::TooMany {
                count: desired.len(),
            });
        }
        for (index, passive) in desired.iter().enumerate() {
            if desired[..index].contains(passive) {
                return Err(DesiredPassivesError::Duplicate(passive.clone()));
            }
            if !pool.contains(passive) {
                return Err(DesiredPassivesError::NotInPool(passive.clone()));
            }
        }

        Ok((desired.len()..=MAX_TOTAL_PASSIVES)
            .map(|num_final| self.exact_total_probability(pool.len(), desired.len(), num_final))
            .sum())
    }

    /// Probability that the child ends with exactly `num_final`
    /// passives, all `desired_count` targets among them, given
    /// `pool_size` distinct passives across both parents.
    ///
    /// Contract (matching upstream): `desired_count <= pool_size` and
    /// `num_final <= MAX_TOTAL_PASSIVES`; [`Self::all_desired_probability`]
    /// is the validated entry point.
    #[must_use]
    pub fn exact_total_probability(
        &self,
        pool_size: usize,
        desired_count: usize,
        num_final: usize,
    ) -> f64 {
        let mut total = 0.0;

        for num_inherited in desired_count..=MAX_TOTAL_PASSIVES {
            // The inherit roll can exceed what the parents actually
            // have; the surplus inherits nothing but the roll's
            // probability still applies (upstream Passives.cs).
            let actual_inherited = num_inherited.min(pool_size);
            let irrelevant_from_parent = actual_inherited.saturating_sub(desired_count);
            let irrelevant_from_random = num_final.saturating_sub(actual_inherited);
            if actual_inherited + irrelevant_from_random > num_final {
                continue;
            }

            let from_parent = if desired_count == 0 {
                self.direct[num_inherited]
            } else if irrelevant_from_parent == 0 {
                self.direct[num_inherited] / choose(pool_size, desired_count)
            } else {
                self.direct[num_inherited]
                    * choose(pool_size - desired_count, irrelevant_from_parent)
                    / choose(pool_size, actual_inherited)
            };

            let from_random = if num_final == MAX_TOTAL_PASSIVES {
                self.random_at_least[irrelevant_from_random]
            } else {
                self.random_exact[irrelevant_from_random]
            };

            total += from_parent * from_random;
        }

        total
    }
}

fn normalize(
    weights: &BTreeMap<u8, u32>,
    table: &'static str,
) -> Result<[f64; TABLE_LEN], PassiveOddsError> {
    let sum: u32 = weights.values().sum();
    if sum == 0 {
        return Err(PassiveOddsError::ZeroWeightSum { table });
    }
    let mut normalized = [0.0; TABLE_LEN];
    for count in 0..=4u8 {
        let weight = weights.get(&count).copied().unwrap_or(0);
        normalized[usize::from(count)] = f64::from(weight) / f64::from(sum);
    }
    Ok(normalized)
}

#[expect(
    clippy::cast_precision_loss,
    reason = "operands are passive counts (<= 8); exactly representable in f64"
)]
fn choose(n: usize, k: usize) -> f64 {
    if k > n {
        return 0.0;
    }
    let k = k.min(n - k);
    let mut result = 1.0;
    for i in 0..k {
        result = result * ((n - i) as f64) / ((i + 1) as f64);
    }
    result
}

#[cfg(test)]
mod tests {
    use super::*;

    fn v27_mechanics() -> BreedingMechanics {
        BreedingMechanics {
            iv_inheritance_weights: BTreeMap::from([(1, 3), (2, 2), (3, 1)]),
            passive_inheritance_weights: BTreeMap::from([(1, 4), (2, 3), (3, 2), (4, 1)]),
            passive_random_weights: BTreeMap::from([(0, 4), (1, 3), (2, 2), (3, 1)]),
        }
    }

    fn odds() -> PassiveOdds {
        PassiveOdds::from_mechanics(&v27_mechanics()).unwrap()
    }

    fn assert_close(actual: f64, expected: f64) {
        assert!(
            (actual - expected).abs() < 1e-9,
            "expected {expected}, got {actual}"
        );
    }

    #[test]
    fn tables_normalize_like_upstream() {
        let odds = odds();
        for (actual, expected) in odds.direct.iter().zip([0.0, 0.4, 0.3, 0.2, 0.1]) {
            assert_close(*actual, expected);
        }
        for (actual, expected) in odds.random_exact.iter().zip([0.4, 0.3, 0.2, 0.1, 0.0]) {
            assert_close(*actual, expected);
        }
        for (actual, expected) in odds.random_at_least.iter().zip([1.0, 0.6, 0.3, 0.1, 0.0]) {
            assert_close(*actual, expected);
        }
    }

    #[test]
    fn zero_weights_are_rejected() {
        let mut mechanics = v27_mechanics();
        mechanics.passive_random_weights = BTreeMap::new();
        assert_eq!(
            PassiveOdds::from_mechanics(&mechanics).unwrap_err(),
            PassiveOddsError::ZeroWeightSum {
                table: "passive random"
            }
        );
    }

    #[test]
    fn exact_total_matches_hand_computed_vectors() {
        let odds = odds();
        // (pool, desired, final) -> expected, derived by hand from the
        // upstream formula with the v27 weight tables.
        let vectors = [
            (2, 2, 2, 0.24),
            (2, 2, 3, 0.18),
            (2, 2, 4, 0.18),
            (4, 2, 2, 0.02),
            (4, 2, 3, 0.055),
            (4, 2, 4, 0.175),
            (2, 0, 1, 0.16),
            (0, 0, 2, 0.2),
        ];
        for (pool, desired, num_final, expected) in vectors {
            assert_close(
                odds.exact_total_probability(pool, desired, num_final),
                expected,
            );
        }
    }

    #[test]
    fn desired_equal_to_pool_reduces_to_at_least_roll() {
        // With every pool passive desired, the subset choice is forced:
        // the probability collapses to P(inherit roll >= pool size).
        let odds = odds();
        let pool = ParentPool::from_parents(&[passive("a")], &[passive("b")]);
        let result = odds
            .all_desired_probability(&pool, &[passive("a"), passive("b")])
            .unwrap();
        assert_close(result, 0.6);
    }

    #[test]
    fn partial_desired_set_sums_over_final_counts() {
        let odds = odds();
        let pool =
            ParentPool::from_parents(&[passive("a"), passive("b"), passive("c")], &[passive("d")]);
        let result = odds
            .all_desired_probability(&pool, &[passive("a"), passive("b")])
            .unwrap();
        assert_close(result, 0.25);
    }

    #[test]
    fn empty_desired_set_is_certain() {
        let odds = odds();
        for pool_sides in [(vec![], vec![]), (two_each()), (four_each())] {
            let pool = ParentPool::from_parents(&pool_sides.0, &pool_sides.1);
            assert_close(odds.all_desired_probability(&pool, &[]).unwrap(), 1.0);
        }
    }

    #[test]
    fn desired_validation_rejects_bad_inputs() {
        let odds = odds();
        let pool = ParentPool::from_parents(&[passive("a")], &[passive("b")]);
        assert_eq!(
            odds.all_desired_probability(&pool, &[passive("x")]),
            Err(DesiredPassivesError::NotInPool(passive("x")))
        );
        assert_eq!(
            odds.all_desired_probability(&pool, &[passive("a"), passive("a")]),
            Err(DesiredPassivesError::Duplicate(passive("a")))
        );
        let five = ["a", "b", "c", "d", "e"].map(passive);
        assert_eq!(
            odds.all_desired_probability(&pool, &five),
            Err(DesiredPassivesError::TooMany { count: 5 })
        );
    }

    #[test]
    fn parent_pool_deduplicates_across_parents() {
        let pool =
            ParentPool::from_parents(&[passive("a"), passive("b")], &[passive("b"), passive("c")]);
        assert_eq!(pool.len(), 3);
    }

    fn passive(name: &str) -> PassiveName {
        PassiveName::new(name)
    }

    fn two_each() -> (Vec<PassiveName>, Vec<PassiveName>) {
        (vec![passive("a")], vec![passive("b")])
    }

    fn four_each() -> (Vec<PassiveName>, Vec<PassiveName>) {
        (
            vec![passive("a"), passive("b"), passive("c"), passive("d")],
            vec![passive("e"), passive("f"), passive("g"), passive("h")],
        )
    }
}
