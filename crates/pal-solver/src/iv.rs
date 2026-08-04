//! IV-threshold inheritance probability for a single breeding pair.
//!
//! Ported term-for-term from palcalc — the desired-count table from
//! `PalCalc.Model/BreedingMechanics.cs` (`BuildDesiredIVProbabilities`)
//! and the per-pair factor from `PalCalc.Solver/Probabilities/IVs.cs`
//! — so results parity-match upstream for the same inputs.
//!
//! The model: a child inherits 1–3 of its three IV stats from the
//! parents (weighted roll, uniform choice among stat combinations);
//! each inherited stat copies a uniformly random parent's value.
//! A desired stat is *met* by a parent when that parent's value
//! reaches the goal's minimum; a stat neither parent meets cannot be
//! met by the child at all (random rolls are ignored, matching
//! upstream), so per-pair probability concerns only supplyable stats.

use pal_core::model::{BreedingMechanics, IvSpread, IvStat, IvValue};

/// A pal has this many breeding-relevant IV stats.
pub const IV_STAT_COUNT: usize = 3;

/// Per-stat minimums the goal pal must meet; `None` = don't care.
#[derive(Clone, Copy, PartialEq, Eq, Debug, Default, serde::Serialize, serde::Deserialize)]
pub struct IvThresholds {
    pub hp: Option<IvValue>,
    pub attack: Option<IvValue>,
    pub defense: Option<IvValue>,
}

impl IvThresholds {
    #[must_use]
    pub fn get(self, stat: IvStat) -> Option<IvValue> {
        match stat {
            IvStat::Hp => self.hp,
            IvStat::Attack => self.attack,
            IvStat::Defense => self.defense,
        }
    }

    /// The active requirements, in [`IvStat::ALL`] order.
    #[must_use]
    pub fn active(self) -> Vec<(IvStat, IvValue)> {
        IvStat::ALL
            .into_iter()
            .filter_map(|stat| self.get(stat).map(|minimum| (stat, minimum)))
            .collect()
    }

    #[must_use]
    pub fn is_empty(self) -> bool {
        self.hp.is_none() && self.attack.is_none() && self.defense.is_none()
    }

    /// Whether `ivs` meets every active minimum.
    #[must_use]
    pub fn met_by(self, ivs: IvSpread) -> bool {
        self.active()
            .into_iter()
            .all(|(stat, minimum)| ivs.get(stat) >= minimum)
    }
}

/// Probability table derived from the database's IV inheritance
/// weights.
#[derive(Clone, Debug)]
pub struct IvOdds {
    /// P(the inherited stat categories include `n` specific desired
    /// stats), indexed by `n - 1`.
    desired: [f64; IV_STAT_COUNT],
}

#[derive(Debug, PartialEq, Eq, thiserror::Error)]
pub enum IvOddsError {
    #[error("IV inheritance weights sum to zero; cannot normalize")]
    ZeroWeightSum,
}

impl IvOdds {
    /// # Errors
    ///
    /// Fails if the IV inheritance weights in `mechanics` sum to zero.
    pub fn from_mechanics(mechanics: &BreedingMechanics) -> Result<Self, IvOddsError> {
        let weights = &mechanics.iv_inheritance_weights;
        let sum: u32 = weights.values().sum();
        if sum == 0 {
            return Err(IvOddsError::ZeroWeightSum);
        }
        let mut direct = [0.0; IV_STAT_COUNT];
        for (slot, count) in direct.iter_mut().zip(1u8..) {
            *slot = f64::from(weights.get(&count).copied().unwrap_or(0)) / f64::from(sum);
        }

        let mut desired = [0.0; IV_STAT_COUNT];
        for (inherited_index, direct_p) in direct.iter().enumerate() {
            for (desired_index, slot) in desired.iter_mut().enumerate() {
                *slot += direct_p * contained_probability(inherited_index + 1, desired_index + 1);
            }
        }
        Ok(Self { desired })
    }

