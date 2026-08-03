//! `pals.toml` — the owned-pal pool the search breeds from.
//!
//! Species and passive names accept internal or display names,
//! case-insensitively; resolution happens here so the rest of the app
//! only sees typed [`OwnedPal`]s. Save-file import (the planned
//! pal-save crate) replaces this file eventually.
//!
//! ```toml
//! [[pals]]
//! species = "Lamball"
//! gender = "male"
//! passives = ["Swift"]
//! ```

use anyhow::{Context, Result, bail};
use pal_core::model::{Gender, PalDb};
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
}

pub fn parse(text: &str, db: &PalDb) -> Result<Vec<OwnedPal>> {
    let raw: RawFile = toml::from_str(text).context("pals file is not valid TOML")?;
    raw.pals.iter().map(|pal| resolve(pal, db)).collect()
}

fn resolve(raw: &RawPal, db: &PalDb) -> Result<OwnedPal> {
    let pal = db
        .find_pal(&raw.species)
        .with_context(|| format!("unknown species {:?}", raw.species))?;
    let gender = match raw.gender.to_ascii_lowercase().as_str() {
        "male" | "m" => Gender::Male,
        "female" | "f" => Gender::Female,
        other => bail!("gender must be male or female, got {other:?}"),
    };
    let passives = raw
        .passives
        .iter()
        .map(|name| {
            db.find_passive(name)
                .map(|skill| skill.name.clone())
                .with_context(|| format!("unknown passive {name:?} on {:?}", raw.species))
        })
        .collect::<Result<Vec<_>>>()?;
    Ok(OwnedPal {
        species: pal.name.clone(),
        gender,
        passives,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::test_support::fixture;
    use pal_core::model::PalName;

    #[test]
    fn resolves_display_names_case_insensitively() {
        let f = fixture();
        let pals = parse(
            r#"
            [[pals]]
            species = "lamball"
            gender = "M"
            passives = ["swift"]

            [[pals]]
            species = "PinkCat"
            gender = "Female"
            "#,
            f.db,
        )
        .unwrap();

        assert_eq!(pals.len(), 2);
        assert_eq!(pals[0].species, PalName::new("SheepBall"));
        assert_eq!(pals[0].gender, Gender::Male);
        assert_eq!(pals[0].passives.len(), 1);
        assert_eq!(pals[1].species, PalName::new("PinkCat"));
    }

    #[test]
    fn unknown_names_are_rejected_with_context() {
        let f = fixture();
        let species_err =
            parse("[[pals]]\nspecies = \"NotAPal\"\ngender = \"male\"\n", f.db).unwrap_err();
        assert!(species_err.to_string().contains("NotAPal"));

        let passive_err = parse(
            "[[pals]]\nspecies = \"Lamball\"\ngender = \"male\"\npassives = [\"NotASkill\"]\n",
            f.db,
        )
        .unwrap_err();
        assert!(passive_err.to_string().contains("NotASkill"));

        let gender_err =
            parse("[[pals]]\nspecies = \"Lamball\"\ngender = \"yes\"\n", f.db).unwrap_err();
        assert!(gender_err.to_string().contains("male or female"));
    }
}
