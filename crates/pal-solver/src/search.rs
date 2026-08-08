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
//! every passive the node must carry (see [`crate::passives`]), every
//! supplyable goal IV minimum (see [`crate::iv`]), and, when the node
//! is later used as a gendered parent, the required gender. Bred
//! intermediates are re-bred until they succeed, so obtaining one
//! costs `parents' cost + 1 / (P(passives) · P(IVs) · P(gender))`
//! expected eggs; owned pals cost nothing. Goals may name required
//! progenitors: candidate state then also tracks which progenitors a
//! tree includes, and only fully-anchored plans are returned. Goals
//! may set IV minimums: candidate state tracks which minimums a
//! tree's product meets, and only fully-met plans are returned.
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
//!
//! Two disciplines keep multi-passive searches from churning: the
//! frontier holds only candidates that opened a new (species, masks)
//! state or strictly improved their state's best cost — mid-beam
//! refinements are ranked but never re-expanded — and pair attempts
//! stay heap-free ([`Candidate`]) until they survive a group-local
//! per-state beam and the branch-and-bound cut.

use std::collections::{HashMap, HashSet};
use std::sync::{Arc, Mutex};

use pal_core::model::{Gender, IvSpread, Pal, PalDb, PalName, PassiveName};
use rayon::prelude::*;

use crate::child::ChildIndex;
use crate::iv::{IvOdds, IvThresholds};
use crate::passives::{MAX_TOTAL_PASSIVES, PassiveOdds};
use crate::steps::SpeciesAdjacency;

/// A pal the player already has: the leaf material of every plan.
/// `ivs` defaults for stores written before IVs were modeled.
#[derive(Clone, PartialEq, Eq, Debug, serde::Serialize, serde::Deserialize)]
pub struct OwnedPal {
    pub species: PalName,
    pub gender: Gender,
    pub passives: Vec<PassiveName>,
    #[serde(default)]
    pub ivs: IvSpread,
}

/// What the search is for: a species, carrying all listed passives
/// and meeting all IV minimums, bred from all listed progenitors.
#[derive(Clone, PartialEq, Eq, Debug)]
pub struct BreedingGoal {
    pub species: PalName,
    pub passives: Vec<PassiveName>,
    /// Required anchor species: every returned plan must include each
    /// of these as a leaf at least once. Progenitors are available as
    /// free any-gender leaves regardless of wild spawns (the caller
    /// has them), and carry no passives and no IVs.
    pub progenitors: Vec<PalName>,
    pub iv_thresholds: IvThresholds,
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
#[derive(Clone, PartialEq, Eq, Debug, serde::Serialize, serde::Deserialize)]
pub enum PlanNode {
    Owned(OwnedPal),
    Wild(PalName),
    Progenitor(PalName),
    Bred(Box<BredNode>),
}

/// A breeding step: pair `male` × `female`, re-hatching until the
/// child carries `carried_passives`.
#[derive(Clone, PartialEq, Eq, Debug, serde::Serialize, serde::Deserialize)]
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
    iv_odds: &'db IvOdds,
    adjacency: SpeciesAdjacency,
    distance_cache: Mutex<HashMap<PalName, Arc<HashMap<PalName, u32>>>>,
}

