use std::fs;
use std::path::PathBuf;

use pal_core::db;
use pal_core::model::PalName;

fn data(file: &str) -> String {
    let path = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../../data")
        .join(file);
    fs::read_to_string(path).unwrap()
}

#[test]
fn vendored_pal_db_parses() {
    let pal_db = db::parse_pal_db(&data("db.json")).unwrap();

    assert_eq!(pal_db.version().as_str(), db::SUPPORTED_VERSION);
    assert!(pal_db.pals().count() > 200);
    assert!(!pal_db.mechanics().passive_inheritance_weights.is_empty());

    let lamball = pal_db.pal(&PalName::new("SheepBall")).unwrap();
    assert_eq!(lamball.display_name, "Lamball");
    assert!(lamball.breeding_power > 0);
}

#[test]
fn vendored_breeding_db_parses_and_cross_references() {
    let pal_db = db::parse_pal_db(&data("db.json")).unwrap();
    let breeding = db::parse_breeding_db(&data("breeding.json"), &pal_db).unwrap();

    assert!(breeding.combos().len() > 40_000);

    let alpaca = PalName::new("Alpaca");
    assert!(breeding.combos().iter().any(|combo| {
        combo.parents[0].name == alpaca && combo.parents[1].name == alpaca && combo.child == alpaca
    }));
    assert_eq!(breeding.min_steps(&alpaca, &alpaca), Some(0));
}

#[test]
fn wrong_version_is_rejected() {
    let json = r#"{
        "Version": "v0",
        "Pals": [],
        "PassiveSkills": [],
        "BreedingGenderProbability": {},
        "BreedingMechanics": {
            "IVInheritanceWeights": {},
            "PassiveInheritanceWeights": {},
            "PassiveRandomWeights": {}
        }
    }"#;

    match db::parse_pal_db(json) {
        Err(db::ParseError::UnsupportedVersion { found }) => assert_eq!(found, "v0"),
        other => panic!("expected UnsupportedVersion, got {other:?}"),
    }
}
