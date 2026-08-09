//! The MCP tool surface: thin translation between tool-call JSON and
//! the typed library APIs. Raw strings (species, passives, genders)
//! resolve here at the boundary; everything past resolution works on
//! typed values.

use std::sync::{Arc, Mutex};

use pal_core::model::{Gender, IvValue, Pal, PalDb, PalName, PassiveSkill};
use pal_solver::iv::IvThresholds;
use pal_solver::search::{BreedingGoal, BreedingPlan, OwnedPal, SearchConfig, Solver};
use rmcp::handler::server::router::tool::ToolRouter;
use rmcp::handler::server::wrapper::Parameters;
use rmcp::model::{Implementation, ServerCapabilities, ServerInfo};
use rmcp::{Json, ServerHandler, tool, tool_handler, tool_router};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

use crate::output::{
    OwnedPalJson, PassiveRef, PlanJson, SpeciesRef, owned_pal_json, passive_ref, plan_json,
    species_ref,
};

const DEFAULT_BREEDING_STEPS: usize = 3;
const MAX_BREEDING_STEPS: usize = 24;
const DEFAULT_RESULTS: usize = 5;
const MAX_RESULTS: usize = 10;
const DEFAULT_LIST_LIMIT: usize = 100;
const DEFAULT_COMBO_LIMIT: usize = 50;

const INSTRUCTIONS: &str = "Breeding-path calculator for Palworld, searching over the user's own \
pal pool (imported from their save file or a pals TOML at server launch). The main tool is \
find_breeding_path: it returns plans ranked by expected eggs — the expected number of breeding \
attempts across the whole plan, so lower is better. Species and passive parameters accept \
display names (e.g. \"Lamball\", \"Swift\") or the game's internal names, case-insensitively. \
Call reload_pool after the user saves in-game so searches see their latest boxes.";

/// The pool the searches breed from, swapped wholesale on reload.
struct Pool {
    path: String,
    owned: Vec<OwnedPal>,
    status: String,
}

#[derive(Clone)]
pub struct PalMcpServer {
    solver: &'static Solver<'static>,
    pool: Arc<Mutex<Pool>>,
    tool_router: ToolRouter<Self>,
}

/// Per-stat IV minimums, each 0-100; omitted stats are unconstrained.
#[derive(Clone, Copy, Serialize, Deserialize, JsonSchema)]
pub struct IvMinimums {
    pub hp: Option<u8>,
    pub attack: Option<u8>,
    pub defense: Option<u8>,
}

#[derive(Deserialize, JsonSchema)]
pub struct FindBreedingPathRequest {
    /// Target species to breed (display or internal name).
    pub target: String,
    /// Passives the bred pal must carry (up to 4).
    #[serde(default)]
    pub passives: Vec<String>,
    /// Anchor species: every plan must include each of these as a
    /// leaf. Implicitly enables wild capture.
    #[serde(default)]
    pub progenitors: Vec<String>,
    /// IV minimums the bred pal must meet.
    pub min_ivs: Option<IvMinimums>,
    /// Allow catching wild pals as free plan leaves (default false).
    #[serde(default)]
    pub allow_wild: bool,
    /// Most breeding steps per plan, 1-24 (default 3).
    pub max_breeding_steps: Option<usize>,
    /// Ranked plans to return, 1-10 (default 5).
    pub max_results: Option<usize>,
}

/// The goal as actually searched, echoed so the response is
/// self-describing after name resolution and defaulting.
#[derive(Serialize, JsonSchema)]
pub struct GoalEcho {
    pub species: SpeciesRef,
    pub passives: Vec<PassiveRef>,
    pub progenitors: Vec<SpeciesRef>,
    pub min_ivs: Option<IvMinimums>,
    pub allow_wild: bool,
    pub max_breeding_steps: usize,
    pub max_results: usize,
}

#[derive(Serialize, JsonSchema)]
pub struct FindBreedingPathResponse {
    pub goal: GoalEcho,
    pub plans: Vec<PlanJson>,
    pub summary: String,
}

