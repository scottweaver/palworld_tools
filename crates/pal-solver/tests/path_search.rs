use std::fs;
use std::path::PathBuf;

use pal_core::db;
use pal_core::model::{BreedingDb, Gender, PalDb, PalName, PassiveName};
use pal_solver::child::ChildIndex;
use pal_solver::passives::PassiveOdds;
use pal_solver::search::{BreedingGoal, OwnedPal, PlanNode, SearchConfig, SearchError, find_paths};
use pal_solver::steps::{MinStepsTable, SpeciesAdjacency};

fn data(file: &str) -> String {
    let path = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../../data")
        .join(file);
    fs::read_to_string(path).unwrap()
}

struct Fixture {
    pal_db: PalDb,
    breeding: BreedingDb,
    index: ChildIndex,
    odds: PassiveOdds,
}

fn fixture() -> Fixture {
    let pal_db = db::parse_pal_db(&data("db.json")).unwrap();
    let breeding = db::parse_breeding_db(&data("breeding.json"), &pal_db).unwrap();
    let index = ChildIndex::build(&breeding).unwrap();
    let odds = PassiveOdds::from_mechanics(pal_db.mechanics()).unwrap();
    Fixture {
        pal_db,
        breeding,
        index,
        odds,
    }
}

fn assert_close(actual: f64, expected: f64) {
    assert!(
        (actual - expected).abs() < 1e-9,
        "expected {expected}, got {actual}"
    );
}

fn owned(species: &str, gender: Gender, passives: &[&str]) -> OwnedPal {
    OwnedPal {
        species: PalName::new(species),
        gender,
        passives: passives.iter().copied().map(PassiveName::new).collect(),
    }
}

fn goal(species: &str, passives: &[&str]) -> BreedingGoal {
    BreedingGoal {
        species: PalName::new(species),
        passives: passives.iter().copied().map(PassiveName::new).collect(),
        progenitors: Vec::new(),
    }
}

fn goal_from(species: &str, progenitors: &[&str]) -> BreedingGoal {
    BreedingGoal {
        species: PalName::new(species),
        passives: Vec::new(),
        progenitors: progenitors.iter().copied().map(PalName::new).collect(),
    }
}

fn progenitor_leaves(node: &PlanNode, out: &mut Vec<PalName>) {
    match node {
        PlanNode::Progenitor(species) => out.push(species.clone()),
        PlanNode::Owned(_) | PlanNode::Wild(_) => {}
        PlanNode::Bred(bred) => {
            progenitor_leaves(&bred.male, out);
            progenitor_leaves(&bred.female, out);
        }
    }
}

const CONFIG: SearchConfig = SearchConfig {
    max_breeding_steps: 3,
    max_results: 5,
    allow_wild_pals: false,
};

const WILD_CONFIG: SearchConfig = SearchConfig {
    allow_wild_pals: true,
    ..CONFIG
};

#[test]
fn min_steps_matrix_matches_vendored_table() {
    let f = fixture();
    let table = MinStepsTable::compute(&SpeciesAdjacency::build(&f.pal_db, &f.index));
    for from in f.pal_db.pals() {
        for to in f.pal_db.pals() {
            assert_eq!(
                table.steps_between(&from.name, &to.name),
                f.breeding.min_steps(&from.name, &to.name),
                "{} -> {}",
                from.name,
                to.name
            );
        }
    }
}

#[test]
fn owned_pal_already_satisfying_the_goal_is_a_zero_step_plan() {
    let f = fixture();
    let pool = [owned("DreamDemon", Gender::Female, &["Swift"])];
    let plans = find_paths(
        &f.pal_db,
        &f.index,
        &f.odds,
        &pool,
        &goal("DreamDemon", &["Swift"]),
        &CONFIG,
    )
    .unwrap();

    let best = &plans[0];
    assert_eq!(best.steps, 0);
    assert_close(best.expected_eggs, 0.0);
    assert!(
        matches!(&best.root, PlanNode::Owned(pal) if pal.species == PalName::new("DreamDemon"))
    );
}

