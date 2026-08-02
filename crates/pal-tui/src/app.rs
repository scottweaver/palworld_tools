//! Application state and key handling — pure transitions over the
//! solver API, testable without a terminal. Rendering lives in
//! [`crate::ui`]; nothing here draws.

use pal_core::model::{Gender, Pal, PalDb, PalName, PassiveName, PassiveSkill};
use pal_solver::child::ChildIndex;
use pal_solver::passives::{MAX_TOTAL_PASSIVES, PassiveOdds};
use pal_solver::search::{BreedingGoal, BreedingPlan, OwnedPal, SearchConfig, find_paths};
use ratatui::crossterm::event::{KeyCode, KeyEvent};

const MAX_RESULTS: usize = 5;
const DEFAULT_BREEDING_STEPS: usize = 3;
pub const MIN_BREEDING_STEPS: usize = 1;
/// Species reach never needs more than 7 steps (the vendored
/// min-steps matrix tops out there); one extra for passive
/// consolidation. Deeper searches also get slow on the UI thread.
pub const MAX_BREEDING_STEPS: usize = 8;

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Pane {
    Species,
    Passives,
    Results,
}

pub struct App<'db> {
    db: &'db PalDb,
    index: &'db ChildIndex,
    odds: &'db PassiveOdds,
    pub owned: Vec<OwnedPal>,
    pub focus: Pane,
    pub species_filter: String,
    pub species_cursor: usize,
    pub target: Option<PalName>,
    /// Progenitor species marked in the Species pane. Non-empty marks
    /// switch the search to progenitor mode: plans start from a free
    /// male + female pair of each marked species and nothing else —
    /// the toml pool and wild captures stay out.
    pub progenitors: Vec<PalName>,
    pub passive_filter: String,
    pub passive_cursor: usize,
    pub selected_passives: Vec<PassiveName>,
    pub plans: Vec<BreedingPlan>,
    pub plan_cursor: usize,
    pub max_breeding_steps: usize,
    pub allow_wild: bool,
    pub status: String,
    pub should_quit: bool,
}

impl<'db> App<'db> {
    #[must_use]
    pub fn new(
        db: &'db PalDb,
        index: &'db ChildIndex,
        odds: &'db PassiveOdds,
        owned: Vec<OwnedPal>,
    ) -> Self {
        Self {
            db,
            index,
            odds,
            owned,
            focus: Pane::Species,
            species_filter: String::new(),
            species_cursor: 0,
            target: None,
            progenitors: Vec::new(),
            passive_filter: String::new(),
            passive_cursor: 0,
            selected_passives: Vec::new(),
            plans: Vec::new(),
            plan_cursor: 0,
            max_breeding_steps: DEFAULT_BREEDING_STEPS,
            allow_wild: true,
            status: String::new(),
            should_quit: false,
        }
    }

