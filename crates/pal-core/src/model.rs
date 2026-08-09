//! Domain types for the Palworld game database.
//!
//! Values of these types exist only after [`crate::db`] has parsed and
//! cross-checked the vendored JSON: a [`PalName`] stored inside a
//! [`Pal`], [`BreedingCombo`], or min-steps table is guaranteed to
//! resolve in its [`PalDb`]. A free-standing [`PalName`] built via
//! [`PalName::new`] carries no such guarantee — resolve it with
//! [`PalDb::pal`].

use std::collections::{BTreeMap, HashMap};
use std::fmt;

/// Canonical pal identifier — the game's internal name (e.g.
/// `SheepBall` for Lamball). Every table in the database keys on this;
/// display names are localization.
#[derive(Clone, PartialEq, Eq, Hash, Debug, serde::Serialize, serde::Deserialize)]
#[serde(transparent)]
pub struct PalName(String);

impl PalName {
    pub fn new(name: impl Into<String>) -> Self {
        Self(name.into())
    }

    #[must_use]
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl fmt::Display for PalName {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}

/// Canonical passive-skill identifier (the game's internal name).
#[derive(Clone, PartialEq, Eq, Hash, Debug, serde::Serialize, serde::Deserialize)]
#[serde(transparent)]
pub struct PassiveName(String);

impl PassiveName {
    pub fn new(name: impl Into<String>) -> Self {
        Self(name.into())
    }

    #[must_use]
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl fmt::Display for PassiveName {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}

/// Canonical item identifier (the game's internal name). The
/// database references items (surgery requirements) without carrying
/// item definitions, so this name resolves outside the [`PalDb`].
#[derive(Clone, PartialEq, Eq, Hash, Debug, serde::Serialize, serde::Deserialize)]
#[serde(transparent)]
pub struct ItemName(String);

impl ItemName {
    pub fn new(name: impl Into<String>) -> Self {
        Self(name.into())
    }

    #[must_use]
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl fmt::Display for ItemName {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}

/// How a passive is installed at the surgery table. Absent from
/// [`PassiveSkill::surgery`] when the skill cannot be installed.
#[derive(Clone, PartialEq, Eq, Debug)]
pub enum Surgery {
    Gold { cost: u32 },
    GoldAndItem { cost: u32, item: ItemName },
    Item { item: ItemName },
}

impl Surgery {
    #[must_use]
    pub fn gold_cost(&self) -> Option<u32> {
        match self {
            Self::Gold { cost } | Self::GoldAndItem { cost, .. } => Some(*cost),
            Self::Item { .. } => None,
        }
    }

    #[must_use]
    pub fn required_item(&self) -> Option<&ItemName> {
        match self {
            Self::Gold { .. } => None,
            Self::GoldAndItem { item, .. } | Self::Item { item } => Some(item),
        }
    }
}

#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug, serde::Serialize, serde::Deserialize)]
pub enum Gender {
    Male,
    Female,
}

/// One individual-value stat, `0..=100`. Values are born at the save
/// boundary via [`IvValue::from_save`], which clamps rather than
/// fails: an out-of-range talent is game corruption we render inert,
/// not a condition callers can act on.
#[derive(
    Clone,
    Copy,
    PartialEq,
    Eq,
    PartialOrd,
    Ord,
    Hash,
    Debug,
    Default,
    serde::Serialize,
    serde::Deserialize,
)]
#[serde(into = "u8", try_from = "u8")]
pub struct IvValue(u8);

#[derive(Debug, PartialEq, Eq, thiserror::Error)]
#[error("IV value {0} is out of range (0..=100)")]
pub struct InvalidIvValue(u8);

impl IvValue {
    pub const MAX: Self = Self(100);

    #[must_use]
    #[expect(
        clippy::cast_sign_loss,
        reason = "clamped to 0..=100 first; the cast is lossless"
    )]
    pub fn from_save(raw: i64) -> Self {
        Self(raw.clamp(0, 100) as u8)
    }

    #[must_use]
    pub fn get(self) -> u8 {
        self.0
    }
}

impl From<IvValue> for u8 {
    fn from(value: IvValue) -> Self {
        value.0
    }
}

impl TryFrom<u8> for IvValue {
    type Error = InvalidIvValue;