#[derive(Deserialize, JsonSchema)]
pub struct ListPalsRequest {
    /// Only this species (display or internal name).
    pub species: Option<String>,
    /// Only pals carrying this passive.
    pub passive: Option<String>,
    /// Only this gender: "male" or "female".
    pub gender: Option<String>,
    /// Only pals meeting these IV minimums.
    pub min_ivs: Option<IvMinimums>,
    /// Most entries to return (default 100).
    pub limit: Option<usize>,
}

#[derive(Serialize, JsonSchema)]
pub struct ListPalsResponse {
    /// How the pool was loaded, including import counts.
    pub pool_status: String,
    /// Identical breeding profiles are deduplicated; this counts
    /// distinct profiles, not box pals.
    pub total_matching: usize,
    pub returned: usize,
    pub truncated: bool,
    pub pals: Vec<OwnedPalJson>,
}

#[derive(Serialize, JsonSchema)]
pub struct ReloadPoolResponse {
    pub status: String,
    pub unique_profiles: usize,
}

#[derive(Deserialize, JsonSchema)]
pub struct PalInfoRequest {
    /// Species to describe (display or internal name).
    pub species: String,
}

#[derive(Serialize, JsonSchema)]
pub struct WildLevelsJson {
    pub min: u8,
    pub max: u8,
}

#[derive(Serialize, JsonSchema)]
pub struct PalInfoResponse {
    pub species: SpeciesRef,
    pub dex_number: u16,
    pub dex_variant: bool,
    pub breeding_power: u32,
    /// Probability a bred child of this species is male.
    pub male_probability: f64,
    pub guaranteed_passives: Vec<PassiveRef>,
    /// Wild spawn level range; absent when the species never spawns
    /// in the wild.
    pub wild_levels: Option<WildLevelsJson>,
    /// Distinct breeding profiles of this species in the pool.
    pub owned_profiles: usize,
}

#[derive(Deserialize, JsonSchema)]
pub struct BreedingCombosRequest {
    /// One parent species; constrains the pair.
    pub parent_a: Option<String>,
    /// The other parent species.
    pub parent_b: Option<String>,
    /// The child species the pair must produce.
    pub child: Option<String>,
    /// Most combos to return (default 50).
    pub limit: Option<usize>,
}

#[derive(Serialize, JsonSchema)]
pub struct ComboJson {
    pub parent_male: SpeciesRef,
    pub parent_female: SpeciesRef,
    pub child: SpeciesRef,
    /// True when the pairing produces this child regardless of which
    /// parent is male (the common case; a few combos are
    /// gender-locked).
    pub either_direction: bool,
}

#[derive(Serialize, JsonSchema)]
pub struct BreedingCombosResponse {
    pub total_matching: usize,
    pub returned: usize,
    pub truncated: bool,
    pub combos: Vec<ComboJson>,
}

#[derive(Deserialize, JsonSchema)]
pub struct ListPassivesRequest {
    /// Substring filter on display or internal name.
    pub filter: Option<String>,
}

#[derive(Serialize, JsonSchema)]
pub struct PassiveInfoJson {
    pub name: String,
    pub display_name: String,
    /// Tier: negative ranks are detrimental passives.
    pub rank: i8,
    pub random_inheritance_allowed: bool,
    pub random_inheritance_weight: u32,
}

#[derive(Serialize, JsonSchema)]
pub struct ListPassivesResponse {
    pub total: usize,
    pub passives: Vec<PassiveInfoJson>,
}

#[tool_router(router = tool_router)]
impl PalMcpServer {
    #[must_use]
    pub fn new(
        solver: &'static Solver<'static>,
        pool_path: String,
        owned: Vec<OwnedPal>,
        status: String,
    ) -> Self {
        Self {
            solver,
            pool: Arc::new(Mutex::new(Pool {
                path: pool_path,
                owned,
                status,
            })),
            tool_router: Self::tool_router(),
        }
    }

