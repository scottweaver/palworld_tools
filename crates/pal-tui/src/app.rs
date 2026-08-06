//! Application state and key handling — pure transitions over the
//! solver API, testable without a terminal. Rendering lives in
//! [`crate::ui`]; nothing here draws.

use pal_core::model::{IvSpread, IvStat, IvValue, Pal, PalDb, PalName, PassiveName, PassiveSkill};
use pal_solver::iv::IvThresholds;
use pal_solver::passives::MAX_TOTAL_PASSIVES;
use pal_solver::search::{
    BreedingGoal, BreedingPlan, MAX_PROGENITORS, OwnedPal, SearchConfig, Solver,
};
use ratatui::crossterm::event::{KeyCode, KeyEvent, KeyModifiers};

use crate::command::{Command, DocView};
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

/// The modal layer above the panes. Whichever layer is open owns the
/// keyboard; the panes only see keys when no overlay is up.
#[derive(Clone, PartialEq, Eq, Debug)]
pub enum Overlay {
    None,
    /// The `:` command line, holding the text typed so far.
    Prompt(String),
    /// A full-screen document viewer (`:help`, `:readme`).
    Doc(DocView),
}

/// How searches run: `Inline` executes on the caller's thread and
/// applies before returning (tests, degraded mode); `Deferred` parks
/// a [`SearchRequest`] for the shell to ship to the worker thread.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum SearchMode {
    Inline,
    Deferred,
}

/// Background-search activity. `Awaiting` runs from request creation
/// until the latest generation's outcome applies, and owns the
/// spinner's frame counter — an idle app cannot have a spinner.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum SearchActivity {
    Idle,
    Awaiting { spinner_tick: usize },
}

/// Monotonic search id; only the outcome matching the latest
/// generation may apply, so superseded searches die on arrival.
#[derive(Clone, Copy, PartialEq, Eq, Debug, Default)]
pub struct SearchGen(u64);

impl SearchGen {
    #[must_use]
    fn next(self) -> Self {
        Self(self.0 + 1)
    }
}

/// How the applied outcome announces itself on the status line.
#[derive(Clone, PartialEq, Debug)]
pub enum Announce {
    Standard,
    /// Re-planning a saved plan compares the fresh best cost with
    /// the bookmarked one.
    Replan {
        label: String,
        saved_eggs: f64,
    },
}

/// Everything the worker needs to run one search, snapshotted at
/// request time so the app state may keep moving underneath it.
#[derive(Debug)]
pub struct SearchRequest {
    pub generation: SearchGen,
    pool: Vec<OwnedPal>,
    goal: BreedingGoal,
    config: SearchConfig,
    focus_results: bool,
    announce: Announce,
}

/// A finished search, tagged with the request it answers.
#[derive(Debug)]
pub struct SearchOutcome {
    generation: SearchGen,
    focus_results: bool,
    announce: Announce,
    result: Result<Vec<BreedingPlan>, pal_solver::search::SearchError>,
}

/// Runs one search request to completion. The single execution path
/// shared by inline mode and the worker thread.
#[must_use]
pub fn run_search(solver: &Solver, request: SearchRequest) -> SearchOutcome {
    let result = solver.find_paths(&request.pool, &request.goal, &request.config);
    SearchOutcome {
        generation: request.generation,
        focus_results: request.focus_results,
        announce: request.announce,
        result,
    }
}

pub struct App<'db> {
    solver: &'db Solver<'db>,
    pub owned: Vec<OwnedPal>,
    /// Where the owned pool came from, when it came from a file —
    /// the reload target for F6. `None` for in-memory pools (tests).
    pub pool_path: Option<String>,
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
    /// Per-stat IV minimums on the target; adjusted with h/a/d (+10)
    /// and H/A/D (-10) while the Plans pane is focused.
    pub iv_thresholds: IvThresholds,
    pub plans: Vec<BreedingPlan>,
    pub plan_cursor: usize,
    pub saved: PlanStore,
    /// When set, the Plans pane shows the saved-plan library instead
    /// of live search results.
    pub viewing_saved: bool,
    pub saved_cursor: usize,
    pub max_breeding_steps: usize,
    pub allow_wild: bool,
    pub overlay: Overlay,
    pub status: String,
    pub should_quit: bool,
    mode: SearchMode,
    latest_gen: SearchGen,
    /// A search request built but not yet handed to the shell.
    pending_search: Option<SearchRequest>,
    activity: SearchActivity,
}

impl<'db> App<'db> {
    #[must_use]
    pub fn new(solver: &'db Solver<'db>, owned: Vec<OwnedPal>, saved: PlanStore) -> Self {
        Self {
            solver,
            owned,
            pool_path: None,
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
            iv_thresholds: IvThresholds::default(),
            plans: Vec::new(),
            plan_cursor: 0,
            max_breeding_steps: DEFAULT_BREEDING_STEPS,
            allow_wild: false,
            overlay: Overlay::None,
            status: String::new(),
            should_quit: false,
            mode: SearchMode::Inline,
            latest_gen: SearchGen::default(),
            pending_search: None,
            activity: SearchActivity::Idle,
        }
    }

    fn library_focused(&self) -> bool {
        self.viewing_saved && self.focus == Pane::Results
    }

    /// Switches searches to the request/apply protocol: [`Self::search`]
    /// parks a [`SearchRequest`] that the shell ships to the worker
    /// thread, and results land via [`Self::apply_search_outcome`].
    pub fn defer_searches(&mut self) {
        self.mode = SearchMode::Deferred;
    }

    /// True while a search runs (or waits to run) in the background.
    #[must_use]
    pub fn searching(&self) -> bool {
        match self.activity {
            SearchActivity::Awaiting { .. } => true,
            SearchActivity::Idle => false,
        }
    }