impl<'db> Solver<'db> {
    /// `index` must be built from the same database generation as
    /// `pal_db`; species the index yields but `pal_db` lacks are
    /// skipped.
    #[must_use]
    pub fn new(
        pal_db: &'db PalDb,
        index: &'db ChildIndex,
        odds: &'db PassiveOdds,
        iv_odds: &'db IvOdds,
    ) -> Self {
        Self {
            pal_db,
            index,
            odds,
            iv_odds,
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
        let owned_junk = owned_junk_table(owned, &goal.passives);

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

        let full = DesiredMask::full(goal.passives.len());
        let all_progenitors = DesiredMask::full(goal.progenitors.len());
        let iv_full = DesiredMask::full(goal.iv_thresholds.active().len());
        let goal_state: BredState = (goal.species.clone(), full, all_progenitors, iv_full);

        for _ in 0..config.max_breeding_steps {
            // Branch-and-bound: once `max_results` goal plans exist,
            // a candidate whose root cost already matches the worst
            // of them can never improve the results — any plan built
            // on it costs at least that much (`cost_as >= root_cost`
            // since gender probabilities are <= 1, and downstream
            // breeding only adds eggs). Deep searches converge fast
            // because of this cut.
            let incumbent_worst = buckets
                .get(&goal_state)
                .filter(|bucket| bucket.len() >= config.max_results)
                .map(|bucket| {
                    bucket
                        .iter()
                        .map(|id| arena[id.0].root_cost())
                        .fold(f64::NEG_INFINITY, f64::max)
                });

            let children = expand_frontier(
                ExpandContext {
                    pal_db: self.pal_db,
                    index: self.index,
                    odds: self.odds,
                    iv_odds: self.iv_odds,
                    distance_to_goal: &distance_to_goal,
                    config,
                    owned_junk: &owned_junk,
                    goal_species: &goal.species,
                    full,
                    all_progenitors,
                    iv_full,
                    incumbent_worst,
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
            keep_last_per_state(&arena, &mut frontier);
            if frontier.is_empty() {
                break;
            }
        }
        Ok(ranked_plans(
            &arena,
            owned,
            &goal.passives,
            leaves.iter().chain(buckets.values().flatten()),
            &goal_state,
            config.max_results,
        ))
    }
}

/// Materializes every candidate matching the goal state, ranked by
/// expected eggs (ties to fewer steps), truncated to `max_results`.
fn ranked_plans<'a>(
    arena: &[Record],
    owned: &[OwnedPal],
    desired: &[PassiveName],
    candidates: impl Iterator<Item = &'a RecordId>,
    goal_state: &BredState,
    max_results: usize,
) -> Vec<BreedingPlan> {
    let (species, full, all_progenitors, iv_full) = goal_state;
    let mut plans: Vec<BreedingPlan> = candidates
        .filter(|id| {
            let record = &arena[id.0];
            record.species == *species
                && record.carried == *full
                && record.required == *all_progenitors
                && record.iv_met == *iv_full
        })
        .map(|id| {
            let record = &arena[id.0];
            BreedingPlan {
                root: materialize(arena, owned, desired, *id),
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
    plans.truncate(max_results);
    plans
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
    iv_odds: &IvOdds,
    owned: &[OwnedPal],
    goal: &BreedingGoal,
    config: &SearchConfig,
) -> Result<Vec<BreedingPlan>, SearchError> {
    Solver::new(pal_db, index, odds, iv_odds).find_paths(owned, goal, config)
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

    fn intersection(self, other: Self) -> Self {
        Self(self.0 & other.0)
    }

    fn symmetric_difference(self, other: Self) -> Self {
        Self(self.0 ^ other.0)
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
/// (species, carried, required, iv-met) bucket.
type BredState = (PalName, DesiredMask, DesiredMask, DesiredMask);

/// One expansion per state and round: admissions only happen on a
/// strict per-state best improvement, so the last admission for a
/// state is its best — keep that one and drop its earlier, already
/// superseded frontier entries.
fn keep_last_per_state(arena: &[Record], frontier: &mut Vec<RecordId>) {
    let mut seen: HashSet<BredState> = HashSet::new();
    let mut kept = Vec::with_capacity(frontier.len());
    for &id in frontier.iter().rev() {
        let record = &arena[id.0];
        let key = (
            record.species.clone(),
            record.carried,
            record.required,
            record.iv_met,
        );
        if seen.insert(key) {
            kept.push(id);
        }
    }
    kept.reverse();
    *frontier = kept;
}

#[derive(Clone, Debug)]
struct Record {
    source: Source,
    species: PalName,
    gender: GenderAvailability,
    carried: DesiredMask,
    required: DesiredMask,
    /// Active goal IV minimums this candidate meets. For bred pals
    /// this is every stat either parent met: a successful hatch
    /// inherited each such stat from a parent at/above the minimum.
    ///
    /// A candidate's contribution to a child's inheritance pool is
    /// not stored: bred pals contribute exactly their carried
    /// (desired-only) set, wilds and progenitors nothing, and owned
    /// leaves add the junk names precomputed per leaf in
    /// [`ExpandContext::owned_junk`].
    iv_met: DesiredMask,
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
        iv_met: DesiredMask::empty(),
        parents_cost: 0.0,
        egg_p: 1.0,
        bred_count: 0,
    }
}

/// Which active goal minimums `ivs` meets, one bit per active stat.
fn iv_mask(thresholds: IvThresholds, ivs: IvSpread) -> DesiredMask {
    let mut mask = DesiredMask::empty();
    for (position, (stat, minimum)) in thresholds.active().into_iter().enumerate() {
        if ivs.get(stat) >= minimum {
            mask = mask.union(DesiredMask::single(position));
        }
    }
    mask
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
        iv_met: iv_mask(goal.iv_thresholds, pal.ivs),
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
    iv_odds: &'a IvOdds,
    distance_to_goal: &'a HashMap<PalName, u32>,
    config: &'a SearchConfig,
    /// Per owned leaf (same indexing as the `owned` slice): its
    /// distinct passives that are not goal passives. The only
    /// pool-size contribution the carried masks cannot express.
    owned_junk: &'a [Vec<PassiveName>],
    /// The goal state the branch-and-bound cut measures against.
    goal_species: &'a PalName,
    full: DesiredMask,
    all_progenitors: DesiredMask,
    iv_full: DesiredMask,
    /// Worst incumbent goal cost this round, once `max_results` goal
    /// plans exist; candidates that cannot beat it are dropped inside
    /// the group expansion.
    incumbent_worst: Option<f64>,
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
/// side is fresh, in deterministic order. Candidates flow through
/// the incumbent cut and a group-local per-state beam before any
/// heap-carrying [`Record`] exists, so a round's millions of
/// attempts allocate only for the handful of survivors.
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

    let goal_child = child == context.goal_species;
    let mut local = LocalBeam::new(context.config.max_results);
    let mut try_pair = |male_id: RecordId, female_id: RecordId| {
        let Some(candidate) = breed(context, arena, distance, male_id, female_id) else {
            return;
        };
        if let Some(worst) = context.incumbent_worst {
            // A non-goal candidate still needs at least `distance`
            // further breeding steps, each of which costs at least
            // one expected egg; a goal-species candidate short of a
            // mask needs at least one more.
            let remaining_steps = if goal_child {
                u32::from(
                    candidate.carried != context.full
                        || candidate.required != context.all_progenitors
                        || candidate.iv_met != context.iv_full,
                )
            } else {
                distance.max(1)
            };
            if candidate.root_cost() + f64::from(remaining_steps) >= worst {
                return;
            }
        }
        local.offer(candidate);
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
    local.emit(child, child_pal)
}

/// Per-state beam applied inside one species-group expansion, before
/// records are materialized: keeps the best `beam` candidates per
/// (carried, required, iv-met) state by root cost. The global beam
/// applies the same rule across groups, so nothing this drops could
/// have survived globally.
struct LocalBeam {
    beam: usize,
    states: HashMap<(DesiredMask, DesiredMask, DesiredMask), Vec<Candidate>>,
}

impl LocalBeam {
    fn new(beam: usize) -> Self {
        Self {
            beam,
            states: HashMap::new(),
        }
    }

    fn offer(&mut self, candidate: Candidate) {
        let bucket = self
            .states
            .entry((candidate.carried, candidate.required, candidate.iv_met))
            .or_default();
        if bucket.len() < self.beam {
            bucket.push(candidate);
            return;
        }
        let Some(worst_slot) = (0..bucket.len())
            .max_by(|&a, &b| bucket[a].root_cost().total_cmp(&bucket[b].root_cost()))
        else {
            return;
        };
        if candidate.root_cost() < bucket[worst_slot].root_cost() {
            bucket[worst_slot] = candidate;
        }
    }

    /// Materializes the survivors, in deterministic state order.
    fn emit(self, child: &PalName, child_pal: &Pal) -> Vec<Record> {
        let mut states: Vec<_> = self.states.into_iter().collect();
        states.sort_by_key(|((carried, required, iv_met), _)| (carried.0, required.0, iv_met.0));
        states
            .into_iter()
            .flat_map(|(_, bucket)| bucket)
            .map(|candidate| Record {
                source: Source::Bred {
                    male: candidate.male,
                    female: candidate.female,
                },
                species: child.clone(),
                gender: GenderAvailability::Flexible {
                    male: child_pal.gender_probability.of(Gender::Male),
                    female: child_pal.gender_probability.of(Gender::Female),
                },
                carried: candidate.carried,
                required: candidate.required,
                iv_met: candidate.iv_met,
                parents_cost: candidate.parents_cost,
                egg_p: candidate.egg_p,
                bred_count: candidate.bred_count,
            })
            .collect()
    }
}

/// A bred child before it is admitted anywhere: everything a
/// [`Record`] holds except the species name and gender availability,
/// which are group-wide facts attached only to the candidates that
/// survive the group's local beam. Heap-free on purpose — rounds
/// attempt tens of millions of these.
#[derive(Clone, Copy, Debug)]
struct Candidate {
    male: RecordId,
    female: RecordId,
    carried: DesiredMask,
    required: DesiredMask,
    iv_met: DesiredMask,
    parents_cost: f64,
    egg_p: f64,
    bred_count: usize,
}

impl Candidate {
    /// Mirrors [`Record::root_cost`] for pre-admission pruning.
    fn root_cost(&self) -> f64 {
        self.parents_cost + 1.0 / self.egg_p
    }
}

/// Attempts one breeding arrangement; `None` when it is impossible or
/// over the step budget. Species-level checks (child, reachability,
/// `distance`) already happened at the group level.
fn breed(
    context: ExpandContext<'_>,
    arena: &[Record],
    distance: u32,
    male_id: RecordId,
    female_id: RecordId,
) -> Option<Candidate> {
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
    if usize::try_from(distance).map_or(true, |steps| steps > remaining) {
        return None;
    }

    let male_cost = male.cost_as(Gender::Male)?;
    let female_cost = female.cost_as(Gender::Female)?;

    let carried = male.carried.union(female.carried);
    let pool_size = carried.count() + junk_union(context.owned_junk, male, female);
    let passives_p: f64 = (carried.count()..=MAX_TOTAL_PASSIVES)
        .map(|num_final| {
            context
                .odds
                .exact_total_probability(pool_size, carried.count(), num_final)
        })
        .sum();

    // A successful egg also inherits every supplyable goal IV: stats
    // met by one parent flip the extra right-parent coin, stats met
    // by both do not (see crate::iv).
    let iv_met = male.iv_met.union(female.iv_met);
    let both_met = male.iv_met.intersection(female.iv_met);
    let single_met = male.iv_met.symmetric_difference(female.iv_met);
    let iv_p = context
        .iv_odds
        .pair_probability(single_met.count(), both_met.count());

    let egg_p = passives_p * iv_p;
    if egg_p <= 0.0 {
        return None;
    }

    Some(Candidate {
        male: male_id,
        female: female_id,
        carried,
        required: male.required.union(female.required),
        iv_met,
        parents_cost: male_cost + female_cost,
        egg_p,
        bred_count,
    })
}

/// Beam-prunes into a bucket: keeps at most `beam` bred candidates
/// per (species, carried, required, iv-met) state, best root-cost
/// first. Returns the arena id only when the candidate opened a new
/// state or became its state's new best — the frontier signal.
/// Mid-bucket admissions are kept for ranking but never re-expand:
/// their state already has an equal-or-better representative that
/// did, and re-expanding every marginal refinement is what made
/// multi-passive searches churn for minutes.
fn insert_pruned(
    arena: &mut Vec<Record>,
    buckets: &mut HashMap<BredState, Vec<RecordId>>,
    record: Record,
    beam: usize,
) -> Option<RecordId> {
    let key = (
        record.species.clone(),
        record.carried,
        record.required,
        record.iv_met,
    );
    let bucket = buckets.entry(key).or_default();
    let best = bucket
        .iter()
        .map(|id| arena[id.0].root_cost())
        .fold(f64::INFINITY, f64::min);
    let improves_best = record.root_cost() < best;
    if bucket.len() < beam {
        let id = RecordId(arena.len());
        arena.push(record);
        bucket.push(id);
        return improves_best.then_some(id);
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
        improves_best.then_some(id)
    } else {
        None
    }
}

fn materialize(
    arena: &[Record],
    owned: &[OwnedPal],
    desired: &[PassiveName],
    id: RecordId,
) -> PlanNode {
    let record = &arena[id.0];
    match record.source {
        Source::Owned(position) => PlanNode::Owned(owned[position].clone()),
        Source::Wild => PlanNode::Wild(record.species.clone()),
        Source::Progenitor => PlanNode::Progenitor(record.species.clone()),
        Source::Bred { male, female } => PlanNode::Bred(Box::new(BredNode {
            male: materialize(arena, owned, desired, male),
            female: materialize(arena, owned, desired, female),
            species: record.species.clone(),
            carried_passives: record.carried.names(desired),
        })),
    }
}

/// Distinct non-goal passives the pair contributes to the child's
/// inheritance pool. Only owned leaves contribute junk — bred pals
/// contribute exactly their carried set, wilds and progenitors
/// nothing — so the union is a lookup except for leaf × leaf pairs.
fn junk_union(owned_junk: &[Vec<PassiveName>], male: &Record, female: &Record) -> usize {
    match (male.source, female.source) {
        (Source::Owned(left), Source::Owned(right)) => {
            let (a, b) = (&owned_junk[left], &owned_junk[right]);
            a.len() + b.iter().filter(|passive| !a.contains(passive)).count()
        }
        (Source::Owned(position), _) | (_, Source::Owned(position)) => owned_junk[position].len(),
        _ => 0,
    }
}

/// The per-leaf junk table for [`ExpandContext::owned_junk`].
fn owned_junk_table(owned: &[OwnedPal], desired: &[PassiveName]) -> Vec<Vec<PassiveName>> {
    owned
        .iter()
        .map(|pal| {
            let mut junk: Vec<PassiveName> = Vec::new();
            for passive in &pal.passives {
                if !desired.contains(passive) && !junk.contains(passive) {
                    junk.push(passive.clone());
                }
            }
            junk
        })
        .collect()
}
