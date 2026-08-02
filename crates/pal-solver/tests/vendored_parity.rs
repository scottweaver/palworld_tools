use std::fs;
use std::path::PathBuf;

use pal_core::db;
use pal_core::model::PalName;
use pal_solver::child::{BreedingPair, ChildIndex};
use pal_solver::passives::PassiveOdds;

fn data(file: &str) -> String {
    let path = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../../data")
        .join(file);
    fs::read_to_string(path).unwrap()
}

fn child_index() -> (pal_core::model::PalDb, ChildIndex) {
    let pal_db = db::parse_pal_db(&data("db.json")).unwrap();
    let breeding = db::parse_breeding_db(&data("breeding.json"), &pal_db).unwrap();
    let index = ChildIndex::build(&breeding).unwrap();
    (pal_db, index)
}

#[test]
fn every_ordered_species_pair_has_a_child() {
    let (pal_db, index) = child_index();
    for male in pal_db.pals() {
        for female in pal_db.pals() {
            let pair = BreedingPair {
                male: male.name.clone(),
                female: female.name.clone(),
            };
            assert!(
                index.child_of(&pair).is_some(),
                "no child for {} × {}",
                male.name,
                female.name
            );
        }
    }
}

#[test]
fn known_combos_match_the_vendored_data() {
    let (_, index) = child_index();
    // (male, female, child) triples taken verbatim from breeding.json.
    let known = [
        ("Alpaca", "Alpaca", "Alpaca"),
        ("PinkCat", "SheepBall", "DreamDemon"),
        ("SheepBall", "PinkCat", "DreamDemon"),
        ("LazyDragon", "ElecCat", "LazyDragon_Electric"),
    ];
    for (male, female, child) in known {
        let pair = BreedingPair {
            male: PalName::new(male),
            female: PalName::new(female),
        };
        assert_eq!(
            index.child_of(&pair),
            Some(&PalName::new(child)),
            "{male} × {female}"
        );
    }
}

#[test]
fn gender_dependent_combo_resolves_per_arrangement() {
    // The one gendered pair in v27: Katress (CatMage) × Wixen (FoxMage).
    let (_, index) = child_index();
    let katress_mother = BreedingPair {
        male: PalName::new("FoxMage"),
        female: PalName::new("CatMage"),
    };
    let wixen_mother = BreedingPair {
        male: PalName::new("CatMage"),
        female: PalName::new("FoxMage"),
    };
    assert_eq!(
        index.child_of(&katress_mother),
        Some(&PalName::new("CatMage_Fire"))
    );
    assert_eq!(
        index.child_of(&wixen_mother),
        Some(&PalName::new("FoxMage_Dark"))
    );
}

#[test]
fn vendored_mechanics_produce_the_expected_tables() {
    let pal_db = db::parse_pal_db(&data("db.json")).unwrap();
    let odds = PassiveOdds::from_mechanics(pal_db.mechanics()).unwrap();
    // The unit-test vectors in passives.rs assume the v27 weight
    // tables; this pins the vendored data to those assumptions.
    let expected = [(2, 2, 2, 0.24), (4, 2, 4, 0.175), (0, 0, 2, 0.2)];
    for (pool, desired, num_final, value) in expected {
        let actual = odds.exact_total_probability(pool, desired, num_final);
        assert!(
            (actual - value).abs() < 1e-9,
            "({pool}, {desired}, {num_final}): expected {value}, got {actual}"
        );
    }
}
