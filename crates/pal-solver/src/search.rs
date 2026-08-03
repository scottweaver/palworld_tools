//! Multi-step breeding-path search over an owned-pal pool.
//!
//! A simplified take on palcalc's `BreedingSolver`: candidate pals
//! (owned, then bred intermediates) are pairwise combined for up to
//! `max_breeding_steps` rounds, beam-pruned per (species,
//! carried-passives, required-progenitors) state and by species
//! reachability, and ranked by **expected eggs** — the expected
//! number of breeding attempts across the whole plan.
//!
//! Cost model, per bred node: one egg succeeds when the child rolls
//! every passive the node must carry (see [`crate::passives`]) and,
//! when the node is later used as a gendered parent, the required
//! gender. Bred intermediates are re-bred until they succeed, so
//! obtaining one costs `parents' cost + 1 / (P(passives) · P(gender))`
//! expected eggs; owned pals cost nothing. Goals may name required
//! progenitors: candidate state then also tracks which progenitors a
//! tree includes, and only fully-anchored plans are returned.
//! Simplifications versus palcalc: bred parents contribute exactly
//! their carried passives to the child's inheritance pool, effort is
//! measured in eggs rather than wall-clock time, and capture effort
//! is not modeled.
//!
//! Construct a [`Solver`] once per session: it precomputes the
//! species adjacency and memoizes per-goal distance maps, so repeated
//! searches skip that setup. Internally the search works on an
//! append-only arena of candidate records — plan trees are only
//! materialized for the returned results — expands one frontier per
//! round (pairs where at least one side is new; older pairs already
//! produced their children), enumerates pairs species-group-first so
//! unreachable combinations are rejected wholesale, and fans the
//! group work out with rayon.

use std::collections::HashMap;
use std::sync::{Arc, Mutex};

use pal_core::model::{Gender, Pal, PalDb, PalName, PassiveName};
use rayon::prelude::*;

use crate::child::ChildIndex;
use crate::passives::{MAX_TOTAL_PASSIVES, PassiveOdds};
use crate::steps::SpeciesAdjacency;

/// A pal the player already has: the leaf material of every plan.
#[derive(Clone, PartialEq, Eq, Debug)]
pub struct OwnedPal {
    pub species: PalName,
    pub gender: Gender,
    pub passives: Vec<PassiveName>,
}

/// What the search is for: a species, carrying all listed passives,
/// bred from all listed progenitors.
#[derive(Clone, PartialEq, Eq, Debug)]
pub struct BreedingGoal {
    pub species: PalName,
    pub passives: Vec<PassiveName>,
    /// Required anchor species: every returned plan must include each
    /// of these as a leaf at least once. Progenitors are available as
    /// free any-gender leaves regardless of wild spawns (the caller
    /// has them), and carry no passives.
    pub progenitors: Vec<PalName>,
}

#[derive(Clone, Copy, Debug)]
pub struct SearchConfig {
    /// Upper bound on bred nodes in a single plan.
    pub max_breeding_steps: usize,
    /// Ranked plans to return; also the per-state beam width kept
    /// during expansion.
    pub max_results: usize,
    /// When set, every species with wild spawns
    /// ([`pal_core::model::Pal::wild_levels`]) is available as a
    /// free-capture leaf: any gender at zero egg cost, contributing
    /// no passives. Capture effort itself is not yet modeled.
    pub allow_wild_pals: bool,
}

/// One node of a finished plan: an owned pal, a wild pal to catch, a
/// required progenitor (the pairing position implies the gender to
/// use), or a breeding step whose parents are themselves plan nodes.
#[derive(Clone, PartialEq, Eq, Debug)]
pub enum PlanNode {
    Owned(OwnedPal),
    Wild(PalName),
    Progenitor(PalName),
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
            Self::Wild(species) | Self::Progenitor(species) => species,
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

/// Most progenitors a goal may require (mask width).
pub const MAX_PROGENITORS: usize = 8;

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
    #[error("progenitor species {0} is not in the database")]
    UnknownProgenitor(PalName),
    #[error("progenitor {0} listed more than once")]
    DuplicateProgenitor(PalName),
    #[error("{count} progenitors exceed the cap of {MAX_PROGENITORS}")]
    TooManyProgenitors { count: usize },
}

