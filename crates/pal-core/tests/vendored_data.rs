use std::fs;
use std::path::PathBuf;

use pal_core::db;
use pal_core::model::{PalName, PassiveName, Surgery};

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
fn vendored_surgery_data_covers_all_three_install_shapes() {
    let pal_db = db::parse_pal_db(&data("db.json")).unwrap();

    let gold_only = pal_db.passive(&PassiveName::new("Deffence_up1")).unwrap();
    assert!(matches!(
        gold_only.surgery,
        Some(Surgery::Gold { cost }) if cost > 0
    ));

    let gold_and_item = pal_db.passive(&PassiveName::new("Deffence_up2")).unwrap();
    let Some(Surgery::GoldAndItem { cost, ref item }) = gold_and_item.surgery else {
        panic!("Deffence_up2 should need gold + item");
    };
    assert!(cost > 0);
    assert!(item.as_str().contains("Deffence_up2"));

    let item_only = pal_db.passive(&PassiveName::new("Deffence_up3")).unwrap();
    assert!(matches!(
        item_only.surgery,
        Some(Surgery::Item { ref item }) if item.as_str().contains("Consumable")
    ));

    let installable = pal_db
        .passives()
        .filter(|skill| skill.surgery.is_some())
        .count();
    let total = pal_db.passives().count();
    assert!(
        installable > 20,
        "expected a real surgery set, got {installable}"
    );
    assert!(
        installable < total / 2,
        "most passives should not be installable ({installable}/{total})"
    );
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