    fn db(&self) -> &'static PalDb {
        self.solver.pal_db()
    }

    fn locked_pool(&self) -> std::sync::MutexGuard<'_, Pool> {
        self.pool
            .lock()
            .expect("poisoned only if a panicking thread held the pool lock")
    }

    #[tool(
        description = "Find ranked breeding plans producing a target pal (optionally with specific passives, IV minimums, or required progenitor species) from the user's owned pool. Plans are ranked by expected eggs: the expected number of breeding attempts across the whole plan, lower is better."
    )]
    pub async fn find_breeding_path(
        &self,
        Parameters(request): Parameters<FindBreedingPathRequest>,
    ) -> Result<Json<FindBreedingPathResponse>, String> {
        let db = self.db();
        let target = resolve_species(db, &request.target)?;
        let passives = request
            .passives
            .iter()
            .map(|raw| resolve_passive(db, raw).map(|skill| skill.name.clone()))
            .collect::<Result<Vec<_>, String>>()?;
        let progenitors = request
            .progenitors
            .iter()
            .map(|raw| resolve_species(db, raw).map(|pal| pal.name.clone()))
            .collect::<Result<Vec<_>, String>>()?;
        let allow_wild = request.allow_wild || !progenitors.is_empty();
        let config = SearchConfig {
            max_breeding_steps: request
                .max_breeding_steps
                .unwrap_or(DEFAULT_BREEDING_STEPS)
                .clamp(1, MAX_BREEDING_STEPS),
            max_results: request
                .max_results
                .unwrap_or(DEFAULT_RESULTS)
                .clamp(1, MAX_RESULTS),
            allow_wild_pals: allow_wild,
        };
        let goal = BreedingGoal {
            species: target.name.clone(),
            passives,
            progenitors,
            iv_thresholds: thresholds(request.min_ivs)?,
        };
        let echo = GoalEcho {
            species: species_ref(db, &goal.species),
            passives: goal.passives.iter().map(|p| passive_ref(db, p)).collect(),
            progenitors: goal
                .progenitors
                .iter()
                .map(|p| species_ref(db, p))
                .collect(),
            min_ivs: request.min_ivs,
            allow_wild,
            max_breeding_steps: config.max_breeding_steps,
            max_results: config.max_results,
        };

        let owned = self.locked_pool().owned.clone();
        let solver = self.solver;
        let plans = tokio::task::spawn_blocking(move || solver.find_paths(&owned, &goal, &config))
            .await
            .map_err(|error| format!("search task failed: {error}"))?
            .map_err(|error| error.to_string())?;

        let summary = summarize(
            &plans,
            &echo.species.display_name,
            config.max_breeding_steps,
        );
        let plans = plans.iter().map(|plan| plan_json(db, plan)).collect();
        Ok(Json(FindBreedingPathResponse {
            goal: echo,
            plans,
            summary,
        }))
    }

    #[tool(
        description = "List the user's owned pals (deduplicated breeding profiles), optionally filtered by species, passive, gender, or IV minimums."
    )]
    pub fn list_pals(
        &self,
        Parameters(request): Parameters<ListPalsRequest>,
    ) -> Result<Json<ListPalsResponse>, String> {
        let db = self.db();
        let species = request
            .species
            .as_deref()
            .map(|raw| resolve_species(db, raw).map(|pal| pal.name.clone()))
            .transpose()?;
        let passive = request
            .passive
            .as_deref()
            .map(|raw| resolve_passive(db, raw).map(|skill| skill.name.clone()))
            .transpose()?;
        let gender = request.gender.as_deref().map(parse_gender).transpose()?;
        let minimums = thresholds(request.min_ivs)?;

        let pool = self.locked_pool();
        let mut matching: Vec<&OwnedPal> = pool
            .owned
            .iter()
            .filter(|pal| {
                species.as_ref().is_none_or(|wanted| &pal.species == wanted)
                    && passive
                        .as_ref()
                        .is_none_or(|wanted| pal.passives.contains(wanted))
                    && gender.is_none_or(|wanted| pal.gender == wanted)
                    && minimums.met_by(pal.ivs)
            })
            .collect();
        matching.sort_by(|a, b| {
            display_of(db, &a.species)
                .cmp(display_of(db, &b.species))
                .then_with(|| a.passives.len().cmp(&b.passives.len()))
        });

        let total_matching = matching.len();
        let limit = request.limit.unwrap_or(DEFAULT_LIST_LIMIT).max(1);
        let pals: Vec<OwnedPalJson> = matching
            .iter()
            .take(limit)
            .map(|pal| owned_pal_json(db, pal))
            .collect();
        Ok(Json(ListPalsResponse {
            pool_status: pool.status.clone(),
            total_matching,
            returned: pals.len(),
            truncated: total_matching > pals.len(),
            pals,
        }))
    }

    #[tool(
        description = "Re-import the pool from the save/TOML file the server was launched with, picking up new in-game saves without restarting."
    )]
    pub async fn reload_pool(&self) -> Result<Json<ReloadPoolResponse>, String> {
        let db = self.db();
        let path = self.locked_pool().path.clone();
        let loaded = tokio::task::spawn_blocking({
            let path = path.clone();
            move || pal_pool::load(&path, db)
        })
        .await
        .map_err(|error| format!("reload task failed: {error}"))?
        .map_err(|error| error.to_string())?;

        match loaded {
            pal_pool::Loaded::Pool { owned, status } => {
                let unique_profiles = owned.len();
                let mut pool = self.locked_pool();
                pool.owned = owned;
                pool.status.clone_from(&status);
                Ok(Json(ReloadPoolResponse {
                    status,
                    unique_profiles,
                }))
            }
            pal_pool::Loaded::Missing => {
                Err(format!("{path} not found — keeping the current pool"))
            }
        }
    }

    #[tool(
        description = "Describe a species: dex number, breeding power, gender probability, guaranteed passives, wild spawn levels, and how many the user owns."
    )]
    pub fn pal_info(
        &self,
        Parameters(request): Parameters<PalInfoRequest>,
    ) -> Result<Json<PalInfoResponse>, String> {
        let db = self.db();
        let pal = resolve_species(db, &request.species)?;
        let owned_profiles = self
            .locked_pool()
            .owned
            .iter()
            .filter(|owned| owned.species == pal.name)
            .count();
        Ok(Json(PalInfoResponse {
            species: species_ref(db, &pal.name),
            dex_number: pal.id.dex,
            dex_variant: pal.id.variant,
            breeding_power: pal.breeding_power,
            male_probability: pal.gender_probability.of(Gender::Male),
            guaranteed_passives: pal
                .guaranteed_passives
                .iter()
                .map(|passive| passive_ref(db, passive))
                .collect(),
            wild_levels: pal.wild_levels.map(|levels| WildLevelsJson {
                min: levels.min,
                max: levels.max,
            }),
            owned_profiles,
        }))
    }

    #[tool(
        description = "Query breeding combos: which child two parents produce, or which parent pairs produce a given child. Give at least one of parent_a, parent_b, or child."
    )]
    pub fn breeding_combos(
        &self,
        Parameters(request): Parameters<BreedingCombosRequest>,
    ) -> Result<Json<BreedingCombosResponse>, String> {
        let db = self.db();
        let parent_a = request
            .parent_a
            .as_deref()
            .map(|raw| resolve_species(db, raw).map(|pal| pal.name.clone()))
            .transpose()?;
        let parent_b = request
            .parent_b
            .as_deref()
            .map(|raw| resolve_species(db, raw).map(|pal| pal.name.clone()))
            .transpose()?;
        let child = request
            .child
            .as_deref()
            .map(|raw| resolve_species(db, raw).map(|pal| pal.name.clone()))
            .transpose()?;
        if parent_a.is_none() && parent_b.is_none() && child.is_none() {
            return Err("give at least one of parent_a, parent_b, or child".to_owned());
        }

        let index = self.solver.child_index();
        let mut matching: Vec<(&PalName, &PalName, &PalName, bool)> = index
            .pairings()
            .filter_map(|(male, female, produced)| {
                let either_direction = index.child_between(female, male) == Some(produced);
                if either_direction && male.as_str() > female.as_str() {
                    return None;
                }
                let pair_matches = match (&parent_a, &parent_b) {
                    (Some(a), Some(b)) => (male == a && female == b) || (male == b && female == a),
                    (Some(only), None) | (None, Some(only)) => male == only || female == only,
                    (None, None) => true,
                };
                let child_matches = child.as_ref().is_none_or(|wanted| produced == wanted);
                (pair_matches && child_matches).then_some((
                    male,
                    female,
                    produced,
                    either_direction,
                ))
            })
            .collect();
        matching.sort_by(|a, b| {
            display_of(db, a.2)
                .cmp(display_of(db, b.2))
                .then_with(|| display_of(db, a.0).cmp(display_of(db, b.0)))
                .then_with(|| display_of(db, a.1).cmp(display_of(db, b.1)))
        });

        let total_matching = matching.len();
        let limit = request.limit.unwrap_or(DEFAULT_COMBO_LIMIT).max(1);
        let combos: Vec<ComboJson> = matching
            .iter()
            .take(limit)
            .map(|(male, female, produced, either_direction)| ComboJson {
                parent_male: species_ref(db, male),
                parent_female: species_ref(db, female),
                child: species_ref(db, produced),
                either_direction: *either_direction,
            })
            .collect();
        Ok(Json(BreedingCombosResponse {
            total_matching,
            returned: combos.len(),
            truncated: total_matching > combos.len(),
            combos,
        }))
    }

    #[tool(
        description = "List passive skills (name, tier rank, random-inheritance data), optionally filtered by a name substring."
    )]
    pub fn list_passives(
        &self,
        Parameters(request): Parameters<ListPassivesRequest>,
    ) -> Json<ListPassivesResponse> {
        let db = self.db();
        let query = request.filter.map(|filter| filter.to_ascii_lowercase());
        let mut matching: Vec<&PassiveSkill> = db
            .passives()
            .filter(|skill| {
                query.as_deref().is_none_or(|wanted| {
                    skill.display_name.to_ascii_lowercase().contains(wanted)
                        || skill.name.as_str().to_ascii_lowercase().contains(wanted)
                })
            })
            .collect();
        matching.sort_by(|a, b| a.display_name.cmp(&b.display_name));
        let passives: Vec<PassiveInfoJson> = matching
            .iter()
            .map(|skill| PassiveInfoJson {
                name: skill.name.to_string(),
                display_name: skill.display_name.clone(),
                rank: skill.rank,
                random_inheritance_allowed: skill.random_inheritance_allowed,
                random_inheritance_weight: skill.random_inheritance_weight,
            })
            .collect();
        Json(ListPassivesResponse {
            total: passives.len(),
            passives,
        })
    }
}

