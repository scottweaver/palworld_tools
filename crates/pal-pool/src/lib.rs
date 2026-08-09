//! Loading the owned pool from disk — a Palworld save or a
//! [`pals_file`] TOML, sniffed by content. Shared by every frontend
//! (TUI startup and reload, the MCP server), so the pool a frontend
//! searches can never drift from what the others would load. A
//! missing file is its own variant because callers differ: startup
//! begins with an empty pool, a reload keeps the current one.

pub mod pals_file;

use pal_core::model::PalDb;
use pal_save::level::SaveError;
use pal_solver::search::OwnedPal;

use crate::pals_file::PalsFileError;

#[derive(Debug)]
pub enum Loaded {
    Pool {
        owned: Vec<OwnedPal>,
        status: String,
    },
    Missing,
}

#[derive(Debug, thiserror::Error)]
pub enum PoolError {
    #[error("reading {path}: {source}")]
    Read {
        path: String,
        source: std::io::Error,
    },
    #[error("parsing save file {path}: {source}")]
    Save { path: String, source: SaveError },
    #[error("{path} is neither a save file nor UTF-8 TOML")]
    NotUtf8 { path: String },
    #[error("in {path}: {source}")]
    PalsFile { path: String, source: PalsFileError },
}

/// Loads the owned pool at `path`: a Palworld `Level.sav` (identical
/// breeding profiles deduped — interchangeable to the solver) or a
/// pals TOML.
///
/// # Errors
///
/// Fails when the file exists but cannot be read, or parses as
/// neither a save file nor a valid pals TOML. A missing file is
/// [`Loaded::Missing`], not an error.
pub fn load(path: &str, db: &PalDb) -> Result<Loaded, PoolError> {
    let bytes = match std::fs::read(path) {
        Ok(bytes) => bytes,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(Loaded::Missing),
        Err(source) => {
            return Err(PoolError::Read {
                path: path.to_owned(),
                source,
            });
        }
    };
    if pal_save::looks_like_sav(&bytes) {
        let save = pal_save::level::read_level_sav(&bytes).map_err(|source| PoolError::Save {
            path: path.to_owned(),
            source,
        })?;
        let report = pal_save::import::import_pals(db, &save.characters);
        let status = format!(
            "{} pal(s) imported from {path} ({} player(s), {} other entries skipped)",
            report.pals.len(),
            report.skipped_players(),
            report.skipped.len() - report.skipped_players(),
        );
        let owned = dedupe_profiles(report.pals);
        let status = format!("{status}; {} unique breeding profile(s)", owned.len());
        Ok(Loaded::Pool { owned, status })
    } else {
        let text = String::from_utf8(bytes).map_err(|_| PoolError::NotUtf8 {
            path: path.to_owned(),
        })?;
        let owned = pals_file::parse(&text, db).map_err(|source| PoolError::PalsFile {
            path: path.to_owned(),
            source,
        })?;
        let status = format!("{} owned pal(s) loaded from {path}", owned.len());
        Ok(Loaded::Pool { owned, status })
    }
}

fn dedupe_profiles(pals: Vec<pal_save::import::ImportedPal>) -> Vec<OwnedPal> {
    let mut owned: Vec<OwnedPal> = Vec::new();
    for pal in pals {
        let candidate = OwnedPal {
            species: pal.species,
            gender: pal.gender,
            passives: pal.passives,
            ivs: pal.ivs,
        };
        if !owned.contains(&candidate) {
            owned.push(candidate);
        }
    }
    owned
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::OnceLock;

    fn db() -> &'static PalDb {
        static DB: OnceLock<PalDb> = OnceLock::new();
        DB.get_or_init(|| pal_core::vendored::pal_db().unwrap())
    }

    fn scratch_path(name: &str) -> std::path::PathBuf {
        std::env::temp_dir().join(format!("pal-pool-{name}-{}.toml", std::process::id()))
    }

    #[test]
    fn missing_file_is_not_an_error() {
        let loaded = load("/nonexistent/pal-pool-test.toml", db()).unwrap();
        assert!(matches!(loaded, Loaded::Missing));
    }

    #[test]
    fn toml_pool_loads_with_status() {
        let path = scratch_path("toml-pool");
        std::fs::write(
            &path,
            "[[pals]]\nspecies = \"Lamball\"\ngender = \"male\"\n",
        )
        .unwrap();
        let loaded = load(path.to_str().unwrap(), db()).unwrap();
        let Loaded::Pool { owned, status } = loaded else {
            panic!("expected a pool");
        };
        assert_eq!(owned.len(), 1);
        assert!(status.contains("1 owned pal(s)"));
        std::fs::remove_file(&path).ok();
    }

    #[test]
    fn unparsable_file_reports_its_path() {
        let path = scratch_path("bad-pool");
        std::fs::write(&path, "not toml = = =").unwrap();
        let error = load(path.to_str().unwrap(), db()).unwrap_err();
        assert!(matches!(error, PoolError::PalsFile { .. }));
        assert!(error.to_string().contains("pal-pool-bad-pool"));
        std::fs::remove_file(&path).ok();
    }
}
