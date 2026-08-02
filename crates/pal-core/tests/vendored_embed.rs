#![cfg(feature = "vendored-data")]

use pal_core::model::PalName;
use pal_core::vendored;

#[test]
fn embedded_pair_parses_and_matches_the_on_disk_data() {
    let pal_db = vendored::pal_db().unwrap();
    let breeding = vendored::breeding_db(&pal_db).unwrap();

    assert!(pal_db.pal(&PalName::new("SheepBall")).is_some());
    assert!(!breeding.combos().is_empty());
}
