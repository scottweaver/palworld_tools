//! Diagnostic reproduction of a search against a real save — local
//! only:
//!
//! ```sh
//! PAL_SAVE_PATH=... PROBE_TARGET=LanternButler \
//!   cargo test --release -p pal-tui --test repro -- --ignored --nocapture
//! ```

use pal_core::model::PalName;
use pal_solver::child::ChildIndex;
use pal_solver::passives::PassiveOdds;
use pal_solver::search::{BreedingGoal, OwnedPal, PlanNode, SearchConfig, Solver};

fn describe(node: &PlanNode, depth: usize, out: &mut Vec<String>) {
    let indent = "  ".repeat(depth);
    match node {
        PlanNode::Owned(pal) => out.push(format!(
            "{indent}own {} {:?} {:?}",
            pal.species, pal.gender, pal.passives
        )),
        PlanNode::Wild(s) => out.push(format!("{indent}wild {s}")),
        PlanNode::Progenitor(s) => out.push(format!("{indent}progenitor {s}")),
        PlanNode::Bred(b) => {
            out.push(format!("{indent}breed {}", b.species));
            describe(&b.male, depth + 1, out);
            describe(&b.female, depth + 1, out);
        }
    }
}

#[test]
#[ignore = "diagnostic against a real save; set PAL_SAVE_PATH and PROBE_TARGET"]
fn reproduce_search_from_save() {
    let save_path = std::env::var("PAL_SAVE_PATH").expect("set PAL_SAVE_PATH");
    let target = std::env::var("PROBE_TARGET").expect("set PROBE_TARGET (internal name)");

    let db = pal_core::vendored::pal_db().unwrap();
    let breeding = pal_core::vendored::breeding_db(&db).unwrap();
    let index = ChildIndex::build(&breeding).unwrap();
    let odds = PassiveOdds::from_mechanics(db.mechanics()).unwrap();
    let solver = Solver::new(&db, &index, &odds);

    let bytes = std::fs::read(&save_path).unwrap();
    let save = pal_save::level::read_level_sav(&bytes).unwrap();
    let report = pal_save::import::import_pals(&db, &save.characters);
    let mut owned: Vec<OwnedPal> = Vec::new();
    for pal in report.pals {
        let candidate = OwnedPal {
            species: pal.species,
            gender: pal.gender,
            passives: pal.passives,
        };
        if !owned.contains(&candidate) {
            owned.push(candidate);
        }
    }
    println!("pool: {} unique profiles", owned.len());
    for species in ["Anubis", "Manticore"] {
        let name = PalName::new(species);
        let genders: Vec<_> = owned
            .iter()
            .filter(|pal| pal.species == name)
            .map(|pal| (pal.gender, pal.passives.len()))
            .collect();
        println!("owned {species}: {genders:?}");
    }

    let goal = BreedingGoal {
        species: PalName::new(target.as_str()),
        passives: Vec::new(),
        progenitors: Vec::new(),
    };
    let depth = std::env::var("PROBE_DEPTH")
        .ok()
        .and_then(|value| value.parse().ok())
        .unwrap_or(3);
    let config = SearchConfig {
        max_breeding_steps: depth,
        max_results: 5,
        allow_wild_pals: std::env::var("PROBE_WILD").is_ok(),
    };
    let start = std::time::Instant::now();
    let plans = solver.find_paths(&owned, &goal, &config).unwrap();
    println!(
        "{} plan(s) at depth {depth} in {:?}",
        plans.len(),
        start.elapsed()
    );
    for (position, plan) in plans.iter().enumerate() {
        println!(
            "plan {}: {:.2} eggs, {} step(s)",
            position + 1,
            plan.expected_eggs,
            plan.steps
        );
        let mut lines = Vec::new();
        describe(&plan.root, 1, &mut lines);
        for line in lines {
            println!("{line}");
        }
    }
}
