//! `pals.toml` — a hand-written owned-pal pool, the file alternative
//! to save-file import.
//!
//! Species and passive names accept internal or display names,
//! case-insensitively; resolution happens here so callers only see
//! typed [`OwnedPal`]s.
//!
//! ```toml
//! [[pals]]
//! species = "Lamball"
//! gender = "male"
//! passives = ["Swift"]
//! ivs = { hp = 85, attack = 92, defense = 70 }  # optional; 0 if omitted
//! ```

use pal_core::model::{Gender, IvSpread, PalDb};
use pal_solver::search::OwnedPal;
use serde::Deserialize;

#[derive(Deserialize)]
struct RawFile {
    #[serde(default)]
    pals: Vec<RawPal>,
}

#[derive(Deserialize)]
struct RawPal {
    species: String,
    gender: String,
    #[serde(default)]
    passives: Vec<String>,
    #[serde(default)]
    ivs: IvSpread,
}

#[derive(Debug, thiserror::Error)]
pub enum PalsFileError {
    #[error("pals file is not valid TOML: {0}")]
    InvalidToml(#[from] toml::de::Error),
    #[error("unknown species {species:?}")]
    UnknownSpecies { species: String },
    #[error("gender must be male or female, got {gender:?}")]
    InvalidGender { gender: String },
    #[error("unknown passive {passive:?} on {species:?}")]
    UnknownPassive { passive: String, species: String },
}

/// Parses a pals TOML against `db`.
///
/// # Errors
///
/// Fails on invalid TOML or on any species, passive, or gender that
/// does not resolve.
pub fn parse(text: &str, db: &PalDb) -> Result<Vec<OwnedPal>, PalsFileError> {
    let raw: RawFile = toml::from_str(text)?;
    raw.pals.iter().map(|pal| resolve(pal, db)).collect()
}

fn resolve(raw: &RawPal, db: &PalDb) -> Result<OwnedPal, PalsFileError> {
    let pal = db
        .find_pal(&raw.species)
        .ok_or_else(|| PalsFileError::UnknownSpecies {
            species: raw.species.clone(),
        })?;
    let gender = match raw.gender.to_ascii_lowercase().as_str() {
        "male" | "m" => Gender::Male,
        "female" | "f" => Gender::Female,
        _ => {
            return Err(PalsFileError::InvalidGender {
                gender: raw.gender.clone(),
            });
        }
    };
    let passives = raw
        .passives
        .iter()
        .map(|name| {
            db.find_passive(name)
                .map(|skill| skill.name.clone())
                .ok_or_else(|| PalsFileError::UnknownPassive {
                    passive: name.clone(),
                    species: raw.species.clone(),
                })
        })
        .collect::<Result<Vec<_>, _>>()?;
    Ok(OwnedPal {
        species: pal.name.clone(),
        gender,
        passives,
        ivs: raw.ivs,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use pal_core::model::PalName;
    use std::sync::OnceLock;

    fn db() -> &'static PalDb {
        static DB: OnceLock<PalDb> = OnceLock::new();
        DB.get_or_init(|| pal_core::vendored::pal_db().unwrap())
    }

    #[test]
    fn resolves_display_names_case_insensitively() {
        let pals = parse(
            r#"
            [[pals]]
            species = "lamball"
            gender = "M"
            passives = ["swift"]
            ivs = { hp = 85, attack = 92 }

            [[pals]]
            species = "PinkCat"
            gender = "Female"
            "#,
            db(),
        )
        .unwrap();

        assert_eq!(pals.len(), 2);
        assert_eq!(pals[0].species, PalName::new("SheepBall"));
        assert_eq!(pals[0].gender, Gender::Male);
        assert_eq!(pals[0].passives.len(), 1);
        assert_eq!(pals[0].ivs.hp.get(), 85);
        assert_eq!(pals[0].ivs.attack.get(), 92);
        assert_eq!(pals[0].ivs.defense.get(), 0);
        assert_eq!(pals[1].species, PalName::new("PinkCat"));
        assert_eq!(pals[1].ivs, IvSpread::default());
    }

    #[test]
    fn out_of_range_ivs_are_rejected() {
        let error = parse(
            "[[pals]]\nspecies = \"Lamball\"\ngender = \"male\"\nivs = { hp = 101 }\n",
            db(),
        )
        .unwrap_err();
        assert!(matches!(error, PalsFileError::InvalidToml(_)));
        assert!(error.to_string().contains("TOML"));
    }

    #[test]
    fn unknown_names_are_rejected_with_context() {
        let species_err =
            parse("[[pals]]\nspecies = \"NotAPal\"\ngender = \"male\"\n", db()).unwrap_err();
        assert!(matches!(species_err, PalsFileError::UnknownSpecies { .. }));
        assert!(species_err.to_string().contains("NotAPal"));

        let passive_err = parse(
            "[[pals]]\nspecies = \"Lamball\"\ngender = \"male\"\npassives = [\"NotASkill\"]\n",
            db(),
        )
        .unwrap_err();
        assert!(matches!(passive_err, PalsFileError::UnknownPassive { .. }));
        assert!(passive_err.to_string().contains("NotASkill"));

        let gender_err =
            parse("[[pals]]\nspecies = \"Lamball\"\ngender = \"yes\"\n", db()).unwrap_err();
        assert!(matches!(gender_err, PalsFileError::InvalidGender { .. }));
        assert!(gender_err.to_string().contains("male or female"));
    }
}
