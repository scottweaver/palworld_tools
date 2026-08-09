//! JSON shapes the tools return. Every reference to a species or
//! passive carries both the canonical internal name and the display
//! name, so responses stay self-describing wherever the client quotes
//! them.

use pal_core::model::{Gender, IvSpread, PalDb, PalName, PassiveName};
use pal_solver::search::{BreedingPlan, OwnedPal, PlanNode};
use schemars::JsonSchema;
use serde::Serialize;

#[derive(Clone, Serialize, JsonSchema)]
pub struct SpeciesRef {
    /// Canonical internal name (the game's identifier).
    pub name: String,
    /// Localized display name.
    pub display_name: String,
}

#[derive(Clone, Serialize, JsonSchema)]
pub struct PassiveRef {
    /// Canonical internal name.
    pub name: String,
    /// Localized display name.
    pub display_name: String,
}

#[derive(Clone, Copy, Serialize, JsonSchema)]
#[serde(rename_all = "lowercase")]
pub enum GenderJson {
    Male,
    Female,
}

impl From<Gender> for GenderJson {
    fn from(gender: Gender) -> Self {
        match gender {
            Gender::Male => Self::Male,
            Gender::Female => Self::Female,
        }
    }
}

#[derive(Clone, Copy, Serialize, JsonSchema)]
pub struct IvsJson {
    pub hp: u8,
    pub attack: u8,
    pub defense: u8,
}

impl From<IvSpread> for IvsJson {
    fn from(ivs: IvSpread) -> Self {
        Self {
            hp: ivs.hp.get(),
            attack: ivs.attack.get(),
            defense: ivs.defense.get(),
        }
    }
}

#[derive(Clone, Serialize, JsonSchema)]
pub struct OwnedPalJson {
    pub species: SpeciesRef,
    pub gender: GenderJson,
    pub passives: Vec<PassiveRef>,
    pub ivs: IvsJson,
}

/// One node of a plan tree, tagged by how the pal is obtained.
#[derive(Clone, Serialize, JsonSchema)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum PlanNodeJson {
    /// A pal already in the pool.
    Owned { pal: OwnedPalJson },
    /// A wild pal to catch (any gender, no passives).
    Wild { species: SpeciesRef },
    /// A required progenitor the caller supplies (any gender, no
    /// passives).
    Progenitor { species: SpeciesRef },
    /// A breeding step: pair male × female, re-hatch until the child
    /// carries every passive in `carried_passives`.
    Bred {
        species: SpeciesRef,
        carried_passives: Vec<PassiveRef>,
        male: Box<PlanNodeJson>,
        female: Box<PlanNodeJson>,
    },
}

#[derive(Clone, Serialize, JsonSchema)]
pub struct PlanJson {
    /// Expected breeding attempts across the whole plan; 0 when an
    /// owned pal already satisfies the goal.
    pub expected_eggs: f64,
    /// Number of breeding steps in the plan.
    pub steps: usize,
    pub root: PlanNodeJson,
}

pub fn species_ref(db: &PalDb, name: &PalName) -> SpeciesRef {
    SpeciesRef {
        name: name.to_string(),
        display_name: db
            .pal(name)
            .map_or_else(|| name.to_string(), |pal| pal.display_name.clone()),
    }
}

pub fn passive_ref(db: &PalDb, name: &PassiveName) -> PassiveRef {
    PassiveRef {
        name: name.to_string(),
        display_name: db
            .passive(name)
            .map_or_else(|| name.to_string(), |skill| skill.display_name.clone()),
    }
}

pub fn owned_pal_json(db: &PalDb, pal: &OwnedPal) -> OwnedPalJson {
    OwnedPalJson {
        species: species_ref(db, &pal.species),
        gender: pal.gender.into(),
        passives: pal
            .passives
            .iter()
            .map(|passive| passive_ref(db, passive))
            .collect(),
        ivs: pal.ivs.into(),
    }
}

pub fn plan_json(db: &PalDb, plan: &BreedingPlan) -> PlanJson {
    PlanJson {
        expected_eggs: plan.expected_eggs,
        steps: plan.steps,
        root: node_json(db, &plan.root),
    }
}

fn node_json(db: &PalDb, node: &PlanNode) -> PlanNodeJson {
    match node {
        PlanNode::Owned(pal) => PlanNodeJson::Owned {
            pal: owned_pal_json(db, pal),
        },
        PlanNode::Wild(species) => PlanNodeJson::Wild {
            species: species_ref(db, species),
        },
        PlanNode::Progenitor(species) => PlanNodeJson::Progenitor {
            species: species_ref(db, species),
        },
        PlanNode::Bred(bred) => PlanNodeJson::Bred {
            species: species_ref(db, &bred.species),
            carried_passives: bred
                .carried_passives
                .iter()
                .map(|passive| passive_ref(db, passive))
                .collect(),
            male: Box::new(node_json(db, &bred.male)),
            female: Box::new(node_json(db, &bred.female)),
        },
    }
}
