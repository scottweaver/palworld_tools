//! Diagnostic reproduction of a search against a real save — local
//! only:
//!
//! ```sh
//! PAL_SAVE_PATH=... PROBE_TARGET=LanternButler \
//!   cargo test --release -p pal-tui --test repro -- --ignored --nocapture
//! ```

use pal_core::model::{IvSpread, PalName};
use pal_solver::child::ChildIndex;
use pal_solver::iv::{IvOdds, IvThresholds};
use pal_solver::passives::PassiveOdds;
use pal_solver::search::{BreedingGoal, OwnedPal, PlanNode, SearchConfig, Solver};

fn probe_iv(var: &str) -> Option<pal_core::model::IvValue> {
    let value = std::env::var(var).ok()?.parse::<u8>().expect("0..=100");
    Some(pal_core::model::IvValue::try_from(value).expect("0..=100"))
}

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
#[expect(
    clippy::too_many_lines,
    reason = "linear diagnostic script; splitting would hurt readability"
)]
fn reproduce_search_from_save() {
    let save_path = std::env::var("PAL_SAVE_PATH").expect("set PAL_SAVE_PATH");
    let target = std::env::var("PROBE_TARGET").expect("set PROBE_TARGET (internal name)");

    let db = pal_core::vendored::pal_db().unwrap();
    let breeding = pal_core::vendored::breeding_db(&db).unwrap();
    let index = ChildIndex::build(&breeding).unwrap();
    let odds = PassiveOdds::from_mechanics(db.mechanics()).unwrap();
    let iv_odds = IvOdds::from_mechanics(db.mechanics()).unwrap();
    let solver = Solver::new(&db, &index, &odds, &iv_odds);

    let bytes = std::fs::read(&save_path).unwrap();
    let save = pal_save::level::read_level_sav(&bytes).unwrap();
    let report = pal_save::import::import_pals(&db, &save.characters);
    let mut owned: Vec<OwnedPal> = Vec::new();
    for pal in report.pals {
        let candidate = OwnedPal {
            species: pal.species,
            gender: pal.gender,
            passives: pal.passives,
            ivs: pal.ivs,
        };
        if !owned.contains(&candidate) {
            owned.push(candidate);
        }
    }
    // Hypothetical pals: PROBE_EXTRA="Species,gender,Passive|Passive".
    if let Ok(spec) = std::env::var("PROBE_EXTRA") {
        let parts: Vec<&str> = spec.split(',').collect();
        let species = db.find_pal(parts[0]).expect("extra species").name.clone();
        let gender = if parts[1].eq_ignore_ascii_case("male") {
            pal_core::model::Gender::Male
        } else {
            pal_core::model::Gender::Female
        };
        let passives = parts[2]
            .split('|')
            .map(|name| db.find_passive(name).expect("extra passive").name.clone())
            .collect();
        let extra = OwnedPal {
            species,
            gender,
            passives,
            ivs: IvSpread::default(),
        };
        println!("injected hypothetical: {extra:?}");
        owned.push(extra);
    }
    println!("pool: {} unique profiles", owned.len());

    let desired: Vec<_> = std::env::var("PROBE_PASSIVES")
        .unwrap_or_default()
        .split(',')
        .filter(|name| !name.is_empty())
        .map(|name| {
            let skill = db
                .find_passive(name.trim())
                .unwrap_or_else(|| panic!("unknown passive {name:?}"));
            println!(
                "desired passive: {} = {} (rank {})",
                name.trim(),
                skill.name,
                skill.rank
            );
            skill.name.clone()
        })
        .collect();

    // Every owned pal carrying all desired passives.
    for pal in &owned {
        if !desired.is_empty() && desired.iter().all(|want| pal.passives.contains(want)) {
            println!(
                "carrier: {} {:?} passives {:?}",
                pal.species, pal.gender, pal.passives
            );
        }
    }

    let goal = BreedingGoal {
        species: PalName::new(target.as_str()),
        passives: desired.clone(),
        progenitors: Vec::new(),
        iv_thresholds: IvThresholds {
            hp: probe_iv("PROBE_IV_HP"),
            attack: probe_iv("PROBE_IV_ATTACK"),
            defense: probe_iv("PROBE_IV_DEFENSE"),
        },
    };

    // Independent oracle: brute-force owned male x female pairs whose
    // child is the target and whose combined passives cover the
    // desired set.
    let mut direct_pairs = 0;
    for male in owned
        .iter()
        .filter(|p| p.gender == pal_core::model::Gender::Male)
    {
        for female in owned
            .iter()
            .filter(|p| p.gender == pal_core::model::Gender::Female)
        {
            let child = index.child_between(&male.species, &female.species);
            if child == Some(&goal.species)
                && desired
                    .iter()
                    .all(|want| male.passives.contains(want) || female.passives.contains(want))
            {
                if direct_pairs < 5 {
                    println!(
                        "oracle 1-step pair: {} ♂ × {} ♀",
                        male.species, female.species
                    );
                }
                direct_pairs += 1;
            }
        }
    }
    println!("oracle: {direct_pairs} direct pair(s) satisfy species+passives");
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