#[test]
fn one_step_plan_without_passives_costs_one_egg() {
    let f = fixture();
    let pool = [
        owned("SheepBall", Gender::Male, &[]),
        owned("PinkCat", Gender::Female, &[]),
    ];
    let plans = find_paths(
        &f.pal_db,
        &f.index,
        &f.odds,
        &pool,
        &goal("DreamDemon", &[]),
        &CONFIG,
    )
    .unwrap();

    let best = &plans[0];
    assert_eq!(best.steps, 1);
    assert_close(best.expected_eggs, 1.0);
    let PlanNode::Bred(node) = &best.root else {
        panic!("expected a bred root, got {:?}", best.root);
    };
    assert_eq!(node.species, PalName::new("DreamDemon"));
    assert_eq!(*node.male.species(), PalName::new("SheepBall"));
    assert_eq!(*node.female.species(), PalName::new("PinkCat"));
}

#[test]
fn one_step_plan_with_two_desired_passives_costs_the_inverse_probability() {
    let f = fixture();
    let pool = [
        owned("SheepBall", Gender::Male, &["Swift"]),
        owned("PinkCat", Gender::Female, &["Brave"]),
    ];
    let plans = find_paths(
        &f.pal_db,
        &f.index,
        &f.odds,
        &pool,
        &goal("DreamDemon", &["Swift", "Brave"]),
        &CONFIG,
    )
    .unwrap();

    let best = &plans[0];
    assert_eq!(best.steps, 1);
    // Pool of 2 with both desired: P = 0.6 (see passives.rs vectors).
    assert_close(best.expected_eggs, 1.0 / 0.6);
    let PlanNode::Bred(node) = &best.root else {
        panic!("expected a bred root, got {:?}", best.root);
    };
    assert_eq!(
        node.carried_passives,
        vec![PassiveName::new("Swift"), PassiveName::new("Brave")]
    );
}

#[test]
fn two_step_plan_pays_for_the_intermediate_gender_reroll() {
    let f = fixture();
    // Lamball(M) × Cattiva(F) -> Daedream; Lamball(M) × Daedream(F)
    // -> Fuack (BluePlatypus). The intermediate Daedream must come
    // out female, so its expected cost is 1 / P(female).
    let pool = [
        owned("SheepBall", Gender::Male, &[]),
        owned("PinkCat", Gender::Female, &[]),
    ];
    let plans = find_paths(
        &f.pal_db,
        &f.index,
        &f.odds,
        &pool,
        &goal("BluePlatypus", &[]),
        &CONFIG,
    )
    .unwrap();

    let best = &plans[0];
    assert_eq!(best.steps, 2);
    let daedream_female_p = f
        .pal_db
        .pal(&PalName::new("DreamDemon"))
        .unwrap()
        .gender_probability
        .of(Gender::Female);
    assert_close(best.expected_eggs, 1.0 / daedream_female_p + 1.0);

    let PlanNode::Bred(root) = &best.root else {
        panic!("expected a bred root, got {:?}", best.root);
    };
    assert_eq!(root.species, PalName::new("BluePlatypus"));
    assert_eq!(*root.male.species(), PalName::new("SheepBall"));
    assert_eq!(*root.female.species(), PalName::new("DreamDemon"));
    assert!(matches!(&root.female, PlanNode::Bred(_)));
}

#[test]
fn results_are_ranked_by_expected_eggs() {
    let f = fixture();
    let pool = [
        owned("SheepBall", Gender::Male, &[]),
        owned("SheepBall", Gender::Female, &[]),
        owned("PinkCat", Gender::Female, &[]),
    ];
    let plans = find_paths(
        &f.pal_db,
        &f.index,
        &f.odds,
        &pool,
        &goal("DreamDemon", &[]),
        &CONFIG,
    )
    .unwrap();

    assert!(!plans.is_empty());
    for window in plans.windows(2) {
        assert!(window[0].expected_eggs <= window[1].expected_eggs);
    }
}

