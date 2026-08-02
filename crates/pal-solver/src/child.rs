//! Child-species lookup: which pal a concrete male × female pairing
//! produces, expanded from the vendored combo list.

use std::collections::HashMap;
use std::collections::hash_map::Entry;

use pal_core::model::{BreedingDb, Gender, PalName};

/// A concrete breeding pairing. Palworld breeding always takes one
/// male and one female; the named fields carry that assignment, which
/// matters for the gender-dependent combos (e.g. Katress × Wixen
/// produces a different child per gender arrangement).
#[derive(Clone, PartialEq, Eq, Hash, Debug)]
pub struct BreedingPair {
    pub male: PalName,
    pub female: PalName,
}

#[derive(Debug, thiserror::Error)]
pub enum ChildIndexError {
    #[error("combos disagree for male {male} × female {female}: {first} vs {second}")]
    ConflictingCombos {
        male: PalName,
        female: PalName,
        first: PalName,
        second: PalName,
    },
}

/// Lookup from (male species, female species) to child species.
///
/// Built by expanding each vendored combo into the concrete gender
/// arrangements it covers: wildcard parents cover both assignments,
/// gender-locked parents exactly one.
#[derive(Clone, Debug)]
pub struct ChildIndex {
    by_male: HashMap<PalName, HashMap<PalName, PalName>>,
}

impl ChildIndex {
    /// # Errors
    ///
    /// Fails if two combos assign different children to the same
    /// concrete (male, female) pairing — a database defect.
    pub fn build(breeding: &BreedingDb) -> Result<Self, ChildIndexError> {
        let mut by_male: HashMap<PalName, HashMap<PalName, PalName>> = HashMap::new();
        for combo in breeding.combos() {
            let [first, second] = &combo.parents;
            for (male, female) in [(first, second), (second, first)] {
                if !(male.gender.accepts(Gender::Male) && female.gender.accepts(Gender::Female)) {
                    continue;
                }
                match by_male
                    .entry(male.name.clone())
                    .or_default()
                    .entry(female.name.clone())
                {
                    Entry::Vacant(slot) => {
                        slot.insert(combo.child.clone());
                    }
                    Entry::Occupied(existing) if *existing.get() != combo.child => {
                        return Err(ChildIndexError::ConflictingCombos {
                            male: male.name.clone(),
                            female: female.name.clone(),
                            first: existing.get().clone(),
                            second: combo.child.clone(),
                        });
                    }
                    Entry::Occupied(_) => {}
                }
            }
        }
        Ok(Self { by_male })
    }

    #[must_use]
    pub fn child_of(&self, pair: &BreedingPair) -> Option<&PalName> {
        self.by_male.get(&pair.male)?.get(&pair.female)
    }
}
