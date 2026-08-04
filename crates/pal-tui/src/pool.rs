//! Loading the owned pool from disk — a Palworld save or a TOML
//! file, sniffed by content. Shared by startup and the in-app
//! reload (F6), which is why a missing file is its own variant:
//! startup begins with an empty pool, a reload keeps the current one.

use anyhow::{Context, Result};
use pal_core::model::PalDb;
use pal_solver::search::OwnedPal;

pub enum Loaded {
    Pool {
        owned: Vec<OwnedPal>,
        status: String,
    },
    Missing,
}

pub fn load(path: &str, db: &PalDb) -> Result<Loaded> {
    let bytes = match std::fs::read(path) {
        Ok(bytes) => bytes,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(Loaded::Missing),
        Err(error) => return Err(error).with_context(|| format!("reading {path}")),
    };
    if pal_save::looks_like_sav(&bytes) {
        let save = pal_save::level::read_level_sav(&bytes)
            .with_context(|| format!("parsing save file {path}"))?;
        let report = pal_save::import::import_pals(db, &save.characters);
        let status = format!(
            "{} pal(s) imported from {path} ({} player(s), {} other entries skipped)",
            report.pals.len(),
            report.skipped_players(),
            report.skipped.len() - report.skipped_players(),
        );
        // Identical breeding profiles are interchangeable to the
        // solver; deduping keeps big box collections fast.
        let mut owned: Vec<OwnedPal> = Vec::new();
        for pal in report.pals {
            let candidate = OwnedPal {
                species: pal.species,
                gender: pal.gender,
                passives: pal.passives,
            };
            if !owned.contains(&candidate) {
                owned.push(candidate);
            }
        }
        let status = format!("{status}; {} unique breeding profile(s)", owned.len());
        Ok(Loaded::Pool { owned, status })
    } else {
        let text = String::from_utf8(bytes)
            .with_context(|| format!("{path} is neither a save file nor UTF-8 TOML"))?;
        let owned = crate::pals_file::parse(&text, db).with_context(|| format!("in {path}"))?;
        let status = format!("{} owned pal(s) loaded from {path}", owned.len());
        Ok(Loaded::Pool { owned, status })
    }
}