    #[must_use]
    pub fn db(&self) -> &'db PalDb {
        self.db
    }

    /// Species matching the filter, sorted by display name.
    #[must_use]
    pub fn species_rows(&self) -> Vec<&'db Pal> {
        let mut rows: Vec<&Pal> = self
            .db
            .pals()
            .filter(|pal| {
                matches_filter(&self.species_filter, &pal.display_name, pal.name.as_str())
            })
            .collect();
        rows.sort_by(|a, b| a.display_name.cmp(&b.display_name));
        rows
    }

    /// Standard passives matching the filter, sorted by display name.
    #[must_use]
    pub fn passive_rows(&self) -> Vec<&'db PassiveSkill> {
        let mut rows: Vec<&PassiveSkill> = self
            .db
            .passives()
            .filter(|skill| skill.standard)
            .filter(|skill| {
                matches_filter(
                    &self.passive_filter,
                    &skill.display_name,
                    skill.name.as_str(),
                )
            })
            .collect();
        rows.sort_by(|a, b| a.display_name.cmp(&b.display_name));
        rows
    }

    pub fn handle_key(&mut self, key: KeyEvent) {
        match key.code {
            KeyCode::Esc => self.should_quit = true,
            KeyCode::Tab => self.focus = next_pane(self.focus),
            KeyCode::BackTab => self.focus = next_pane(next_pane(self.focus)),
            KeyCode::Up => self.move_cursor(-1),
            KeyCode::Down => self.move_cursor(1),
            KeyCode::Left => self.adjust_depth(-1),
            KeyCode::Right => self.adjust_depth(1),
            KeyCode::Enter => self.confirm(),
            KeyCode::F(2) => self.toggle_wild(),
            KeyCode::F(4) => self.toggle_progenitor(),
            KeyCode::F(5) => self.run_search(),
            KeyCode::Backspace => {
                if let Some(filter) = self.active_filter() {
                    filter.pop();
                    self.reset_cursor();
                }
            }
            KeyCode::Char(c) => {
                if let Some(filter) = self.active_filter() {
                    filter.push(c);
                    self.reset_cursor();
                }
            }
            _ => {}
        }
    }

    pub fn run_search(&mut self) {
        let Some(target) = self.target.clone() else {
            "pick a target species first (Enter in the Species pane)".clone_into(&mut self.status);
            return;
        };
        let goal = BreedingGoal {
            species: target,
            passives: self.selected_passives.clone(),
        };
        let progenitor_mode = !self.progenitors.is_empty();
        let pool = if progenitor_mode {
            progenitor_pool(&self.progenitors)
        } else {
            self.owned.clone()
        };
        let config = SearchConfig {
            max_breeding_steps: self.max_breeding_steps,
            max_results: MAX_RESULTS,
            allow_wild_pals: self.allow_wild && !progenitor_mode,
        };
        match find_paths(self.db, self.index, self.odds, &pool, &goal, &config) {
            Ok(plans) => {
                self.status = if plans.is_empty() {
                    if progenitor_mode && !self.selected_passives.is_empty() {
                        "no plans — species progenitors carry no passives; \
                         deselect passives or plan from pals.toml"
                            .to_owned()
                    } else {
                        format!(
                            "no plans within {} step(s) — raise the depth with → or check the pool",
                            self.max_breeding_steps
                        )
                    }
                } else if progenitor_mode {
                    format!(
                        "{} plan(s) from {} progenitor species",
                        plans.len(),
                        self.progenitors.len()
                    )
                } else {
                    format!("{} plan(s) found", plans.len())
                };
                self.plans = plans;
                self.plan_cursor = 0;
                if !self.plans.is_empty() {
                    self.focus = Pane::Results;
                }
            }
            Err(error) => self.status = error.to_string(),
        }
    }

    fn confirm(&mut self) {
        match self.focus {
            Pane::Species => {
                if let Some(pal) = self.species_rows().get(self.species_cursor).copied() {
                    self.target = Some(pal.name.clone());
                    self.status = format!("target: {}", pal.display_name);
                }
            }
            Pane::Passives => {
                if let Some(skill) = self.passive_rows().get(self.passive_cursor).copied() {
                    self.toggle_passive(skill.name.clone(), &skill.display_name);
                }
            }
            Pane::Results => self.run_search(),
        }
    }

    fn toggle_passive(&mut self, name: PassiveName, display: &str) {
        if let Some(position) = self.selected_passives.iter().position(|p| *p == name) {
            self.selected_passives.remove(position);
            self.status = format!("dropped {display}");
        } else if self.selected_passives.len() == MAX_TOTAL_PASSIVES {
            self.status = format!("a pal carries at most {MAX_TOTAL_PASSIVES} passives");
        } else {
            self.selected_passives.push(name);
            self.status = format!("added {display}");
        }
    }

    fn toggle_progenitor(&mut self) {
        if self.focus != Pane::Species {
            return;
        }
        let Some(pal) = self.species_rows().get(self.species_cursor).copied() else {
            return;
        };
        if let Some(position) = self.progenitors.iter().position(|p| *p == pal.name) {
            self.progenitors.remove(position);
            self.status = format!("progenitor removed: {}", pal.display_name);
        } else {
            self.progenitors.push(pal.name.clone());
            self.status = format!(
                "progenitor added: {} — plans will start from the {} marked species only",
                pal.display_name,
                self.progenitors.len()
            );
        }
    }

    fn toggle_wild(&mut self) {
        self.allow_wild = !self.allow_wild;
        self.status = if self.allow_wild {
            "wild pals: on — any catchable species can join plans".to_owned()
        } else {
            "wild pals: off — plans use only the owned pool".to_owned()
        };
    }

    fn adjust_depth(&mut self, delta: isize) {
        self.max_breeding_steps = self
            .max_breeding_steps
            .saturating_add_signed(delta)
            .clamp(MIN_BREEDING_STEPS, MAX_BREEDING_STEPS);
        self.status = format!(
            "search depth: up to {} breeding step(s)",
            self.max_breeding_steps
        );
    }

    fn move_cursor(&mut self, delta: isize) {
        let len = match self.focus {
            Pane::Species => self.species_rows().len(),
            Pane::Passives => self.passive_rows().len(),
            Pane::Results => self.plans.len(),
        };
        let cursor = match self.focus {
            Pane::Species => &mut self.species_cursor,
            Pane::Passives => &mut self.passive_cursor,
            Pane::Results => &mut self.plan_cursor,
        };
        *cursor = step(*cursor, delta, len);
    }

    fn active_filter(&mut self) -> Option<&mut String> {
        match self.focus {
            Pane::Species => Some(&mut self.species_filter),
            Pane::Passives => Some(&mut self.passive_filter),
            Pane::Results => None,
        }
    }

    fn reset_cursor(&mut self) {
        match self.focus {
            Pane::Species => self.species_cursor = 0,
            Pane::Passives => self.passive_cursor = 0,
            Pane::Results => {}
        }
    }
}