    fn try_from(raw: u8) -> Result<Self, Self::Error> {
        if raw > 100 {
            return Err(InvalidIvValue(raw));
        }
        Ok(Self(raw))
    }
}

/// The three breeding-relevant individual values of one pal. The
/// game's `Talent_Melee` is vestigial (no stat uses it) and is not
/// modeled, matching palcalc.
#[derive(
    Clone, Copy, PartialEq, Eq, Hash, Debug, Default, serde::Serialize, serde::Deserialize,
)]
#[serde(default)]
pub struct IvSpread {
    pub hp: IvValue,
    pub attack: IvValue,
    pub defense: IvValue,
}

impl IvSpread {
    #[must_use]
    pub fn get(self, stat: IvStat) -> IvValue {
        match stat {
            IvStat::Hp => self.hp,
            IvStat::Attack => self.attack,
            IvStat::Defense => self.defense,
        }
    }
}

#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug, serde::Serialize, serde::Deserialize)]
pub enum IvStat {
    Hp,
    Attack,
    Defense,
}

impl IvStat {
    pub const ALL: [Self; 3] = [Self::Hp, Self::Attack, Self::Defense];
}

/// Gender requirement on one parent slot of a [`BreedingCombo`].
/// Most combos accept either gender; a few (e.g. Katress + Wixen)
/// produce different children depending on which parent is which.
#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
pub enum ParentGender {
    Wildcard,
    Exactly(Gender),
}

impl ParentGender {
    #[must_use]
    pub fn accepts(self, gender: Gender) -> bool {
        match self {
            Self::Wildcard => true,
            Self::Exactly(required) => required == gender,
        }
    }
}

/// Paldex entry: dex number plus the variant flag. Alternate forms
/// (e.g. elemental variants) share a dex number with `variant` set.
#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
pub struct PalId {
    pub dex: u16,
    pub variant: bool,
}

/// Level range at which a species spawns in the wild. Species that
/// never spawn (raid/special forms) carry no [`WildLevels`] at all —
/// see [`Pal::wild_levels`].
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct WildLevels {
    pub min: u8,
    pub max: u8,
}

/// Probability that a bred child of a species is male vs female.
/// Invariant: both components lie in `[0, 1]` and sum to 1.
#[derive(Clone, Copy, Debug)]
pub struct GenderProbability {
    male: f64,
    female: f64,
}

impl GenderProbability {
    /// # Errors
    ///
    /// Rejects components outside `[0, 1]` or a pair not summing to 1.
    pub fn new(male: f64, female: f64) -> Result<Self, InvalidGenderProbability> {
        let in_range = (0.0..=1.0).contains(&male) && (0.0..=1.0).contains(&female);
        if in_range && (male + female - 1.0).abs() <= 1e-6 {
            Ok(Self { male, female })
        } else {
            Err(InvalidGenderProbability { male, female })
        }
    }

    #[must_use]
    pub fn of(self, gender: Gender) -> f64 {
        match gender {
            Gender::Male => self.male,
            Gender::Female => self.female,
        }
    }
}

#[derive(Clone, Copy, PartialEq, Debug, thiserror::Error)]
#[error("gender probabilities must lie in [0, 1] and sum to 1 (male={male}, female={female})")]
pub struct InvalidGenderProbability {
    pub male: f64,
    pub female: f64,
}

#[derive(Clone, Debug)]
pub struct Pal {
    pub id: PalId,
    pub name: PalName,
    pub display_name: String,
    pub breeding_power: u32,
    pub breeding_power_priority: u32,
    pub gender_probability: GenderProbability,
    pub guaranteed_passives: Vec<PassiveName>,
    /// `None` when the species never spawns in the wild (raid and
    /// special forms) — it can only be bred or otherwise granted.
    pub wild_levels: Option<WildLevels>,
}

#[derive(Clone, Debug)]
pub struct PassiveSkill {
    pub name: PassiveName,
    pub display_name: String,
    pub rank: i8,
    pub standard: bool,
    pub random_inheritance_allowed: bool,
    pub random_inheritance_weight: u32,
    pub surgery: Option<Surgery>,
}

