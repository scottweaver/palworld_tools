//! Application state and key handling — pure transitions over the
//! solver API, testable without a terminal. Rendering lives in
//! [`crate::ui`]; nothing here draws.

use pal_core::model::{Pal, PalDb, PalName, PassiveName, PassiveSkill};
use pal_solver::passives::MAX_TOTAL_PASSIVES;
use pal_solver::search::{
    BreedingGoal, BreedingPlan, MAX_PROGENITORS, OwnedPal, SearchConfig, Solver,
};
use ratatui::crossterm::event::{KeyCode, KeyEvent, KeyModifiers};

use crate::plan_store::{PlanStore, SavedPlan};

const MAX_RESULTS: usize = 5;
const DEFAULT_BREEDING_STEPS: usize = 3;
pub const MIN_BREEDING_STEPS: usize = 1;
/// Deep plans matter with wild mode off, where routing through a
/// limited owned pool can far exceed the 7-step wild-partner bound.
/// The solver's incumbent cut makes deep searches converge early, so
/// a high ceiling costs little.
pub const MAX_BREEDING_STEPS: usize = 24;

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Pane {
    Pals,
    Passives,
    Results,
}

/// A resolved mouse click: the pane it landed in, and the list row
/// under the pointer when it hit one.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct Click {
    pub pane: Pane,
    pub row: Option<usize>,
}

pub struct App<'db> {
    solver: &'db Solver<'db>,
    pub owned: Vec<OwnedPal>,
    pub focus: Pane,
    pub species_filter: String,
    pub species_cursor: usize,
    pub target: Option<PalName>,
    /// Progenitor species marked in the Pals pane. Non-empty marks
    /// switch the search to progenitor mode: every plan must include
    /// each marked pal, with catchable wild partners recruited around
    /// them; the toml pool stays out.
    pub progenitors: Vec<PalName>,
    pub passive_filter: String,
    pub passive_cursor: usize,
    pub selected_passives: Vec<PassiveName>,
    pub plans: Vec<BreedingPlan>,
    pub plan_cursor: usize,
    pub saved: PlanStore,
    /// When set, the Plans pane shows the saved-plan library instead
    /// of live search results.
    pub viewing_saved: bool,
    pub saved_cursor: usize,
    pub max_breeding_steps: usize,
    pub allow_wild: bool,
    pub status: String,
    pub should_quit: bool,
}

impl<'db> App<'db> {
    #[must_use]
    pub fn new(solver: &'db Solver<'db>, owned: Vec<OwnedPal>, saved: PlanStore) -> Self {
        Self {
            solver,
            owned,
            saved,
            viewing_saved: false,
            saved_cursor: 0,
            focus: Pane::Pals,
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
            allow_wild: false,
            status: String::new(),
            should_quit: false,
        }
    }

