//! The saved-plan library: a `plans.json` beside the app (like
//! `pals.toml`). Saved plans are fully self-describing — the whole
//! tree with species, genders, and passives inline — so they remain
//! readable as the owned pool changes underneath them.

use std::path::PathBuf;

use anyhow::{Context, Result};
use pal_core::model::{PalName, PassiveName};
use pal_solver::search::PlanNode;
use serde::{Deserialize, Serialize};

#[derive(Clone, PartialEq, Debug, Serialize, Deserialize)]
pub struct SavedPlan {
    pub label: String,
    pub goal_species: PalName,
    pub goal_passives: Vec<PassiveName>,
    pub expected_eggs: f64,
    pub steps: usize,
    pub root: PlanNode,
}

#[derive(Debug)]
pub struct PlanStore {
    path: PathBuf,
    pub plans: Vec<SavedPlan>,
}

impl PlanStore {
    /// An empty store that will write to `path` on first save.
    #[must_use]
    pub fn fresh(path: PathBuf) -> Self {
        Self {
            path,
            plans: Vec::new(),
        }
    }

    /// Loads the store, tolerating a missing file. An unparsable file
    /// is preserved as `<path>.bak` rather than silently clobbered on
    /// the next save.
    pub fn load(path: PathBuf) -> Result<Self> {
        let plans = match std::fs::read_to_string(&path) {
            Err(_) => Vec::new(),
            Ok(text) => match serde_json::from_str(&text) {
                Ok(plans) => plans,
                Err(error) => {
                    let backup = path.with_extension("json.bak");
                    std::fs::rename(&path, &backup).with_context(|| {
                        format!(
                            "{} is unparsable ({error}) and could not be backed up",
                            path.display()
                        )
                    })?;
                    return Err(anyhow::anyhow!(
                        "{} was unparsable ({error}); moved to {}",
                        path.display(),
                        backup.display()
                    ));
                }
            },
        };
        Ok(Self { path, plans })
    }

    /// Appends and persists.
    pub fn add(&mut self, plan: SavedPlan) -> Result<()> {
        self.plans.push(plan);
        self.persist()
    }

    /// Removes by index and persists. Out-of-range indexes are a
    /// no-op.
    pub fn remove(&mut self, index: usize) -> Result<Option<SavedPlan>> {
        if index >= self.plans.len() {
            return Ok(None);
        }
        let removed = self.plans.remove(index);
        self.persist()?;
        Ok(Some(removed))
    }

    fn persist(&self) -> Result<()> {
        let text = serde_json::to_string_pretty(&self.plans).context("serializing plans")?;
        std::fs::write(&self.path, text).with_context(|| format!("writing {}", self.path.display()))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use pal_core::model::Gender;
    use pal_solver::search::OwnedPal;

    fn scratch_path(name: &str) -> PathBuf {
        std::env::temp_dir().join(format!("pal-tui-plan-store-{}-{name}", std::process::id()))
    }

    fn sample(label: &str) -> SavedPlan {
        SavedPlan {
            label: label.to_owned(),
            goal_species: PalName::new("DreamDemon"),
            goal_passives: vec![PassiveName::new("Runner")],
            expected_eggs: 1.67,
            steps: 1,
            root: PlanNode::Owned(OwnedPal {
                species: PalName::new("SheepBall"),
                gender: Gender::Male,
                passives: vec![PassiveName::new("Runner")],
            }),
        }
    }

    #[test]
    fn round_trips_add_load_remove() {
        let path = scratch_path("roundtrip.json");
        let _ = std::fs::remove_file(&path);

        let mut store = PlanStore::load(path.clone()).unwrap();
        assert!(store.plans.is_empty());
        store.add(sample("first")).unwrap();
        store.add(sample("second")).unwrap();

        let reloaded = PlanStore::load(path.clone()).unwrap();
        assert_eq!(reloaded.plans.len(), 2);
        assert_eq!(reloaded.plans[0], sample("first"));

        let mut store = reloaded;
        let removed = store.remove(0).unwrap();
        assert_eq!(removed.unwrap().label, "first");
        assert!(store.remove(5).unwrap().is_none());

        let reloaded = PlanStore::load(path.clone()).unwrap();
        assert_eq!(reloaded.plans.len(), 1);
        assert_eq!(reloaded.plans[0].label, "second");

        let _ = std::fs::remove_file(&path);
    }

    #[test]
    fn unparsable_stores_are_backed_up_not_clobbered() {
        let path = scratch_path("corrupt.json");
        let backup = path.with_extension("json.bak");
        let _ = std::fs::remove_file(&backup);
        std::fs::write(&path, "not json at all").unwrap();

        let error = PlanStore::load(path.clone()).unwrap_err();
        assert!(error.to_string().contains("unparsable"));
        assert!(backup.exists());
        assert!(!path.exists());

        let _ = std::fs::remove_file(&backup);
    }
}
