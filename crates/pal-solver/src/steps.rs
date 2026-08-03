//! Species-level reachability: how many breeding steps separate two
//! species, assuming arbitrary partners at every step.
//!
//! Upstream ships this precomputed (`MinBreedingSteps` in
//! `breeding.json`); computing it ourselves from [`ChildIndex`] gives
//! the search a pruning heuristic and gives the tests a full-matrix
//! parity oracle against the vendored table.

use std::collections::{HashMap, VecDeque};

use pal_core::model::{PalDb, PalName};

use crate::child::ChildIndex;

/// For each species, the distinct child species reachable in one
/// breeding step with any partner (either gender arrangement).
#[derive(Clone, Debug)]
pub struct SpeciesAdjacency {
    children: HashMap<PalName, Vec<PalName>>,
}

impl SpeciesAdjacency {
    #[must_use]
    pub fn build(pal_db: &PalDb, index: &ChildIndex) -> Self {
        let children = pal_db
            .pals()
            .map(|species| {
                let mut reachable: Vec<PalName> = Vec::new();
                for partner in pal_db.pals() {
                    for (male, female) in [(species, partner), (partner, species)] {
                        if let Some(child) = index.child_between(&male.name, &female.name)
                            && !reachable.contains(child)
                        {
                            reachable.push(child.clone());
                        }
                    }
                }
                (species.name.clone(), reachable)
            })
            .collect();
        Self { children }
    }

    #[must_use]
    pub fn children_of(&self, species: &PalName) -> &[PalName] {
        self.children.get(species).map_or(&[], Vec::as_slice)
    }

    /// Minimum breeding steps from every species to `goal` (breadth-
    /// first search over reversed edges). Species that cannot reach
    /// `goal` are absent.
    #[must_use]
    pub fn distances_to(&self, goal: &PalName) -> HashMap<PalName, u32> {
        let mut parents_of: HashMap<&PalName, Vec<&PalName>> = HashMap::new();
        for (parent, children) in &self.children {
            for child in children {
                parents_of.entry(child).or_default().push(parent);
            }
        }

        let mut distances = HashMap::from([(goal.clone(), 0)]);
        let mut frontier = VecDeque::from([goal]);
        while let Some(species) = frontier.pop_front() {
            let next_distance = distances[species] + 1;
            for parent in parents_of.get(species).into_iter().flatten() {
                if !distances.contains_key(*parent) {
                    distances.insert((*parent).clone(), next_distance);
                    frontier.push_back(*parent);
                }
            }
        }
        distances
    }
}

/// The full species × species minimum-steps matrix, the shape of the
/// vendored `MinBreedingSteps` table.
#[derive(Clone, Debug)]
pub struct MinStepsTable {
    to_goal: HashMap<PalName, HashMap<PalName, u32>>,
}

impl MinStepsTable {
    #[must_use]
    pub fn compute(adjacency: &SpeciesAdjacency) -> Self {
        let to_goal = adjacency
            .children
            .keys()
            .map(|goal| {
                let from_each = adjacency.distances_to(goal);
                (goal.clone(), from_each)
            })
            .collect();
        Self { to_goal }
    }

    /// Minimum breeding steps from `from` to `to`; `None` when `to` is
    /// unreachable from `from`.
    #[must_use]
    pub fn steps_between(&self, from: &PalName, to: &PalName) -> Option<u32> {
        self.to_goal.get(to)?.get(from).copied()
    }
}
