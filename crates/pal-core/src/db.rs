//! The JSON boundary for the vendored palcalc database files.
//!
//! Raw `db.json` / `breeding.json` text is parsed here exactly once
//! into [`crate::model`] types. Cross-references are checked during
//! parsing —
//! gender probabilities exist for every pal, guaranteed passives and
//! breeding names resolve — so downstream code trusts the model
//! without re-validating.

use std::collections::{BTreeMap, HashMap};

use serde::Deserialize;

use crate::model::{
    BreedingCombo, BreedingDb, BreedingMechanics, BreedingParent, DbVersion, Gender,
    GenderProbability, InvalidGenderProbability, Pal, PalDb, PalId, PalName, ParentGender,
    PassiveName, PassiveSkill,
};

/// The palcalc database version this loader understands. Bump together
/// with a refresh of the vendored files (see `data/README.md`).
pub const SUPPORTED_VERSION: &str = "v27";

#[derive(Debug, thiserror::Error)]
pub enum ParseError {
    #[error("malformed database JSON")]
    Json(#[from] serde_json::Error),
    #[error("unsupported database version {found} (this loader supports {SUPPORTED_VERSION})")]
    UnsupportedVersion { found: String },
    #[error("pal {pal} has no gender-probability entry")]
    MissingGenderProbability { pal: String },
    #[error("pal {pal}: {source}")]
    GenderProbability {
        pal: String,
        source: InvalidGenderProbability,
    },
    #[error("pal {pal} references unknown passive {passive}")]
    UnknownPassive { pal: String, passive: String },
    #[error("breeding entry references pal {name} absent from the pal database")]
    UnknownPal { name: String },
}

/// Parses `db.json` content into a [`PalDb`].
///
/// # Errors
///
/// Fails on malformed JSON, a version other than [`SUPPORTED_VERSION`],
/// a pal without a gender-probability entry, an invalid probability
/// pair, or a guaranteed passive that no passive-skill entry defines.
pub fn parse_pal_db(json: &str) -> Result<PalDb, ParseError> {
    let raw: RawDb = serde_json::from_str(json)?;
    if raw.version != SUPPORTED_VERSION {
        return Err(ParseError::UnsupportedVersion { found: raw.version });
    }

    let passives: HashMap<PassiveName, PassiveSkill> = raw
        .passive_skills
        .into_iter()
        .map(|raw| {
            let skill = raw.into_model();
            (skill.name.clone(), skill)
        })
        .collect();

    let mut gender_probabilities = raw.breeding_gender_probability;
    let pals = raw
        .pals
        .into_iter()
        .map(|raw| {
            let pal = raw.into_model(&mut gender_probabilities, &passives)?;
            Ok((pal.name.clone(), pal))
        })
        .collect::<Result<HashMap<_, _>, ParseError>>()?;

    Ok(PalDb::new(
        DbVersion::new(raw.version),
        pals,
        passives,
        raw.breeding_mechanics.into_model(),
    ))
}

/// Parses `breeding.json` content into a [`BreedingDb`], resolving
/// every pal name against `pal_db`.
///
/// # Errors
///
/// Fails on malformed JSON, an unrecognized parent-gender tag, or any
/// name that does not resolve in `pal_db`.
pub fn parse_breeding_db(json: &str, pal_db: &PalDb) -> Result<BreedingDb, ParseError> {
    let raw: RawBreedingFile = serde_json::from_str(json)?;

    let combos = raw
        .breeding
        .into_iter()
        .map(|entry| entry.into_model(pal_db))
        .collect::<Result<Vec<_>, _>>()?;

    let min_steps = raw
        .min_breeding_steps
        .into_iter()
        .map(|(from, to_steps)| {
            let to_steps = to_steps
                .into_iter()
                .map(|(to, steps)| Ok((known_pal(pal_db, to)?, steps)))
                .collect::<Result<HashMap<_, _>, ParseError>>()?;
            Ok((known_pal(pal_db, from)?, to_steps))
        })
        .collect::<Result<HashMap<_, _>, ParseError>>()?;

    Ok(BreedingDb::new(combos, min_steps))
}

fn known_pal(pal_db: &PalDb, name: String) -> Result<PalName, ParseError> {
    let name = PalName::new(name);
    if pal_db.pal(&name).is_some() {
        Ok(name)
    } else {
        Err(ParseError::UnknownPal {
            name: name.as_str().to_owned(),
        })
    }
}

#[derive(Deserialize)]
#[serde(rename_all = "PascalCase")]
struct RawDb {
    version: String,
    pals: Vec<RawPal>,
    passive_skills: Vec<RawPassiveSkill>,
    breeding_gender_probability: HashMap<String, RawGenderProbability>,
    breeding_mechanics: RawBreedingMechanics,
}

#[derive(Deserialize)]
#[serde(rename_all = "PascalCase")]
struct RawPal {
    id: RawPalId,
    name: String,
    internal_name: String,
    breeding_power: u32,
    breeding_power_priority: u32,
    guaranteed_passives_internal_ids: Vec<String>,
}

impl RawPal {
    fn into_model(
        self,
        gender_probabilities: &mut HashMap<String, RawGenderProbability>,
        passives: &HashMap<PassiveName, PassiveSkill>,
    ) -> Result<Pal, ParseError> {
        let raw_probability = gender_probabilities
            .remove(&self.internal_name)
            .ok_or_else(|| ParseError::MissingGenderProbability {
                pal: self.internal_name.clone(),
            })?;
        let gender_probability =
            GenderProbability::new(raw_probability.male, raw_probability.female).map_err(
                |source| ParseError::GenderProbability {
                    pal: self.internal_name.clone(),
                    source,
                },
            )?;
        let guaranteed_passives = self
            .guaranteed_passives_internal_ids
            .into_iter()
            .map(|id| {
                let passive = PassiveName::new(id);
                if passives.contains_key(&passive) {
                    Ok(passive)
                } else {
                    Err(ParseError::UnknownPassive {
                        pal: self.internal_name.clone(),
                        passive: passive.as_str().to_owned(),
                    })
                }
            })
            .collect::<Result<Vec<_>, _>>()?;

        Ok(Pal {
            id: PalId {
                dex: self.id.pal_dex_no,
                variant: self.id.is_variant,
            },
            name: PalName::new(self.internal_name),
            display_name: self.name,
            breeding_power: self.breeding_power,
            breeding_power_priority: self.breeding_power_priority,
            gender_probability,
            guaranteed_passives,
        })
    }
}

#[derive(Deserialize)]
#[serde(rename_all = "PascalCase")]
struct RawPalId {
    pal_dex_no: u16,
    is_variant: bool,
}

#[derive(Deserialize)]
#[serde(rename_all = "PascalCase")]
struct RawPassiveSkill {
    name: String,
    internal_name: String,
    rank: i8,
    is_standard_passive_skill: bool,
    random_inheritance_allowed: bool,
    random_inheritance_weight: u32,
}

impl RawPassiveSkill {
    fn into_model(self) -> PassiveSkill {
        PassiveSkill {
            name: PassiveName::new(self.internal_name),
            display_name: self.name,
            rank: self.rank,
            standard: self.is_standard_passive_skill,
            random_inheritance_allowed: self.random_inheritance_allowed,
            random_inheritance_weight: self.random_inheritance_weight,
        }
    }
}

#[derive(Deserialize)]
struct RawGenderProbability {
    #[serde(rename = "MALE")]
    male: f64,
    #[serde(rename = "FEMALE")]
    female: f64,
}

#[derive(Deserialize)]
#[serde(rename_all = "PascalCase")]
// Field names mirror the upstream JSON keys; the shared postfix is theirs.
#[allow(clippy::struct_field_names)]
struct RawBreedingMechanics {
    #[serde(rename = "IVInheritanceWeights")]
    iv_inheritance_weights: BTreeMap<u8, u32>,
    passive_inheritance_weights: BTreeMap<u8, u32>,
    passive_random_weights: BTreeMap<u8, u32>,
}

impl RawBreedingMechanics {
    fn into_model(self) -> BreedingMechanics {
        BreedingMechanics {
            iv_inheritance_weights: self.iv_inheritance_weights,
            passive_inheritance_weights: self.passive_inheritance_weights,
            passive_random_weights: self.passive_random_weights,
        }
    }
}

#[derive(Deserialize)]
#[serde(rename_all = "PascalCase")]
struct RawBreedingFile {
    breeding: Vec<RawBreedingEntry>,
    min_breeding_steps: HashMap<String, HashMap<String, u32>>,
}

#[derive(Deserialize)]
#[serde(rename_all = "PascalCase")]
struct RawBreedingEntry {
    parent1_internal_name: String,
    parent1_gender: RawParentGender,
    parent2_internal_name: String,
    parent2_gender: RawParentGender,
    child_internal_name: String,
}

impl RawBreedingEntry {
    fn into_model(self, pal_db: &PalDb) -> Result<BreedingCombo, ParseError> {
        Ok(BreedingCombo {
            parents: [
                BreedingParent {
                    name: known_pal(pal_db, self.parent1_internal_name)?,
                    gender: self.parent1_gender.into(),
                },
                BreedingParent {
                    name: known_pal(pal_db, self.parent2_internal_name)?,
                    gender: self.parent2_gender.into(),
                },
            ],
            child: known_pal(pal_db, self.child_internal_name)?,
        })
    }
}

#[derive(Deserialize, Clone, Copy)]
#[serde(rename_all = "UPPERCASE")]
enum RawParentGender {
    Wildcard,
    Male,
    Female,
}

impl From<RawParentGender> for ParentGender {
    fn from(raw: RawParentGender) -> Self {
        match raw {
            RawParentGender::Wildcard => Self::Wildcard,
            RawParentGender::Male => Self::Exactly(Gender::Male),
            RawParentGender::Female => Self::Exactly(Gender::Female),
        }
    }
}