#[tool_handler(router = self.tool_router)]
impl ServerHandler for PalMcpServer {
    fn get_info(&self) -> ServerInfo {
        let mut info = ServerInfo::default();
        info.server_info = Implementation::new(env!("CARGO_PKG_NAME"), env!("CARGO_PKG_VERSION"));
        info.capabilities = ServerCapabilities::builder().enable_tools().build();
        info.instructions = Some(INSTRUCTIONS.to_owned());
        info
    }
}

fn resolve_species<'db>(db: &'db PalDb, raw: &str) -> Result<&'db Pal, String> {
    db.find_pal(raw).ok_or_else(|| {
        let query = raw.to_ascii_lowercase();
        let suggestions = suggest(
            db.pals()
                .map(|pal| (pal.name.as_str(), pal.display_name.as_str())),
            &query,
        );
        if suggestions.is_empty() {
            format!("unknown species {raw:?}")
        } else {
            format!("unknown species {raw:?} — did you mean {suggestions}?")
        }
    })
}

fn resolve_passive<'db>(db: &'db PalDb, raw: &str) -> Result<&'db PassiveSkill, String> {
    db.find_passive(raw).ok_or_else(|| {
        let query = raw.to_ascii_lowercase();
        let suggestions = suggest(
            db.passives()
                .map(|skill| (skill.name.as_str(), skill.display_name.as_str())),
            &query,
        );
        if suggestions.is_empty() {
            format!("unknown passive {raw:?}")
        } else {
            format!("unknown passive {raw:?} — did you mean {suggestions}?")
        }
    })
}

