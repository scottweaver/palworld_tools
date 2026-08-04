//! Manual validation against a real save — real files are private
//! and never committed, so this runs locally only:
//!
//! ```sh
//! PAL_SAVE_PATH=/path/to/Level.sav \
//!   cargo test --release -p pal-save --test real_save -- --ignored --nocapture
//! ```

use std::collections::BTreeMap;
use std::fs;
use std::path::PathBuf;

use pal_save::import::import_pals;
use pal_save::level::read_level_sav;

#[test]
#[ignore = "manual validation against a real save; set PAL_SAVE_PATH"]
fn real_save_imports() {
    let path = std::env::var("PAL_SAVE_PATH").expect("set PAL_SAVE_PATH to a Level.sav");
    let bytes = fs::read(&path).expect("read the save file");

    let save = read_level_sav(&bytes).expect("parse Level.sav");
    println!(
        "characters extracted: {} (malformed entries: {})",
        save.characters.len(),
        save.malformed_entries
    );
    for (issue, count) in &save.decode_issues {
        println!("decode issue ×{count}: {issue}");
    }
    if save.discovered_hints.is_empty() {
        println!("no hints discovered beyond the seed list");
    } else {
        println!("discovered hints (graduate these into SEED_HINTS):");
        for (hint_path, ty) in &save.discovered_hints {
            println!("  {hint_path} = {ty}");
        }
    }

    let db_path = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../data/db.json");
    let pal_db = pal_core::db::parse_pal_db(&fs::read_to_string(db_path).unwrap()).unwrap();
    let report = import_pals(&pal_db, &save.characters);

    let with_passives = report
        .pals
        .iter()
        .filter(|pal| !pal.passives.is_empty())
        .count();
    println!(
        "imported: {} pal(s), {} with passives; players skipped: {}; other skips: {}; unknown passives: {}",
        report.pals.len(),
        with_passives,
        report.skipped_players(),
        report.skipped.len() - report.skipped_players(),
        report.unknown_passives.len(),
    );
    assert!(
        with_passives > 0,
        "a real box always holds pals with passives — extraction is silently empty"
    );

    let mut by_species: BTreeMap<&str, usize> = BTreeMap::new();
    for pal in &report.pals {
        *by_species.entry(pal.species.as_str()).or_default() += 1;
    }
    println!("species histogram ({} distinct):", by_species.len());
    for (species, count) in &by_species {
        println!("  {count:>3} × {species}");
    }
    for reason in report.skipped.iter().take(20) {
        println!("skip: {reason:?}");
    }

    assert!(!report.pals.is_empty(), "a real save should yield pals");
}
