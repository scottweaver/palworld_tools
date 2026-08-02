//! Multi-step breeding-path search over an owned-pal pool.
//!
//! A simplified take on palcalc's `BreedingSolver`: candidate pals
//! (owned, then bred intermediates) are pairwise combined for up to
//! `max_breeding_steps` rounds, beam-pruned per (species,
//! carried-passives) state and by species reachability, and ranked by
//! **expected eggs** — the expected number of breeding attempts
//! across the whole plan.
//!
//! Cost model, per bred node: one egg succeeds when the child rolls
//! every passive the node must carry (see [`crate::passives`]) and,
//! when the node is later used as a gendered parent, the required
//! gender. Bred intermediates are re-bred until they succeed, so
//! obtaining one costs `parents' cost + 1 / (P(passives) · P(gender))`
//! expected eggs; owned pals cost nothing. Simplifications versus
//! palcalc: bred parents contribute exactly their carried passives to
//! the child's inheritance pool, effort is measured in eggs rather
//! than wall-clock time, and wild-pal capture is not modeled.

use std::collections::HashMap;

use pal_core::model::{Gender, PalDb, PalName, PassiveName};

use crate::child::{BreedingPair, ChildIndex};
use crate::passives::{MAX_TOTAL_PASSIVES, PassiveOdds};
use crate::steps::SpeciesAdjacency;

/// A pal the player already has: the leaf material of every plan.
#[derive(Clone, PartialEq, Eq, Debug)]
pub struct OwnedPal {
    pub species: PalName,
    pub gender: Gender,
    pub passives: Vec<PassiveName>,
}

/// What the search is for: a species, carrying all listed passives.
#[derive(Clone, PartialEq, Eq, Debug)]
pub struct BreedingGoal {
    pub species: PalName,
    pub passives: Vec<PassiveName>,
}

#[derive(Clone, Copy, Debug)]
pub struct SearchConfig {
    /// Upper bound on bred nodes in a single plan.
    pub max_breeding_steps: usize,
    /// Ranked plans to return; also the per-state beam width kept
    /// during expansion.
    pub max_results: usize,
}

/// One node of a finished plan: an owned pal, or a breeding step
/// whose parents are themselves plan nodes.
#[derive(Clone, PartialEq, Eq, Debug)]
pub enum PlanNode {
    Owned(OwnedPal),
    Bred(Box<BredNode>),
}

/// A breeding step: pair `male` × `female`, re-hatching until the
/// child carries `carried_passives`.
#[derive(Clone, PartialEq, Eq, Debug)]
pub struct BredNode {
    pub male: PlanNode,
    pub female: PlanNode,
    pub species: PalName,
    pub carried_passives: Vec<PassiveName>,
}

impl PlanNode {
    #[must_use]
    pub fn species(&self) -> &PalName {
        match self {
            Self::Owned(pal) => &pal.species,
            Self::Bred(node) => &node.species,
        }
    }
}

#[derive(Clone, Debug)]
pub struct BreedingPlan {
    pub root: PlanNode,
    /// Expected eggs across every breeding step of the plan; 0 for a
    /// plan satisfied by an owned pal.
    pub expected_eggs: f64,
    /// Number of bred nodes in the plan.
    pub steps: usize,
}

#[derive(Debug, PartialEq, Eq, thiserror::Error)]
pub enum SearchError {
    #[error("goal species {0} is not in the database")]
    UnknownGoalSpecies(PalName),
    #[error("owned pal species {0} is not in the database")]
    UnknownOwnedSpecies(PalName),
    #[error("desired passive {0} listed more than once")]
    DuplicateDesired(PassiveName),
    #[error("{count} desired passives exceed the {MAX_TOTAL_PASSIVES}-passive cap")]
    TooManyDesired { count: usize },
}

/// Finds breeding plans producing `goal` from `owned`, ranked by
/// expected eggs (ascending). Returns an empty list when the goal is
/// unreachable within `config.max_breeding_steps`.
///
/// `index` must be built from the same database generation as
/// `pal_db`; species the index yields but `pal_db` lacks are skipped.
///
/// # Errors
///
/// Fails when the goal or an owned pal names an unknown species, or
/// when the goal's passive list has duplicates or exceeds the cap.
pub fn find_paths(
    pal_db: &PalDb,
    index: &ChildIndex,
    odds: &PassiveOdds,
    owned: &[OwnedPal],
    goal: &BreedingGoal,
    config: &SearchConfig,
) -> Result<Vec<BreedingPlan>, SearchError> {
    validate(pal_db, owned, goal)?;
    if config.max_results == 0 {
        return Ok(Vec::new());
    }

    let adjacency = SpeciesAdjacency::build(pal_db, index);
    let distance_to_goal = adjacency.distances_to(&goal.species);

    let mut working: Vec<Candidate> = owned
        .iter()
        .map(|pal| Candidate::owned(pal, &goal.passives))
        .collect();

    for _ in 0..config.max_breeding_steps {
        let round_children = expand_round(
            pal_db,
            index,
            odds,
            &working,
            goal,
            &distance_to_goal,
            config,
        );
        let mut added = false;
        for child in round_children {
            added |= insert_pruned(&mut working, child, config.max_results);
        }
        if !added {
            break;
        }
    }

    let full = DesiredMask::full(goal.passives.len());
    let mut plans: Vec<BreedingPlan> = working
        .into_iter()
        .filter(|candidate| candidate.species == goal.species && candidate.carried == full)
        .map(|candidate| BreedingPlan {
            expected_eggs: candidate.root_cost(),
            steps: candidate.bred_count,
            root: candidate.node,
        })
        .collect();
    plans.sort_by(|a, b| {
        a.expected_eggs
            .total_cmp(&b.expected_eggs)
            .then(a.steps.cmp(&b.steps))
    });
    plans.truncate(config.max_results);
    Ok(plans)
}