/// Display names whose internal or display form contains `query`
/// (already lowercased), capped at five, joined for an error message.
fn suggest<'db>(names: impl Iterator<Item = (&'db str, &'db str)>, query: &str) -> String {
    let mut matches: Vec<&str> = names
        .filter(|(internal, display)| {
            internal.to_ascii_lowercase().contains(query)
                || display.to_ascii_lowercase().contains(query)
        })
        .map(|(_, display)| display)
        .collect();
    matches.sort_unstable();
    matches.dedup();
    matches.truncate(5);
    matches.join(", ")
}

fn parse_gender(raw: &str) -> Result<Gender, String> {
    match raw.to_ascii_lowercase().as_str() {
        "male" | "m" => Ok(Gender::Male),
        "female" | "f" => Ok(Gender::Female),
        other => Err(format!("gender must be male or female, got {other:?}")),
    }
}

fn thresholds(min_ivs: Option<IvMinimums>) -> Result<IvThresholds, String> {
    let Some(minimums) = min_ivs else {
        return Ok(IvThresholds::default());
    };
    let convert = |value: Option<u8>, stat: &str| {
        value
            .map(|raw| IvValue::try_from(raw).map_err(|error| format!("{stat}: {error}")))
            .transpose()
    };
    Ok(IvThresholds {
        hp: convert(minimums.hp, "hp")?,
        attack: convert(minimums.attack, "attack")?,
        defense: convert(minimums.defense, "defense")?,
    })
}