    /// Probability that one egg inherits every supplyable desired
    /// stat: `single_met` stats are met by exactly one parent (extra
    /// right-parent coin flip each), `both_met` by both. Certain when
    /// nothing is supplyable.
    #[must_use]
    pub fn pair_probability(&self, single_met: usize, both_met: usize) -> f64 {
        let required = single_met + both_met;
        if required == 0 {
            return 1.0;
        }
        (0..single_met).fold(self.desired[required - 1], |probability, _| {
            probability * 0.5
        })
    }
}

/// P(a specific `desired`-stat set is contained in a uniformly random
/// `inherited`-stat combination): `C(3-d, k-d) / C(3, k)`. Matches
/// upstream's hard-coded combinations table.
fn contained_probability(inherited: usize, desired: usize) -> f64 {
    if desired > inherited {
        return 0.0;
    }
    choose(IV_STAT_COUNT - desired, inherited - desired) / choose(IV_STAT_COUNT, inherited)
}

#[expect(
    clippy::cast_precision_loss,
    reason = "operands are stat counts (<= 3); exactly representable in f64"
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
    use std::collections::BTreeMap;

    fn v27_mechanics() -> BreedingMechanics {
        BreedingMechanics {
            iv_inheritance_weights: BTreeMap::from([(1, 3), (2, 2), (3, 1)]),
            passive_inheritance_weights: BTreeMap::from([(1, 4), (2, 3), (3, 2), (4, 1)]),
            passive_random_weights: BTreeMap::from([(0, 4), (1, 3), (2, 2), (3, 1)]),
        }
    }

    fn odds() -> IvOdds {
        IvOdds::from_mechanics(&v27_mechanics()).unwrap()
    }

    fn assert_close(actual: f64, expected: f64) {
        assert!(
            (actual - expected).abs() < 1e-9,
            "expected {expected}, got {actual}"
        );
    }

    #[test]
    fn desired_table_matches_upstream_derivation() {
        // Weights 3:2:1 normalize to 1/2, 1/3, 1/6; folded through the
        // combinations table (upstream BuildDesiredIVProbabilities):
        //   d=1: 1/2·1/3 + 1/3·2/3 + 1/6·1 = 5/9
        //   d=2:           1/3·1/3 + 1/6·1 = 5/18
        //   d=3:                     1/6·1 = 1/6
        let odds = odds();
        assert_close(odds.desired[0], 5.0 / 9.0);
        assert_close(odds.desired[1], 5.0 / 18.0);
        assert_close(odds.desired[2], 1.0 / 6.0);
    }

    #[test]
    fn pair_probability_matches_hand_computed_vectors() {
        let odds = odds();
        let vectors = [
            (0, 0, 1.0),
            (0, 1, 5.0 / 9.0),
            (1, 0, 5.0 / 18.0),
            (0, 2, 5.0 / 18.0),
            (1, 1, 5.0 / 36.0),
            (2, 0, 5.0 / 72.0),
            (0, 3, 1.0 / 6.0),
            (3, 0, 1.0 / 48.0),
        ];
        for (single, both, expected) in vectors {
            assert_close(odds.pair_probability(single, both), expected);
        }
    }

    #[test]
    fn zero_weights_are_rejected() {
        let mut mechanics = v27_mechanics();
        mechanics.iv_inheritance_weights = BTreeMap::new();
        assert_eq!(
            IvOdds::from_mechanics(&mechanics).unwrap_err(),
            IvOddsError::ZeroWeightSum
        );
    }

    #[test]
    fn thresholds_report_active_stats_and_membership() {
        let thresholds = IvThresholds {
            hp: None,
            attack: Some(IvValue::try_from(90).unwrap()),
            defense: Some(IvValue::try_from(70).unwrap()),
        };
        assert_eq!(thresholds.active().len(), 2);
        assert!(!thresholds.is_empty());
        assert!(IvThresholds::default().is_empty());

        let good = IvSpread {
            hp: IvValue::default(),
            attack: IvValue::try_from(95).unwrap(),
            defense: IvValue::try_from(70).unwrap(),
        };
        let bad = IvSpread {
            attack: IvValue::try_from(95).unwrap(),
            ..IvSpread::default()
        };
        assert!(thresholds.met_by(good));
        assert!(!thresholds.met_by(bad));
    }
}