/// Inheritance weight tables, each keyed by outcome count (e.g. weight
/// of a child inheriting exactly N passives).
#[derive(Clone, Debug)]
pub struct BreedingMechanics {
    pub iv_inheritance_weights: BTreeMap<u8, u32>,
    pub passive_inheritance_weights: BTreeMap<u8, u32>,
    pub passive_random_weights: BTreeMap<u8, u32>,
}

/// Version tag of the vendored database (palcalc's `Version` field).
#[derive(Clone, PartialEq, Eq, Debug)]
pub struct DbVersion(String);

impl DbVersion {
    pub(crate) fn new(version: String) -> Self {
        Self(version)
    }

    #[must_use]
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl fmt::Display for DbVersion {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}

/// The pal/passive database: species, skills, and breeding mechanics.
/// Built only by [`crate::db::parse_pal_db`].
#[derive(Clone, Debug)]
pub struct PalDb {
    version: DbVersion,
    pals: HashMap<PalName, Pal>,
    passives: HashMap<PassiveName, PassiveSkill>,
    mechanics: BreedingMechanics,
}

impl PalDb {
    pub(crate) fn new(
        version: DbVersion,
        pals: HashMap<PalName, Pal>,
        passives: HashMap<PassiveName, PassiveSkill>,
        mechanics: BreedingMechanics,
    ) -> Self {
        Self {
            version,
            pals,
            passives,
            mechanics,
        }
    }

    #[must_use]
    pub fn version(&self) -> &DbVersion {
        &self.version
    }

    #[must_use]
    pub fn pal(&self, name: &PalName) -> Option<&Pal> {
        self.pals.get(name)
    }

    /// Case-insensitive lookup by internal or display name — the
    /// resolution step for user-facing input (config files, search
    /// boxes).
    #[must_use]
    pub fn find_pal(&self, name: &str) -> Option<&Pal> {
        self.pals.values().find(|pal| {
            pal.name.as_str().eq_ignore_ascii_case(name)
                || pal.display_name.eq_ignore_ascii_case(name)
        })
    }

    /// Case-insensitive passive lookup by internal or display name.
    #[must_use]
    pub fn find_passive(&self, name: &str) -> Option<&PassiveSkill> {
        self.passives.values().find(|skill| {
            skill.name.as_str().eq_ignore_ascii_case(name)
                || skill.display_name.eq_ignore_ascii_case(name)
        })
    }

    pub fn pals(&self) -> impl Iterator<Item = &Pal> {
        self.pals.values()
    }

    #[must_use]
    pub fn passive(&self, name: &PassiveName) -> Option<&PassiveSkill> {
        self.passives.get(name)
    }

    pub fn passives(&self) -> impl Iterator<Item = &PassiveSkill> {
        self.passives.values()
    }

    #[must_use]
    pub fn mechanics(&self) -> &BreedingMechanics {
        &self.mechanics
    }
}

#[derive(Clone, Debug)]
pub struct BreedingParent {
    pub name: PalName,
    pub gender: ParentGender,
}

/// One breeding rule: the two parent slots (order as listed upstream)
/// and the resulting child species.
#[derive(Clone, Debug)]
pub struct BreedingCombo {
    pub parents: [BreedingParent; 2],
    pub child: PalName,
}

/// Breeding rules plus the upstream-precomputed minimum number of
/// breeding steps between any two species. Built only by
/// [`crate::db::parse_breeding_db`]; every name resolves in the
/// [`PalDb`] it was parsed against.
#[derive(Clone, Debug)]
pub struct BreedingDb {
    combos: Vec<BreedingCombo>,
    min_steps: HashMap<PalName, HashMap<PalName, u32>>,
}

impl BreedingDb {
    pub(crate) fn new(
        combos: Vec<BreedingCombo>,
        min_steps: HashMap<PalName, HashMap<PalName, u32>>,
    ) -> Self {
        Self { combos, min_steps }
    }

    #[must_use]
    pub fn combos(&self) -> &[BreedingCombo] {
        &self.combos
    }

    /// Minimum breeding steps from `from` to `to`; `None` when `to`
    /// cannot be bred into from `from` (the loader normalizes
    /// upstream's unreachable sentinel to absence).
    #[must_use]
    pub fn min_steps(&self, from: &PalName, to: &PalName) -> Option<u32> {
        self.min_steps.get(from)?.get(to).copied()
    }
}