fn display_of<'a>(db: &'a PalDb, name: &'a PalName) -> &'a str {
    db.pal(name)
        .map_or_else(|| name.as_str(), |pal| pal.display_name.as_str())
}

fn summarize(plans: &[BreedingPlan], target_display: &str, steps_limit: usize) -> String {
    match plans.first() {
        None => format!(
            "no plans for {target_display} within {steps_limit} breeding step(s) — consider \
             raising max_breeding_steps, enabling allow_wild, or calling reload_pool if the \
             save changed"
        ),
        Some(best) if best.steps == 0 => format!(
            "an owned pal already satisfies the goal ({} plan(s))",
            plans.len()
        ),
        Some(best) => format!(
            "{} plan(s) for {target_display}; best needs {} breeding step(s), ~{:.1} expected eggs",
            plans.len(),
            best.steps,
            best.expected_eggs
        ),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::output::PlanNodeJson;
    use pal_solver::child::ChildIndex;
    use pal_solver::iv::IvOdds;
    use pal_solver::passives::PassiveOdds;
    use std::sync::OnceLock;

    struct Data {
        db: PalDb,
        index: ChildIndex,
        odds: PassiveOdds,
        iv_odds: IvOdds,
    }

    fn solver() -> &'static Solver<'static> {
        static DATA: OnceLock<Data> = OnceLock::new();
        static SOLVER: OnceLock<Solver<'static>> = OnceLock::new();
        let data = DATA.get_or_init(|| {
            let db = pal_core::vendored::pal_db().unwrap();
            let breeding = pal_core::vendored::breeding_db(&db).unwrap();
            let index = ChildIndex::build(&breeding).unwrap();
            let odds = PassiveOdds::from_mechanics(db.mechanics()).unwrap();
            let iv_odds = IvOdds::from_mechanics(db.mechanics()).unwrap();
            Data {
                db,
                index,
                odds,
                iv_odds,
            }
        });
        SOLVER.get_or_init(|| Solver::new(&data.db, &data.index, &data.odds, &data.iv_odds))
    }

    fn server_with_toml(pals_toml: &str) -> PalMcpServer {
        let solver = solver();
        let owned = pal_pool::pals_file::parse(pals_toml, solver.pal_db()).unwrap();
        PalMcpServer::new(
            solver,
            "unused.toml".to_owned(),
            owned,
            "test pool".to_owned(),
        )
    }

    fn params<T>(value: T) -> Parameters<T> {
        Parameters(value)
    }

    #[tokio::test]
    async fn an_owned_pal_satisfies_its_own_goal_with_zero_steps() {
        let server = server_with_toml(
            "[[pals]]\nspecies = \"Lamball\"\ngender = \"male\"\npassives = [\"Swift\"]\n",
        );
        let Json(response) = server
            .find_breeding_path(params(FindBreedingPathRequest {
                target: "lamball".to_owned(),
                passives: vec!["swift".to_owned()],
                progenitors: Vec::new(),
                min_ivs: None,
                allow_wild: false,
                max_breeding_steps: None,
                max_results: None,
            }))
            .await
            .unwrap();

        assert_eq!(response.goal.species.name, "SheepBall");
        assert_eq!(response.goal.species.display_name, "Lamball");
        assert!(!response.plans.is_empty());
        let best = &response.plans[0];
        assert_eq!(best.steps, 0);
        assert!(best.expected_eggs.abs() < 1e-9);
        assert!(matches!(best.root, PlanNodeJson::Owned { .. }));
        assert!(response.summary.contains("already satisfies"));
    }

    #[tokio::test]
    async fn unknown_names_error_with_suggestions() {
        let server = server_with_toml("");
        let error = server
            .find_breeding_path(params(FindBreedingPathRequest {
                target: "lambal".to_owned(),
                passives: Vec::new(),
                progenitors: Vec::new(),
                min_ivs: None,
                allow_wild: false,
                max_breeding_steps: None,
                max_results: None,
            }))
            .await
            .err()
            .expect("unknown species should fail");
        assert!(error.contains("unknown species"));
        assert!(error.contains("Lamball"));

        let error = server
            .find_breeding_path(params(FindBreedingPathRequest {
                target: "Lamball".to_owned(),
                passives: vec!["swif".to_owned()],
                progenitors: Vec::new(),
                min_ivs: None,
                allow_wild: false,
                max_breeding_steps: None,
                max_results: None,
            }))
            .await
            .err()
            .expect("unknown passive should fail");
        assert!(error.contains("unknown passive"));
        assert!(error.contains("Swift"));
    }

    #[tokio::test]
    async fn out_of_range_iv_minimums_are_rejected() {
        let server = server_with_toml("");
        let error = server
            .find_breeding_path(params(FindBreedingPathRequest {
                target: "Lamball".to_owned(),
                passives: Vec::new(),
                progenitors: Vec::new(),
                min_ivs: Some(IvMinimums {
                    hp: Some(101),
                    attack: None,
                    defense: None,
                }),
                allow_wild: false,
                max_breeding_steps: None,
                max_results: None,
            }))
            .await
            .err()
            .expect("out-of-range IV should fail");
        assert!(error.contains("hp"));
        assert!(error.contains("out of range"));
    }

    #[test]
    fn list_pals_filters_and_reports_truncation() {
        let server = server_with_toml(
            "[[pals]]\nspecies = \"Lamball\"\ngender = \"male\"\npassives = [\"Swift\"]\n\n\
             [[pals]]\nspecies = \"Cattiva\"\ngender = \"female\"\n",
        );

        let Json(all) = server
            .list_pals(params(ListPalsRequest {
                species: None,
                passive: None,
                gender: None,
                min_ivs: None,
                limit: None,
            }))
            .unwrap();
        assert_eq!(all.total_matching, 2);
        assert!(!all.truncated);
        assert_eq!(all.pool_status, "test pool");

        let Json(swift_only) = server
            .list_pals(params(ListPalsRequest {
                species: None,
                passive: Some("swift".to_owned()),
                gender: None,
                min_ivs: None,
                limit: None,
            }))
            .unwrap();
        assert_eq!(swift_only.total_matching, 1);
        assert_eq!(swift_only.pals[0].species.display_name, "Lamball");

        let Json(capped) = server
            .list_pals(params(ListPalsRequest {
                species: None,
                passive: None,
                gender: None,
                min_ivs: None,
                limit: Some(1),
            }))
            .unwrap();
        assert_eq!(capped.total_matching, 2);
        assert_eq!(capped.returned, 1);
        assert!(capped.truncated);
    }

    #[tokio::test]
    async fn reload_pool_reimports_from_disk() {
        let path = std::env::temp_dir().join(format!("pal-mcp-reload-{}.toml", std::process::id()));
        std::fs::write(
            &path,
            "[[pals]]\nspecies = \"Lamball\"\ngender = \"male\"\n",
        )
        .unwrap();
        let server = PalMcpServer::new(
            solver(),
            path.to_str().unwrap().to_owned(),
            Vec::new(),
            "empty".to_owned(),
        );

        let Json(reloaded) = server.reload_pool().await.unwrap();
        assert_eq!(reloaded.unique_profiles, 1);

        std::fs::write(
            &path,
            "[[pals]]\nspecies = \"Lamball\"\ngender = \"male\"\n\n\
             [[pals]]\nspecies = \"Cattiva\"\ngender = \"female\"\n",
        )
        .unwrap();
        let Json(reloaded) = server.reload_pool().await.unwrap();
        assert_eq!(reloaded.unique_profiles, 2);

        let Json(listed) = server
            .list_pals(params(ListPalsRequest {
                species: None,
                passive: None,
                gender: None,
                min_ivs: None,
                limit: None,
            }))
            .unwrap();
        assert_eq!(listed.total_matching, 2);
        std::fs::remove_file(&path).ok();
    }

    #[test]
    fn pal_info_describes_a_species_and_counts_owned() {
        let server = server_with_toml("[[pals]]\nspecies = \"Lamball\"\ngender = \"male\"\n");
        let Json(info) = server
            .pal_info(params(PalInfoRequest {
                species: "lamball".to_owned(),
            }))
            .unwrap();
        assert_eq!(info.species.name, "SheepBall");
        assert_eq!(info.species.display_name, "Lamball");
        assert!(info.wild_levels.is_some());
        assert_eq!(info.owned_profiles, 1);
        assert!(info.male_probability > 0.0 && info.male_probability < 1.0);
    }

    #[test]
    fn breeding_combos_between_two_parents_matches_the_child_index() {
        let server = server_with_toml("");
        let solver = solver();
        let db = solver.pal_db();
        let lamball = db.find_pal("Lamball").unwrap().name.clone();
        let cattiva = db.find_pal("Cattiva").unwrap().name.clone();
        let expected = solver
            .child_index()
            .child_between(&lamball, &cattiva)
            .unwrap();

        let Json(response) = server
            .breeding_combos(params(BreedingCombosRequest {
                parent_a: Some("Lamball".to_owned()),
                parent_b: Some("Cattiva".to_owned()),
                child: None,
                limit: None,
            }))
            .unwrap();
        assert_eq!(response.total_matching, 1);
        let combo = &response.combos[0];
        assert_eq!(combo.child.name, expected.as_str());
        assert!(combo.either_direction);
    }

    #[test]
    fn breeding_combos_for_a_child_lists_pairs_and_requires_a_filter() {
        let server = server_with_toml("");
        let Json(response) = server
            .breeding_combos(params(BreedingCombosRequest {
                parent_a: None,
                parent_b: None,
                child: Some("Lamball".to_owned()),
                limit: Some(10),
            }))
            .unwrap();
        assert!(response.total_matching > 0);
        assert_eq!(response.returned, response.combos.len());
        assert!(
            response
                .combos
                .iter()
                .all(|combo| combo.child.name == "SheepBall")
        );

        let error = server
            .breeding_combos(params(BreedingCombosRequest {
                parent_a: None,
                parent_b: None,
                child: None,
                limit: None,
            }))
            .err()
            .expect("filterless combos query should fail");
        assert!(error.contains("at least one"));
    }

    #[test]
    fn list_passives_filters_by_substring() {
        let server = server_with_toml("");
        let Json(response) = server.list_passives(params(ListPassivesRequest {
            filter: Some("swift".to_owned()),
        }));
        assert!(response.total >= 1);
        assert!(
            response
                .passives
                .iter()
                .any(|passive| passive.display_name == "Swift")
        );
    }

    #[test]
    fn the_tool_router_exposes_the_full_surface() {
        let server = server_with_toml("");
        let mut names: Vec<String> = server
            .tool_router
            .list_all()
            .into_iter()
            .map(|tool| tool.name.to_string())
            .collect();
        names.sort();
        assert_eq!(
            names,
            [
                "breeding_combos",
                "find_breeding_path",
                "list_pals",
                "list_passives",
                "pal_info",
                "reload_pool",
            ]
        );
    }

    #[test]
    fn the_server_identifies_as_pal_mcp() {
        let info = server_with_toml("").get_info();
        assert_eq!(info.server_info.name, "pal-mcp");
        assert_eq!(info.server_info.version, env!("CARGO_PKG_VERSION"));
        assert!(info.instructions.is_some());
    }
}
