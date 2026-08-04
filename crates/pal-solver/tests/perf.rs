//! Manual performance scenario — heavy search shapes that the TUI can
//! trigger (deep + multiple progenitors + wild mode). Ignored by
//! default; run with:
//!
//! ```sh
//! cargo test --release -p pal-solver --test perf -- --ignored --nocapture
//! ```

use std::fs;
use std::path::PathBuf;
use std::time::Instant;

use pal_core::db;
use pal_core::model::PalName;
use pal_solver::child::ChildIndex;
use pal_solver::passives::PassiveOdds;
use pal_solver::search::{BreedingGoal, SearchConfig, Solver};

fn data(file: &str) -> String {
    let path = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../../data")
        .join(file);
    fs::read_to_string(path).unwrap()
}

#[test]
#[ignore = "manual perf scenario; run with --ignored --nocapture in release"]
fn deep_progenitor_search_timing() {
    let pal_db = db::parse_pal_db(&data("db.json")).unwrap();
    let breeding = db::parse_breeding_db(&data("breeding.json"), &pal_db).unwrap();
    let index = ChildIndex::build(&breeding).unwrap();
    let odds = PassiveOdds::from_mechanics(pal_db.mechanics()).unwrap();

    let goal = BreedingGoal {
        species: PalName::new("WingGolem"),
        passives: Vec::new(),
        progenitors: vec![
            PalName::new("SheepBall"),
            PalName::new("PinkCat"),
            PalName::new("BlueThunderHorse"),
        ],
    };
    let config = SearchConfig {
        max_breeding_steps: 6,
        max_results: 5,
        allow_wild_pals: true,
    };

    let setup = Instant::now();
    let solver = Solver::new(&pal_db, &index, &odds);
    println!("solver setup: {:?}", setup.elapsed());

    for depth in [6, 12, 20, 32] {
        let config = SearchConfig {
            max_breeding_steps: depth,
            ..config
        };
        let start = Instant::now();
        let plans = solver.find_paths(&[], &goal, &config).unwrap();
        println!(
            "depth {depth}: {:?} — {} plan(s), best {:.2} eggs / {} steps",
            start.elapsed(),
            plans.len(),
            plans[0].expected_eggs,
            plans[0].steps
        );
    }
}