fn validate(pal_db: &PalDb, owned: &[OwnedPal], goal: &BreedingGoal) -> Result<(), SearchError> {
    if pal_db.pal(&goal.species).is_none() {
        return Err(SearchError::UnknownGoalSpecies(goal.species.clone()));
    }
    for pal in owned {
        if pal_db.pal(&pal.species).is_none() {
            return Err(SearchError::UnknownOwnedSpecies(pal.species.clone()));
        }
    }
    if goal.passives.len() > MAX_TOTAL_PASSIVES {
        return Err(SearchError::TooManyDesired {
            count: goal.passives.len(),
        });
    }
    for (position, passive) in goal.passives.iter().enumerate() {
        if goal.passives[..position].contains(passive) {
            return Err(SearchError::DuplicateDesired(passive.clone()));
        }
    }
    Ok(())
}

/// One expansion round: every arrangement of every candidate pair,
/// in deterministic order. Candidates bred this round only become
/// pairable next round.
fn expand_round(
    pal_db: &PalDb,
    index: &ChildIndex,
    odds: &PassiveOdds,
    working: &[Candidate],
    goal: &BreedingGoal,
    distance_to_goal: &HashMap<PalName, u32>,
    config: &SearchConfig,
) -> Vec<Candidate> {
    let context = BreedContext {
        pal_db,
        index,
        odds,
        goal,
        distance_to_goal,
        config,
    };
    let order = expansion_order(working);
    let mut children = Vec::new();
    for (slot, &first) in order.iter().enumerate() {
        for &second in &order[slot..] {
            let both = [(first, second), (second, first)];
            let arrangements = if first == second {
                &both[..1]
            } else {
                &both[..]
            };
            for &(male, female) in arrangements {
                if let Some(child) = context.breed(&working[male], &working[female]) {
                    children.push(child);
                }
            }
        }
    }
    children
}

/// Subset of the goal's passive list, one bit per desired passive.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
struct DesiredMask(u8);

impl DesiredMask {
    /// Callers guarantee `desired_len <= MAX_TOTAL_PASSIVES` (enforced
    /// by [`validate`]).
    fn full(desired_len: usize) -> Self {
        Self((1u8 << desired_len) - 1)
    }

    fn of(desired: &[PassiveName], passives: &[PassiveName]) -> Self {
        let mut bits = 0u8;
        for (position, passive) in desired.iter().enumerate() {
            if passives.contains(passive) {
                bits |= 1 << position;
            }
        }
        Self(bits)
    }

    fn union(self, other: Self) -> Self {
        Self(self.0 | other.0)
    }

    fn names(self, desired: &[PassiveName]) -> Vec<PassiveName> {
        desired
            .iter()
            .enumerate()
            .filter(|(position, _)| self.0 & (1 << position) != 0)
            .map(|(_, passive)| passive.clone())
            .collect()
    }

    fn count(self) -> usize {
        usize::try_from(self.0.count_ones()).expect("a u8 has at most 8 set bits")
    }
}

#[derive(Clone, Debug)]
enum GenderAvailability {
    /// An owned individual's fixed gender.
    Fixed(Gender),
    /// A bred pal can be re-hatched to either gender at egg cost.
    Flexible { male: f64, female: f64 },
}

#[derive(Clone, Debug)]
struct Candidate {
    node: PlanNode,
    species: PalName,
    gender: GenderAvailability,
    carried: DesiredMask,
    /// Distinct passives this candidate contributes to a child's
    /// inheritance pool: everything an owned pal has, exactly the
    /// carried set for a bred pal.
    contribution: Vec<PassiveName>,
    /// Expected eggs spent producing the parents (0 for owned).
    parents_cost: f64,
    /// P(one egg carries this node's passives); 1 for owned.
    egg_p: f64,
    bred_count: usize,
}

impl Candidate {
    fn owned(pal: &OwnedPal, desired: &[PassiveName]) -> Self {
        Self {
            node: PlanNode::Owned(pal.clone()),
            species: pal.species.clone(),
            gender: GenderAvailability::Fixed(pal.gender),
            carried: DesiredMask::of(desired, &pal.passives),
            contribution: distinct(&pal.passives),
            parents_cost: 0.0,
            egg_p: 1.0,
            bred_count: 0,
        }
    }