/// Reusable search context: build once, search many times. Holds the
/// precomputed species adjacency and a per-goal distance cache.
pub struct Solver<'db> {
    pal_db: &'db PalDb,
    index: &'db ChildIndex,
    odds: &'db PassiveOdds,
    adjacency: SpeciesAdjacency,
    distance_cache: Mutex<HashMap<PalName, Arc<HashMap<PalName, u32>>>>,
}

impl<'db> Solver<'db> {
    /// `index` must be built from the same database generation as
    /// `pal_db`; species the index yields but `pal_db` lacks are
    /// skipped.
    #[must_use]
    pub fn new(pal_db: &'db PalDb, index: &'db ChildIndex, odds: &'db PassiveOdds) -> Self {
        Self {
            pal_db,
            index,
            odds,
            adjacency: SpeciesAdjacency::build(pal_db, index),
            distance_cache: Mutex::new(HashMap::new()),
        }
    }

    /// The database this solver searches over.
    #[must_use]
    pub fn pal_db(&self) -> &'db PalDb {
        self.pal_db
    }

    fn distances_to(&self, goal: &PalName) -> Arc<HashMap<PalName, u32>> {
        let mut cache = self
            .distance_cache
            .lock()
            .expect("poisoned only if a panicking thread held the cache lock");
        cache
            .entry(goal.clone())
            .or_insert_with(|| Arc::new(self.adjacency.distances_to(goal)))
            .clone()
    }

    /// Finds breeding plans producing `goal` from `owned`, ranked by
    /// expected eggs (ascending). Returns an empty list when the goal
    /// is unreachable within `config.max_breeding_steps`.
    ///
    /// # Errors
    ///
    /// Fails when the goal, an owned pal, or a progenitor names an
    /// unknown species, or when the goal's passive or progenitor
    /// lists have duplicates or exceed their caps.
    pub fn find_paths(
        &self,
        owned: &[OwnedPal],
        goal: &BreedingGoal,
        config: &SearchConfig,
    ) -> Result<Vec<BreedingPlan>, SearchError> {
        validate(self.pal_db, owned, goal)?;
        if config.max_results == 0 {
            return Ok(Vec::new());
        }
        let distance_to_goal = self.distances_to(&goal.species);

        let mut arena: Vec<Record> = Vec::new();
        seed_leaves(
            &mut arena,
            self.pal_db,
            owned,
            goal,
            &distance_to_goal,
            config,
        );
        let leaves: Vec<RecordId> = (0..arena.len()).map(RecordId).collect();
        let mut buckets: HashMap<BredState, Vec<RecordId>> = HashMap::new();
        let mut frontier = leaves.clone();

        for _ in 0..config.max_breeding_steps {
            let children = expand_frontier(
                ExpandContext {
                    pal_db: self.pal_db,
                    index: self.index,
                    odds: self.odds,
                    goal,
                    distance_to_goal: &distance_to_goal,
                    config,
                },
                &arena,
                &leaves,
                &buckets,
                &frontier,
            );
            frontier.clear();
            for child in children {
                if let Some(id) = insert_pruned(&mut arena, &mut buckets, child, config.max_results)
                {
                    frontier.push(id);
                }
            }
            if frontier.is_empty() {
                break;
            }
        }

        let full = DesiredMask::full(goal.passives.len());
        let all_progenitors = DesiredMask::full(goal.progenitors.len());
        let mut plans: Vec<BreedingPlan> = leaves
            .iter()
            .chain(buckets.values().flatten())
            .filter(|id| {
                let record = &arena[id.0];
                record.species == goal.species
                    && record.carried == full
                    && record.required == all_progenitors
            })
            .map(|id| {
                let record = &arena[id.0];
                BreedingPlan {
                    root: materialize(&arena, owned, *id),
                    expected_eggs: record.root_cost(),
                    steps: record.bred_count,
                }
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
}

/// One-shot convenience over [`Solver`]. Prefer holding a `Solver`
/// when searching more than once: this rebuilds the adjacency every
/// call.
///
/// # Errors
///
/// Same failure modes as [`Solver::find_paths`].
pub fn find_paths(
    pal_db: &PalDb,
    index: &ChildIndex,
    odds: &PassiveOdds,
    owned: &[OwnedPal],
    goal: &BreedingGoal,
    config: &SearchConfig,
) -> Result<Vec<BreedingPlan>, SearchError> {
    Solver::new(pal_db, index, odds).find_paths(owned, goal, config)
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
    if goal.progenitors.len() > MAX_PROGENITORS {
        return Err(SearchError::TooManyProgenitors {
            count: goal.progenitors.len(),
        });
    }
    for (position, progenitor) in goal.progenitors.iter().enumerate() {
        if pal_db.pal(progenitor).is_none() {
            return Err(SearchError::UnknownProgenitor(progenitor.clone()));
        }
        if goal.progenitors[..position].contains(progenitor) {
            return Err(SearchError::DuplicateProgenitor(progenitor.clone()));
        }
    }
    Ok(())
}

/// Subset of a small goal list (desired passives, required
/// progenitors), one bit per entry.
#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
struct DesiredMask(u8);

impl DesiredMask {
    /// Callers guarantee the list length fits the mask (enforced by
    /// [`validate`]: passives <= `MAX_TOTAL_PASSIVES`, progenitors <=
    /// `MAX_PROGENITORS`).
    fn full(desired_len: usize) -> Self {
        if desired_len == 0 {
            Self(0)
        } else {
            Self(u8::MAX >> (8 - desired_len))
        }
    }

    fn empty() -> Self {
        Self(0)
    }

    fn single(position: usize) -> Self {
        Self(1 << position)
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
    /// A wild pal or progenitor: obtain whichever gender is needed at
    /// no egg cost.
    AnyFree,
}

/// Index into the search arena. Records are append-only, so an id
/// stays valid for the whole search even after beam replacement drops
/// it from a bucket.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
struct RecordId(usize);

/// How a record entered the search — enough to rebuild its
/// [`PlanNode`] tree at materialization time.
#[derive(Clone, Copy, Debug)]
enum Source {
    Owned(usize),
    Wild,
    Progenitor,
    Bred { male: RecordId, female: RecordId },
}

/// Beam-pruning state: bred candidates compete within their
/// (species, carried, required) bucket.
type BredState = (PalName, DesiredMask, DesiredMask);

#[derive(Clone, Debug)]
struct Record {
    source: Source,
    species: PalName,
    gender: GenderAvailability,
    carried: DesiredMask,
    required: DesiredMask,
    /// Distinct passives this candidate contributes to a child's
    /// inheritance pool: everything an owned pal has, exactly the
    /// carried set for a bred pal, nothing for wilds and progenitors.
    contribution: Vec<PassiveName>,
    /// Expected eggs spent producing the parents (0 for leaves).
    parents_cost: f64,
    /// P(one egg carries this node's passives); 1 for leaves.
    egg_p: f64,
    bred_count: usize,
}

impl Record {
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
            GenderAvailability::AnyFree => Some(0.0),
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

fn leaf_record(source: Source, species: PalName, gender: GenderAvailability) -> Record {
    Record {
        source,
        species,
        gender,
        carried: DesiredMask::empty(),
        required: DesiredMask::empty(),
        contribution: Vec::new(),
        parents_cost: 0.0,
        egg_p: 1.0,
        bred_count: 0,
    }
}

/// Seeds the arena: owned pals, then progenitors, then (when enabled)
/// wild-spawning species that can still reach the goal, in
/// deterministic name order.
fn seed_leaves(
    arena: &mut Vec<Record>,
    pal_db: &PalDb,
    owned: &[OwnedPal],
    goal: &BreedingGoal,
    distance_to_goal: &HashMap<PalName, u32>,
    config: &SearchConfig,
) {
    arena.extend(owned.iter().enumerate().map(|(position, pal)| Record {
        carried: DesiredMask::of(&goal.passives, &pal.passives),
        contribution: distinct(&pal.passives),
        ..leaf_record(
            Source::Owned(position),
            pal.species.clone(),
            GenderAvailability::Fixed(pal.gender),
        )
    }));
    arena.extend(
        goal.progenitors
            .iter()
            .enumerate()
            .map(|(position, species)| Record {
                required: DesiredMask::single(position),
                ..leaf_record(
                    Source::Progenitor,
                    species.clone(),
                    GenderAvailability::AnyFree,
                )
            }),
    );
    if config.allow_wild_pals {
        let mut reachable: Vec<&Pal> = pal_db
            .pals()
            .filter(|pal| pal.wild_levels.is_some())
            .filter(|pal| {
                distance_to_goal
                    .get(&pal.name)
                    .and_then(|distance| usize::try_from(*distance).ok())
                    .is_some_and(|distance| distance <= config.max_breeding_steps)
            })
            .collect();
        reachable.sort_by(|a, b| a.name.as_str().cmp(b.name.as_str()));
        arena.extend(
            reachable.into_iter().map(|pal| {
                leaf_record(Source::Wild, pal.name.clone(), GenderAvailability::AnyFree)
            }),
        );
    }
}

#[derive(Clone, Copy)]
struct ExpandContext<'a> {
    pal_db: &'a PalDb,
    index: &'a ChildIndex,
    odds: &'a PassiveOdds,
    goal: &'a BreedingGoal,
    distance_to_goal: &'a HashMap<PalName, u32>,
    config: &'a SearchConfig,
}

/// Members of one species during a round, split into the ids added
/// last round (frontier) and everything older. Pairs where both sides
/// are old already produced their children in an earlier round.
#[derive(Default)]
struct SpeciesGroup<'a> {
    species: Option<&'a PalName>,
    fresh: Vec<RecordId>,
    old: Vec<RecordId>,
}

/// One expansion round, species-group-first: for every ordered
/// (male-species, female-species) pair the child and its reachability
/// are checked once, and only viable groups iterate their members.
/// Groups run in parallel; output order is deterministic.
fn expand_frontier(
    context: ExpandContext<'_>,
    arena: &[Record],
    leaves: &[RecordId],
    buckets: &HashMap<BredState, Vec<RecordId>>,
    frontier: &[RecordId],
) -> Vec<Record> {
    let groups = species_groups(arena, leaves, buckets, frontier);
    let mut ordered: Vec<&SpeciesGroup> = groups.values().collect();
    ordered.sort_by(|a, b| {
        a.species
            .map(PalName::as_str)
            .cmp(&b.species.map(PalName::as_str))
    });

    let species_pairs: Vec<(&SpeciesGroup, &SpeciesGroup)> = ordered
        .iter()
        .flat_map(|&male| ordered.iter().map(move |&female| (male, female)))
        .collect();

    species_pairs
        .par_iter()
        .map(|&(males, females)| expand_group_pair(context, arena, males, females))
        .collect::<Vec<Vec<Record>>>()
        .into_iter()
        .flatten()
        .collect()
}

fn species_groups<'a>(
    arena: &'a [Record],
    leaves: &[RecordId],
    buckets: &HashMap<BredState, Vec<RecordId>>,
    frontier: &[RecordId],
) -> HashMap<&'a PalName, SpeciesGroup<'a>> {
    let mut is_fresh = vec![false; arena.len()];
    for id in frontier {
        is_fresh[id.0] = true;
    }
    let mut groups: HashMap<&PalName, SpeciesGroup> = HashMap::new();
    for &id in leaves.iter().chain(buckets.values().flatten()) {
        let record = &arena[id.0];
        let group = groups.entry(&record.species).or_default();
        group.species = Some(&record.species);
        if is_fresh[id.0] {
            group.fresh.push(id);
        } else {
            group.old.push(id);
        }
    }
    for group in groups.values_mut() {
        group.fresh.sort_by_key(|id| id.0);
        group.old.sort_by_key(|id| id.0);
    }
    groups
}

/// All member pairs of one ordered species pair where at least one
/// side is fresh, in deterministic order.
fn expand_group_pair(
    context: ExpandContext<'_>,
    arena: &[Record],
    males: &SpeciesGroup,
    females: &SpeciesGroup,
) -> Vec<Record> {
    let (Some(male_species), Some(female_species)) = (males.species, females.species) else {
        return Vec::new();
    };
    let Some(child) = context.index.child_between(male_species, female_species) else {
        return Vec::new();
    };
    let Some(&distance) = context.distance_to_goal.get(child) else {
        return Vec::new();
    };
    // Cheapest possible pair is two leaves: one bred step, so the
    // child must be reachable with at least one step spent.
    if usize::try_from(distance).map_or(true, |steps| steps + 1 > context.config.max_breeding_steps)
    {
        return Vec::new();
    }
    let Some(child_pal) = context.pal_db.pal(child) else {
        return Vec::new();
    };

    let mut children = Vec::new();
    let mut try_pair = |male_id: RecordId, female_id: RecordId| {
        if let Some(record) = breed(context, arena, child, child_pal, male_id, female_id) {
            children.push(record);
        }
    };
    for &male in &males.fresh {
        for &female in females.fresh.iter().chain(&females.old) {
            try_pair(male, female);
        }
    }
    for &male in &males.old {
        for &female in &females.fresh {
            try_pair(male, female);
        }
    }
    children
}

/// Attempts one breeding arrangement; `None` when it is impossible or
/// over the step budget. Species-level checks (child, reachability)
/// already happened at the group level.
fn breed(
    context: ExpandContext<'_>,
    arena: &[Record],
    child: &PalName,
    child_pal: &Pal,
    male_id: RecordId,
    female_id: RecordId,
) -> Option<Record> {
    let male = &arena[male_id.0];
    let female = &arena[female_id.0];

    // A single owned individual cannot breed with itself; free
    // leaves (wild, progenitor) represent obtainable pairs.
    if male_id == female_id && matches!(male.gender, GenderAvailability::Fixed(_)) {
        return None;
    }

    let bred_count = male.bred_count + female.bred_count + 1;
    if bred_count > context.config.max_breeding_steps {
        return None;
    }
    let remaining = context.config.max_breeding_steps - bred_count;
    let distance = *context.distance_to_goal.get(child)?;
    if usize::try_from(distance).map_or(true, |steps| steps > remaining) {
        return None;
    }

    let male_cost = male.cost_as(Gender::Male)?;
    let female_cost = female.cost_as(Gender::Female)?;

    let carried = male.carried.union(female.carried);
    let pool = merged_pool(&male.contribution, &female.contribution);
    let egg_p: f64 = (carried.count()..=MAX_TOTAL_PASSIVES)
        .map(|num_final| {
            context
                .odds
                .exact_total_probability(pool.len(), carried.count(), num_final)
        })
        .sum();
    if egg_p <= 0.0 {
        return None;
    }

    Some(Record {
        source: Source::Bred {
            male: male_id,
            female: female_id,
        },
        species: child.clone(),
        gender: GenderAvailability::Flexible {
            male: child_pal.gender_probability.of(Gender::Male),
            female: child_pal.gender_probability.of(Gender::Female),
        },
        carried,
        required: male.required.union(female.required),
        contribution: carried.names(&context.goal.passives),
        parents_cost: male_cost + female_cost,
        egg_p,
        bred_count,
    })
}

/// Beam-prunes into a bucket: keeps at most `beam` bred candidates
/// per (species, carried, required) state, best root-cost first.
/// Returns the arena id when the candidate was kept.
fn insert_pruned(
    arena: &mut Vec<Record>,
    buckets: &mut HashMap<BredState, Vec<RecordId>>,
    record: Record,
    beam: usize,
) -> Option<RecordId> {
    let key = (record.species.clone(), record.carried, record.required);
    let bucket = buckets.entry(key).or_default();
    if bucket.len() < beam {
        let id = RecordId(arena.len());
        arena.push(record);
        bucket.push(id);
        return Some(id);
    }
    let worst_slot = (0..bucket.len()).max_by(|&a, &b| {
        arena[bucket[a].0]
            .root_cost()
            .total_cmp(&arena[bucket[b].0].root_cost())
    })?;
    if record.root_cost() < arena[bucket[worst_slot].0].root_cost() {
        let id = RecordId(arena.len());
        arena.push(record);
        bucket[worst_slot] = id;
        Some(id)
    } else {
        None
    }
}

fn materialize(arena: &[Record], owned: &[OwnedPal], id: RecordId) -> PlanNode {
    let record = &arena[id.0];
    match record.source {
        Source::Owned(position) => PlanNode::Owned(owned[position].clone()),
        Source::Wild => PlanNode::Wild(record.species.clone()),
        Source::Progenitor => PlanNode::Progenitor(record.species.clone()),
        Source::Bred { male, female } => PlanNode::Bred(Box::new(BredNode {
            male: materialize(arena, owned, male),
            female: materialize(arena, owned, female),
            species: record.species.clone(),
            carried_passives: record.contribution.clone(),
        })),
    }
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