    #[must_use]
    pub fn db(&self) -> &'db PalDb {
        self.solver.pal_db()
    }

    /// Rows for the Pals pane: marked progenitors pinned at the top
    /// (in marking order, shown even when the filter would exclude
    /// them, so each stays one keypress from unmarking), then every
    /// filter match sorted by display name.
    #[must_use]
    pub fn species_rows(&self) -> Vec<&'db Pal> {
        let pinned: Vec<&Pal> = self
            .progenitors
            .iter()
            .filter_map(|name| self.db().pal(name))
            .collect();
        let mut rest: Vec<&Pal> = self
            .db()
            .pals()
            .filter(|pal| !self.progenitors.contains(&pal.name))
            .filter(|pal| {
                matches_filter(&self.species_filter, &pal.display_name, pal.name.as_str())
            })
            .collect();
        rest.sort_by(|a, b| a.display_name.cmp(&b.display_name));
        let mut rows = pinned;
        rows.extend(rest);
        rows
    }

    /// Rows for the Passives pane: selected passives pinned at the
    /// top (in selection order, shown even when the filter would
    /// exclude them, so each stays one keypress from deselecting),
    /// then every standard filter match sorted by display name.
    #[must_use]
    pub fn passive_rows(&self) -> Vec<&'db PassiveSkill> {
        let pinned: Vec<&PassiveSkill> = self
            .selected_passives
            .iter()
            .filter_map(|name| self.db().passive(name))
            .collect();
        let mut rest: Vec<&PassiveSkill> = self
            .db()
            .passives()
            .filter(|skill| skill.standard)
            .filter(|skill| !self.selected_passives.contains(&skill.name))
            .filter(|skill| {
                matches_filter(
                    &self.passive_filter,
                    &skill.display_name,
                    skill.name.as_str(),
                )
            })
            .collect();
        rest.sort_by(|a, b| a.display_name.cmp(&b.display_name));
        let mut rows = pinned;
        rows.extend(rest);
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
            KeyCode::F(8) => self.save_current_plan(),
            KeyCode::F(9) => self.toggle_saved_view(),
            // Ctrl+D works on every keyboard; the Delete key needs
            // Fn on Mac laptops but stays supported.
            KeyCode::Char('d' | 'D') if key.modifiers.contains(KeyModifiers::CONTROL) => {
                self.contextual_delete();
            }
            KeyCode::Delete => self.contextual_delete(),
            KeyCode::Backspace => {
                // In the library the Mac delete key (= Backspace)
                // deletes the highlighted plan; elsewhere it edits
                // the focused filter.
                if self.viewing_saved && self.focus == Pane::Results {
                    self.delete_saved_plan();
                } else if let Some(filter) = self.active_filter() {
                    filter.pop();
                    self.reset_cursor();
                }
            }
            KeyCode::Char(c) => {
                if !key.modifiers.contains(KeyModifiers::CONTROL)
                    && let Some(filter) = self.active_filter()
                {
                    filter.push(c);
                    self.reset_cursor();
                }
            }
            _ => {}
        }
    }

    /// Delete in context: the highlighted saved plan when the
    /// library is open, otherwise all progenitor marks.
    fn contextual_delete(&mut self) {
        if self.viewing_saved {
            self.delete_saved_plan();
        } else {
            self.clear_progenitors();
        }
    }

    /// Mouse selection: clicking focuses the pane; clicking a row
    /// acts on it — target in the Pals pane (⇧ marks a progenitor
    /// instead), toggle in Passives, plan selection in Results.
    pub fn handle_click(&mut self, click: Click, shift: bool) {
        self.focus = click.pane;
        let Some(index) = click.row else { return };
        match click.pane {
            Pane::Pals => {
                if index < self.species_rows().len() {
                    self.species_cursor = index;
                    if shift {
                        self.toggle_progenitor();
                    } else {
                        self.confirm();
                    }
                }
            }
            Pane::Passives => {
                if index < self.passive_rows().len() {
                    self.passive_cursor = index;
                    self.confirm();
                }
            }
            Pane::Results if self.viewing_saved => {
                if index < self.saved.plans.len() {
                    self.saved_cursor = index;
                }
            }
            Pane::Results => {
                if index < self.plans.len() {
                    self.plan_cursor = index;
                }
            }
        }
    }

    pub fn run_search(&mut self) {
        self.search(true);
    }

    /// Re-runs the search after a state change: silently when no
    /// target is picked yet (plans just stay empty), and without
    /// stealing focus — only an explicit F5/Enter jumps to Results.
    /// Changing the live question also leaves the saved-plan view.
    fn refresh_plans(&mut self) {
        self.viewing_saved = false;
        if self.target.is_some() {
            self.search(false);
        } else {
            self.plans.clear();
            self.plan_cursor = 0;
        }
    }

    /// F8: bookmark the highlighted plan into the library.
    fn save_current_plan(&mut self) {
        if self.viewing_saved {
            "already viewing the library — F9 returns to live plans".clone_into(&mut self.status);
            return;
        }
        let (Some(target), Some(plan)) = (&self.target, self.plans.get(self.plan_cursor)) else {
            "no plan highlighted to save".clone_into(&mut self.status);
            return;
        };
        let target_display = self.solver.pal_db().pal(target).map_or_else(
            || target.as_str().to_owned(),
            |pal| pal.display_name.clone(),
        );
        let passives_note = if self.selected_passives.is_empty() {
            String::new()
        } else {
            format!(", {} passive(s)", self.selected_passives.len())
        };
        let label = format!(
            "{target_display} — {} step(s), {:.2} eggs{passives_note}",
            plan.steps, plan.expected_eggs
        );
        let saved = SavedPlan {
            label: label.clone(),
            goal_species: target.clone(),
            goal_passives: self.selected_passives.clone(),
            goal_progenitors: self.progenitors.clone(),
            expected_eggs: plan.expected_eggs,
            steps: plan.steps,
            root: plan.root.clone(),
        };
        match self.saved.add(saved) {
            Ok(()) => self.status = format!("saved: {label}"),
            Err(error) => self.status = format!("could not save plan: {error:#}"),
        }
    }

    /// F9: flip the Plans pane between live results and the library.
    fn toggle_saved_view(&mut self) {
        self.viewing_saved = !self.viewing_saved;
        if self.viewing_saved {
            self.focus = Pane::Results;
            self.saved_cursor = self
                .saved_cursor
                .min(self.saved.plans.len().saturating_sub(1));
            self.status = format!(
                "library: {} saved plan(s) · {}",
                self.saved.plans.len(),
                self.saved.path().display()
            );
        } else {
            "live plans".clone_into(&mut self.status);
        }
    }

    /// Delete (library view): remove the highlighted saved plan.
    fn delete_saved_plan(&mut self) {
        match self.saved.remove(self.saved_cursor) {
            Ok(Some(removed)) => {
                self.saved_cursor = self
                    .saved_cursor
                    .min(self.saved.plans.len().saturating_sub(1));
                self.status = format!("deleted: {}", removed.label);
            }
            Ok(None) => "nothing to delete".clone_into(&mut self.status),
            Err(error) => self.status = format!("could not delete plan: {error:#}"),
        }
    }

    fn search(&mut self, focus_results: bool) {
        let Some(target) = self.target.clone() else {
            "pick a target pal first (Enter in the Pals pane)".clone_into(&mut self.status);
            return;
        };
        let progenitor_mode = !self.progenitors.is_empty();
        let goal = BreedingGoal {
            species: target,
            passives: self.selected_passives.clone(),
            progenitors: self.progenitors.clone(),
        };
        let pool = if progenitor_mode {
            Vec::new()
        } else {
            self.owned.clone()
        };
        let config = SearchConfig {
            max_breeding_steps: self.max_breeding_steps,
            max_results: MAX_RESULTS,
            // Progenitor plans need partners from somewhere; captures
            // are the only source once the toml pool steps aside.
            allow_wild_pals: self.allow_wild || progenitor_mode,
        };
        match self.solver.find_paths(&pool, &goal, &config) {
            Ok(plans) => {
                self.status = if plans.is_empty() {
                    if progenitor_mode && !self.selected_passives.is_empty() {
                        "no plans — progenitor pals carry no passives; \
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
                        "{} plan(s) from {} progenitor pal(s)",
                        plans.len(),
                        self.progenitors.len()
                    )
                } else {
                    format!("{} plan(s) found", plans.len())
                };
                self.plans = plans;
                self.plan_cursor = 0;
                if focus_results && !self.plans.is_empty() {
                    self.focus = Pane::Results;
                }
            }
            Err(error) => self.status = error.to_string(),
        }
    }

    fn confirm(&mut self) {
        match self.focus {
            Pane::Pals => {
                if let Some(pal) = self.species_rows().get(self.species_cursor).copied() {
                    self.target = Some(pal.name.clone());
                    self.status = format!("target: {}", pal.display_name);
                    self.refresh_plans();
                }
            }
            Pane::Passives => {
                if let Some(skill) = self.passive_rows().get(self.passive_cursor).copied() {
                    self.toggle_passive(skill.name.clone(), &skill.display_name);
                }
            }
            Pane::Results if self.viewing_saved => self.load_saved_plan(),
            Pane::Results => self.run_search(),
        }
    }

    /// Enter on a library entry: restore the saved goal and re-plan
    /// against the current box, reporting how the cost moved. The
    /// original stays in the library untouched.
    fn load_saved_plan(&mut self) {
        let Some(saved) = self.saved.plans.get(self.saved_cursor).cloned() else {
            return;
        };
        self.target = Some(saved.goal_species.clone());
        self.selected_passives.clone_from(&saved.goal_passives);
        self.progenitors.clone_from(&saved.goal_progenitors);
        self.viewing_saved = false;
        self.search(true);
        self.status = match self.plans.first() {
            Some(best) => format!(
                "re-planned \"{}\": best {:.2} eggs now (saved plan was {:.2})",
                saved.label, best.expected_eggs, saved.expected_eggs
            ),
            None => format!(
                "re-planned \"{}\": no plans with the current box (saved plan was {:.2} eggs)",
                saved.label, saved.expected_eggs
            ),
        };
    }

    /// Owned pals in a saved plan's tree that no longer exist in the
    /// current pool — the plan's staleness measure.
    #[must_use]
    pub fn stale_leaves(&self, saved: &SavedPlan) -> usize {
        fn walk(node: &pal_solver::search::PlanNode, owned: &[OwnedPal], missing: &mut usize) {
            match node {
                pal_solver::search::PlanNode::Owned(pal) => {
                    if !owned.contains(pal) {
                        *missing += 1;
                    }
                }
                pal_solver::search::PlanNode::Bred(bred) => {
                    walk(&bred.male, owned, missing);
                    walk(&bred.female, owned, missing);
                }
                pal_solver::search::PlanNode::Wild(_)
                | pal_solver::search::PlanNode::Progenitor(_) => {}
            }
        }
        let mut missing = 0;
        walk(&saved.root, &self.owned, &mut missing);
        missing
    }

    fn toggle_passive(&mut self, name: PassiveName, display: &str) {
        if let Some(position) = self.selected_passives.iter().position(|p| *p == name) {
            self.selected_passives.remove(position);
            self.status = format!("dropped {display}");
            self.refresh_plans();
        } else if self.selected_passives.len() == MAX_TOTAL_PASSIVES {
            self.status = format!("a pal carries at most {MAX_TOTAL_PASSIVES} passives");
        } else {
            self.selected_passives.push(name);
            self.status = format!("added {display}");
            self.refresh_plans();
        }
    }

    fn toggle_progenitor(&mut self) {
        if self.focus != Pane::Pals {
            return;
        }
        let Some(pal) = self.species_rows().get(self.species_cursor).copied() else {
            return;
        };
        if let Some(position) = self.progenitors.iter().position(|p| *p == pal.name) {
            self.progenitors.remove(position);
            self.status = format!("progenitor removed: {}", pal.display_name);
            self.refresh_plans();
        } else if self.progenitors.len() == MAX_PROGENITORS {
            self.status = format!("at most {MAX_PROGENITORS} progenitors");
        } else {
            self.progenitors.push(pal.name.clone());
            self.status = format!(
                "progenitor added: {} — plans will start from the {} marked pal(s) only",
                pal.display_name,
                self.progenitors.len()
            );
            self.refresh_plans();
        }
    }

    fn clear_progenitors(&mut self) {
        if self.progenitors.is_empty() {
            "no progenitors marked".clone_into(&mut self.status);
        } else {
            self.status = format!("cleared {} progenitor(s)", self.progenitors.len());
            self.progenitors.clear();
            self.refresh_plans();
        }
    }

    fn toggle_wild(&mut self) {
        self.allow_wild = !self.allow_wild;
        self.status = if self.allow_wild {
            "wild pals: on — any catchable pal can join plans".to_owned()
        } else {
            "wild pals: off — plans use only the owned pool".to_owned()
        };
        self.refresh_plans();
    }

    fn adjust_depth(&mut self, delta: isize) {
        let adjusted = self
            .max_breeding_steps
            .saturating_add_signed(delta)
            .clamp(MIN_BREEDING_STEPS, MAX_BREEDING_STEPS);
        let changed = adjusted != self.max_breeding_steps;
        self.max_breeding_steps = adjusted;
        self.status = format!(
            "search depth: up to {} breeding step(s)",
            self.max_breeding_steps
        );
        if changed {
            self.refresh_plans();
        }
    }

    fn move_cursor(&mut self, delta: isize) {
        let len = match self.focus {
            Pane::Pals => self.species_rows().len(),
            Pane::Passives => self.passive_rows().len(),
            Pane::Results if self.viewing_saved => self.saved.plans.len(),
            Pane::Results => self.plans.len(),
        };
        let cursor = match self.focus {
            Pane::Pals => &mut self.species_cursor,
            Pane::Passives => &mut self.passive_cursor,
            Pane::Results if self.viewing_saved => &mut self.saved_cursor,
            Pane::Results => &mut self.plan_cursor,
        };
        *cursor = step(*cursor, delta, len);
    }

    fn active_filter(&mut self) -> Option<&mut String> {
        match self.focus {
            Pane::Pals => Some(&mut self.species_filter),
            Pane::Passives => Some(&mut self.passive_filter),
            Pane::Results => None,
        }
    }

    fn reset_cursor(&mut self) {
        match self.focus {
            Pane::Pals => self.species_cursor = 0,
            Pane::Passives => self.passive_cursor = 0,
            Pane::Results => {}
        }
    }
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
        Pane::Pals => Pane::Passives,
        Pane::Passives => Pane::Results,
        Pane::Results => Pane::Pals,
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
    use crate::test_support::{fixture, test_store};
    use pal_core::model::Gender;

    fn key(code: KeyCode) -> KeyEvent {
        KeyEvent::from(code)
    }

    fn app() -> App<'static> {
        let f = fixture();
        App::new(f.solver, Vec::new(), test_store())
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
        let mut app = App::new(f.solver, Vec::new(), test_store());

        for c in "lamball".chars() {
            app.handle_key(key(KeyCode::Char(c)));
        }
        app.handle_key(key(KeyCode::F(4)));
        assert_eq!(app.progenitors, vec![PalName::new("SheepBall")]);

        app.species_filter.clear();
        for c in "cattiva".chars() {
            app.handle_key(key(KeyCode::Char(c)));
        }
        // Row 0 is the pinned Lamball; the filter match sits below it.
        app.handle_key(key(KeyCode::Down));
        app.handle_key(key(KeyCode::F(4)));
        assert_eq!(app.progenitors.len(), 2);

        // Every plan must include BOTH marked progenitors — the free
        // "catch the target" plan may not crowd them out.
        app.target = Some(PalName::new("BluePlatypus"));
        app.run_search();

        assert!(!app.plans.is_empty());
        for plan in &app.plans {
            let mut anchors = Vec::new();
            collect_progenitor_leaves(&plan.root, &mut anchors);
            assert!(anchors.contains(&PalName::new("SheepBall")));
            assert!(anchors.contains(&PalName::new("PinkCat")));
            assert!(plan.steps > 0);
        }
        assert!(app.status.contains("2 progenitor pal(s)"));

        // Unmarking returns to pool planning.
        app.focus = Pane::Pals;
        app.handle_key(key(KeyCode::F(4)));
        assert_eq!(app.progenitors.len(), 1);
    }

    #[test]
    fn delete_clears_all_progenitors_at_once() {
        let f = fixture();
        let mut app = App::new(f.solver, Vec::new(), test_store());
        app.progenitors = vec![PalName::new("SheepBall"), PalName::new("PinkCat")];

        app.handle_key(key(KeyCode::Delete));
        assert!(app.progenitors.is_empty());
        assert!(app.status.contains("cleared 2"));

        app.handle_key(key(KeyCode::Delete));
        assert!(app.status.contains("no progenitors"));
    }

    #[test]
    fn marked_progenitors_pin_to_the_top_of_the_pals_list() {
        let f = fixture();
        let mut app = App::new(f.solver, Vec::new(), test_store());
        app.progenitors = vec![PalName::new("PinkCat")];

        // Pinned first with no filter, ahead of alphabetical order.
        assert_eq!(app.species_rows()[0].name, PalName::new("PinkCat"));

        // Still pinned (and first) when the filter would exclude it.
        app.species_filter = "lamball".to_owned();
        let rows = app.species_rows();
        assert_eq!(rows[0].name, PalName::new("PinkCat"));
        assert_eq!(rows[1].name, PalName::new("SheepBall"));
        assert_eq!(rows.len(), 2);
    }

    #[test]
    fn selected_passives_pin_to_the_top_of_the_list() {
        let f = fixture();
        let mut app = App::new(f.solver, Vec::new(), test_store());
        let last = app.passive_rows().last().copied().unwrap().name.clone();
        app.selected_passives = vec![last.clone()];

        // Pinned first despite sorting last alphabetically.
        assert_eq!(app.passive_rows()[0].name, last);

        // Still pinned (and the only row) when the filter excludes it.
        app.passive_filter = "zzz-no-such-passive".to_owned();
        let rows = app.passive_rows();
        assert_eq!(rows.len(), 1);
        assert_eq!(rows[0].name, last);
    }

    #[test]
    fn progenitor_mode_reports_that_passives_need_the_pool() {
        let f = fixture();
        let mut app = App::new(f.solver, Vec::new(), test_store());
        app.progenitors = vec![PalName::new("SheepBall"), PalName::new("PinkCat")];
        app.target = Some(PalName::new("DreamDemon"));
        app.selected_passives = vec![PassiveName::new("Swift")];
        app.run_search();

        assert!(app.plans.is_empty());
        assert!(app.status.contains("carry no passives"));
    }

    fn collect_progenitor_leaves(node: &pal_solver::search::PlanNode, out: &mut Vec<PalName>) {
        match node {
            pal_solver::search::PlanNode::Progenitor(species) => out.push(species.clone()),
            pal_solver::search::PlanNode::Owned(_) | pal_solver::search::PlanNode::Wild(_) => {}
            pal_solver::search::PlanNode::Bred(bred) => {
                collect_progenitor_leaves(&bred.male, out);
                collect_progenitor_leaves(&bred.female, out);
            }
        }
    }

    #[test]
    fn plans_save_into_the_library_and_delete_from_it() {
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
        let mut app = App::new(f.solver, owned, test_store());

        // Nothing to save before a search.
        app.handle_key(key(KeyCode::F(8)));
        assert!(app.status.contains("no plan highlighted"));

        for c in "daedream".chars() {
            app.handle_key(key(KeyCode::Char(c)));
        }
        app.handle_key(key(KeyCode::Enter));
        assert!(!app.plans.is_empty());

        app.handle_key(key(KeyCode::F(8)));
        assert_eq!(app.saved.plans.len(), 1);
        assert!(app.saved.plans[0].label.contains("Daedream"));
        assert_eq!(app.saved.plans[0].goal_species, PalName::new("DreamDemon"));

        // F9 shows the library; Del removes the highlighted entry.
        app.handle_key(key(KeyCode::F(9)));
        assert!(app.viewing_saved);
        assert_eq!(app.focus, Pane::Results);
        app.handle_key(key(KeyCode::Delete));
        assert!(app.saved.plans.is_empty());
        assert!(app.status.contains("deleted"));

        // Changing the live question leaves the library view.
        app.handle_key(key(KeyCode::F(2)));
        assert!(!app.viewing_saved);
    }

    #[test]
    fn ctrl_d_and_library_backspace_delete_without_a_delete_key() {
        let f = fixture();
        let mut app = App::new(f.solver, Vec::new(), test_store());
        let ctrl_d = KeyEvent::new(KeyCode::Char('d'), KeyModifiers::CONTROL);

        // Ctrl+D clears progenitors outside the library — and the 'd'
        // must not leak into the focused filter.
        app.progenitors = vec![PalName::new("SheepBall")];
        app.handle_key(ctrl_d);
        assert!(app.progenitors.is_empty());
        assert!(app.species_filter.is_empty());

        // In the library, Ctrl+D and Backspace both delete.
        for label in ["one", "two"] {
            app.saved
                .add(crate::plan_store::SavedPlan {
                    label: (*label).to_owned(),
                    goal_species: PalName::new("DreamDemon"),
                    goal_passives: Vec::new(),
                    goal_progenitors: Vec::new(),
                    expected_eggs: 1.0,
                    steps: 1,
                    root: pal_solver::search::PlanNode::Wild(PalName::new("SheepBall")),
                })
                .unwrap();
        }
        app.handle_key(key(KeyCode::F(9)));
        app.handle_key(ctrl_d);
        assert_eq!(app.saved.plans.len(), 1);
        app.handle_key(key(KeyCode::Backspace));
        assert!(app.saved.plans.is_empty());

        // Outside the library Backspace still edits the filter.
        app.handle_key(key(KeyCode::F(9)));
        app.focus = Pane::Pals;
        app.handle_key(key(KeyCode::Char('a')));
        app.handle_key(key(KeyCode::Backspace));
        assert!(app.species_filter.is_empty());
        assert!(app.progenitors.is_empty());
    }

    #[test]
    fn enter_on_a_saved_plan_restores_the_goal_and_replans() {
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
        let mut app = App::new(f.solver, owned, test_store());

        // Save a Daedream plan, then move the live goal elsewhere.
        for c in "daedream".chars() {
            app.handle_key(key(KeyCode::Char(c)));
        }
        app.handle_key(key(KeyCode::Enter));
        app.handle_key(key(KeyCode::F(8)));
        app.species_filter.clear();
        for c in "fuack".chars() {
            app.handle_key(key(KeyCode::Char(c)));
        }
        app.handle_key(key(KeyCode::Enter));
        assert_eq!(app.target, Some(PalName::new("BluePlatypus")));

        // Enter in the library restores the saved goal and re-plans.
        app.handle_key(key(KeyCode::F(9)));
        app.handle_key(key(KeyCode::Enter));
        assert!(!app.viewing_saved);
        assert_eq!(app.target, Some(PalName::new("DreamDemon")));
        assert!(!app.plans.is_empty());
        assert!(app.status.contains("re-planned"));
        assert!(app.status.contains("saved plan was"));
        // The original stays in the library.
        assert_eq!(app.saved.plans.len(), 1);
    }

    #[test]
    fn stale_leaves_count_pals_missing_from_the_current_box() {
        let f = fixture();
        let lamball = OwnedPal {
            species: PalName::new("SheepBall"),
            gender: Gender::Male,
            passives: Vec::new(),
        };
        let cattiva = OwnedPal {
            species: PalName::new("PinkCat"),
            gender: Gender::Female,
            passives: Vec::new(),
        };
        let saved = SavedPlan {
            label: "test".to_owned(),
            goal_species: PalName::new("DreamDemon"),
            goal_passives: Vec::new(),
            goal_progenitors: Vec::new(),
            expected_eggs: 1.0,
            steps: 1,
            root: pal_solver::search::PlanNode::Bred(Box::new(pal_solver::search::BredNode {
                male: pal_solver::search::PlanNode::Owned(lamball.clone()),
                female: pal_solver::search::PlanNode::Owned(cattiva.clone()),
                species: PalName::new("DreamDemon"),
                carried_passives: Vec::new(),
            })),
        };

        let app = App::new(f.solver, vec![lamball.clone(), cattiva], test_store());
        assert_eq!(app.stale_leaves(&saved), 0);

        // Cattiva left the box: one leaf goes stale.
        let app = App::new(f.solver, vec![lamball], test_store());
        assert_eq!(app.stale_leaves(&saved), 1);
    }

    #[test]
    fn changes_re_search_automatically_and_never_leave_stale_plans() {
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
        let mut app = App::new(f.solver, owned, test_store());

        // Selecting a target searches immediately, without stealing
        // focus.
        for c in "daedream".chars() {
            app.handle_key(key(KeyCode::Char(c)));
        }
        app.handle_key(key(KeyCode::Enter));
        assert!(!app.plans.is_empty());
        assert_eq!(app.focus, Pane::Pals);
        assert_eq!(*app.plans[0].root.species(), PalName::new("DreamDemon"));

        // Changing the target replaces the plans in place.
        app.species_filter.clear();
        for c in "fuack".chars() {
            app.handle_key(key(KeyCode::Char(c)));
        }
        app.handle_key(key(KeyCode::Enter));
        assert!(!app.plans.is_empty());
        assert_eq!(*app.plans[0].root.species(), PalName::new("BluePlatypus"));

        // Depth changes re-search: depth 1 cannot reach Fuack.
        app.handle_key(key(KeyCode::Left));
        app.handle_key(key(KeyCode::Left));
        assert_eq!(app.max_breeding_steps, 1);
        assert!(app.plans.is_empty());

        // Toggling wild re-searches: Fuack becomes catchable.
        app.handle_key(key(KeyCode::F(2)));
        assert!(!app.plans.is_empty());
        assert_eq!(app.plans[0].steps, 0);
    }

    #[test]
    fn clicks_select_and_shift_click_marks_progenitors() {
        let f = fixture();
        let mut app = App::new(f.solver, Vec::new(), test_store());
        let first = app.species_rows()[0].name.clone();

        app.handle_click(
            Click {
                pane: Pane::Pals,
                row: Some(0),
            },
            false,
        );
        assert_eq!(app.focus, Pane::Pals);
        assert_eq!(app.target, Some(first.clone()));

        app.handle_click(
            Click {
                pane: Pane::Pals,
                row: Some(0),
            },
            true,
        );
        assert_eq!(app.progenitors, vec![first]);

        app.handle_click(
            Click {
                pane: Pane::Passives,
                row: Some(0),
            },
            false,
        );
        assert_eq!(app.selected_passives.len(), 1);

        app.handle_click(
            Click {
                pane: Pane::Results,
                row: None,
            },
            false,
        );
        assert_eq!(app.focus, Pane::Results);
    }

    #[test]
    fn f2_toggles_wild_mode_and_search_honors_it() {
        let f = fixture();
        let mut app = App::new(f.solver, Vec::new(), test_store());
        assert!(!app.allow_wild, "wild mode defaults to off");

        app.handle_key(key(KeyCode::F(2)));
        assert!(app.allow_wild);
        assert!(app.status.contains("on"));

        // Empty pool, catchable target: wild mode finds the catch plan.
        app.target = Some(PalName::new("SheepBall"));
        app.run_search();
        assert!(!app.plans.is_empty());
        assert_eq!(app.plans[0].steps, 0);

        // Toggling back re-searches (status now reports the result,
        // not the toggle), and the catch plan disappears.
        app.handle_key(key(KeyCode::F(2)));
        assert!(!app.allow_wild);
        assert!(app.plans.is_empty());
    }

    #[test]
    fn arrow_keys_adjust_search_depth_within_bounds() {
        let mut app = app();
        assert_eq!(app.max_breeding_steps, 3);

        app.handle_key(key(KeyCode::Right));
        assert_eq!(app.max_breeding_steps, 4);
        assert!(app.status.contains("4 breeding step"));

        for _ in 0..=MAX_BREEDING_STEPS {
            app.handle_key(key(KeyCode::Left));
        }
        assert_eq!(app.max_breeding_steps, MIN_BREEDING_STEPS);
        for _ in 0..=MAX_BREEDING_STEPS {
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
        let mut app = App::new(f.solver, owned, test_store());
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
        let mut app = App::new(f.solver, owned, test_store());
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