    /// Expected eggs to obtain this candidate with the given gender;
    /// `None` when impossible (wrong fixed gender, zero probability).
    fn cost_as(&self, required: Gender) -> Option<f64> {
        match &self.gender {
            GenderAvailability::Fixed(gender) => (*gender == required).then_some(0.0),
            GenderAvailability::Flexible { male, female } => {
                let gender_p = match required {
                    Gender::Male => *male,
                    Gender::Female => *female,
                };
                (gender_p > 0.0).then(|| self.parents_cost + 1.0 / (self.egg_p * gender_p))
            }
        }
    }

    /// Expected eggs when the candidate's own gender is irrelevant
    /// (the plan root).
    fn root_cost(&self) -> f64 {
        if self.bred_count == 0 {
            0.0
        } else {
            self.parents_cost + 1.0 / self.egg_p
        }
    }
}

struct BreedContext<'a> {
    pal_db: &'a PalDb,
    index: &'a ChildIndex,
    odds: &'a PassiveOdds,
    goal: &'a BreedingGoal,
    distance_to_goal: &'a HashMap<PalName, u32>,
    config: &'a SearchConfig,
}

impl BreedContext<'_> {
    /// Attempts one breeding arrangement; `None` when it is
    /// impossible, over the step budget, or cannot reach the goal
    /// species in the remaining steps.
    fn breed(&self, male: &Candidate, female: &Candidate) -> Option<Candidate> {
        let bred_count = male.bred_count + female.bred_count + 1;
        if bred_count > self.config.max_breeding_steps {
            return None;
        }

        let male_cost = male.cost_as(Gender::Male)?;
        let female_cost = female.cost_as(Gender::Female)?;

        let child = self.index.child_of(&BreedingPair {
            male: male.species.clone(),
            female: female.species.clone(),
        })?;
        let remaining = self.config.max_breeding_steps - bred_count;
        let distance = *self.distance_to_goal.get(child)?;
        if usize::try_from(distance).map_or(true, |steps| steps > remaining) {
            return None;
        }
        let child_pal = self.pal_db.pal(child)?;

        let carried = male.carried.union(female.carried);
        let pool = merged_pool(&male.contribution, &female.contribution);
        let egg_p: f64 = (carried.count()..=MAX_TOTAL_PASSIVES)
            .map(|num_final| {
                self.odds
                    .exact_total_probability(pool.len(), carried.count(), num_final)
            })
            .sum();
        if egg_p <= 0.0 {
            return None;
        }

        let carried_passives = carried.names(&self.goal.passives);
        Some(Candidate {
            node: PlanNode::Bred(Box::new(BredNode {
                male: male.node.clone(),
                female: female.node.clone(),
                species: child.clone(),
                carried_passives: carried_passives.clone(),
            })),
            species: child.clone(),
            gender: GenderAvailability::Flexible {
                male: child_pal.gender_probability.of(Gender::Male),
                female: child_pal.gender_probability.of(Gender::Female),
            },
            carried,
            contribution: carried_passives,
            parents_cost: male_cost + female_cost,
            egg_p,
            bred_count,
        })
    }
}

/// Deterministic expansion order: candidate indexes sorted by state
/// key and cost, so results never depend on insertion order.
fn expansion_order(working: &[Candidate]) -> Vec<usize> {
    let mut order: Vec<usize> = (0..working.len()).collect();
    order.sort_by(|&a, &b| {
        let (a, b) = (&working[a], &working[b]);
        a.species
            .as_str()
            .cmp(b.species.as_str())
            .then(a.carried.0.cmp(&b.carried.0))
            .then(a.bred_count.cmp(&b.bred_count))
            .then(a.root_cost().total_cmp(&b.root_cost()))
    });
    order
}

/// Beam-prunes into the working set: keeps at most `beam` bred
/// candidates per (species, carried) state, best root-cost first.
/// Owned candidates are never pruned. Returns whether the candidate
/// was kept.
fn insert_pruned(working: &mut Vec<Candidate>, candidate: Candidate, beam: usize) -> bool {
    let same_state: Vec<usize> = working
        .iter()
        .enumerate()
        .filter(|(_, existing)| {
            existing.bred_count > 0
                && existing.species == candidate.species
                && existing.carried == candidate.carried
        })
        .map(|(position, _)| position)
        .collect();

    if same_state.len() < beam {
        working.push(candidate);
        return true;
    }
    let worst = same_state
        .into_iter()
        .max_by(|&a, &b| working[a].root_cost().total_cmp(&working[b].root_cost()))
        .expect("same_state has at least `beam` (>= 1) entries");
    if candidate.root_cost() < working[worst].root_cost() {
        working[worst] = candidate;
        return true;
    }
    false
}

fn merged_pool(first: &[PassiveName], second: &[PassiveName]) -> Vec<PassiveName> {
    let mut pool = first.to_vec();
    for passive in second {
        if !pool.contains(passive) {
            pool.push(passive.clone());
        }
    }
    pool
}

fn distinct(passives: &[PassiveName]) -> Vec<PassiveName> {
    let mut seen = Vec::new();
    for passive in passives {
        if !seen.contains(passive) {
            seen.push(passive.clone());
        }
    }
    seen
}
