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
    }
}

const CONFIG: SearchConfig = SearchConfig {
    max_breeding_steps: 3,
    max_results: 5,
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