    /// The spinner's frame counter — `Some` only while a search is
    /// awaited.
    #[must_use]
    pub fn spinner(&self) -> Option<usize> {
        match self.activity {
            SearchActivity::Awaiting { spinner_tick } => Some(spinner_tick),
            SearchActivity::Idle => None,
        }
    }

    /// Hands the newest undispatched request to the shell. Requests
    /// replace each other while parked, so this is always latest-wins.
    pub fn take_pending_search(&mut self) -> Option<SearchRequest> {
        self.pending_search.take()
    }

    /// Shell timer tick: animates the spinner while a search runs.
    pub fn on_tick(&mut self) {
        if let SearchActivity::Awaiting { spinner_tick } = &mut self.activity {
            *spinner_tick = spinner_tick.wrapping_add(1);
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
        match self.overlay {
            Overlay::None => self.handle_pane_key(key),
            Overlay::Prompt(_) => self.handle_prompt_key(key),
            Overlay::Doc(_) => self.handle_doc_key(key),
        }
    }

    fn handle_prompt_key(&mut self, key: KeyEvent) {
        let Overlay::Prompt(buffer) = &mut self.overlay else {
            return;
        };
        match key.code {
            KeyCode::Esc => self.overlay = Overlay::None,
            KeyCode::Enter => {
                let input = std::mem::take(buffer);
                self.overlay = Overlay::None;
                self.execute_command(&input);
            }
            // Vim behavior: erasing past the start closes the prompt.
            KeyCode::Backspace if buffer.is_empty() => self.overlay = Overlay::None,
            KeyCode::Backspace => {
                buffer.pop();
            }
            KeyCode::Char(c) if !key.modifiers.contains(KeyModifiers::CONTROL) => buffer.push(c),
            _ => {}
        }
    }

    fn handle_doc_key(&mut self, key: KeyEvent) {
        let Overlay::Doc(view) = &mut self.overlay else {
            return;
        };
        match key.code {
            KeyCode::Esc | KeyCode::Char('q' | 'Q') => self.overlay = Overlay::None,
            KeyCode::Up => view.scroll_lines(-1),
            KeyCode::Down => view.scroll_lines(1),
            KeyCode::PageUp => view.scroll_pages(-1),
            KeyCode::PageDown => view.scroll_pages(1),
            KeyCode::Home => view.scroll_to_top(),
            KeyCode::End => view.scroll_to_bottom(),
            KeyCode::Char(':') => self.overlay = Overlay::Prompt(String::new()),
            _ => {}
        }
    }

    fn execute_command(&mut self, input: &str) {
        if input.trim().is_empty() {
            return;
        }
        match Command::parse(input) {
            Ok(Command::Open(doc)) => self.overlay = Overlay::Doc(DocView::open(doc)),
            Ok(Command::SavePlan) => self.save_current_plan(),
            Ok(Command::OpenLibrary) => self.open_library(),
            Ok(Command::DeletePlan) => self.delete_selected_plan(),
            Ok(Command::ClearGoal) => self.clear_goal(),
            Ok(Command::ReloadPool) => self.reload_pool(),
            Ok(Command::Quit) => self.should_quit = true,
            Err(unknown) => self.status = format!("unknown command: {unknown} — try :help"),
        }
    }

    /// `:o` — show the library; already viewing it is a no-op, not a
    /// toggle (vim's `:o` opens, it never closes).
    fn open_library(&mut self) {
        if self.viewing_saved {
            "already viewing the library".clone_into(&mut self.status);
        } else {
            self.toggle_saved_view();
        }
    }

    /// `:dd` — delete the highlighted saved plan. Live plans are
    /// search results and regenerate on every change, so there is
    /// nothing to delete outside the library.
    fn delete_selected_plan(&mut self) {
        if self.viewing_saved {
            self.delete_saved_plan();
        } else {
            "no saved plan selected — :o opens the library".clone_into(&mut self.status);
        }
    }

    /// `:clear` — reset the breeding question: target, progenitor
    /// marks, and selected passives. IV floors and filter text stay.
    fn clear_goal(&mut self) {
        let nothing_selected = self.target.is_none()
            && self.progenitors.is_empty()
            && self.selected_passives.is_empty();
        if nothing_selected {
            "nothing selected to clear".clone_into(&mut self.status);
            return;
        }
        self.target = None;
        self.progenitors.clear();
        self.selected_passives.clear();
        self.refresh_plans();
        "cleared: target, progenitors, and passives".clone_into(&mut self.status);
    }

    /// Mouse-wheel input: scrolls the document viewer when one is
    /// open; means nothing anywhere else.
    pub fn handle_scroll(&mut self, delta: i16) {
        match &mut self.overlay {
            Overlay::Doc(view) => view.scroll_lines(delta),
            Overlay::None | Overlay::Prompt(_) => {}
        }
    }

    fn handle_pane_key(&mut self, key: KeyEvent) {
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
            KeyCode::F(6) => self.reload_pool(),
            KeyCode::F(5) => self.run_search(),
            KeyCode::F(8) => self.save_current_plan(),
            KeyCode::F(9) => self.toggle_saved_view(),
            // Ctrl+D works on every keyboard; the Delete key needs
            // Fn on Mac laptops but stays supported.
            KeyCode::Char('d' | 'D') if key.modifiers.contains(KeyModifiers::CONTROL) => {
                self.contextual_delete();
            }
            // F8/F9 remain as fallbacks for terminals that swallow
            // these Ctrl combos.
            KeyCode::Char('s' | 'S') if key.modifiers.contains(KeyModifiers::CONTROL) => {
                self.save_current_plan();
            }
            KeyCode::Char('l' | 'L') if key.modifiers.contains(KeyModifiers::CONTROL) => {
                self.toggle_saved_view();
            }
            KeyCode::Delete => self.contextual_delete(),
            // Vim's x: single-key delete in the library pane. Chosen
            // over dd because d is an IV-floor key in this pane.
            KeyCode::Char('x')
                if self.library_focused() && !key.modifiers.contains(KeyModifiers::CONTROL) =>
            {
                self.delete_saved_plan();
            }
            KeyCode::Char(c @ ('h' | 'H' | 'a' | 'A' | 'd' | 'D'))
                if self.focus == Pane::Results
                    && !key.modifiers.contains(KeyModifiers::CONTROL) =>
            {
                self.adjust_iv_threshold(c);
            }
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
            KeyCode::Char(':') => self.overlay = Overlay::Prompt(String::new()),
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
    /// An open overlay owns the screen, so clicks stop at it.
    pub fn handle_click(&mut self, click: Click, shift: bool) {
        match self.overlay {
            Overlay::Prompt(_) | Overlay::Doc(_) => return,
            Overlay::None => {}
        }
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
        self.replan();
    }

    fn replan(&mut self) {
        if self.target.is_some() {
            self.search(false);
        } else {
            self.plans.clear();
            self.plan_cursor = 0;
            self.cancel_pending_search();
        }
    }

    /// F6: re-import the pool from the file it was loaded from, so a
    /// fresh in-game save lands without restarting the app. Keeps the
    /// current pool on any failure, and stays in the library view —
    /// reloading refreshes the world, not the question, and staleness
    /// banners recompute against the new box.
    fn reload_pool(&mut self) {
        let Some(path) = self.pool_path.clone() else {
            "the pool did not come from a file — nothing to reload".clone_into(&mut self.status);
            return;
        };
        match crate::pool::load(&path, self.db()) {
            Ok(crate::pool::Loaded::Pool { owned, status }) => {
                self.owned = owned;
                self.replan();
                self.status = format!("reloaded: {status}");
            }
            Ok(crate::pool::Loaded::Missing) => {
                self.status = format!("{path} not found — keeping the current pool");
            }
            Err(error) => {
                self.status = format!("reload failed: {error:#} — keeping the current pool");
            }
        }
    }

    /// Ctrl+S (or F8): bookmark the highlighted plan into the
    /// library.
    fn save_current_plan(&mut self) {
        if self.viewing_saved {
            "already viewing the library — Ctrl+L returns to live plans"
                .clone_into(&mut self.status);
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
            goal_iv_thresholds: self.iv_thresholds,
            expected_eggs: plan.expected_eggs,
            steps: plan.steps,
            root: plan.root.clone(),
        };
        match self.saved.add(saved) {
            Ok(()) => self.status = format!("saved: {label}"),
            Err(error) => self.status = format!("could not save plan: {error:#}"),
        }
    }

    /// Ctrl+L (or F9): flip the Plans pane between live results and
    /// the library.
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
        self.search_announcing(focus_results, Announce::Standard);
    }

    fn search_announcing(&mut self, focus_results: bool, announce: Announce) {
        let Some(target) = self.target.clone() else {
            "pick a target pal first (Enter in the Pals pane)".clone_into(&mut self.status);
            return;
        };
        let progenitor_mode = !self.progenitors.is_empty();
        let goal = BreedingGoal {
            species: target,
            passives: self.selected_passives.clone(),
            progenitors: self.progenitors.clone(),
            iv_thresholds: self.iv_thresholds,
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
        self.latest_gen = self.latest_gen.next();
        let request = SearchRequest {
            generation: self.latest_gen,
            pool,
            goal,
            config,
            focus_results,
            announce,
        };
        match self.mode {
            SearchMode::Inline => {
                let outcome = run_search(self.solver, request);
                self.apply_search_outcome(outcome);
            }
            SearchMode::Deferred => {
                // A superseding request keeps the spinner's phase.
                let spinner_tick = self.spinner().unwrap_or(0);
                self.activity = SearchActivity::Awaiting { spinner_tick };
                self.pending_search = Some(request);
            }
        }
    }

    /// Drops any parked request and invalidates whatever the worker
    /// is still computing, so a stale outcome can never apply.
    fn cancel_pending_search(&mut self) {
        self.latest_gen = self.latest_gen.next();
        self.pending_search = None;
        self.activity = SearchActivity::Idle;
    }

    /// Lands a finished search. Anything but the latest generation is
    /// a superseded question and is discarded unapplied.
    pub fn apply_search_outcome(&mut self, outcome: SearchOutcome) {
        if outcome.generation != self.latest_gen {
            return;
        }
        self.activity = SearchActivity::Idle;
        match outcome.result {
            Ok(plans) => {
                self.status = self.describe_plans(&plans, &outcome.announce);
                self.plans = plans;
                self.plan_cursor = 0;
                if outcome.focus_results && !self.plans.is_empty() {
                    self.focus = Pane::Results;
                }
            }
            Err(error) => self.status = error.to_string(),
        }
    }

    fn describe_plans(&self, plans: &[BreedingPlan], announce: &Announce) -> String {
        if let Announce::Replan { label, saved_eggs } = announce {
            return match plans.first() {
                Some(best) => format!(
                    "re-planned \"{label}\": best {:.2} eggs now (saved plan was {saved_eggs:.2})",
                    best.expected_eggs
                ),
                None => format!(
                    "re-planned \"{label}\": no plans with the current box \
                     (saved plan was {saved_eggs:.2} eggs)"
                ),
            };
        }
        let progenitor_mode = !self.progenitors.is_empty();
        if plans.is_empty() {
            if progenitor_mode
                && (!self.selected_passives.is_empty() || !self.iv_thresholds.is_empty())
            {
                "no plans — progenitor pals carry no passives or IVs; \
                 drop those requirements or plan from the pool"
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
        self.iv_thresholds = saved.goal_iv_thresholds;
        self.viewing_saved = false;
        self.search_announcing(
            true,
            Announce::Replan {
                label: saved.label,
                saved_eggs: saved.expected_eggs,
            },
        );
    }

    /// Owned pals in a saved plan's tree that no longer exist in the
    /// current pool — the plan's staleness measure. Leaves saved
    /// before IVs were modeled deserialize with zero IVs; those match
    /// on species/gender/passives alone so old plans don't all read
    /// stale.
    #[must_use]
    pub fn stale_leaves(&self, saved: &SavedPlan) -> usize {
        fn leaf_matches(owned: &OwnedPal, saved: &OwnedPal) -> bool {
            owned.species == saved.species
                && owned.gender == saved.gender
                && owned.passives == saved.passives
                && (saved.ivs == IvSpread::default() || owned.ivs == saved.ivs)
        }
        fn walk(node: &pal_solver::search::PlanNode, owned: &[OwnedPal], missing: &mut usize) {
            match node {
                pal_solver::search::PlanNode::Owned(pal) => {
                    if !owned.iter().any(|candidate| leaf_matches(candidate, pal)) {
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

    /// h/a/d raise (lowercase) or lower (uppercase) one stat's IV
    /// minimum in steps of 10; stepping below 10 turns the minimum
    /// off.
    fn adjust_iv_threshold(&mut self, key: char) {
        let stat = match key.to_ascii_lowercase() {
            'h' => IvStat::Hp,
            'a' => IvStat::Attack,
            _ => IvStat::Defense,
        };
        let slot = match stat {
            IvStat::Hp => &mut self.iv_thresholds.hp,
            IvStat::Attack => &mut self.iv_thresholds.attack,
            IvStat::Defense => &mut self.iv_thresholds.defense,
        };
        let stepped = step_iv_threshold(*slot, key.is_ascii_lowercase());
        let changed = stepped != *slot;
        *slot = stepped;
        if changed {
            self.refresh_plans();
        }
        self.status = format!(
            "IV minimums — HP {} · Attack {} · Defense {}",
            describe_threshold(self.iv_thresholds.hp),
            describe_threshold(self.iv_thresholds.attack),
            describe_threshold(self.iv_thresholds.defense),
        );
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

fn step_iv_threshold(current: Option<IvValue>, up: bool) -> Option<IvValue> {
    let value = current.map_or(0, IvValue::get);
    let stepped = if up {
        value.saturating_add(10).min(100)
    } else {
        value.saturating_sub(10)
    };
    (stepped > 0).then(|| IvValue::try_from(stepped).expect("stepped in 10s within 0..=100"))
}

fn describe_threshold(threshold: Option<IvValue>) -> String {
    threshold.map_or_else(
        || "off".to_owned(),
        |minimum| format!("≥ {}", minimum.get()),
    )
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
    use crate::command::Doc;
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
    fn iv_threshold_keys_step_in_the_results_pane_and_replan() {
        let f = fixture();
        let owned = vec![
            OwnedPal {
                species: PalName::new("SheepBall"),
                gender: Gender::Male,
                passives: Vec::new(),
                ivs: IvSpread {
                    hp: IvValue::try_from(90).unwrap(),
                    ..IvSpread::default()
                },
            },
            OwnedPal {
                species: PalName::new("PinkCat"),
                gender: Gender::Female,
                passives: Vec::new(),
                ivs: IvSpread {
                    hp: IvValue::try_from(85).unwrap(),
                    ..IvSpread::default()
                },
            },
        ];
        let mut app = App::new(f.solver, owned, test_store());
        app.target = Some(PalName::new("DreamDemon"));
        app.run_search();
        assert!(!app.plans.is_empty());
        assert_eq!(app.focus, Pane::Results);
        let unconstrained_eggs = app.plans[0].expected_eggs;

        // In a filter pane the same letters keep typing.
        app.focus = Pane::Pals;
        app.handle_key(key(KeyCode::Char('h')));
        assert!(app.iv_thresholds.is_empty());
        assert_eq!(app.species_filter, "h");
        app.species_filter.clear();

        // In the Plans pane, h raises the HP minimum by 10s and the
        // search re-plans: both parents meet 80, so plans remain but
        // cost the IV roll.
        app.focus = Pane::Results;
        for _ in 0..8 {
            app.handle_key(key(KeyCode::Char('h')));
        }
        assert_eq!(app.iv_thresholds.hp, Some(IvValue::try_from(80).unwrap()));
        assert!(app.status.contains("HP ≥ 80"));
        assert!(!app.plans.is_empty());
        assert!(app.plans[0].expected_eggs > unconstrained_eggs);

        // 100 is the ceiling; nobody meets it, so plans vanish.
        for _ in 0..3 {
            app.handle_key(key(KeyCode::Char('h')));
        }
        assert_eq!(app.iv_thresholds.hp, Some(IvValue::MAX));
        assert!(app.plans.is_empty());

        // Uppercase steps down; below 10 the minimum turns off.
        for _ in 0..10 {
            app.handle_key(key(KeyCode::Char('H')));
        }
        assert!(app.iv_thresholds.hp.is_none());
        assert!(!app.plans.is_empty());

        // a/d drive the other two stats.
        app.handle_key(key(KeyCode::Char('a')));
        app.handle_key(key(KeyCode::Char('d')));
        assert_eq!(
            app.iv_thresholds.attack,
            Some(IvValue::try_from(10).unwrap())
        );
        assert_eq!(
            app.iv_thresholds.defense,
            Some(IvValue::try_from(10).unwrap())
        );
    }

    #[test]
    fn f6_and_the_reload_verb_reload_the_pool_from_disk() {
        let f = fixture();
        let path = std::env::temp_dir().join(format!("pal-tui-reload-{}.toml", std::process::id()));
        std::fs::write(
            &path,
            "[[pals]]\nspecies = \"Lamball\"\ngender = \"male\"\n",
        )
        .unwrap();
        let mut app = App::new(f.solver, Vec::new(), test_store());

        // Without a file-backed pool, F6 explains itself — and the
        // :reload verb lands on the same logic.
        app.handle_key(key(KeyCode::F(6)));
        assert!(app.status.contains("nothing to reload"));
        app.status.clear();
        type_line(&mut app, ":reload");
        app.handle_key(key(KeyCode::Enter));
        assert!(app.status.contains("nothing to reload"));

        app.pool_path = Some(path.to_string_lossy().into_owned());
        app.handle_key(key(KeyCode::F(6)));
        assert_eq!(app.owned.len(), 1);
        assert!(app.status.contains("reloaded"));

        // A grown file lands on the next reload (driven through the
        // prompt this time), and plans refresh.
        std::fs::write(
            &path,
            "[[pals]]\nspecies = \"Lamball\"\ngender = \"male\"\n\
             [[pals]]\nspecies = \"Cattiva\"\ngender = \"female\"\n",
        )
        .unwrap();
        app.target = Some(PalName::new("DreamDemon"));
        type_line(&mut app, ":reload");
        app.handle_key(key(KeyCode::Enter));
        assert_eq!(app.owned.len(), 2);
        assert!(!app.plans.is_empty());

        // Reloading refreshes the world, not the question — the
        // library view survives it.
        app.viewing_saved = true;
        app.handle_key(key(KeyCode::F(6)));
        assert!(app.viewing_saved);

        // A vanished file keeps the current pool.
        std::fs::remove_file(&path).unwrap();
        app.handle_key(key(KeyCode::F(6)));
        assert_eq!(app.owned.len(), 2);
        assert!(app.status.contains("keeping the current pool"));
    }

    #[test]
    fn plans_save_into_the_library_and_delete_from_it() {
        let f = fixture();
        let owned = vec![
            OwnedPal {
                species: PalName::new("SheepBall"),
                gender: Gender::Male,
                passives: Vec::new(),
                ivs: IvSpread::default(),
            },
            OwnedPal {
                species: PalName::new("PinkCat"),
                gender: Gender::Female,
                passives: Vec::new(),
                ivs: IvSpread::default(),
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
    fn ctrl_s_saves_and_ctrl_l_toggles_the_library() {
        let f = fixture();
        let owned = vec![
            OwnedPal {
                species: PalName::new("SheepBall"),
                gender: Gender::Male,
                passives: Vec::new(),
                ivs: IvSpread::default(),
            },
            OwnedPal {
                species: PalName::new("PinkCat"),
                gender: Gender::Female,
                passives: Vec::new(),
                ivs: IvSpread::default(),
            },
        ];
        let mut app = App::new(f.solver, owned, test_store());
        let ctrl = |c| KeyEvent::new(KeyCode::Char(c), KeyModifiers::CONTROL);

        for c in "daedream".chars() {
            app.handle_key(key(KeyCode::Char(c)));
        }
        app.handle_key(key(KeyCode::Enter));
        assert!(!app.plans.is_empty());

        // Ctrl+S saves without leaking 's' into a filter.
        app.focus = Pane::Pals;
        app.species_filter.clear();
        app.handle_key(ctrl('s'));
        assert_eq!(app.saved.plans.len(), 1);
        assert!(app.species_filter.is_empty());

        // Ctrl+L opens the library and closes it again; 'l' stays
        // out of the filter, and plain 'l' still types.
        app.handle_key(ctrl('l'));
        assert!(app.viewing_saved);
        app.handle_key(ctrl('L'));
        assert!(!app.viewing_saved);
        assert!(app.species_filter.is_empty());
        app.focus = Pane::Pals;
        app.handle_key(key(KeyCode::Char('l')));
        assert_eq!(app.species_filter, "l");
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
                    goal_iv_thresholds: IvThresholds::default(),
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
                ivs: IvSpread::default(),
            },
            OwnedPal {
                species: PalName::new("PinkCat"),
                gender: Gender::Female,
                passives: Vec::new(),
                ivs: IvSpread::default(),
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
            ivs: IvSpread::default(),
        };
        let cattiva = OwnedPal {
            species: PalName::new("PinkCat"),
            gender: Gender::Female,
            passives: Vec::new(),
            ivs: IvSpread::default(),
        };
        let saved = SavedPlan {
            label: "test".to_owned(),
            goal_species: PalName::new("DreamDemon"),
            goal_passives: Vec::new(),
            goal_progenitors: Vec::new(),
            goal_iv_thresholds: IvThresholds::default(),
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
                ivs: IvSpread::default(),
            },
            OwnedPal {
                species: PalName::new("PinkCat"),
                gender: Gender::Female,
                passives: Vec::new(),
                ivs: IvSpread::default(),
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
                ivs: IvSpread::default(),
            },
            OwnedPal {
                species: PalName::new("PinkCat"),
                gender: Gender::Female,
                passives: Vec::new(),
                ivs: IvSpread::default(),
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
                ivs: IvSpread::default(),
            },
            OwnedPal {
                species: PalName::new("PinkCat"),
                gender: Gender::Female,
                passives: Vec::new(),
                ivs: IvSpread::default(),
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

    fn type_line(app: &mut App, line: &str) {
        for c in line.chars() {
            app.handle_key(key(KeyCode::Char(c)));
        }
    }

    #[test]
    fn colon_opens_the_prompt_and_keeps_typing_out_of_filters() {
        let mut app = app();
        type_line(&mut app, ":help");
        assert_eq!(app.overlay, Overlay::Prompt("help".to_owned()));
        assert!(app.species_filter.is_empty());

        // Esc cancels the prompt, not the app.
        app.handle_key(key(KeyCode::Esc));
        assert_eq!(app.overlay, Overlay::None);
        assert!(!app.should_quit);
    }

    #[test]
    fn backspacing_past_the_start_closes_the_prompt() {
        let mut app = app();
        type_line(&mut app, ":x");
        app.handle_key(key(KeyCode::Backspace));
        assert_eq!(app.overlay, Overlay::Prompt(String::new()));
        app.handle_key(key(KeyCode::Backspace));
        assert_eq!(app.overlay, Overlay::None);
    }

    #[test]
    fn help_and_readme_commands_open_the_viewer() {
        let mut app = app();
        type_line(&mut app, ":help");
        app.handle_key(key(KeyCode::Enter));
        let Overlay::Doc(view) = &app.overlay else {
            panic!("expected the help viewer, got {:?}", app.overlay);
        };
        assert_eq!(view.doc, Doc::Help);

        // q closes the viewer; the app stays up.
        app.handle_key(key(KeyCode::Char('q')));
        assert_eq!(app.overlay, Overlay::None);
        assert!(!app.should_quit);

        type_line(&mut app, ":readme");
        app.handle_key(key(KeyCode::Enter));
        let Overlay::Doc(view) = &app.overlay else {
            panic!("expected the readme viewer, got {:?}", app.overlay);
        };
        assert_eq!(view.doc, Doc::Readme);

        // ':' switches straight from the viewer to the prompt (so
        // :help → :readme chains); Esc then cancels to the panes.
        type_line(&mut app, ":");
        assert_eq!(app.overlay, Overlay::Prompt(String::new()));
        app.handle_key(key(KeyCode::Esc));
        assert_eq!(app.overlay, Overlay::None);
        assert!(!app.should_quit);
    }

    #[test]
    fn doc_viewer_scrolls_with_keys_and_wheel_and_clamps() {
        let mut app = app();
        type_line(&mut app, ":readme");
        app.handle_key(key(KeyCode::Enter));
        let scroll_of = |app: &App| {
            let Overlay::Doc(view) = &app.overlay else {
                panic!("expected an open viewer");
            };
            view.scroll
        };

        app.handle_key(key(KeyCode::Up));
        assert_eq!(scroll_of(&app), 0, "clamped at the top");

        app.handle_key(key(KeyCode::Down));
        assert_eq!(scroll_of(&app), 1);
        app.handle_scroll(3);
        assert_eq!(scroll_of(&app), 4);
        app.handle_key(key(KeyCode::PageDown));
        assert_eq!(scroll_of(&app), 19);
        app.handle_key(key(KeyCode::Home));
        assert_eq!(scroll_of(&app), 0);

        app.handle_key(key(KeyCode::End));
        let bottom = scroll_of(&app);
        assert!(bottom > 0);
        app.handle_key(key(KeyCode::Down));
        assert_eq!(scroll_of(&app), bottom, "clamped at the bottom");

        // The wheel means nothing once the viewer closes.
        app.handle_key(key(KeyCode::Esc));
        app.handle_scroll(3);
        assert_eq!(app.overlay, Overlay::None);
    }

    #[test]
    fn unknown_and_empty_commands_report_via_status() {
        let mut app = app();
        type_line(&mut app, ":frobnicate");
        app.handle_key(key(KeyCode::Enter));
        assert_eq!(app.overlay, Overlay::None);
        assert!(app.status.contains("unknown command: frobnicate"));

        app.status.clear();
        type_line(&mut app, ":");
        app.handle_key(key(KeyCode::Enter));
        assert_eq!(app.overlay, Overlay::None);
        assert!(app.status.is_empty(), "an empty line closes silently");
    }

    #[test]
    fn q_and_quit_commands_quit_the_app() {
        for line in [":q", ":quit"] {
            let mut app = app();
            type_line(&mut app, line);
            app.handle_key(key(KeyCode::Enter));
            assert!(app.should_quit);
        }
    }

    fn deferred_app_with_pool() -> App<'static> {
        let f = fixture();
        let owned = vec![
            OwnedPal {
                species: PalName::new("SheepBall"),
                gender: Gender::Male,
                passives: Vec::new(),
                ivs: IvSpread::default(),
            },
            OwnedPal {
                species: PalName::new("PinkCat"),
                gender: Gender::Female,
                passives: Vec::new(),
                ivs: IvSpread::default(),
            },
        ];
        let mut app = App::new(f.solver, owned, test_store());
        app.defer_searches();
        app
    }

    #[test]
    fn deferred_searches_park_a_request_and_apply_its_outcome() {
        let f = fixture();
        let mut app = deferred_app_with_pool();

        for c in "daedream".chars() {
            app.handle_key(key(KeyCode::Char(c)));
        }
        app.handle_key(key(KeyCode::Enter));
        assert!(app.searching());
        assert!(app.plans.is_empty(), "plans arrive only via apply");
        assert!(app.status.contains("target"), "status not overwritten yet");

        // The spinner advances only while a search is awaited.
        app.on_tick();
        app.on_tick();
        assert_eq!(app.spinner(), Some(2));

        let request = app.take_pending_search().expect("request parked");
        assert!(app.take_pending_search().is_none());
        app.apply_search_outcome(run_search(f.solver, request));
        assert!(!app.searching());
        assert!(!app.plans.is_empty());
        assert!(app.status.contains("plan(s) found"));

        app.on_tick();
        assert_eq!(app.spinner(), None, "idle apps have no spinner");
    }

    #[test]
    fn a_superseded_search_outcome_is_discarded() {
        let f = fixture();
        let mut app = deferred_app_with_pool();

        for c in "daedream".chars() {
            app.handle_key(key(KeyCode::Char(c)));
        }
        app.handle_key(key(KeyCode::Enter));
        let stale = app.take_pending_search().expect("first request");

        // Changing the depth re-asks the question; the parked request
        // is replaced, not queued.
        app.handle_key(key(KeyCode::Right));
        let fresh = app.take_pending_search().expect("superseding request");

        app.apply_search_outcome(run_search(f.solver, stale));
        assert!(app.plans.is_empty(), "stale outcome must not apply");
        assert!(app.searching(), "still waiting for the fresh answer");

        app.apply_search_outcome(run_search(f.solver, fresh));
        assert!(!app.plans.is_empty());
        assert!(!app.searching());
    }

    #[test]
    fn dropping_the_target_cancels_an_in_flight_search() {
        let f = fixture();
        let mut app = deferred_app_with_pool();

        for c in "daedream".chars() {
            app.handle_key(key(KeyCode::Char(c)));
        }
        app.handle_key(key(KeyCode::Enter));
        let in_flight = app.take_pending_search().expect("request dispatched");

        // With no target, the next replan clears instead of searching
        // — and must also invalidate what the worker still computes.
        app.target = None;
        app.handle_key(key(KeyCode::F(2)));
        assert!(!app.searching(), "spinner must not run forever");

        app.apply_search_outcome(run_search(f.solver, in_flight));
        assert!(app.plans.is_empty(), "cancelled outcome must not apply");
    }

    fn app_with_saved_plans(labels: &[&str]) -> App<'static> {
        let f = fixture();
        let mut app = App::new(f.solver, Vec::new(), test_store());
        for label in labels {
            app.saved
                .add(SavedPlan {
                    label: (*label).to_owned(),
                    goal_species: PalName::new("DreamDemon"),
                    goal_passives: Vec::new(),
                    goal_progenitors: Vec::new(),
                    goal_iv_thresholds: IvThresholds::default(),
                    expected_eggs: 1.0,
                    steps: 1,
                    root: pal_solver::search::PlanNode::Wild(PalName::new("SheepBall")),
                })
                .unwrap();
        }
        app
    }

    #[test]
    fn x_in_the_library_deletes_the_selected_plan() {
        let mut app = app_with_saved_plans(&["one", "two"]);
        app.handle_key(key(KeyCode::F(9)));

        app.handle_key(key(KeyCode::Char('x')));
        assert_eq!(app.saved.plans.len(), 1);
        assert!(app.status.contains("deleted"));

        app.handle_key(key(KeyCode::Char('x')));
        assert!(app.saved.plans.is_empty());

        app.handle_key(key(KeyCode::Char('x')));
        assert!(app.status.contains("nothing to delete"));
    }

    #[test]
    fn x_types_into_filters_and_stays_inert_in_live_results() {
        let mut app = app_with_saved_plans(&["one"]);

        // In a filter pane, x is just a letter.
        app.handle_key(key(KeyCode::Char('x')));
        assert_eq!(app.species_filter, "x");
        assert_eq!(app.saved.plans.len(), 1);

        // In the live Plans pane it deletes nothing.
        app.focus = Pane::Results;
        app.handle_key(key(KeyCode::Char('x')));
        assert_eq!(app.saved.plans.len(), 1);
    }

    #[test]
    fn iv_keys_fire_from_the_library_as_before() {
        let mut app = app_with_saved_plans(&["one"]);
        app.handle_key(key(KeyCode::F(9)));

        // d adjusts the Defense floor and, because the question
        // changed, the view returns to live plans — the pre-x
        // behavior, restored.
        app.handle_key(key(KeyCode::Char('d')));
        assert_eq!(
            app.iv_thresholds.defense,
            Some(IvValue::try_from(10).unwrap())
        );
        assert!(!app.viewing_saved);
        assert_eq!(app.saved.plans.len(), 1, "the saved plan is untouched");
    }

    #[test]
    fn clear_command_cancels_an_in_flight_search() {
        let f = fixture();
        let mut app = deferred_app_with_pool();

        for c in "daedream".chars() {
            app.handle_key(key(KeyCode::Char(c)));
        }
        app.handle_key(key(KeyCode::Enter));
        let in_flight = app.take_pending_search().expect("request dispatched");
        assert!(app.searching());

        type_line(&mut app, ":clear");
        app.handle_key(key(KeyCode::Enter));
        assert!(!app.searching(), ":clear must stop the spinner");
        assert!(app.status.contains("cleared"));

        app.apply_search_outcome(run_search(f.solver, in_flight));
        assert!(app.plans.is_empty(), "cleared question must stay cleared");
    }

    #[test]
    fn deferred_replan_of_a_saved_plan_announces_the_comparison() {
        let f = fixture();
        let mut app = deferred_app_with_pool();
        app.saved
            .add(SavedPlan {
                label: "Daedream — 1 step(s), 2.00 eggs".to_owned(),
                goal_species: PalName::new("DreamDemon"),
                goal_passives: Vec::new(),
                goal_progenitors: Vec::new(),
                goal_iv_thresholds: IvThresholds::default(),
                expected_eggs: 2.0,
                steps: 1,
                root: pal_solver::search::PlanNode::Wild(PalName::new("SheepBall")),
            })
            .unwrap();

        app.handle_key(key(KeyCode::F(9)));
        app.handle_key(key(KeyCode::Enter));
        assert!(app.searching());

        let request = app.take_pending_search().expect("replan request");
        app.apply_search_outcome(run_search(f.solver, request));
        assert!(app.status.contains("re-planned"));
        assert!(app.status.contains("saved plan was 2.00"));
        assert_eq!(app.focus, Pane::Results);
    }

    #[test]
    fn w_saves_the_plan_and_o_opens_the_library_idempotently() {
        let f = fixture();
        let owned = vec![
            OwnedPal {
                species: PalName::new("SheepBall"),
                gender: Gender::Male,
                passives: Vec::new(),
                ivs: IvSpread::default(),
            },
            OwnedPal {
                species: PalName::new("PinkCat"),
                gender: Gender::Female,
                passives: Vec::new(),
                ivs: IvSpread::default(),
            },
        ];
        let mut app = App::new(f.solver, owned, test_store());
        type_line(&mut app, "daedream");
        app.handle_key(key(KeyCode::Enter));
        assert!(!app.plans.is_empty());

        type_line(&mut app, ":w");
        app.handle_key(key(KeyCode::Enter));
        assert_eq!(app.saved.plans.len(), 1);
        assert!(app.status.contains("saved"));

        type_line(&mut app, ":o");
        app.handle_key(key(KeyCode::Enter));
        assert!(app.viewing_saved);
        assert_eq!(app.focus, Pane::Results);

        // :o again keeps the library open — it never toggles closed.
        type_line(&mut app, ":o");
        app.handle_key(key(KeyCode::Enter));
        assert!(app.viewing_saved);
        assert!(app.status.contains("already"));

        // :w inside the library explains itself instead of saving.
        type_line(&mut app, ":w");
        app.handle_key(key(KeyCode::Enter));
        assert_eq!(app.saved.plans.len(), 1);
        assert!(app.status.contains("already viewing"));
    }

    #[test]
    fn dd_deletes_in_the_library_and_reports_elsewhere() {
        let f = fixture();
        let mut app = App::new(f.solver, Vec::new(), test_store());
        for label in ["one", "two"] {
            app.saved
                .add(crate::plan_store::SavedPlan {
                    label: (*label).to_owned(),
                    goal_species: PalName::new("DreamDemon"),
                    goal_passives: Vec::new(),
                    goal_progenitors: Vec::new(),
                    goal_iv_thresholds: IvThresholds::default(),
                    expected_eggs: 1.0,
                    steps: 1,
                    root: pal_solver::search::PlanNode::Wild(PalName::new("SheepBall")),
                })
                .unwrap();
        }

        // Outside the library there is no saved plan selected.
        type_line(&mut app, ":dd");
        app.handle_key(key(KeyCode::Enter));
        assert_eq!(app.saved.plans.len(), 2);
        assert!(app.status.contains(":o opens the library"));

        type_line(&mut app, ":o");
        app.handle_key(key(KeyCode::Enter));
        type_line(&mut app, ":dd");
        app.handle_key(key(KeyCode::Enter));
        assert_eq!(app.saved.plans.len(), 1);
        assert!(app.status.contains("deleted"));
    }

    #[test]
    fn clear_resets_the_goal_but_keeps_filters_and_iv_floors() {
        let f = fixture();
        // The pool's sire carries whatever passive Enter will select
        // (row 0 of the unfiltered list), so the passive-constrained
        // search still finds plans.
        let first_passive = App::new(f.solver, Vec::new(), test_store()).passive_rows()[0]
            .name
            .clone();
        let owned = vec![
            OwnedPal {
                species: PalName::new("SheepBall"),
                gender: Gender::Male,
                passives: vec![first_passive],
                ivs: IvSpread::default(),
            },
            OwnedPal {
                species: PalName::new("PinkCat"),
                gender: Gender::Female,
                passives: Vec::new(),
                ivs: IvSpread::default(),
            },
        ];
        let mut app = App::new(f.solver, owned, test_store());
        type_line(&mut app, "daedream");
        app.handle_key(key(KeyCode::Enter));
        app.focus = Pane::Passives;
        app.handle_key(key(KeyCode::Enter));
        app.iv_thresholds.hp = Some(IvValue::try_from(50).unwrap());
        assert!(app.target.is_some());
        assert_eq!(app.selected_passives.len(), 1);
        assert!(!app.plans.is_empty());

        type_line(&mut app, ":clear");
        app.handle_key(key(KeyCode::Enter));
        assert!(app.target.is_none());
        assert!(app.progenitors.is_empty());
        assert!(app.selected_passives.is_empty());
        assert!(app.plans.is_empty());
        assert!(app.status.contains("cleared"));
        assert_eq!(app.species_filter, "daedream", "filters survive :clear");
        assert!(app.iv_thresholds.hp.is_some(), "IV floors survive :clear");

        type_line(&mut app, ":clear");
        app.handle_key(key(KeyCode::Enter));
        assert!(app.status.contains("nothing selected"));
    }

    #[test]
    fn clicks_stop_at_an_open_overlay() {
        let mut app = app();
        type_line(&mut app, ":");
        app.handle_click(
            Click {
                pane: Pane::Passives,
                row: Some(0),
            },
            false,
        );
        assert_eq!(app.focus, Pane::Pals);
        assert!(app.selected_passives.is_empty());
    }
}