#[test]
fn progenitor_anubis_reaches_knocklem_in_one_step() {
    let f = fixture();
    // Anubis × Aegidron -> Knocklem (WingGolem) per the vendored data.
    let plans = find_paths(
        &f.pal_db,
        &f.index,
        &f.odds,
        &[],
        &goal_from("WingGolem", &["Anubis"]),
        &WILD_CONFIG,
    )
    .unwrap();

    assert!(!plans.is_empty());
    let best = &plans[0];
    assert_eq!(best.steps, 1);
    for plan in &plans {
        let mut anchors = Vec::new();
        progenitor_leaves(&plan.root, &mut anchors);
        assert!(anchors.contains(&PalName::new("Anubis")), "unanchored plan");
        assert!(plan.steps > 0, "free catch plan crowded in");
    }
}

#[test]
fn progenitor_azurmane_reaches_knocklem() {
    let f = fixture();
    // The user-expected route is Azurmane × Astegon -> Anubis, then
    // Anubis × Aegidron -> Knocklem; the data also holds direct
    // one-step partners, which must rank first.
    let plans = find_paths(
        &f.pal_db,
        &f.index,
        &f.odds,
        &[],
        &goal_from("WingGolem", &["BlueThunderHorse"]),
        &WILD_CONFIG,
    )
    .unwrap();

    assert!(!plans.is_empty());
    assert_eq!(plans[0].steps, 1);
    let mut anchors = Vec::new();
    progenitor_leaves(&plans[0].root, &mut anchors);
    assert_eq!(anchors, vec![PalName::new("BlueThunderHorse")]);
}

#[test]
fn every_plan_includes_all_required_progenitors() {
    let f = fixture();
    let plans = find_paths(
        &f.pal_db,
        &f.index,
        &f.odds,
        &[],
        &goal_from("BluePlatypus", &["SheepBall", "PinkCat"]),
        &WILD_CONFIG,
    )
    .unwrap();

    assert!(!plans.is_empty());
    for plan in &plans {
        let mut anchors = Vec::new();
        progenitor_leaves(&plan.root, &mut anchors);
        assert!(anchors.contains(&PalName::new("SheepBall")));
        assert!(anchors.contains(&PalName::new("PinkCat")));
    }
}

#[test]
fn progenitor_validation_rejects_bad_inputs() {
    let f = fixture();
    assert_eq!(
        find_paths(
            &f.pal_db,
            &f.index,
            &f.odds,
            &[],
            &goal_from("WingGolem", &["NotAPal"]),
            &WILD_CONFIG
        )
        .unwrap_err(),
        SearchError::UnknownProgenitor(PalName::new("NotAPal"))
    );
    assert_eq!(
        find_paths(
            &f.pal_db,
            &f.index,
            &f.odds,
            &[],
            &goal_from("WingGolem", &["Anubis", "Anubis"]),
            &WILD_CONFIG
        )
        .unwrap_err(),
        SearchError::DuplicateProgenitor(PalName::new("Anubis"))
    );
    let nine: Vec<&str> = vec!["a", "b", "c", "d", "e", "f", "g", "h", "i"];
    assert_eq!(
        find_paths(
            &f.pal_db,
            &f.index,
            &f.odds,
            &[],
            &goal_from("WingGolem", &nine),
            &WILD_CONFIG
        )
        .unwrap_err(),
        SearchError::TooManyProgenitors { count: 9 }
    );
}

#[test]
fn wild_mode_offers_a_catch_plan_for_catchable_goals() {
    let f = fixture();
    let plans = find_paths(
        &f.pal_db,
        &f.index,
        &f.odds,
        &[],
        &goal("SheepBall", &[]),
        &WILD_CONFIG,
    )
    .unwrap();

    let best = &plans[0];
    assert_eq!(best.steps, 0);
    assert_close(best.expected_eggs, 0.0);
    assert_eq!(best.root, PlanNode::Wild(PalName::new("SheepBall")));
}