/// A free male + female pair per marked species: "I will obtain a
/// breeding pair of each of these" is the progenitor-mode premise.
fn progenitor_pool(progenitors: &[PalName]) -> Vec<OwnedPal> {
    progenitors
        .iter()
        .flat_map(|species| {
            [Gender::Male, Gender::Female].map(|gender| OwnedPal {
                species: species.clone(),
                gender,
                passives: Vec::new(),
            })
        })
        .collect()
}

fn matches_filter(filter: &str, display_name: &str, internal_name: &str) -> bool {
    filter.is_empty()
        || display_name
            .to_ascii_lowercase()
            .contains(&filter.to_ascii_lowercase())
        || internal_name
            .to_ascii_lowercase()
            .contains(&filter.to_ascii_lowercase())
}

fn next_pane(pane: Pane) -> Pane {
    match pane {
        Pane::Species => Pane::Passives,
        Pane::Passives => Pane::Results,
        Pane::Results => Pane::Species,
    }
}

fn step(cursor: usize, delta: isize, len: usize) -> usize {
    if len == 0 {
        return 0;
    }
    let moved = cursor.saturating_add_signed(delta);
    moved.min(len - 1)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::test_support::fixture;
    use pal_core::model::Gender;

    fn key(code: KeyCode) -> KeyEvent {
        KeyEvent::from(code)
    }

    fn app() -> App<'static> {
        let f = fixture();
        App::new(&f.db, &f.index, &f.odds, Vec::new())
    }

    #[test]
    fn typing_filters_the_species_list_and_enter_selects_the_target() {
        let mut app = app();
        for c in "lamball".chars() {
            app.handle_key(key(KeyCode::Char(c)));
        }
        let rows = app.species_rows();
        assert_eq!(rows.len(), 1);
        assert_eq!(rows[0].display_name, "Lamball");

        app.handle_key(key(KeyCode::Enter));
        assert_eq!(app.target, Some(PalName::new("SheepBall")));
    }

    #[test]
    fn passive_selection_caps_at_four() {
        let mut app = app();
        app.focus = Pane::Passives;
        let names: Vec<PassiveName> = app
            .passive_rows()
            .iter()
            .take(5)
            .map(|skill| skill.name.clone())
            .collect();
        assert!(names.len() == 5, "expected at least 5 standard passives");
        for position in 0..names.len() {
            app.passive_cursor = position;
            app.handle_key(key(KeyCode::Enter));
        }
        assert_eq!(app.selected_passives.len(), 4);
        assert!(!app.selected_passives.contains(&names[4]));
        assert!(app.status.contains("at most"));
    }

    #[test]
    fn search_without_a_target_reports_instead_of_running() {
        let mut app = app();
        app.handle_key(key(KeyCode::F(5)));
        assert!(app.plans.is_empty());
        assert!(app.status.contains("target"));
    }

    #[test]
    fn f4_marks_progenitors_and_search_starts_from_them_only() {
        let f = fixture();
        let mut app = App::new(&f.db, &f.index, &f.odds, Vec::new());

        for c in "lamball".chars() {
            app.handle_key(key(KeyCode::Char(c)));
        }
        app.handle_key(key(KeyCode::F(4)));
        assert_eq!(app.progenitors, vec![PalName::new("SheepBall")]);

        app.species_filter.clear();
        for c in "cattiva".chars() {
            app.handle_key(key(KeyCode::Char(c)));
        }
        app.handle_key(key(KeyCode::F(4)));
        assert_eq!(app.progenitors.len(), 2);

        // Wild mode stays on, but progenitor marks must override it:
        // plans may only start from the two marked species.
        assert!(app.allow_wild);
        app.target = Some(PalName::new("BluePlatypus"));
        app.run_search();

        assert!(!app.plans.is_empty());
        let best = &app.plans[0];
        assert_eq!(best.steps, 2);
        let mut leaves = Vec::new();
        collect_leaf_species(&best.root, &mut leaves);
        for species in leaves {
            assert!(
                species == PalName::new("SheepBall") || species == PalName::new("PinkCat"),
                "unexpected leaf species {species}"
            );
        }
        assert!(app.status.contains("2 progenitor species"));

        // Unmarking returns to pool planning.
        app.focus = Pane::Species;
        app.handle_key(key(KeyCode::F(4)));
        assert_eq!(app.progenitors.len(), 1);
    }

    #[test]
    fn progenitor_mode_reports_that_passives_need_the_pool() {
        let f = fixture();
        let mut app = App::new(&f.db, &f.index, &f.odds, Vec::new());
        app.progenitors = vec![PalName::new("SheepBall"), PalName::new("PinkCat")];
        app.target = Some(PalName::new("DreamDemon"));
        app.selected_passives = vec![PassiveName::new("Swift")];
        app.run_search();

        assert!(app.plans.is_empty());
        assert!(app.status.contains("carry no passives"));
    }

    fn collect_leaf_species(node: &pal_solver::search::PlanNode, out: &mut Vec<PalName>) {
        match node {
            pal_solver::search::PlanNode::Owned(pal) => out.push(pal.species.clone()),
            pal_solver::search::PlanNode::Wild(species) => out.push(species.clone()),
            pal_solver::search::PlanNode::Bred(bred) => {
                collect_leaf_species(&bred.male, out);
                collect_leaf_species(&bred.female, out);
            }
        }
    }

    #[test]
    fn f2_toggles_wild_mode_and_search_honors_it() {
        let f = fixture();
        let mut app = App::new(&f.db, &f.index, &f.odds, Vec::new());
        assert!(app.allow_wild);

        app.handle_key(key(KeyCode::F(2)));
        assert!(!app.allow_wild);
        assert!(app.status.contains("off"));
        app.handle_key(key(KeyCode::F(2)));
        assert!(app.allow_wild);

        // Empty pool, catchable target: wild mode finds the catch plan.
        app.target = Some(PalName::new("SheepBall"));
        app.run_search();
        assert!(!app.plans.is_empty());
        assert_eq!(app.plans[0].steps, 0);
    }

    #[test]
    fn arrow_keys_adjust_search_depth_within_bounds() {
        let mut app = app();
        assert_eq!(app.max_breeding_steps, 3);

        app.handle_key(key(KeyCode::Right));
        assert_eq!(app.max_breeding_steps, 4);
        assert!(app.status.contains("4 breeding step"));

        for _ in 0..20 {
            app.handle_key(key(KeyCode::Left));
        }
        assert_eq!(app.max_breeding_steps, MIN_BREEDING_STEPS);
        for _ in 0..20 {
            app.handle_key(key(KeyCode::Right));
        }
        assert_eq!(app.max_breeding_steps, MAX_BREEDING_STEPS);
    }

    #[test]
    fn search_depth_gates_which_targets_are_reachable() {
        let f = fixture();
        let owned = vec![
            OwnedPal {
                species: PalName::new("SheepBall"),
                gender: Gender::Male,
                passives: Vec::new(),
            },
            OwnedPal {
                species: PalName::new("PinkCat"),
                gender: Gender::Female,
                passives: Vec::new(),
            },
        ];
        let mut app = App::new(&f.db, &f.index, &f.odds, owned);
        // Fuack needs two generations from this pool (via Daedream).
        app.target = Some(PalName::new("BluePlatypus"));
        app.allow_wild = false;

        app.max_breeding_steps = 1;
        app.run_search();
        assert!(app.plans.is_empty());
        assert!(app.status.contains("within 1 step"));

        app.max_breeding_steps = 2;
        app.run_search();
        assert!(!app.plans.is_empty());
        assert_eq!(app.plans[0].steps, 2);
    }

    #[test]
    fn search_returns_ranked_plans_and_focuses_results() {
        let f = fixture();
        let owned = vec![
            OwnedPal {
                species: PalName::new("SheepBall"),
                gender: Gender::Male,
                passives: Vec::new(),
            },
            OwnedPal {
                species: PalName::new("PinkCat"),
                gender: Gender::Female,
                passives: Vec::new(),
            },
        ];
        let mut app = App::new(&f.db, &f.index, &f.odds, owned);
        app.target = Some(PalName::new("DreamDemon"));
        app.allow_wild = false;
        app.run_search();

        assert!(!app.plans.is_empty());
        assert_eq!(app.plans[0].steps, 1);
        assert_eq!(app.focus, Pane::Results);
        for window in app.plans.windows(2) {
            assert!(window[0].expected_eggs <= window[1].expected_eggs);
        }
    }
}
