//! Resolution of raw save characters against the pal database:
//! species, gender, and passives become typed values; everything that
//! cannot resolve is reported, never silently dropped.

use pal_core::model::{Gender, PalDb, PalName, PassiveName};

use crate::level::RawCharacter;

/// A pal imported from a save, in solver-ready terms.
#[derive(Clone, PartialEq, Eq, Debug)]
pub struct ImportedPal {
    pub species: PalName,
    pub gender: Gender,
    pub passives: Vec<PassiveName>,
}

/// Why a character entry did not become an [`ImportedPal`].
#[derive(Clone, PartialEq, Eq, Debug)]
pub enum SkipReason {
    /// The entry is a player character, not a pal.
    Player,
    /// No `CharacterID` present (system entries).
    NoCharacterId,
    /// The species does not resolve in the database.
    UnknownSpecies(String),
    /// The gender value is absent or unrecognized.
    UnknownGender(Option<String>),
}

#[derive(Debug, Default)]
pub struct ImportReport {
    pub pals: Vec<ImportedPal>,
    pub skipped: Vec<SkipReason>,
    /// Passive ids that did not resolve; their pals are kept without
    /// the unknown passive.
    pub unknown_passives: Vec<String>,
}

impl ImportReport {
    /// Skipped-entry count for a compact status line.
    #[must_use]
    pub fn skipped_players(&self) -> usize {
        self.skipped
            .iter()
            .filter(|reason| matches!(reason, SkipReason::Player))
            .count()
    }
}

/// Character ids carry role prefixes for special spawns; the species
/// name follows the prefix (e.g. `BOSS_SheepBall` breeds as
/// `SheepBall`).
const ID_PREFIXES: &[&str] = &["BOSS_", "PREDATOR_", "RAID_", "GYM_"];

/// Resolves raw characters into solver-ready pals.
#[must_use]
pub fn import_pals(pal_db: &PalDb, characters: &[RawCharacter]) -> ImportReport {
    let mut report = ImportReport::default();
    for character in characters {
        match resolve(pal_db, character, &mut report.unknown_passives) {
            Ok(pal) => report.pals.push(pal),
            Err(reason) => report.skipped.push(reason),
        }
    }
    report
}

fn resolve(
    pal_db: &PalDb,
    character: &RawCharacter,
    unknown_passives: &mut Vec<String>,
) -> Result<ImportedPal, SkipReason> {
    if character.is_player {
        return Err(SkipReason::Player);
    }
    let raw_id = character
        .character_id
        .as_deref()
        .filter(|id| !id.is_empty())
        .ok_or(SkipReason::NoCharacterId)?;
    let species_id = strip_role_prefix(raw_id);
    let pal = pal_db
        .find_pal(species_id)
        .ok_or_else(|| SkipReason::UnknownSpecies(raw_id.to_owned()))?;

    let gender = match character.gender.as_deref() {
        Some(value) if value.ends_with("Male") && !value.ends_with("Female") => Gender::Male,
        Some(value) if value.ends_with("Female") => Gender::Female,
        other => return Err(SkipReason::UnknownGender(other.map(str::to_owned))),
    };

    let passives = character
        .passives
        .iter()
        .filter_map(|id| {
            if let Some(skill) = pal_db.find_passive(id) {
                Some(skill.name.clone())
            } else {
                unknown_passives.push(id.clone());
                None
            }
        })
        .collect();

    Ok(ImportedPal {
        species: pal.name.clone(),
        gender,
        passives,
    })
}

fn strip_role_prefix(id: &str) -> &str {
    for prefix in ID_PREFIXES {
        if id.len() > prefix.len() && id[..prefix.len()].eq_ignore_ascii_case(prefix) {
            return &id[prefix.len()..];
        }
    }
    id
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    use std::path::PathBuf;
    use std::sync::OnceLock;

    fn pal_db() -> &'static PalDb {
        static DB: OnceLock<PalDb> = OnceLock::new();
        DB.get_or_init(|| {
            let path = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../data/db.json");
            pal_core::db::parse_pal_db(&fs::read_to_string(path).unwrap()).unwrap()
        })
    }

    fn raw(id: &str, gender: &str, passives: &[&str]) -> RawCharacter {
        RawCharacter {
            character_id: Some(id.to_owned()),
            gender: Some(gender.to_owned()),
            passives: passives.iter().map(|p| (*p).to_owned()).collect(),
            is_player: false,
        }
    }

    #[test]
    fn resolves_pals_players_and_boss_prefixes() {
        let characters = vec![
            raw("SheepBall", "EPalGenderType::Male", &[]),
            raw("BOSS_PinkCat", "EPalGenderType::Female", &[]),
            RawCharacter {
                character_id: Some("PlayerBody".to_owned()),
                is_player: true,
                ..RawCharacter::default()
            },
            raw("NotASpecies", "EPalGenderType::Male", &[]),
            RawCharacter::default(),
        ];
        let report = import_pals(pal_db(), &characters);

        assert_eq!(report.pals.len(), 2);
        assert_eq!(report.pals[0].species, PalName::new("SheepBall"));
        assert_eq!(report.pals[0].gender, Gender::Male);
        assert_eq!(report.pals[1].species, PalName::new("PinkCat"));
        assert_eq!(report.pals[1].gender, Gender::Female);

        assert_eq!(report.skipped_players(), 1);
        assert!(
            report
                .skipped
                .contains(&SkipReason::UnknownSpecies("NotASpecies".to_owned()))
        );
        assert!(report.skipped.contains(&SkipReason::NoCharacterId));
    }

    #[test]
    fn passives_resolve_and_unknowns_are_reported_not_fatal() {
        let lamball_passive = pal_db()
            .passives()
            .find(|skill| skill.standard)
            .unwrap()
            .name
            .as_str()
            .to_owned();
        let characters = vec![raw(
            "SheepBall",
            "EPalGenderType::Male",
            &[lamball_passive.as_str(), "TotallyFakePassive"],
        )];
        let report = import_pals(pal_db(), &characters);

        assert_eq!(report.pals.len(), 1);
        assert_eq!(report.pals[0].passives.len(), 1);
        assert_eq!(
            report.unknown_passives,
            vec!["TotallyFakePassive".to_owned()]
        );
    }

    #[test]
    fn gender_values_must_be_recognized() {
        let characters = vec![raw("SheepBall", "EPalGenderType::Hermaphrodite", &[])];
        let report = import_pals(pal_db(), &characters);
        assert!(report.pals.is_empty());
        assert!(matches!(
            &report.skipped[0],
            SkipReason::UnknownGender(Some(value)) if value.contains("Hermaphrodite")
        ));
    }
}