#[test]
fn wild_partners_bridge_species_gaps() {
    let f = fixture();
    // One male Lamball and nothing else: without wilds nothing can
    // breed; with wilds a caught partner completes the pair.
    let pool = [owned("SheepBall", Gender::Male, &[])];
    let plans = find_paths(
        &f.pal_db,
        &f.index,
        &f.odds,
        &pool,
        &goal("DreamDemon", &[]),
        &WILD_CONFIG,
    )
    .unwrap();

    let bred = plans
        .iter()
        .find(|plan| plan.steps == 1)
        .expect("a one-step plan through a wild partner");
    let PlanNode::Bred(node) = &bred.root else {
        panic!("expected a bred root, got {:?}", bred.root);
    };
    assert!(
        matches!(node.male, PlanNode::Wild(_)) || matches!(node.female, PlanNode::Wild(_)),
        "one parent should be a wild catch"
    );
}

#[test]
fn wild_pals_contribute_no_passives() {
    let f = fixture();
    let plans = find_paths(
        &f.pal_db,
        &f.index,
        &f.odds,
        &[],
        &goal("SheepBall", &["Swift"]),
        &WILD_CONFIG,
    )
    .unwrap();
    assert!(plans.is_empty());
}

#[test]
fn species_without_wild_spawns_are_never_caught() {
    let f = fixture();
    // NightLady (Bellanoir) has no wild levels in the database.
    assert!(
        f.pal_db
            .pal(&PalName::new("NightLady"))
            .unwrap()
            .wild_levels
            .is_none()
    );
    let plans = find_paths(
        &f.pal_db,
        &f.index,
        &f.odds,
        &[],
        &goal("NightLady", &[]),
        &WILD_CONFIG,
    )
    .unwrap();
    assert!(plans.iter().all(|plan| plan.steps > 0));
}

#[test]
fn goal_out_of_reach_returns_no_plans() {
    let f = fixture();
    // A single male pal cannot breed with anything.
    let pool = [owned("SheepBall", Gender::Male, &[])];
    let plans = find_paths(
        &f.pal_db,
        &f.index,
        &f.odds,
        &pool,
        &goal("DreamDemon", &[]),
        &CONFIG,
    )
    .unwrap();
    assert!(plans.is_empty());
}

#[test]
fn invalid_inputs_are_rejected() {
    let f = fixture();
    let pool = [owned("SheepBall", Gender::Male, &[])];

    assert_eq!(
        find_paths(
            &f.pal_db,
            &f.index,
            &f.odds,
            &pool,
            &goal("NotAPal", &[]),
            &CONFIG
        )
        .unwrap_err(),
        SearchError::UnknownGoalSpecies(PalName::new("NotAPal"))
    );
    assert_eq!(
        find_paths(
            &f.pal_db,
            &f.index,
            &f.odds,
            &[owned("NotAPal", Gender::Male, &[])],
            &goal("DreamDemon", &[]),
            &CONFIG
        )
        .unwrap_err(),
        SearchError::UnknownOwnedSpecies(PalName::new("NotAPal"))
    );
    assert_eq!(
        find_paths(
            &f.pal_db,
            &f.index,
            &f.odds,
            &pool,
            &goal("DreamDemon", &["Swift", "Swift"]),
            &CONFIG
        )
        .unwrap_err(),
        SearchError::DuplicateDesired(PassiveName::new("Swift"))
    );
    assert_eq!(
        find_paths(
            &f.pal_db,
            &f.index,
            &f.odds,
            &pool,
            &goal("DreamDemon", &["a", "b", "c", "d", "e"]),
            &CONFIG
        )
        .unwrap_err(),
        SearchError::TooManyDesired { count: 5 }
    );
}
